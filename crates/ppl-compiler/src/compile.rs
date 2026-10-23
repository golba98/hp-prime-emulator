//! Semantic analysis and bytecode generation in one pass over the AST.
//!
//! Name resolution order for an identifier:
//! 1. `LOCAL` variables and parameters in enclosing blocks of the function,
//! 2. file-level `LOCAL`/`EXPORT` variables,
//! 3. user functions,
//! 4. calculator variables (`A`–`Z`, `L0`–`L9`, `M0`–`M9`, `Z0`–`Z9`,
//!    `G0`–`G9`, settings such as `HAngle`),
//! 5. built-in commands.
//!
//! Anything else is an "undeclared identifier" error, which matches the
//! Prime rejecting programs that use undefined names (unverified for every
//! context).

use std::collections::HashMap;

use ppl_math::{Complex, HpInt, IntBase, Real};
use ppl_syntax::ast::*;
use ppl_syntax::{Category as DiagCat, Diagnostic, Diagnostics, Span};

use crate::builtins::{self, Builtin, Status};
use crate::bytecode::*;
use crate::catalog::{self, CatalogKind};

#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// Base used for `#123` literals without a suffix (Home setting).
    pub default_int_base: IntBase,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            default_int_base: IntBase::Hex,
        }
    }
}

pub fn compile(program: &Program, opts: &CompileOptions) -> (Option<CompiledProgram>, Diagnostics) {
    let mut c = Compiler::new(opts.clone());
    c.collect(program);
    c.compile_items(program);
    let has_errors = c.diags.has_errors();
    let mut diags = std::mem::take(&mut c.diags);
    diags.sort();
    if has_errors {
        return (None, diags);
    }
    let prog = CompiledProgram {
        functions: c
            .chunks
            .into_iter()
            .map(|c| c.expect("every function compiled"))
            .collect(),
        consts: c.consts,
        globals: c.globals,
        names: c.names,
        init: c.init.expect("init chunk"),
    };
    (Some(prog), diags)
}

#[derive(Clone, Copy, Debug)]
enum VarRef {
    Local(u16),
    Global(u32),
}

struct FnInfo {
    name: String,
    params: usize,
    def_start: u32,
    span: Span,
}

struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    try_depth: usize,
}

struct LocalInfo {
    span: Span,
    used: bool,
    name: String,
    hidden: bool,
}

struct Compiler {
    opts: CompileOptions,
    diags: Diagnostics,
    consts: Vec<Const>,
    globals: Vec<GlobalSlot>,
    global_map: HashMap<String, u32>,
    fns: Vec<FnInfo>,
    fn_map: HashMap<String, u32>,
    forward_decls: HashMap<String, u32>,
    chunks: Vec<Option<Chunk>>,
    names: Vec<String>,
    init: Option<Chunk>,
    // per-function state
    cur: Chunk,
    scopes: Vec<HashMap<String, u16>>,
    locals: Vec<LocalInfo>,
    loops: Vec<LoopCtx>,
    try_depth: usize,
    in_function: bool,
    /// Nesting depth of arguments to calls the emulator cannot run (CAS
    /// and catalog commands). Undeclared names there are symbolic CAS
    /// variables on the calculator, not program errors.
    cas_arg_depth: u32,
}

fn empty_chunk(name: &str, span: Span) -> Chunk {
    Chunk {
        name: name.to_string(),
        params: 0,
        locals: 0,
        local_names: Vec::new(),
        code: Vec::new(),
        spans: Vec::new(),
        exported: false,
        span,
    }
}

/// True for calculator variables that always exist (`A`–`Z`, `θ`, `L0`–`L9`,
/// `M0`–`M9`, `Z0`–`Z9`, `G0`–`G9`, `Ans`, mode settings). Case-sensitive,
/// exactly as name resolution in the compiler.
pub fn is_system_variable(name: &str) -> bool {
    system_global(name).is_some()
}

/// Calculator variables that always exist.
fn system_global(name: &str) -> Option<(GlobalInit, bool)> {
    let b = name.as_bytes();
    if b.len() == 1 && b[0].is_ascii_uppercase() || name == "θ" {
        return Some((GlobalInit::Zero, false));
    }
    if b.len() == 2 && b[1].is_ascii_digit() {
        let n = b[1] - b'0';
        return match b[0] {
            b'L' => Some((GlobalInit::EmptyList, false)),
            b'M' => Some((GlobalInit::ZeroMatrix, false)),
            b'Z' => Some((GlobalInit::ComplexZero, false)),
            b'G' => Some((GlobalInit::Grob(n), true)),
            _ => None,
        };
    }
    let setting = match name.to_ascii_lowercase().as_str() {
        "hangle" => Setting::HAngle,
        "hformat" => Setting::HFormat,
        "hdigits" => Setting::HDigits,
        "hcomplex" => Setting::HComplex,
        "base" => Setting::Base,
        "bits" => Setting::Bits,
        "signed" => Setting::Signed,
        "ans" => return Some((GlobalInit::Zero, false)),
        _ => return None,
    };
    Some((GlobalInit::Setting(setting), false))
}

fn canonical_system_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "hangle" => "HAngle".into(),
        "hformat" => "HFormat".into(),
        "hdigits" => "HDigits".into(),
        "hcomplex" => "HComplex".into(),
        "base" => "Base".into(),
        "bits" => "Bits".into(),
        "signed" => "Signed".into(),
        "ans" => "Ans".into(),
        _ => name.to_string(),
    }
}

impl Compiler {
    fn new(opts: CompileOptions) -> Compiler {
        Compiler {
            opts,
            diags: Diagnostics::default(),
            consts: Vec::new(),
            globals: Vec::new(),
            global_map: HashMap::new(),
            fns: Vec::new(),
            fn_map: HashMap::new(),
            forward_decls: HashMap::new(),
            chunks: Vec::new(),
            names: Vec::new(),
            init: None,
            cur: empty_chunk("", Span::default()),
            scopes: Vec::new(),
            locals: Vec::new(),
            loops: Vec::new(),
            try_depth: 0,
            in_function: false,
            cas_arg_depth: 0,
        }
    }

    fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::error(DiagCat::Semantic, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    fn warn(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::warning(DiagCat::Semantic, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    /// Valid HP Prime source the emulator cannot run faithfully.
    fn compat(
        &mut self,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> &mut Diagnostic {
        self.diags.push(Diagnostic::compat(code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    /// Informational compatibility note (the emulator substitutes behaviour).
    fn compat_note(
        &mut self,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::note(DiagCat::Compatibility, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    /// Compiles `args` and a call that raises "unsupported" if executed.
    fn unsupported_call(&mut self, name: &str, args: &[Expr], span: Span) {
        self.cas_arg_depth += 1;
        for a in args {
            self.expr(a);
        }
        self.cas_arg_depth -= 1;
        self.unsupported(name, args.len(), span);
    }

    // ---- declarations ----------------------------------------------------

    fn collect(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Function(f) => {
                    if let Some(&prev) = self.fn_map.get(&f.name.name) {
                        let prev_span = self.fns[prev as usize].span;
                        self.error(
                            "duplicate-function",
                            f.name.span,
                            format!("function `{}` is defined twice", f.name.name),
                        )
                        .hint = Some(format!("first definition at byte {}", prev_span.start));
                        continue;
                    }
                    if builtins::lookup(&f.name.name).is_some()
                        || system_global(&f.name.name).is_some()
                    {
                        self.error(
                            "reserved-name",
                            f.name.span,
                            format!(
                                "`{}` is a built-in name and cannot be redefined",
                                f.name.name
                            ),
                        );
                    }
                    self.check_params(&f.params);
                    let id = self.fns.len() as u32;
                    self.fn_map.insert(f.name.name.clone(), id);
                    self.fns.push(FnInfo {
                        name: f.name.name.clone(),
                        params: f.params.len(),
                        def_start: f.span.start,
                        span: f.span,
                    });
                    self.chunks.push(None);
                }
                Item::ForwardDecl { name, span, .. } => {
                    self.forward_decls
                        .entry(name.name.clone())
                        .or_insert(span.start);
                }
                Item::Globals { vars, .. } => {
                    for v in vars {
                        if self.global_map.contains_key(&v.name.name) {
                            self.warn(
                                "duplicate-global",
                                v.name.span,
                                format!("variable `{}` is declared twice", v.name.name),
                            );
                            continue;
                        }
                        if system_global(&v.name.name).is_some() {
                            self.error(
                                "reserved-name",
                                v.name.span,
                                format!(
                                    "`{}` is a calculator variable and cannot be redeclared",
                                    v.name.name
                                ),
                            );
                            continue;
                        }
                        let id = self.globals.len() as u32;
                        self.globals.push(GlobalSlot {
                            name: v.name.name.clone(),
                            init: GlobalInit::Zero,
                            program_var: true,
                            read_only: false,
                        });
                        self.global_map.insert(v.name.name.clone(), id);
                    }
                }
                Item::CasBlock { span, .. } => {
                    self.compat(
                        "cas-block-unsupported",
                        *span,
                        "`#cas` sections are not emulated; functions defined there are unavailable",
                    );
                }
            }
        }
        for (name, &pos) in &self.forward_decls.clone() {
            if !self.fn_map.contains_key(name) {
                self.warn(
                    "forward-without-definition",
                    Span {
                        start: pos,
                        end: pos,
                    },
                    format!("`{name}` is declared but never defined"),
                );
            }
        }
    }

    fn check_params(&mut self, params: &[Ident]) {
        let mut seen = Vec::<&str>::new();
        for p in params {
            if seen.contains(&p.name.as_str()) {
                self.error(
                    "duplicate-parameter",
                    p.span,
                    format!("parameter `{}` appears twice", p.name),
                );
            }
            seen.push(&p.name);
            if let Some(b) = builtins::lookup(&p.name) {
                self.error(
                    "reserved-name",
                    p.span,
                    format!(
                        "parameter `{}` shadows the built-in command {}",
                        p.name, b.name
                    ),
                );
            }
        }
    }

    fn compile_items(&mut self, program: &Program) {
        // File-level initialisers run first, in source order.
        self.begin_chunk("<init>", Span::default(), &[]);
        for item in &program.items {
            if let Item::Globals { vars, .. } = item {
                for v in vars {
                    let Some(&slot) = self.global_map.get(&v.name.name) else {
                        continue;
                    };
                    if let Some(init) = &v.init {
                        self.expr(init);
                        self.cur.emit(Op::StoreGlobal(slot), v.name.span);
                    }
                }
            }
        }
        self.cur.emit(Op::Zero, Span::default());
        self.cur.emit(Op::Return, Span::default());
        self.init = Some(self.end_chunk(false));

        for item in &program.items {
            if let Item::Function(f) = item {
                let Some(&id) = self.fn_map.get(&f.name.name) else {
                    continue;
                };
                if self.chunks[id as usize].is_some()
                    || self.fns[id as usize].def_start != f.span.start
                {
                    continue;
                }
                let chunk = self.function(f);
                self.chunks[id as usize] = Some(chunk);
            }
        }
    }

    fn begin_chunk(&mut self, name: &str, span: Span, params: &[Ident]) {
        self.cur = empty_chunk(name, span);
        self.scopes = vec![HashMap::new()];
        self.locals.clear();
        self.loops.clear();
        self.try_depth = 0;
        for p in params {
            let slot = self.new_local(&p.name, p.span, false);
            self.locals[slot as usize].used = true;
        }
        self.cur.params = params.len() as u8;
    }

    fn end_chunk(&mut self, exported: bool) -> Chunk {
        let mut chunk = std::mem::replace(&mut self.cur, empty_chunk("", Span::default()));
        chunk.locals = self.locals.len() as u16;
        chunk.local_names = self.locals.iter().map(|l| l.name.clone()).collect();
        chunk.exported = exported;
        let unused: Vec<(String, Span)> = self
            .locals
            .iter()
            .filter(|l| !l.used && !l.hidden)
            .map(|l| (l.name.clone(), l.span))
            .collect();
        for (name, span) in unused {
            self.warn(
                "unused-local",
                span,
                format!("LOCAL `{name}` is declared but never used"),
            );
        }
        chunk
    }

    fn function(&mut self, f: &FunctionDef) -> Chunk {
        self.in_function = true;
        self.begin_chunk(&f.name.name, f.span, &f.params);
        self.block(&f.body);
        self.cur.emit(Op::Zero, f.span);
        self.cur.emit(Op::Return, f.span);
        self.in_function = false;
        self.end_chunk(f.exported)
    }

    fn new_local(&mut self, name: &str, span: Span, hidden: bool) -> u16 {
        let slot = self.locals.len() as u16;
        self.locals.push(LocalInfo {
            span,
            used: hidden,
            name: name.to_string(),
            hidden,
        });
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), slot);
        slot
    }

    fn hidden_local(&mut self, tag: &str) -> u16 {
        let name = format!("<{tag}{}>", self.locals.len());
        self.new_local(&name, Span::default(), true)
    }

    // ---- name resolution -------------------------------------------------

    fn resolve_var(&mut self, name: &str) -> Option<VarRef> {
        for scope in self.scopes.iter().rev() {
            if let Some(&slot) = scope.get(name) {
                self.locals[slot as usize].used = true;
                return Some(VarRef::Local(slot));
            }
        }
        if let Some(&g) = self.global_map.get(name) {
            return Some(VarRef::Global(g));
        }
        let (init, read_only) = system_global(name)?;
        let canon = canonical_system_name(name);
        if let Some(&g) = self.global_map.get(&canon) {
            return Some(VarRef::Global(g));
        }
        let id = self.globals.len() as u32;
        self.globals.push(GlobalSlot {
            name: canon.clone(),
            init,
            program_var: false,
            read_only,
        });
        self.global_map.insert(canon, id);
        Some(VarRef::Global(id))
    }

    fn load(&mut self, v: VarRef, span: Span) {
        match v {
            VarRef::Local(s) => self.cur.emit(Op::LoadLocal(s), span),
            VarRef::Global(g) => self.cur.emit(Op::LoadGlobal(g), span),
        };
    }

    fn store(&mut self, v: VarRef, span: Span) {
        if let VarRef::Global(g) = v {
            if self.globals[g as usize].read_only {
                let name = self.globals[g as usize].name.clone();
                self.error("read-only", span, format!("`{name}` cannot be assigned"))
                    .hint = Some("use DIMGROB_P or BLIT_P to change a graphics variable".into());
            }
        }
        match v {
            VarRef::Local(s) => self.cur.emit(Op::StoreLocal(s), span),
            VarRef::Global(g) => self.cur.emit(Op::StoreGlobal(g), span),
        };
    }

    fn constant(&mut self, c: Const, span: Span) {
        let idx = match self.consts.iter().position(|k| *k == c) {
            Some(i) => i,
            None => {
                self.consts.push(c);
                self.consts.len() - 1
            }
        };
        self.cur.emit(Op::Const(idx as u32), span);
    }

    fn check_call_order(&mut self, id: u32, span: Span) {
        let info = &self.fns[id as usize];
        if info.def_start > span.start && self.in_function {
            let declared = self
                .forward_decls
                .get(&info.name)
                .is_some_and(|&p| p < span.start);
            if !declared {
                let name = info.name.clone();
                self.diags.push(
                    Diagnostic::warning(
                        DiagCat::Semantic,
                        "call-before-definition",
                        span,
                        format!("`{name}` is called before it is defined"),
                    )
                    .with_hint(format!(
                        "add a forward declaration `{name}(…);` above the caller"
                    ))
                    .unverified(),
                );
            }
        }
    }

    // ---- statements ------------------------------------------------------

    fn block(&mut self, stmts: &[Stmt]) {
        self.scopes.push(HashMap::new());
        let mut terminated: Option<Span> = None;
        for s in stmts {
            if let Some(t) = terminated.take() {
                self.warn("unreachable-code", s.span, "unreachable code")
                    .hint = Some(format!(
                    "control never reaches here after the statement at byte {}",
                    t.start
                ));
            }
            self.stmt(s);
            if matches!(
                s.kind,
                StmtKind::Return(_) | StmtKind::Break(_) | StmtKind::Continue
            ) {
                terminated = Some(s.span);
            }
        }
        self.scopes.pop();
    }

    fn stmt(&mut self, s: &Stmt) {
        self.cur.emit(Op::Stmt, s.span);
        match &s.kind {
            StmtKind::Expr(e) => match &e.kind {
                ExprKind::Assign(t, v) => self.assign(t, v, false, e.span),
                ExprKind::Store(v, t) => self.assign(t, v, false, e.span),
                _ => {
                    self.expr(e);
                    self.cur.emit(Op::Pop, e.span);
                }
            },
            StmtKind::Local(vars) => {
                for v in vars {
                    if v.name.name.is_empty() {
                        continue;
                    }
                    if let Some(b) = builtins::lookup(&v.name.name) {
                        self.warn(
                            "shadows-builtin",
                            v.name.span,
                            format!(
                                "LOCAL `{}` hides the built-in command {}",
                                v.name.name, b.name
                            ),
                        );
                    }
                    if self.scopes.last().unwrap().contains_key(&v.name.name) {
                        self.warn(
                            "duplicate-local",
                            v.name.span,
                            format!("`{}` is already declared in this block", v.name.name),
                        );
                    }
                    // Evaluate the initialiser before the name is visible.
                    match &v.init {
                        Some(init) => self.expr(init),
                        None => {
                            self.cur.emit(Op::Zero, v.name.span);
                        }
                    }
                    let slot = self.new_local(&v.name.name, v.name.span, false);
                    self.cur.emit(Op::StoreLocal(slot), v.name.span);
                }
            }
            StmtKind::Block(body) => self.block(body),
            StmtKind::If {
                cond,
                then_body,
                else_body,
            } => {
                self.expr(cond);
                let jf = self.cur.emit(Op::JumpIfFalse(0), cond.span);
                self.block(then_body);
                match else_body {
                    Some(eb) => {
                        let jend = self.cur.emit(Op::Jump(0), s.span);
                        let here = self.cur.here();
                        self.cur.patch_jump(jf, here);
                        self.block(eb);
                        let here = self.cur.here();
                        self.cur.patch_jump(jend, here);
                    }
                    None => {
                        let here = self.cur.here();
                        self.cur.patch_jump(jf, here);
                    }
                }
            }
            StmtKind::Case { arms, default } => {
                let mut ends = Vec::new();
                for (cond, body) in arms {
                    self.expr(cond);
                    let jf = self.cur.emit(Op::JumpIfFalse(0), cond.span);
                    self.block(body);
                    ends.push(self.cur.emit(Op::Jump(0), cond.span));
                    let here = self.cur.here();
                    self.cur.patch_jump(jf, here);
                }
                if let Some(d) = default {
                    self.block(d);
                }
                let here = self.cur.here();
                for j in ends {
                    self.cur.patch_jump(j, here);
                }
            }
            StmtKind::IfErr {
                body,
                handler,
                else_body,
            } => {
                let t = self.cur.emit(Op::TryBegin(0), s.span);
                self.try_depth += 1;
                self.block(body);
                self.try_depth -= 1;
                self.cur.emit(Op::TryEnd, s.span);
                if let Some(eb) = else_body {
                    self.block(eb);
                }
                let jend = self.cur.emit(Op::Jump(0), s.span);
                let here = self.cur.here();
                self.cur.patch_jump(t, here);
                self.block(handler);
                let here = self.cur.here();
                self.cur.patch_jump(jend, here);
            }
            StmtKind::For {
                var,
                from,
                to,
                step,
                down,
                body,
            } => self.for_loop(var, from, to, step.as_ref(), *down, body, s.span),
            StmtKind::While { cond, body } => {
                let top = self.cur.here();
                self.expr(cond);
                let jf = self.cur.emit(Op::JumpIfFalse(0), cond.span);
                self.loops.push(LoopCtx {
                    breaks: Vec::new(),
                    continues: Vec::new(),
                    try_depth: self.try_depth,
                });
                self.block(body);
                self.cur.emit(Op::Jump(top), s.span);
                let end = self.cur.here();
                self.cur.patch_jump(jf, end);
                self.finish_loop(top, end);
            }
            StmtKind::Repeat { body, until } => {
                let top = self.cur.here();
                self.loops.push(LoopCtx {
                    breaks: Vec::new(),
                    continues: Vec::new(),
                    try_depth: self.try_depth,
                });
                self.block(body);
                let cont = self.cur.here();
                self.expr(until);
                self.cur.emit(Op::JumpIfFalse(top), until.span);
                let end = self.cur.here();
                self.finish_loop(cont, end);
            }
            StmtKind::Return(e) => {
                if !self.in_function {
                    self.error(
                        "return-outside-function",
                        s.span,
                        "RETURN outside a function",
                    );
                }
                match e {
                    Some(e) => self.expr(e),
                    None => {
                        self.cur.emit(Op::Zero, s.span);
                    }
                }
                self.cur.emit(Op::Return, s.span);
            }
            StmtKind::Break(levels) => {
                if levels.is_some() {
                    self.warn(
                        "break-levels",
                        s.span,
                        "BREAK with a level count is treated as a single-level BREAK",
                    )
                    .unverified = true;
                }
                self.loop_jump(true, s.span);
            }
            StmtKind::Continue => self.loop_jump(false, s.span),
            StmtKind::Kill => {
                self.cur.emit(Op::Kill, s.span);
            }
        }
    }

    fn loop_jump(&mut self, is_break: bool, span: Span) {
        let Some(ctx) = self.loops.last() else {
            let what = if is_break { "BREAK" } else { "CONTINUE" };
            self.error("outside-loop", span, format!("{what} outside a loop"));
            return;
        };
        for _ in ctx.try_depth..self.try_depth {
            self.cur.emit(Op::TryEnd, span);
        }
        let j = self.cur.emit(Op::Jump(0), span);
        let ctx = self.loops.last_mut().unwrap();
        if is_break {
            ctx.breaks.push(j)
        } else {
            ctx.continues.push(j)
        }
    }

    fn finish_loop(&mut self, continue_target: u32, end: u32) {
        let ctx = self.loops.pop().unwrap();
        for j in ctx.breaks {
            self.cur.patch_jump(j, end);
        }
        for j in ctx.continues {
            self.cur.patch_jump(j, continue_target);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn for_loop(
        &mut self,
        var: &Ident,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        down: bool,
        body: &[Stmt],
        span: Span,
    ) {
        let Some(v) = self.resolve_var(&var.name) else {
            self.error(
                "undeclared-identifier",
                var.span,
                format!("undeclared variable `{}` used as FOR counter", var.name),
            )
            .hint = Some(format!("declare it first: LOCAL {};", var.name));
            return;
        };
        self.scopes.push(HashMap::new());
        let to_slot = self.hidden_local("to");
        let step_slot = self.hidden_local("step");
        self.expr(from);
        self.store(v, var.span);
        self.expr(to);
        self.cur.emit(Op::StoreLocal(to_slot), to.span);
        match step {
            Some(s) => self.expr(s),
            None => self.constant(Const::Real(Real::ONE), span),
        }
        self.cur.emit(Op::StoreLocal(step_slot), span);
        let top = self.cur.here();
        self.load(v, var.span);
        self.cur.emit(Op::LoadLocal(to_slot), span);
        self.cur.emit(Op::ForTest { down }, span);
        let jf = self.cur.emit(Op::JumpIfFalse(0), span);
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            try_depth: self.try_depth,
        });
        self.block(body);
        let cont = self.cur.here();
        self.load(v, var.span);
        self.cur.emit(Op::LoadLocal(step_slot), span);
        self.cur
            .emit(Op::Bin(if down { BinOp::Sub } else { BinOp::Add }), span);
        self.store(v, var.span);
        self.cur.emit(Op::Jump(top), span);
        let end = self.cur.here();
        self.cur.patch_jump(jf, end);
        self.finish_loop(cont, end);
        self.scopes.pop();
    }

    // ---- assignment ------------------------------------------------------

    fn assign(&mut self, target: &Expr, value: &Expr, want_value: bool, span: Span) {
        match &target.kind {
            ExprKind::Ident(name) => {
                let Some(v) = self.resolve_var(name) else {
                    let msg = if self.fn_map.contains_key(name) || builtins::lookup(name).is_some()
                    {
                        format!("cannot assign to function `{name}`")
                    } else {
                        format!("undeclared variable `{name}`")
                    };
                    self.error("undeclared-identifier", target.span, msg).hint =
                        Some(format!("declare it first: LOCAL {name};"));
                    self.expr(value);
                    if !want_value {
                        self.cur.emit(Op::Pop, span);
                    }
                    return;
                };
                self.expr(value);
                if want_value {
                    self.cur.emit(Op::Dup, span);
                }
                self.store(v, target.span);
            }
            ExprKind::Call(base, idx) | ExprKind::Index(base, idx) => {
                let ExprKind::Ident(name) = &base.kind else {
                    self.error(
                        "invalid-assignment-target",
                        target.span,
                        "only variables and their elements can be assigned",
                    );
                    return;
                };
                // Indexed system variables are assignable on the Prime
                // (`Notes("X"):=…`, `Programs("X"):=…`; hardware-verified by
                // the result-note writers of campaign runs). They resolve to
                // built-ins here, not variables, so they get their own store
                // operations.
                if idx.len() == 1 {
                    let sys_store =
                        builtins::lookup_case_insensitive(name).and_then(|s| match s.builtin {
                            Builtin::Notes => Some(Op::StoreNote),
                            Builtin::Programs => Some(Op::StoreProgram),
                            _ => None,
                        });
                    if let Some(op) = sys_store {
                        self.expr(&idx[0]);
                        self.expr(value);
                        self.cur.emit(op, target.span);
                        if !want_value {
                            self.cur.emit(Op::Pop, span);
                        }
                        return;
                    }
                }
                let Some(v) = self.resolve_var(name) else {
                    self.error(
                        "invalid-assignment-target",
                        target.span,
                        format!("`{name}` is not a variable"),
                    )
                    .hint = Some(format!("declare it first: LOCAL {name};"));
                    return;
                };
                if idx.is_empty() || idx.len() > 255 {
                    self.error(
                        "invalid-assignment-target",
                        target.span,
                        "an element assignment needs at least one index",
                    );
                    return;
                }
                for i in idx {
                    self.expr(i);
                }
                self.expr(value);
                let n = idx.len() as u8;
                match v {
                    VarRef::Local(slot) => {
                        self.cur.emit(Op::StoreIndexLocal { slot, n }, target.span)
                    }
                    VarRef::Global(slot) => {
                        if self.globals[slot as usize].read_only {
                            self.error(
                                "read-only",
                                target.span,
                                format!("`{name}` cannot be assigned"),
                            );
                        }
                        self.cur.emit(Op::StoreIndexGlobal { slot, n }, target.span)
                    }
                };
                if !want_value {
                    self.cur.emit(Op::Pop, span);
                }
            }
            _ => {
                self.error(
                    "invalid-assignment-target",
                    target.span,
                    "L-value must be a variable or an element of a variable",
                )
                .hint =
                    Some("the left side of := (or right side of ▶) must name a variable".into());
                self.expr(value);
                if !want_value {
                    self.cur.emit(Op::Pop, span);
                }
            }
        }
    }

    // ---- expressions -----------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        let span = e.span;
        match &e.kind {
            ExprKind::Number(text) => match Real::parse(text) {
                Some(r) => self.constant(Const::Real(r), span),
                None => {
                    self.error(
                        "number-out-of-range",
                        span,
                        format!("number `{text}` is out of range"),
                    );
                    self.cur.emit(Op::Zero, span);
                }
            },
            ExprKind::HashInt(text) => match HpInt::parse_literal(text, self.opts.default_int_base)
            {
                Some(i) => self.constant(Const::Int(i), span),
                None => {
                    self.error(
                        "malformed-integer",
                        span,
                        format!("malformed integer literal `#{text}`"),
                    )
                    .hint = Some("integers look like #FFh, #1010b, #777o or #255d".into());
                    self.cur.emit(Op::Zero, span);
                }
            },
            ExprKind::Str(s) => self.constant(Const::Str(s.clone()), span),
            ExprKind::Ident(name) => self.ident(name, span),
            ExprKind::List(items) => {
                for i in items {
                    self.expr(i);
                }
                self.cur.emit(Op::MakeList(items.len() as u32), span);
            }
            ExprKind::Bracket(items) => self.bracket(items, span),
            ExprKind::ComplexPair(a, b) => {
                self.expr(a);
                self.expr(b);
                self.cur.emit(Op::MakeComplex, span);
            }
            ExprKind::Unary(op, x) => {
                self.expr(x);
                self.cur.emit(
                    match op {
                        UnOp::Neg => Op::Neg,
                        UnOp::Not => Op::Not,
                        UnOp::Sqrt => Op::Sqrt,
                    },
                    span,
                );
            }
            ExprKind::Postfix(op, x) => {
                self.expr(x);
                self.cur.emit(
                    match op {
                        PostOp::Factorial => Op::Factorial,
                        PostOp::Square => Op::Square,
                        PostOp::Cube => Op::Cube,
                    },
                    span,
                );
            }
            ExprKind::Binary(op, l, r) => {
                // Hardware-verified (2026-10-23, G1 build 15157): the
                // calculator's compiler rejects real literals as DIV
                // operands with a "Syntax error" when the program is run
                // (`3.5 DIV 1E-11` → error at that line; `7 DIV -3` from
                // the verified core suite is fine). DIV is the
                // integer-quotient operator.
                if *op == BinOp::IntDiv {
                    let real_lit = |e: &Expr| {
                        matches!(&e.kind, ExprKind::Number(s)
                            if s.contains('.') || s.to_ascii_uppercase().contains('E'))
                    };
                    if real_lit(l) || real_lit(r) {
                        self.error(
                            "div-requires-integers",
                            span,
                            "DIV requires integer operands; real literals are a syntax error on the calculator",
                        )
                        .hint = Some(
                            "use IP() to take the integer part of a real, or divide with /".into(),
                        );
                    }
                }
                self.expr(l);
                self.expr(r);
                self.cur.emit(Op::Bin(*op), span);
            }
            ExprKind::Call(callee, args) => self.call(callee, args, span),
            ExprKind::Index(base, args) => {
                self.expr(base);
                for a in args {
                    self.expr(a);
                }
                self.cur.emit(Op::Index(args.len().min(255) as u8), span);
            }
            ExprKind::Assign(t, v) => self.assign(t, v, true, span),
            ExprKind::Store(v, t) => self.assign(t, v, true, span),
            ExprKind::Quoted(inner) => {
                let mut unknown = Vec::new();
                self.collect_unknown(inner, &mut unknown);
                if let Some(name) = unknown.first() {
                    self.error(
                        "undeclared-identifier",
                        span,
                        format!("undeclared name `{name}` in quoted expression"),
                    )
                    .hint = Some(
                        "strings use double quotes; single quotes make a symbolic expression"
                            .into(),
                    );
                } else {
                    self.compat(
                        "symbolic-unsupported",
                        span,
                        "quoted (symbolic) expressions are not emulated",
                    )
                    .hint =
                        Some("evaluating this expression raises an \"unsupported\" error".into());
                }
                self.unsupported("'symbolic expression'", 0, span);
            }
            ExprKind::Error => {
                self.cur.emit(Op::Zero, span);
            }
        }
    }

    fn collect_unknown(&mut self, e: &Expr, out: &mut Vec<String>) {
        match &e.kind {
            ExprKind::Ident(n) => {
                if self.resolve_var(n).is_none()
                    && !self.fn_map.contains_key(n)
                    && builtins::lookup(n).is_none()
                    && !matches!(n.as_str(), "π" | "ⅈ" | "ℯ" | "∞" | "\u{E003}")
                {
                    out.push(n.clone());
                }
            }
            ExprKind::Binary(_, a, b)
            | ExprKind::ComplexPair(a, b)
            | ExprKind::Assign(a, b)
            | ExprKind::Store(a, b) => {
                self.collect_unknown(a, out);
                self.collect_unknown(b, out);
            }
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => {
                self.collect_unknown(a, out)
            }
            ExprKind::List(xs) | ExprKind::Bracket(xs) => {
                for x in xs {
                    self.collect_unknown(x, out);
                }
            }
            ExprKind::Call(_, xs) | ExprKind::Index(_, xs) => {
                for x in xs {
                    self.collect_unknown(x, out);
                }
            }
            _ => {}
        }
    }

    fn bracket(&mut self, items: &[Expr], span: Span) {
        let is_matrix =
            !items.is_empty() && items.iter().all(|i| matches!(i.kind, ExprKind::Bracket(_)));
        if !is_matrix {
            for i in items {
                self.expr(i);
            }
            self.cur.emit(Op::MakeVector(items.len() as u32), span);
            return;
        }
        let mut cols = None;
        for row in items {
            let ExprKind::Bracket(cells) = &row.kind else {
                unreachable!()
            };
            match cols {
                None => cols = Some(cells.len()),
                Some(c) if c != cells.len() => {
                    self.error(
                        "invalid-dimension",
                        row.span,
                        format!("matrix row has {} elements, expected {c}", cells.len()),
                    );
                }
                _ => {}
            }
            for c in cells {
                self.expr(c);
            }
        }
        // Rows of unequal length were reported; pad so code generation stays sane.
        let cols = cols.unwrap_or(0) as u32;
        self.cur.emit(
            Op::MakeMatrix {
                rows: items.len() as u32,
                cols,
            },
            span,
        );
    }

    fn ident(&mut self, name: &str, span: Span) {
        match name {
            "π" => return self.constant(Const::Real(Real::parse("3.14159265359").unwrap()), span),
            "ℯ" => {
                return self.constant(Const::Real(Real::parse("2.71828182846").unwrap()), span);
            }
            "ⅈ" | "\u{E003}" => {
                return self.constant(Const::Complex(Complex::new(Real::ZERO, Real::ONE)), span);
            }
            "∞" => {
                self.compat("unsupported-value", span, "∞ is not emulated")
                    .hint = Some("evaluating it raises an \"unsupported\" error".into());
                self.unsupported("∞", 0, span);
                return;
            }
            _ => {}
        }
        if let Some(v) = self.resolve_var(name) {
            return self.load(v, span);
        }
        if let Some(&id) = self.fn_map.get(name) {
            let params = self.fns[id as usize].params;
            if params != 0 {
                self.error(
                    "wrong-arg-count",
                    span,
                    format!("`{name}` expects {params} argument(s), got 0"),
                );
            }
            self.check_call_order(id, span);
            self.cur.emit(Op::Call { func: id, argc: 0 }, span);
            return;
        }
        if let Some(spec) = builtins::lookup(name) {
            if spec.accepts(0) && spec.ref_args == 0 {
                return self.builtin(spec.builtin, &[], span);
            }
            self.error(
                "wrong-arg-count",
                span,
                format!("{} needs arguments ({})", spec.name, spec.arity_text()),
            );
            self.cur.emit(Op::Zero, span);
            return;
        }
        if self.cas_arg_depth > 0 {
            // `solve(x^2-4=0, x)`: `x` is a CAS symbol, not a program
            // variable, and the whole call is already reported as
            // unsupported, so no diagnostic of its own.
            self.cur.emit(Op::Zero, span);
            return;
        }
        if let Some(entry) = catalog::lookup(name).filter(|e| e.kind == CatalogKind::Command) {
            // Documented zero-argument commands and constants (MAXREAL,
            // VERSION, AFiles, ...).
            self.compat(
                "unsupported-command",
                span,
                format!(
                    "{} is an HP Prime {} that is not emulated",
                    entry.name,
                    entry.kind.label()
                ),
            )
            .hint = Some(format!(
                "evaluating it raises an \"unsupported\" error ({})",
                catalog::SOURCE
            ));
            self.unsupported(entry.name, 0, span);
            return;
        }
        let mut d = Diagnostic::error(
            DiagCat::Semantic,
            "undeclared-identifier",
            span,
            format!("undeclared variable `{name}`"),
        )
        .with_hint(format!(
            "declare it with LOCAL {name}; or use a calculator variable such as A–Z"
        ));
        if let Some(spec) = builtins::lookup_case_insensitive(name) {
            d = d.with_hint(format!("did you mean the command {}?", spec.name));
        } else if let Some(entry) = catalog::lookup(name) {
            d = d.with_hint(format!(
                "`{}` is also a {}; call it with its arguments if that was intended",
                entry.name,
                entry.kind.label()
            ));
        }
        self.diags.push(d);
        self.cur.emit(Op::Zero, span);
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) {
        let ExprKind::Ident(name) = &callee.kind else {
            // `L(1)(2)`: index the result.
            self.expr(callee);
            for a in args {
                self.expr(a);
            }
            self.cur.emit(Op::Index(args.len().min(255) as u8), span);
            return;
        };
        if let Some(v) = self.resolve_var(name) {
            self.load(v, callee.span);
            if args.is_empty() {
                return;
            }
            for a in args {
                self.expr(a);
            }
            self.cur.emit(Op::Index(args.len().min(255) as u8), span);
            return;
        }
        if let Some(&id) = self.fn_map.get(name) {
            let params = self.fns[id as usize].params;
            if params != args.len() {
                self.error(
                    "wrong-arg-count",
                    span,
                    format!("`{name}` expects {params} argument(s), got {}", args.len()),
                );
            }
            self.check_call_order(id, span);
            for a in args {
                self.expr(a);
            }
            self.cur.emit(
                Op::Call {
                    func: id,
                    argc: args.len().min(255) as u8,
                },
                span,
            );
            return;
        }
        if name.contains('.') {
            // `CAS.solve(...)`, `Finance.TvmFV(...)`: documented forms for
            // reaching the CAS and app functions from a program.
            self.compat(
                "cas-unsupported",
                callee.span,
                format!("`{name}` is a CAS/app function and is not emulated"),
            )
            .hint = Some("running this call raises an \"unsupported\" error".into());
            self.unsupported_call(name, args, span);
            return;
        }
        let spec = match builtins::lookup(name) {
            Some(s) => Some(s),
            None => {
                let ci = builtins::lookup_case_insensitive(name);
                if let Some(s) = ci {
                    if s.status != Status::Unsupported {
                        let d = self.compat_note(
                            "lowercase-command",
                            callee.span,
                            format!(
                                "`{name}` calls the CAS command on the calculator; \
                                 the emulator runs the Home command {} instead",
                                s.name
                            ),
                        );
                        d.hint = Some(
                            "results can differ where the CAS and Home versions differ".into(),
                        );
                        d.unverified = true;
                    }
                }
                ci
            }
        };
        let Some(spec) = spec else {
            if let Some(entry) = catalog::lookup_case_insensitive(name) {
                let d = self.compat(
                    "unsupported-command",
                    callee.span,
                    format!(
                        "{} is an HP Prime {} that is not emulated",
                        entry.name,
                        entry.kind.label()
                    ),
                );
                d.hint = Some(format!(
                    "running this call raises an \"unsupported\" error ({})",
                    catalog::SOURCE
                ));
                if entry.name != name {
                    d.message = format!(
                        "`{name}` matches the documented {} {} (spelling differs in case) and is not emulated",
                        entry.kind.label(),
                        entry.name
                    );
                    d.unverified = true;
                }
                self.unsupported_call(entry.name, args, span);
                return;
            }
            let known_upper = builtins::lookup_case_insensitive(name).is_some();
            let mut d = Diagnostic::error(
                DiagCat::Semantic,
                "unknown-function",
                callee.span,
                format!("unknown function `{name}`"),
            );
            if known_upper {
                d = d.with_hint(format!("did you mean {}?", name.to_uppercase()));
            }
            self.diags.push(d);
            self.cur.emit(Op::Zero, span);
            return;
        };
        if !spec.accepts(args.len()) {
            self.error(
                "wrong-arg-count",
                span,
                format!(
                    "{} expects {} argument(s), got {}",
                    spec.name,
                    spec.arity_text(),
                    args.len()
                ),
            );
        }
        if spec.status == Status::Unsupported {
            self.compat(
                "unsupported-command",
                callee.span,
                format!("{} is not emulated yet", spec.name),
            )
            .hint = Some("running this call raises an \"unsupported\" error".into());
        }
        if spec.builtin == Builtin::Input && args.len() >= 5 {
            self.compat_note(
                "input-defaults-ignored",
                args[4].span.to(args[args.len() - 1].span),
                "the emulator's INPUT dialog does not apply reset_value / initial_value yet",
            );
        }
        match spec.builtin {
            Builtin::Makelist if args.len() >= 4 => self.makelist(args, span),
            _ if spec.ref_args > 0 => self.ref_builtin(spec.builtin, args, span),
            _ => {
                self.builtin(spec.builtin, args, span);
            }
        }
    }

    fn builtin(&mut self, id: Builtin, args: &[Expr], span: Span) {
        for a in args {
            self.expr(a);
        }
        self.cur.emit(
            Op::Builtin {
                id,
                argc: args.len().min(255) as u8,
            },
            span,
        );
    }

    fn unsupported(&mut self, name: &str, argc: usize, span: Span) {
        let idx = match self.names.iter().position(|n| n == name) {
            Some(i) => i,
            None => {
                self.names.push(name.to_string());
                self.names.len() - 1
            }
        };
        self.cur.emit(
            Op::Unsupported {
                name: idx as u32,
                argc: argc.min(255) as u8,
            },
            span,
        );
    }

    /// Commands whose first argument is a variable they write to.
    fn ref_builtin(&mut self, id: Builtin, args: &[Expr], span: Span) {
        let Some(first) = args.first() else {
            self.builtin(id, args, span);
            return;
        };
        let mut targets: Vec<(VarRef, Span)> = Vec::new();
        let mut collect = |c: &mut Compiler, e: &Expr| match &e.kind {
            ExprKind::Ident(n) => match c.resolve_var(n) {
                Some(v) => targets.push((v, e.span)),
                None => {
                    c.error(
                        "undeclared-identifier",
                        e.span,
                        format!("undeclared variable `{n}`"),
                    )
                    .hint = Some(format!("declare it first: LOCAL {n};"));
                }
            },
            _ => {
                c.error(
                    "expected-variable",
                    e.span,
                    "this argument must be a variable name",
                );
            }
        };
        match &first.kind {
            ExprKind::List(items) if id == Builtin::Input => {
                for it in items {
                    match &it.kind {
                        // {{A,[0]}}: variable plus allowed-type list.
                        ExprKind::List(inner) if !inner.is_empty() => collect(self, &inner[0]),
                        _ => collect(self, it),
                    }
                }
            }
            _ => collect(self, first),
        }
        for (v, s) in &targets {
            self.load(*v, *s);
        }
        self.cur
            .emit(Op::MakeList(targets.len() as u32), first.span);
        for a in &args[1..] {
            self.expr(a);
        }
        let refs = targets.len().min(255) as u8;
        self.cur.emit(
            Op::BuiltinRef {
                id,
                argc: args.len().min(255) as u8,
                refs,
            },
            span,
        );
        // Stack: result, new values…; store them back in reverse order.
        for (v, s) in targets.iter().rev() {
            self.store(*v, *s);
        }
    }

    /// `MAKELIST(expr, var, start, end[, step])` evaluates `expr` once per
    /// value of `var`, so it compiles to a loop.
    fn makelist(&mut self, args: &[Expr], span: Span) {
        let ExprKind::Ident(var_name) = &args[1].kind else {
            self.error(
                "expected-variable",
                args[1].span,
                "MAKELIST's second argument must be a variable name",
            );
            self.cur.emit(Op::Zero, span);
            return;
        };
        self.scopes.push(HashMap::new());
        let v = match self.resolve_var(var_name) {
            Some(v) => v,
            // An undeclared name acts as a temporary bound variable.
            None => VarRef::Local(self.new_local(var_name, args[1].span, true)),
        };
        let saved = self.hidden_local("saved");
        let acc = self.hidden_local("acc");
        let end = self.hidden_local("end");
        let step = self.hidden_local("step");
        self.load(v, span);
        self.cur.emit(Op::StoreLocal(saved), span);
        self.cur.emit(Op::MakeList(0), span);
        self.cur.emit(Op::StoreLocal(acc), span);
        self.expr(&args[2]);
        self.store(v, args[1].span);
        self.expr(&args[3]);
        self.cur.emit(Op::StoreLocal(end), span);
        match args.get(4) {
            Some(s) => self.expr(s),
            None => self.constant(Const::Real(Real::ONE), span),
        }
        self.cur.emit(Op::StoreLocal(step), span);
        let top = self.cur.here();
        self.load(v, span);
        self.cur.emit(Op::LoadLocal(end), span);
        self.cur.emit(Op::ForTest { down: false }, span);
        let jf = self.cur.emit(Op::JumpIfFalse(0), span);
        self.expr(&args[0]);
        self.cur.emit(Op::AppendLocal(acc), span);
        self.load(v, span);
        self.cur.emit(Op::LoadLocal(step), span);
        self.cur.emit(Op::Bin(BinOp::Add), span);
        self.store(v, args[1].span);
        self.cur.emit(Op::Jump(top), span);
        let here = self.cur.here();
        self.cur.patch_jump(jf, here);
        self.cur.emit(Op::LoadLocal(saved), span);
        self.store(v, args[1].span);
        self.cur.emit(Op::LoadLocal(acc), span);
        self.scopes.pop();
    }
}

#[cfg(test)]
mod div_tests {
    //! DIV is the integer-quotient operator; real literals as operands are
    //! a compile-time syntax error on the G1 (hardware-verified 2026-10-23:
    //! `3.5 DIV 1E-11` reported "Syntax error" at its line when the
    //! program was run, while integer DIV from the verified core suite
    //! compiles).

    use crate::*;

    fn has_error(src: &str) -> bool {
        let file = ppl_syntax::SourceFile::new("<t>", src.to_string());
        let (_, d) = compile_source(&file, &CompileOptions::default());
        d.errors().any(|e| e.code == "div-requires-integers")
    }

    #[test]
    fn real_literal_div_is_rejected() {
        assert!(has_error("EXPORT T() BEGIN RETURN 3.5 DIV 1E-11; END;"));
        assert!(has_error("EXPORT T() BEGIN RETURN 3 DIV 2.5; END;"));
    }

    #[test]
    fn integer_div_is_accepted() {
        assert!(!has_error("EXPORT T() BEGIN RETURN 7 DIV -3; END;"));
        assert!(!has_error("EXPORT T() BEGIN RETURN -42 DIV 7; END;"));
    }
}
