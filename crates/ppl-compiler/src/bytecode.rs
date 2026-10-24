//! Bytecode for the PPL stack machine.

use ppl_math::{Complex, HpInt, Real};
use ppl_syntax::Span;
use ppl_syntax::ast::BinOp;

use crate::builtins::Builtin;

#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Real(Real),
    Int(HpInt),
    Complex(Complex),
    Str(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Const(u32),
    Zero,
    LoadLocal(u16),
    StoreLocal(u16),
    LoadGlobal(u32),
    StoreGlobal(u32),
    Pop,
    Dup,
    Bin(BinOp),
    Neg,
    Not,
    Sqrt,
    Factorial,
    Square,
    Cube,
    Jump(u32),
    /// Pops a condition; jumps when it is false (zero).
    JumpIfFalse(u32),
    /// Calls user function `func` with `argc` arguments on the stack.
    Call {
        func: u32,
        argc: u8,
    },
    Builtin {
        id: Builtin,
        argc: u8,
    },
    /// Like `Builtin`, but pushes `refs` updated values for variable
    /// arguments after the result; the compiler stores them back.
    BuiltinRef {
        id: Builtin,
        argc: u8,
        refs: u8,
    },
    /// Unsupported command called by name (CAS functions, app commands…).
    Unsupported {
        name: u32,
        argc: u8,
    },
    Return,
    MakeList(u32),
    MakeVector(u32),
    MakeMatrix {
        rows: u32,
        cols: u32,
    },
    MakeComplex,
    /// Stack: base, i1..in → element.
    Index(u8),
    /// Stack: i1..in, value → stores into the local's element.
    StoreIndexLocal {
        slot: u16,
        n: u8,
    },
    StoreIndexGlobal {
        slot: u32,
        n: u8,
    },
    /// Stack: name, value → writes a calculator note (empty value deletes
    /// it); pushes the value back.
    StoreNote,
    /// Stack: name, value → writes a calculator program (empty value
    /// deletes it); pushes the value back.
    StoreProgram,
    /// Registers an IFERR handler at the given address.
    TryBegin(u32),
    TryEnd,
    /// Loop guard: stack: var, limit → pushes 1 to continue, 0 to stop.
    ForTest {
        down: bool,
    },
    /// Stack: value → appends to the list held in a local (MAKELIST).
    AppendLocal(u16),
    Kill,
    /// Marks a statement boundary for the debugger (statement start).
    Stmt,
}

/// One compiled function.
#[derive(Clone, Debug)]
pub struct Chunk {
    pub name: String,
    pub params: u8,
    pub locals: u16,
    pub local_names: Vec<String>,
    pub code: Vec<Op>,
    /// Source span for each instruction (same length as `code`).
    pub spans: Vec<Span>,
    pub exported: bool,
    pub span: Span,
}

impl Chunk {
    pub fn emit(&mut self, op: Op, span: Span) -> usize {
        self.code.push(op);
        self.spans.push(span);
        self.code.len() - 1
    }

    pub fn here(&self) -> u32 {
        self.code.len() as u32
    }

    pub fn patch_jump(&mut self, at: usize, target: u32) {
        match &mut self.code[at] {
            Op::Jump(t) | Op::JumpIfFalse(t) | Op::TryBegin(t) => *t = target,
            other => panic!("patching non-jump {other:?}"),
        }
    }
}

/// How a global slot is initialised before the program runs.
#[derive(Clone, Debug, PartialEq)]
pub enum GlobalInit {
    Zero,
    EmptyList,
    ZeroMatrix,
    ComplexZero,
    Grob(u8),
    Setting(Setting),
}

/// Calculator settings exposed as variables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    HAngle,
    HFormat,
    HDigits,
    HComplex,
    Base,
    Bits,
    Signed,
}

#[derive(Clone, Debug)]
pub struct GlobalSlot {
    pub name: String,
    pub init: GlobalInit,
    /// Declared by the program (file-level LOCAL/EXPORT).
    pub program_var: bool,
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub struct CompiledProgram {
    pub functions: Vec<Chunk>,
    pub consts: Vec<Const>,
    pub globals: Vec<GlobalSlot>,
    /// Names used by `Op::Unsupported`.
    pub names: Vec<String>,
    /// Chunk that initialises file-level variables.
    pub init: Chunk,
}

impl CompiledProgram {
    pub fn function_index(&self, name: &str) -> Option<usize> {
        self.functions.iter().position(|f| f.name == name)
    }

    pub fn exported(&self) -> impl Iterator<Item = (usize, &Chunk)> {
        self.functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.exported)
    }

    /// The function a run starts in: `entry` when given, else the only (or
    /// first) exported function, else the program's only function. Shared
    /// by `ppl run` and the IDE so both pick the same entry point.
    pub fn pick_entry(&self, entry: Option<&str>) -> Result<String, String> {
        self.pick_entry_with_stem(entry, None)
    }

    /// Like [`pick_entry`], but if `entry` is not given and an exported function
    /// matches `preferred_stem` (e.g. the file or program name), it is preferred
    /// over the first exported function in file order.
    pub fn pick_entry_with_stem(
        &self,
        entry: Option<&str>,
        preferred_stem: Option<&str>,
    ) -> Result<String, String> {
        if let Some(e) = entry {
            return self
                .function_index(e)
                .map(|_| e.to_string())
                .ok_or_else(|| format!("no function named `{e}` in this program"));
        }
        if let Some(stem) = preferred_stem {
            if let Some((_, f)) = self
                .exported()
                .find(|(_, f)| f.name.eq_ignore_ascii_case(stem))
            {
                return Ok(f.name.clone());
            }
        }
        if let Some((_, first)) = self.exported().next() {
            return Ok(first.name.clone());
        }
        match self.functions.as_slice() {
            [only] => Ok(only.name.clone()),
            _ => Err("the program has no EXPORT function to run (use --entry NAME)".into()),
        }
    }

    /// Disassembly for `ppl check --dump-bytecode`.
    pub fn disassemble(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        for c in std::iter::once(&self.init).chain(self.functions.iter()) {
            let _ = writeln!(
                out,
                "fn {} (params {}, locals {}):",
                c.name, c.params, c.locals
            );
            for (i, op) in c.code.iter().enumerate() {
                let detail = match op {
                    Op::Const(k) => format!("    ; {:?}", self.consts[*k as usize]),
                    Op::LoadGlobal(g) | Op::StoreGlobal(g) => {
                        format!("    ; {}", self.globals[*g as usize].name)
                    }
                    Op::LoadLocal(l) | Op::StoreLocal(l) => {
                        format!(
                            "    ; {}",
                            c.local_names
                                .get(*l as usize)
                                .map(String::as_str)
                                .unwrap_or("?")
                        )
                    }
                    Op::Call { func, .. } => {
                        format!("    ; {}", self.functions[*func as usize].name)
                    }
                    _ => String::new(),
                };
                let _ = writeln!(out, "  {i:4}  {op:?}{detail}");
            }
        }
        out
    }
}
