//! Loading, compiling and reporting shared by every subcommand.

use std::io::IsTerminal;
use std::path::Path;
use std::rc::Rc;

use ppl_compiler::{CompileOptions, CompiledProgram};
use ppl_syntax::{Diagnostics, SourceFile};
use ppl_vm::{RuntimeError, Settings, Value};

/// Exit codes of the `ppl` binary.
pub mod exit {
    pub const OK: u8 = 0;
    /// Syntax or semantic errors.
    pub const COMPILE: u8 = 1;
    /// Bad command-line usage (clap also uses 2).
    pub const USAGE: u8 = 2;
    /// The program raised a runtime error.
    pub const RUNTIME: u8 = 3;
    /// Host protection limit, interrupt, or exhausted input script.
    pub const LIMIT: u8 = 4;
    /// Device or I/O failure.
    pub const IO: u8 = 5;
}

pub fn color_stderr() -> bool {
    std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn load_source(file: Option<&Path>, code: Option<&str>) -> Result<SourceFile, String> {
    if let Some(code) = code {
        let name = file
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<inline>".into());
        return Ok(SourceFile::new(name, code.replace("\r\n", "\n")));
    }
    match file {
        Some(p) => {
            let (f, _) = ppl_syntax::read_source_file(p)
                .map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            Ok(f)
        }
        None => {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| format!("cannot read stdin: {e}"))?;
            Ok(SourceFile::new("<stdin>", s))
        }
    }
}

pub fn print_diagnostics(file: &SourceFile, diags: &Diagnostics, show_warnings: bool) {
    let color = color_stderr();
    for d in &diags.items {
        if d.is_error() || show_warnings {
            eprint!("{}", d.render(file, color));
        }
    }
    let errors = diags.errors().count();
    let warnings = diags.warnings().count();
    if errors > 0 || (show_warnings && warnings > 0) {
        eprintln!(
            "{errors} error(s), {warnings} warning(s){}",
            if errors > 0 {
                " -- fix errors before running"
            } else {
                ""
            }
        );
    }
}

/// Compiles and prints diagnostics; `None` means compilation failed.
pub fn compile(
    file: &SourceFile,
    show_warnings: bool,
) -> (Option<Rc<CompiledProgram>>, Diagnostics) {
    if file.text.trim().is_empty() {
        eprintln!("error: no PPL code provided ({})", file.name);
        return (None, Diagnostics::default());
    }
    let (prog, diags) = ppl_compiler::compile_source(file, &CompileOptions::default());
    print_diagnostics(file, &diags, show_warnings);
    (prog.map(Rc::new), diags)
}

/// Chooses the function to run: `--entry`, else the only (or first)
/// exported function.
pub fn pick_entry(prog: &CompiledProgram, entry: Option<&str>) -> Result<String, String> {
    if let Some(e) = entry {
        return prog
            .function_index(e)
            .map(|_| e.to_string())
            .ok_or_else(|| format!("no function named `{e}` in this program"));
    }
    let exported: Vec<&str> = prog.exported().map(|(_, c)| c.name.as_str()).collect();
    match exported.as_slice() {
        [] => Err("the program has no EXPORT function to run (use --entry NAME)".into()),
        [one] => Ok(one.to_string()),
        [first, rest @ ..] => {
            eprintln!(
                "note: several EXPORT functions; running `{first}` (others: {}; choose with --entry)",
                rest.join(", ")
            );
            Ok(first.to_string())
        }
    }
}

/// Parses `--args "50, {1,2}, \"s\""` into values (top-level commas split).
pub fn parse_args(text: Option<&str>, settings: &Settings) -> Result<Vec<Value>, String> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '"' => in_str = !in_str,
            '{' | '[' | '(' if !in_str => depth += 1,
            '}' | ']' | ')' if !in_str => depth -= 1,
            ',' if depth == 0 && !in_str => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    parts.push(cur);
    parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| {
            ppl_vm::eval_text_with(p, settings).map_err(|e| format!("bad argument `{p}`: {e}"))
        })
        .collect()
}

pub fn report_runtime_error(file: &SourceFile, err: &RuntimeError) {
    let color = color_stderr();
    let (r, b, e) = if color {
        ("\x1b[0m", "\x1b[1m", "\x1b[1;31m")
    } else {
        ("", "", "")
    };
    eprintln!("{e}runtime error{r}{b}: {err}{r}");
    for (i, f) in err.trace.iter().enumerate() {
        let lc = file.line_col(f.span.start);
        let line = file.line_text(lc.line).trim();
        let arrow = if i == 0 { "-->" } else { "   " };
        eprintln!(
            "  {arrow} {}:{}:{} in {}   {}",
            file.name, lc.line, lc.col, f.function, line
        );
    }
}

pub fn exit_code_for(err: &RuntimeError) -> u8 {
    use ppl_vm::ErrorKind as K;
    match err.kind {
        K::ResourceLimit(_) | K::Interrupted | K::ScriptExhausted => exit::LIMIT,
        _ => exit::RUNTIME,
    }
}
