//! Identifier completion. Suggestions come only from real sources: PPL
//! keywords, the emulator's builtin table, the catalog of documented HP
//! Prime commands (flagged when not emulated), and names declared in the
//! current file. Nothing is invented.

use ppl_compiler::builtins::{self, Status};
use ppl_compiler::catalog::{self, CatalogKind};
use ppl_compiler::highlight::DocumentSymbols;
use ppl_syntax::lexer::Keyword;

/// Where a suggestion comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionKind {
    /// Declared in this file.
    Function,
    Variable,
    Keyword,
    Command,
    CasCommand,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    /// Short description: parameters, arity, emulation status.
    pub detail: String,
}

const KEYWORDS: &[Keyword] = &[
    Keyword::Export,
    Keyword::Local,
    Keyword::Begin,
    Keyword::End,
    Keyword::If,
    Keyword::Then,
    Keyword::Else,
    Keyword::Case,
    Keyword::Default,
    Keyword::Iferr,
    Keyword::For,
    Keyword::From,
    Keyword::To,
    Keyword::Downto,
    Keyword::Step,
    Keyword::Do,
    Keyword::While,
    Keyword::Repeat,
    Keyword::Until,
    Keyword::Return,
    Keyword::Break,
    Keyword::Continue,
    Keyword::Kill,
    Keyword::Key,
    Keyword::And,
    Keyword::Or,
    Keyword::Xor,
    Keyword::Not,
    Keyword::Mod,
    Keyword::Div,
];

/// All candidates for a document (unfiltered).
pub fn candidates(symbols: &DocumentSymbols) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for f in &symbols.functions {
        out.push(CompletionItem {
            label: f.name.clone(),
            kind: CompletionKind::Function,
            detail: format!("({})", f.params.join(", ")),
        });
    }
    for v in symbols.globals.iter().chain(&symbols.locals) {
        if !out.iter().any(|i: &CompletionItem| &i.label == v) {
            out.push(CompletionItem {
                label: v.clone(),
                kind: CompletionKind::Variable,
                detail: "variable".into(),
            });
        }
    }
    for k in KEYWORDS {
        out.push(CompletionItem {
            label: k.as_str().into(),
            kind: CompletionKind::Keyword,
            detail: "keyword".into(),
        });
    }
    for s in builtins::BUILTINS {
        if s.name == "CAS" {
            continue;
        }
        let status = match s.status {
            Status::Unsupported => ", not emulated",
            Status::Approximate => ", approximate",
            Status::Implemented => "",
        };
        out.push(CompletionItem {
            label: s.name.into(),
            kind: CompletionKind::Command,
            detail: format!("{} args{status}", s.arity_text()),
        });
    }
    for e in catalog::entries() {
        out.push(CompletionItem {
            label: e.name.into(),
            kind: match e.kind {
                CatalogKind::Cas => CompletionKind::CasCommand,
                CatalogKind::Command => CompletionKind::Command,
            },
            detail: format!("{}, not emulated", e.kind.label()),
        });
    }
    out
}

/// Candidates starting with `prefix` (case-insensitive), best first:
/// exact-case prefix, then file symbols, then shorter names.
pub fn filter(items: &[CompletionItem], prefix: &str, limit: usize) -> Vec<CompletionItem> {
    if prefix.is_empty() {
        return Vec::new();
    }
    let lower = prefix.to_lowercase();
    let mut hits: Vec<&CompletionItem> = items
        .iter()
        .filter(|i| i.label.to_lowercase().starts_with(&lower) && i.label != prefix)
        .collect();
    hits.sort_by_key(|i| {
        (
            !i.label.starts_with(prefix),
            i.kind,
            i.label.len(),
            i.label.clone(),
        )
    });
    hits.into_iter().take(limit).cloned().collect()
}

/// Open completion popup state.
#[derive(Clone, Debug, Default)]
pub struct CompletionState {
    pub open: bool,
    /// Char index where the word being completed starts.
    pub word_start: usize,
    pub items: Vec<CompletionItem>,
    pub selected: usize,
}

impl CompletionState {
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.selected = 0;
    }
}

/// Char index where the identifier ending at `cursor` starts.
pub fn word_start(text: &str, cursor: usize) -> usize {
    let chars: Vec<char> = text.chars().take(cursor).collect();
    let mut s = chars.len();
    while s > 0 && (chars[s - 1].is_alphanumeric() || chars[s - 1] == '_' || chars[s - 1] == '.') {
        s -= 1;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_compiler::highlight::analyze;

    #[test]
    fn suggestions_come_from_real_sources_only() {
        let a = analyze("EXPORT MYFUNC(n)\nBEGIN\n  LOCAL total;\nEND;\n");
        let items = candidates(&a.symbols);
        let f = filter(&items, "MY", 10);
        assert_eq!(f[0].label, "MYFUNC");
        assert_eq!(f[0].detail, "(n)");
        let p = filter(&items, "PRI", 10);
        assert!(
            p.iter()
                .any(|i| i.label == "PRINT" && i.kind == CompletionKind::Command)
        );
        let s = filter(&items, "SOL", 10);
        let solve = s.iter().find(|i| i.label == "SOLVE").unwrap();
        assert!(solve.detail.contains("not emulated"));
        assert!(filter(&items, "tot", 10).iter().any(|i| i.label == "total"));
        assert!(filter(&items, "ZZZQ", 10).is_empty());
        // Every suggestion is a keyword, a builtin, a documented catalog
        // name or a symbol of the file.
        for i in &items {
            let known = Keyword::from_upper(&i.label).is_some()
                || builtins::lookup(&i.label).is_some()
                || catalog::lookup(&i.label).is_some()
                || ["MYFUNC", "total"].contains(&i.label.as_str());
            assert!(known, "invented suggestion {}", i.label);
        }
    }

    #[test]
    fn word_start_handles_unicode_and_qualified_names() {
        assert_eq!(word_start("x := CAS.so", 11), 5);
        assert_eq!(word_start("π+abc", 5), 2);
        assert_eq!(word_start("(", 1), 1);
    }
}
