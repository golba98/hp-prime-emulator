//! Semantic analysis and bytecode compilation of PPL programs.

pub mod builtins;
pub mod bytecode;
pub mod catalog;
mod compile;
pub mod highlight;

pub use bytecode::{Chunk, CompiledProgram, Const, GlobalInit, GlobalSlot, Op, Setting};
pub use compile::{CompileOptions, compile, is_system_variable};

use ppl_syntax::{Diagnostics, SourceFile};

/// Parses and compiles a source file, returning every diagnostic.
pub fn compile_source(
    file: &SourceFile,
    opts: &CompileOptions,
) -> (Option<CompiledProgram>, Diagnostics) {
    let (ast, mut diags) = ppl_syntax::parse(&file.text);
    // The firmware checks syntax before resolving names (location probe
    // c06_earlier_unknown). Semantic diagnostics from a recovered AST can
    // otherwise obscure the actual syntax error, even on earlier lines.
    if diags.has_errors() {
        return (None, diags);
    }
    let (prog, sema) = compile(&ast, opts);
    diags.extend(sema);
    diags.sort();
    if diags.has_errors() {
        return (None, diags);
    }
    (prog, diags)
}
