//! PPL front end: source decoding, lexing, parsing and diagnostics.

pub mod ast;
pub mod diag;
pub mod lexer;
pub mod parser;
pub mod source;

pub use diag::{Category, Diagnostic, DiagnosticClass, Diagnostics, Severity};
pub use parser::{parse, parse_expression};
pub use source::{LineCol, SourceEncoding, SourceFile, Span, decode_source, read_source_file};
