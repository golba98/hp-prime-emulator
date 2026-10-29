//! Editor language services: token classification, bracket pairs and
//! document symbols for PPL source, independent of any UI toolkit.
//!
//! Everything comes from one pass of the real lexer
//! ([`ppl_syntax::lexer::lex_with_trivia`]), so highlighting can never
//! disagree with the compiler about where a string, comment or token ends.
//! Classification is purely about the *language*: a documented HP Prime
//! command is a command whether or not the emulator implements it.

use std::collections::{HashMap, HashSet};

use ppl_syntax::Diagnostics;
use ppl_syntax::lexer::{Keyword, Tok, Token, lex_with_trivia};

use crate::builtins;
use crate::catalog::{self, CatalogKind};

/// What a span of source is, for coloring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    /// Whitespace.
    Plain,
    /// Structural keyword: `BEGIN`, `IF`, `FOR`, `RETURN`, ...
    Keyword,
    /// Word operators: `AND`, `OR`, `XOR`, `NOT`, `MOD`, `DIV`.
    OperatorKeyword,
    /// Documented Home/program command or app function (`PRINT`, `SOLVE`).
    Command,
    /// CAS command (`solve`, `CAS.diff`).
    CasCommand,
    /// Function defined in this file (declaration or call).
    UserFunction,
    /// Parameter of the enclosing function.
    Parameter,
    /// Local, exported or otherwise unclassified identifier.
    Variable,
    /// Calculator variables: `A`–`Z`, `θ`, `G0`–`G9`, `L0`–`L9`, `M0`–`M9`,
    /// `Z0`–`Z9`, `Ans`, mode settings.
    SystemVariable,
    /// `π`, `ⅈ`, `ℯ`, `∞`.
    Constant,
    /// Decimal or `#` based integer literal.
    Number,
    /// String literal body and quotes.
    String,
    /// Backslash escape inside a string literal.
    StringEscape,
    /// String or quoted expression without its closing quote.
    InvalidString,
    /// `'...'` symbolic expression.
    Symbolic,
    Comment,
    /// `#pragma` line.
    Pragma,
    /// `#cas ... #end` section.
    CasBlock,
    /// Arithmetic, comparison and assignment operators.
    Operator,
    /// `,` and `;`.
    Punctuation,
    /// `( ) [ ] { }`.
    Bracket,
    /// Characters the lexer rejects, malformed literals.
    Error,
}

/// A classified byte range. Spans are sorted, contiguous and cover the
/// whole source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

/// A function declared in the analyzed source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSymbol {
    pub name: String,
    pub params: Vec<String>,
    pub exported: bool,
    /// Byte offset of the name.
    pub offset: usize,
}

/// Names declared in the analyzed source, for completion and outlines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentSymbols {
    pub functions: Vec<FunctionSymbol>,
    /// Top-level `EXPORT x;` / `LOCAL x;` variables.
    pub globals: Vec<String>,
    /// `LOCAL` names inside function bodies (all functions).
    pub locals: Vec<String>,
}

/// Result of [`analyze`].
#[derive(Clone, Debug, Default)]
pub struct SourceAnalysis {
    pub spans: Vec<HighlightSpan>,
    /// `(open, close)` byte offsets of every bracket; `close` is `None`
    /// for an unbalanced opener. Unbalanced closers are listed with
    /// `open == close_offset` and `close == None` in [`Self::stray_closers`].
    pub bracket_pairs: Vec<(usize, Option<usize>)>,
    pub stray_closers: Vec<usize>,
    pub symbols: DocumentSymbols,
}

impl SourceAnalysis {
    /// The bracket at `offset` (byte) and its partner, if any.
    pub fn bracket_partner(&self, offset: usize) -> Option<(usize, Option<usize>)> {
        for &(open, close) in &self.bracket_pairs {
            if open == offset {
                return Some((open, close));
            }
            if close == Some(offset) {
                return Some((offset, Some(open)));
            }
        }
        self.stray_closers
            .contains(&offset)
            .then_some((offset, None))
    }

    /// The kind of the span containing byte `offset`.
    pub fn kind_at(&self, offset: usize) -> Option<TokenKind> {
        let i = self.spans.partition_point(|s| s.end <= offset);
        self.spans
            .get(i)
            .filter(|s| s.start <= offset)
            .map(|s| s.kind)
    }
}

fn is_constant(name: &str) -> bool {
    matches!(name, "π" | "ⅈ" | "ℯ" | "∞" | "\u{E003}")
}

/// How a name resolves among commands, regardless of emulator support.
fn command_kind(name: &str) -> Option<TokenKind> {
    if let Some((prefix, _)) = name.split_once('.') {
        return Some(if prefix == "CAS" {
            TokenKind::CasCommand
        } else {
            TokenKind::Command
        });
    }
    if builtins::lookup(name).is_some() {
        return Some(TokenKind::Command);
    }
    if let Some(e) = catalog::lookup(name) {
        return Some(match e.kind {
            CatalogKind::Cas => TokenKind::CasCommand,
            CatalogKind::Command => TokenKind::Command,
        });
    }
    // `append`, `sin`: lower-case spellings of commands call the CAS.
    if name.starts_with(|c: char| c.is_lowercase())
        && builtins::lookup_case_insensitive(name).is_some()
    {
        return Some(TokenKind::CasCommand);
    }
    None
}

/// Block nesting effect of a keyword: `+1` opens, `-1` closes.
fn block_delta(k: Keyword) -> i32 {
    match k {
        Keyword::Begin
        | Keyword::If
        | Keyword::For
        | Keyword::While
        | Keyword::Case
        | Keyword::Iferr
        | Keyword::Repeat => 1,
        Keyword::End | Keyword::Until => -1,
        _ => 0,
    }
}

/// Collects declarations: functions with parameters, top-level variables
/// and function-local names, plus per-token "which function body am I in".
struct Declarations {
    symbols: DocumentSymbols,
    /// Token index of a function-name declaration.
    decl_names: HashSet<usize>,
    /// For each token, the function whose body or header contains it.
    owner: Vec<Option<usize>>,
    /// Per function: parameter and local names.
    scopes: Vec<(HashSet<String>, HashSet<String>)>,
}

fn next_significant(toks: &[Token], mut i: usize) -> Option<usize> {
    while i < toks.len() {
        if !matches!(toks[i].tok, Tok::Comment) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn declarations(toks: &[Token]) -> Declarations {
    let mut d = Declarations {
        symbols: DocumentSymbols::default(),
        decl_names: HashSet::new(),
        owner: vec![None; toks.len()],
        scopes: Vec::new(),
    };
    let mut depth = 0i32;
    let mut current: Option<usize> = None;
    let mut i = 0;
    while i < toks.len() {
        d.owner[i] = current;
        match &toks[i].tok {
            Tok::Kw(Keyword::Local) if depth > 0 || current.is_some() => {
                i = declare_list(
                    toks,
                    i + 1,
                    &mut |name| {
                        if let Some(f) = current {
                            d.scopes[f].1.insert(name.to_string());
                        }
                        d.symbols.locals.push(name.to_string());
                    },
                    &mut d.owner,
                    current,
                );
                continue;
            }
            // Top-level variable declarations: `LOCAL a, b := f(1);` and
            // `EXPORT a := 1;`. `LOCAL NAME(...)` / `EXPORT NAME(...)` are
            // function headers, handled by the identifier arm below (the
            // same rule as the parser's `header_or_globals`).
            Tok::Kw(Keyword::Local | Keyword::Export) if !starts_header(toks, i + 1) => {
                i = declare_list(
                    toks,
                    i + 1,
                    &mut |name| d.symbols.globals.push(name.to_string()),
                    &mut d.owner,
                    None,
                );
                continue;
            }
            Tok::Kw(k) => {
                depth += block_delta(*k);
                if depth <= 0 {
                    depth = 0;
                    if matches!(k, Keyword::End) {
                        current = None;
                    }
                }
            }
            Tok::Ident(name) if depth == 0 => {
                let exported = i > 0 && matches!(toks[i - 1].tok, Tok::Kw(Keyword::Export));
                let next = next_significant(toks, i + 1);
                if next.is_some_and(|n| matches!(toks[n].tok, Tok::LParen)) {
                    // Function header (definition or forward declaration).
                    let mut params = Vec::new();
                    let mut j = next.unwrap() + 1;
                    while j < toks.len() && !matches!(toks[j].tok, Tok::RParen | Tok::Eof) {
                        if let Tok::Ident(p) = &toks[j].tok {
                            params.push(p.clone());
                        }
                        j += 1;
                    }
                    d.decl_names.insert(i);
                    let id = d.scopes.len();
                    d.scopes
                        .push((params.iter().cloned().collect(), HashSet::new()));
                    if !d.symbols.functions.iter().any(|f| &f.name == name) {
                        d.symbols.functions.push(FunctionSymbol {
                            name: name.clone(),
                            params,
                            exported,
                            offset: toks[i].span.start as usize,
                        });
                    }
                    for o in &mut d.owner[i..=j.min(toks.len() - 1)] {
                        *o = Some(id);
                    }
                    let after = next_significant(toks, j + 1);
                    let is_definition =
                        after.is_some_and(|a| matches!(toks[a].tok, Tok::Kw(Keyword::Begin)));
                    current = is_definition.then_some(id);
                    i = j + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    d
}

/// Whether the tokens at `i` are `NAME (`: a function header.
fn starts_header(toks: &[Token], i: usize) -> bool {
    next_significant(toks, i).is_some_and(|n| {
        matches!(toks[n].tok, Tok::Ident(_))
            && next_significant(toks, n + 1).is_some_and(|p| matches!(toks[p].tok, Tok::LParen))
    })
}

/// Walks a `LOCAL a, b := expr, c;` list starting after `LOCAL`, calling
/// `declare` for every declared name. Returns the index after the list.
fn declare_list(
    toks: &[Token],
    mut i: usize,
    declare: &mut dyn FnMut(&str),
    owner: &mut [Option<usize>],
    current: Option<usize>,
) -> usize {
    let mut nest = 0i32;
    let mut expect_name = true;
    while i < toks.len() {
        owner[i] = current;
        match &toks[i].tok {
            Tok::Semi | Tok::Eof if nest == 0 => return i,
            Tok::Kw(k) if block_delta(*k) != 0 => return i,
            Tok::LParen | Tok::LBrace | Tok::LBracket => nest += 1,
            Tok::RParen | Tok::RBrace | Tok::RBracket => nest -= 1,
            Tok::Comma if nest == 0 => {
                expect_name = true;
                i += 1;
                continue;
            }
            Tok::Ident(n) if nest == 0 && expect_name => declare(n),
            Tok::Comment => {
                i += 1;
                continue;
            }
            _ => {}
        }
        expect_name = false;
        i += 1;
    }
    i
}

/// Byte offsets of the openers whose literal never closed.
fn unclosed_literals(diags: &Diagnostics) -> HashSet<usize> {
    diags
        .items
        .iter()
        .filter(|d| matches!(d.code, "unclosed-string" | "unclosed-quote"))
        .map(|d| d.span.start as usize)
        .collect()
}

fn push(spans: &mut Vec<HighlightSpan>, start: usize, end: usize, kind: TokenKind) {
    if end > start {
        spans.push(HighlightSpan { start, end, kind });
    }
}

/// Splits a string literal into body and escape spans.
fn push_string(spans: &mut Vec<HighlightSpan>, src: &str, start: usize, end: usize, invalid: bool) {
    if invalid {
        push(spans, start, end, TokenKind::InvalidString);
        return;
    }
    let text = &src[start..end];
    let mut run = start;
    let mut chars = text.char_indices().peekable();
    // Skip the opening quote: it is part of the string run.
    chars.next();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            if let Some(&(j, esc)) = chars.peek() {
                let at = start + i;
                push(spans, run, at, TokenKind::String);
                let esc_end = start + j + esc.len_utf8();
                push(spans, at, esc_end, TokenKind::StringEscape);
                run = esc_end;
                chars.next();
            }
        }
    }
    push(spans, run, end, TokenKind::String);
}

/// Classifies every byte of `src` and collects brackets and symbols.
pub fn analyze(src: &str) -> SourceAnalysis {
    let mut diags = Diagnostics::default();
    let mut toks = lex_with_trivia(src, &mut diags);
    ppl_syntax::parser::resolve_soft_keywords(&mut toks);
    let unclosed = unclosed_literals(&diags);
    let invalid_chars: Vec<(usize, usize)> = diags
        .items
        .iter()
        .filter(|d| d.code == "invalid-character")
        .map(|d| (d.span.start as usize, d.span.end as usize))
        .collect();
    let decls = declarations(&toks);
    let fn_names: HashMap<&str, ()> = decls
        .symbols
        .functions
        .iter()
        .map(|f| (f.name.as_str(), ()))
        .collect();
    let globals: HashSet<&str> = decls.symbols.globals.iter().map(String::as_str).collect();

    let mut out = SourceAnalysis {
        spans: Vec::with_capacity(toks.len() * 2),
        ..Default::default()
    };
    let mut open_stack: Vec<(usize, char)> = Vec::new();
    let mut cursor = 0usize;
    let mut bad = invalid_chars.into_iter().peekable();

    let mut gap = |spans: &mut Vec<HighlightSpan>, from: usize, to: usize| {
        let mut at = from;
        while let Some(&(s, e)) = bad.peek() {
            if s >= to {
                break;
            }
            bad.next();
            if s < at {
                continue;
            }
            push(spans, at, s, TokenKind::Plain);
            push(spans, s, e.min(to), TokenKind::Error);
            at = e.min(to);
        }
        push(spans, at, to, TokenKind::Plain);
    };

    for (i, t) in toks.iter().enumerate() {
        if matches!(t.tok, Tok::Eof) {
            break;
        }
        let start = (t.span.start as usize).min(src.len());
        let end = (t.span.end as usize).clamp(start, src.len());
        if start < cursor {
            continue;
        }
        gap(&mut out.spans, cursor, start);
        cursor = end;
        let kind = match &t.tok {
            Tok::Kw(k) => match k {
                Keyword::And
                | Keyword::Or
                | Keyword::Xor
                | Keyword::Not
                | Keyword::Mod
                | Keyword::Div => TokenKind::OperatorKeyword,
                _ => TokenKind::Keyword,
            },
            Tok::Number(_) => TokenKind::Number,
            Tok::HashInt(body) if body.is_empty() => TokenKind::Error,
            Tok::HashInt(_) => TokenKind::Number,
            Tok::Str(_) => {
                push_string(&mut out.spans, src, start, end, unclosed.contains(&start));
                continue;
            }
            Tok::Quoted(_) if unclosed.contains(&start) => TokenKind::InvalidString,
            Tok::Quoted(_) => TokenKind::Symbolic,
            Tok::Comment => TokenKind::Comment,
            Tok::Pragma(_) => TokenKind::Pragma,
            Tok::CasBlock(_) => TokenKind::CasBlock,
            Tok::Ident(name) => classify_ident(name, i, &toks, &decls, &fn_names, &globals),
            Tok::LParen | Tok::LBrace | Tok::LBracket => {
                let ch = src[start..].chars().next().unwrap_or('(');
                open_stack.push((start, ch));
                out.bracket_pairs.push((start, None));
                TokenKind::Bracket
            }
            Tok::RParen | Tok::RBrace | Tok::RBracket => {
                let want = match t.tok {
                    Tok::RParen => '(',
                    Tok::RBrace => '{',
                    _ => '[',
                };
                match open_stack.iter().rposition(|&(_, c)| c == want) {
                    // Brackets opened after the partner are left unbalanced.
                    Some(pos) => {
                        let (open, _) = open_stack[pos];
                        open_stack.truncate(pos);
                        if let Some(p) = out.bracket_pairs.iter_mut().rev().find(|p| p.0 == open) {
                            p.1 = Some(start);
                        }
                    }
                    None => out.stray_closers.push(start),
                }
                TokenKind::Bracket
            }
            Tok::Comma | Tok::Semi => TokenKind::Punctuation,
            _ => TokenKind::Operator,
        };
        push(&mut out.spans, start, end, kind);
    }
    gap(&mut out.spans, cursor, src.len());
    out.symbols = decls.symbols;
    out
}

fn classify_ident(
    name: &str,
    i: usize,
    toks: &[Token],
    decls: &Declarations,
    fn_names: &HashMap<&str, ()>,
    globals: &HashSet<&str>,
) -> TokenKind {
    if decls.decl_names.contains(&i) {
        return TokenKind::UserFunction;
    }
    if is_constant(name) {
        return TokenKind::Constant;
    }
    if let Some(f) = decls.owner[i] {
        let (params, locals) = &decls.scopes[f];
        if params.contains(name) {
            return TokenKind::Parameter;
        }
        if locals.contains(name) {
            return TokenKind::Variable;
        }
    }
    let is_call = next_significant(toks, i + 1).is_some_and(|n| matches!(toks[n].tok, Tok::LParen));
    if fn_names.contains_key(name) {
        return TokenKind::UserFunction;
    }
    if globals.contains(name) {
        return TokenKind::Variable;
    }
    if crate::is_system_variable(name) {
        return TokenKind::SystemVariable;
    }
    match command_kind(name) {
        // A bare lower-case CAS name that is not called is far more likely
        // a variable (`count`, `e`, `id`) than a command reference.
        Some(TokenKind::CasCommand) if !is_call && !name.contains('.') => TokenKind::Variable,
        Some(k) => k,
        None => TokenKind::Variable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(&str, TokenKind)> {
        analyze(src)
            .spans
            .iter()
            .filter(|s| s.kind != TokenKind::Plain)
            .map(|s| (&src[s.start..s.end], s.kind))
            .collect()
    }

    fn kind_of(src: &str, text: &str) -> TokenKind {
        kinds(src)
            .into_iter()
            .find(|(t, _)| *t == text)
            .unwrap_or_else(|| panic!("no span {text:?} in {:?}", kinds(src)))
            .1
    }

    fn assert_full_coverage(src: &str) {
        let a = analyze(src);
        let mut at = 0;
        for s in &a.spans {
            assert_eq!(s.start, at, "gap or overlap at {at} in {src:?}");
            assert!(s.end > s.start);
            assert!(src.is_char_boundary(s.start) && src.is_char_boundary(s.end));
            at = s.end;
        }
        assert_eq!(at, src.len(), "coverage ends early in {src:?}");
    }

    const PROGRAM: &str = r#"#pragma mode( separator(.,;) integer(h32) )
EXPORT counter;
SQUARE(n);
EXPORT MAIN(a, b)
BEGIN
  LOCAL i, total := 0, lst := {1, [2, 3]};
  // sum of squares
  FOR i FROM 1 TO a DO
    total := total + SQUARE(i) MOD 7;
  END;
  IF total > b AND NOT 0 THEN PRINT("big\n"); END;
  counter := solve(x^2 = 4, x) + SOLVE(X^2-4, X, 1) + RANDINT(G1, 3);
  RETURN 'X+1' + #1Fh + 1.5E-3 + π;
END;
SQUARE(n)
BEGIN
  RETURN n * n;
END;
"#;

    #[test]
    fn classifies_every_construct() {
        let k = |t| kind_of(PROGRAM, t);
        assert_eq!(k("EXPORT"), TokenKind::Keyword);
        assert_eq!(k("BEGIN"), TokenKind::Keyword);
        assert_eq!(k("FOR"), TokenKind::Keyword);
        assert_eq!(k("MOD"), TokenKind::OperatorKeyword);
        assert_eq!(k("AND"), TokenKind::OperatorKeyword);
        assert_eq!(k("MAIN"), TokenKind::UserFunction);
        assert_eq!(k("SQUARE"), TokenKind::UserFunction);
        assert_eq!(k("a"), TokenKind::Parameter);
        assert_eq!(k("total"), TokenKind::Variable);
        assert_eq!(k("counter"), TokenKind::Variable);
        assert_eq!(k("PRINT"), TokenKind::Command);
        assert_eq!(k("SOLVE"), TokenKind::Command);
        assert_eq!(k("RANDINT"), TokenKind::Command);
        assert_eq!(k("solve"), TokenKind::CasCommand);
        assert_eq!(k("G1"), TokenKind::SystemVariable);
        assert_eq!(k("X"), TokenKind::SystemVariable);
        assert_eq!(k("π"), TokenKind::Constant);
        assert_eq!(k("#1Fh"), TokenKind::Number);
        assert_eq!(k("1.5E-3"), TokenKind::Number);
        assert_eq!(k("'X+1'"), TokenKind::Symbolic);
        assert_eq!(k("// sum of squares"), TokenKind::Comment);
        assert_eq!(k("\\n"), TokenKind::StringEscape);
        assert_eq!(k("\"big"), TokenKind::String);
        assert_eq!(k(":="), TokenKind::Operator);
        assert_eq!(k("{"), TokenKind::Bracket);
        assert_eq!(k(";"), TokenKind::Punctuation);
        assert_eq!(
            k("#pragma mode( separator(.,;) integer(h32) )"),
            TokenKind::Pragma
        );
        assert_full_coverage(PROGRAM);
    }

    #[test]
    fn local_and_key_headers_are_functions() {
        let src = "LOCAL HELPER(a)\nBEGIN\n  RETURN a;\nEND;\nLOCAL v := HELPER(1);\nKEY K_On()\nBEGIN\nEND;\n";
        let a = analyze(src);
        let names: Vec<_> = a
            .symbols
            .functions
            .iter()
            .map(|f| (f.name.as_str(), f.exported))
            .collect();
        assert_eq!(names, vec![("HELPER", false), ("K_On", false)]);
        assert_eq!(a.symbols.globals, vec!["v"]);
        let a_kinds: Vec<_> = kinds(src)
            .into_iter()
            .filter(|(t, _)| *t == "a")
            .map(|k| k.1)
            .collect();
        assert_eq!(a_kinds, vec![TokenKind::Parameter, TokenKind::Parameter]);
    }

    #[test]
    fn parameters_are_scoped_to_their_function() {
        // `n` is a parameter of SQUARE only; in MAIN it would be a variable.
        let src = "F(n)\nBEGIN\n  RETURN n;\nEND;\nG()\nBEGIN\n  RETURN n;\nEND;\n";
        let spans: Vec<_> = kinds(src).into_iter().filter(|(t, _)| *t == "n").collect();
        assert_eq!(
            spans.iter().map(|s| s.1).collect::<Vec<_>>(),
            vec![
                TokenKind::Parameter,
                TokenKind::Parameter,
                TokenKind::Variable
            ]
        );
    }

    #[test]
    fn symbols_collect_functions_params_and_locals() {
        let a = analyze(PROGRAM);
        let main = a
            .symbols
            .functions
            .iter()
            .find(|f| f.name == "MAIN")
            .unwrap();
        assert_eq!(main.params, vec!["a", "b"]);
        assert!(main.exported);
        assert!(
            a.symbols
                .functions
                .iter()
                .any(|f| f.name == "SQUARE" && !f.exported)
        );
        assert_eq!(a.symbols.globals, vec!["counter"]);
        assert_eq!(a.symbols.locals, vec!["i", "total", "lst"]);
    }

    #[test]
    fn unclosed_string_does_not_swallow_later_lines() {
        let src = "PRINT(\"oops);\nPRINT(2);\nEND;\n";
        let k = kinds(src);
        assert_eq!(k[2], ("\"oops);", TokenKind::InvalidString));
        // Line 2 is classified normally.
        assert!(k.contains(&("2", TokenKind::Number)));
        assert!(k.contains(&("END", TokenKind::Keyword)));
        assert_full_coverage(src);
    }

    #[test]
    fn closed_multiline_string_is_one_string() {
        let src = "PRINT(\"a\nb\"); x := 1;";
        assert_eq!(kind_of(src, "\"a\nb\""), TokenKind::String);
        assert_eq!(kind_of(src, "1"), TokenKind::Number);
    }

    #[test]
    fn comment_markers_inside_strings_and_escaped_quotes() {
        let src = "PRINT(\"a//b \\\" c\"); // real\n";
        let k = kinds(src);
        assert!(k.contains(&("\\\"", TokenKind::StringEscape)), "{k:?}");
        assert!(k.contains(&("// real", TokenKind::Comment)), "{k:?}");
        assert!(
            !k.iter()
                .any(|(t, kind)| *kind == TokenKind::Comment && t.contains("//b"))
        );
    }

    #[test]
    fn invalid_characters_and_malformed_integers_are_errors() {
        let src = "x := 1 @ 2 + #;";
        assert_eq!(kind_of(src, "@"), TokenKind::Error);
        assert_eq!(kind_of(src, "#"), TokenKind::Error);
        assert_full_coverage(src);
    }

    #[test]
    fn brackets_pair_across_nesting_and_ignore_strings() {
        let src = "F({1, [2, \")\"], (3)})";
        let a = analyze(src);
        // '(' at 1 closes at the last ')'.
        assert_eq!(a.bracket_partner(1), Some((1, Some(src.len() - 1))));
        // '[' pairs with ']' (the ')' inside the string is not a bracket).
        let open = src.find('[').unwrap();
        let close = src.find(']').unwrap();
        assert_eq!(a.bracket_partner(close), Some((close, Some(open))));
        let b = analyze("F((1)");
        assert_eq!(b.bracket_partner(1), Some((1, None)));
        let c = analyze("x)");
        assert_eq!(c.bracket_partner(1), Some((1, None)));
    }

    #[test]
    fn soft_keywords_are_coloured_as_the_parser_reads_them() {
        let k = kinds("export F()\nbegin\n local begin:=4;\n return begin+1;\nend;");
        assert_eq!(k[0], ("export", TokenKind::Keyword));
        assert_eq!(k[4], ("begin", TokenKind::Keyword));
        assert_eq!(k[5], ("local", TokenKind::Keyword));
        assert_eq!(k[6], ("begin", TokenKind::Variable));
        assert_eq!(k[11], ("begin", TokenKind::Variable));
    }

    #[test]
    fn lowercase_cas_names_only_when_called() {
        let src = "LOCAL r := count + count({1,2}, 1);";
        let k = kinds(src);
        assert_eq!(k[3], ("count", TokenKind::Variable));
        assert_eq!(k[5], ("count", TokenKind::CasCommand));
    }

    #[test]
    fn fixtures_and_their_prefixes_are_fully_covered() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let Ok((file, _)) = ppl_syntax::read_source_file(&path) else {
                continue;
            };
            seen += 1;
            assert_full_coverage(&file.text);
            // Incomplete code: every truncation point on a char boundary.
            let step = (file.text.len() / 200).max(1);
            for cut in (0..file.text.len()).step_by(step) {
                if file.text.is_char_boundary(cut) {
                    assert_full_coverage(&file.text[..cut]);
                }
            }
        }
        assert!(seen >= 5, "fixtures not found in {dir}");
    }
}
