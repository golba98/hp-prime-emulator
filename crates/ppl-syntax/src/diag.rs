//! Diagnostics with source positions and rustc-style rendering.

use std::fmt::Write as _;

use crate::source::{SourceFile, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

/// Which stage produced a diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Lexer,
    Parser,
    Semantic,
    /// Valid HP Prime source that this emulator cannot (fully) execute:
    /// CAS calls, app functions, commands not implemented yet. Never an
    /// error, because the program is legitimate on the calculator.
    Compatibility,
    Runtime,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Lexer => "LEXER",
            Category::Parser => "PARSER",
            Category::Semantic => "SEMANTIC",
            Category::Compatibility => "COMPATIBILITY",
            Category::Runtime => "RUNTIME",
        }
    }

    /// The user-facing class of this category.
    pub fn class(self) -> DiagnosticClass {
        match self {
            Category::Lexer | Category::Parser => DiagnosticClass::Syntax,
            Category::Semantic => DiagnosticClass::Semantic,
            Category::Compatibility => DiagnosticClass::Compatibility,
            Category::Runtime => DiagnosticClass::Runtime,
        }
    }
}

/// The four kinds of problem an editor shows, kept strictly apart:
/// a missing emulator feature is never reported as a language error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticClass {
    /// Malformed source: invalid tokens, statements or expressions.
    Syntax,
    /// Well-formed source that breaks a language rule (undeclared names,
    /// wrong argument counts, invalid assignment targets).
    Semantic,
    /// Valid PPL that the emulator does not support.
    Compatibility,
    /// A failure while executing the program.
    Runtime,
}

impl DiagnosticClass {
    pub const ALL: [DiagnosticClass; 4] = [
        DiagnosticClass::Syntax,
        DiagnosticClass::Semantic,
        DiagnosticClass::Compatibility,
        DiagnosticClass::Runtime,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DiagnosticClass::Syntax => "Syntax",
            DiagnosticClass::Semantic => "Semantic",
            DiagnosticClass::Compatibility => "Compatibility",
            DiagnosticClass::Runtime => "Runtime",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub category: Category,
    /// Stable kebab-case identifier, used by tests and the compatibility matrix.
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub hint: Option<String>,
    /// True when the rule is an emulator heuristic that has not been checked
    /// against a physical calculator.
    pub unverified: bool,
}

impl Diagnostic {
    pub fn error(
        category: Category,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> Self {
        Diagnostic {
            severity: Severity::Error,
            category,
            code,
            message: msg.into(),
            span,
            hint: None,
            unverified: false,
        }
    }

    pub fn warning(
        category: Category,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(category, code, span, msg)
        }
    }

    pub fn note(
        category: Category,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> Self {
        Diagnostic {
            severity: Severity::Note,
            ..Diagnostic::error(category, code, span, msg)
        }
    }

    /// A [`Category::Compatibility`] warning: valid PPL the emulator cannot
    /// run faithfully. Never blocks compilation.
    pub fn compat(code: &'static str, span: Span, msg: impl Into<String>) -> Self {
        Diagnostic::warning(Category::Compatibility, code, span, msg)
    }

    pub fn class(&self) -> DiagnosticClass {
        self.category.class()
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn unverified(mut self) -> Self {
        self.unverified = true;
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Renders like rustc: header, location, source line, caret, hint.
    pub fn render(&self, file: &SourceFile, color: bool) -> String {
        let (sev, c) = match self.severity {
            Severity::Error => ("error", "\x1b[1;31m"),
            Severity::Warning => ("warning", "\x1b[1;33m"),
            Severity::Note => ("note", "\x1b[1;36m"),
        };
        let (c, b, r) = if color {
            (c, "\x1b[1m", "\x1b[0m")
        } else {
            ("", "", "")
        };
        let lc = file.line_col(self.span.start);
        let mut out = String::new();
        let tag = if self.unverified {
            " (unverified rule)"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "{c}{sev}[{}]{r}{b}: {}{tag}{r}",
            self.code, self.message
        );
        let _ = writeln!(out, "  --> {}:{}:{}", file.name, lc.line, lc.col);
        let line = file.line_text(lc.line);
        if !line.is_empty() {
            let gutter = lc.line.to_string().len();
            let end_lc = file.line_col(self.span.end.max(self.span.start));
            let width = if end_lc.line == lc.line {
                (end_lc.col.saturating_sub(lc.col)).max(1)
            } else {
                1
            };
            // Window the source line around the diagnostic: pathological
            // input (e.g. binary garbage decoded as text) can produce one
            // very long line and many per-character diagnostics, and
            // rendering the full line with a space-padded caret for each
            // was quadratic (gigabytes from kilobytes). The window keeps
            // every render O(width).
            const MAX_LINE: usize = 160;
            let line_chars: Vec<char> = line.chars().collect();
            let col_us = lc.col as usize;
            let (shown, off, truncated) = if line_chars.len() <= MAX_LINE {
                (line.to_string(), 0, false)
            } else {
                let start = col_us.saturating_sub(MAX_LINE / 2).min(line_chars.len());
                let end = (start + MAX_LINE).min(line_chars.len());
                let mut s: String = line_chars[start..end].iter().collect();
                if start > 0 {
                    s.insert(0, '…');
                }
                if end < line_chars.len() {
                    s.push('…');
                }
                (s, start, true)
            };
            let _ = writeln!(out, "{:gutter$} |", "");
            let _ = writeln!(out, "{} | {}", lc.line, shown);
            let caret_pad = " ".repeat(col_us.saturating_sub(1).saturating_sub(off));
            let caret_marks = if truncated {
                "^…".to_string()
            } else {
                "^".repeat(width as usize)
            };
            let _ = writeln!(out, "{:gutter$} | {caret_pad}{c}{caret_marks}{r}", "");
        }
        if let Some(h) = &self.hint {
            let _ = writeln!(out, "  = hint: {h}");
        }
        out
    }
}

/// Collected diagnostics for one compilation.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items.iter().filter(|d| d.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.items
            .iter()
            .filter(|d| d.severity == Severity::Warning)
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }

    pub fn sort(&mut self) {
        self.items.sort_by_key(|d| (d.span.start, d.severity));
    }
}
