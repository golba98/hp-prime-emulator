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
            let source = ppl_engine::source::Source::file(p)?;
            Ok(SourceFile::new(source.identity.display_name, source.text))
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

/// Chooses the function to run ([`CompiledProgram::pick_entry_with_stem`]) and
/// notes the alternatives when several functions are exported.
pub fn pick_entry(
    prog: &CompiledProgram,
    entry: Option<&str>,
    preferred_stem: Option<&str>,
) -> Result<String, String> {
    let picked = prog.pick_entry_with_stem(entry, preferred_stem)?;
    if entry.is_none() {
        let others: Vec<&str> = prog
            .exported()
            .map(|(_, c)| c.name.as_str())
            .filter(|n| *n != picked)
            .collect();
        if !others.is_empty() {
            eprintln!(
                "note: several EXPORT functions; running `{picked}` (others: {}; choose with --entry)",
                others.join(", ")
            );
        }
    }
    Ok(picked)
}

/// Parses `--args "50, {1,2}, \"s\""` into values (top-level commas split).
pub fn parse_args(text: Option<&str>, settings: &Settings) -> Result<Vec<Value>, String> {
    ppl_engine::arguments::ppl_args(text, settings)
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
