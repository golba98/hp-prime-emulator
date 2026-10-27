//! Recursive-descent parser with precedence climbing for expressions.
//!
//! Operator precedence, lowest first:
//!
//! | level | operators |
//! |---|---|
//! | 1 | `:=` (right), `▶` (left) |
//! | 2 | `OR`, `XOR` |
//! | 3 | `AND` |
//! | 4 | `NOT` (prefix) |
//! | 5 | `=` `==` `≠` `<>` `<` `≤` `>` `≥` |
//! | 6 | `+` `-` |
//! | 7 | `*` `/` `MOD` `DIV` |
//! | 8 | unary `-` |
//! | 9 | `^` (right) |
//! | 10 | postfix `!` `²` `³`, calls `f(…)`, indexing `x[…]`, prefix `√` |
//!
//! Levels 2–4 and the position of `MOD`/`DIV` follow the HP Prime User
//! Guide's operator table and are **unverified** against the G1.

use crate::ast::*;
use crate::diag::{Category, Diagnostic, Diagnostics};
use crate::lexer::{Keyword as K, Tok, Token, lex};
use crate::source::Span;

pub fn parse(src: &str) -> (Program, Diagnostics) {
    let mut diags = Diagnostics::default();
    let tokens = lex(src, &mut diags);
    let mut p = Parser {
        toks: tokens,
        pos: 0,
        diags,
        prev_end: 0,
    };
    let mut program = p.program();
    // Compact existing lexer spans as tokens are released, rather than
    // allocating metadata throughout recursive parsing or keeping strings.
    program.punctuation = p
        .toks
        .into_iter()
        .filter_map(|token| {
            let kind = match token.tok {
                Tok::LParen | Tok::LBrace | Tok::LBracket => PunctuationKind::Open,
                Tok::RParen | Tok::RBrace | Tok::RBracket => PunctuationKind::Close,
                Tok::Comma => PunctuationKind::Comma,
                _ => return None,
            };
            Some(Punctuation {
                kind,
                span: token.span,
            })
        })
        .collect();
    let mut diags = p.diags;
    diags.sort();
    (program, diags)
}

/// Applies the parser's reading of soft keywords (see [`Token::soft`]) to
/// `tokens`, which may include comments: each soft keyword the parser reads
/// as a name becomes [`Tok::Ident`], so highlighting and indentation agree
/// with the analyzer.
pub fn resolve_soft_keywords(tokens: &mut [Token]) {
    if tokens.iter().all(|t| t.soft.is_none()) {
        return;
    }
    let code: Vec<usize> = (0..tokens.len())
        .filter(|&i| tokens[i].tok != Tok::Comment)
        .collect();
    let mut p = Parser {
        toks: code.iter().map(|&i| tokens[i].clone()).collect(),
        pos: 0,
        diags: Diagnostics::default(),
        prev_end: 0,
    };
    p.program();
    for (resolved, &i) in p.toks.into_iter().zip(&code) {
        if let (Tok::Ident(name), Some(_)) = (resolved.tok, &tokens[i].soft) {
            tokens[i].tok = Tok::Ident(name);
            tokens[i].soft = None;
        }
    }
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    diags: Diagnostics,
    prev_end: u32,
}

/// Keywords that end a statement list.
const BLOCK_END: &[K] = &[K::End, K::Else, K::Until, K::Then, K::Default];

impl Parser {
    // ---- token helpers ------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, k: usize) -> &Tok {
        let i = (self.pos + k).min(self.toks.len() - 1);
        &self.toks[i].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn prev_span(&self) -> Span {
        Span {
            start: self.prev_end,
            end: self.prev_end,
        }
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        self.prev_end = t.span.end;
        t
    }

    fn at_kw(&self, k: K) -> bool {
        matches!(self.peek(), Tok::Kw(x) if *x == k)
    }

    /// The token at `pos + k` is a soft keyword (a keyword spelled in other
    /// than upper case; see [`Token::soft`]).
    fn soft_at(&self, k: usize) -> bool {
        let i = (self.pos + k).min(self.toks.len() - 1);
        self.toks[i].soft.is_some()
    }

    /// The current token as written, for messages: `div`, not `DIV`.
    fn found(&self) -> String {
        match &self.toks[self.pos].soft {
            Some(spelling) => format!("`{spelling}`"),
            None => self.peek().describe(),
        }
    }

    /// Reads the current token as a name when it is a soft keyword. The
    /// original firmware accepts `LOCAL begin`, `RETURN end+1` and
    /// `end:=5` (diagnostic corpus k08-k23, k36).
    fn soft_to_ident(&mut self) {
        let t = &mut self.toks[self.pos];
        if let Some(name) = t.soft.take() {
            t.tok = Tok::Ident(name);
        }
    }

    /// The current soft keyword is used as a value: assigned, or followed by
    /// an operator that needs a left operand (`end+1`, `default:=2`). A
    /// soft END before `;` still closes its block (k35).
    fn soft_operand(&self) -> bool {
        self.soft_at(0)
            && matches!(
                self.peek_at(1),
                Tok::Assign
                    | Tok::Plus
                    | Tok::Star
                    | Tok::Slash
                    | Tok::Caret
                    | Tok::Eq
                    | Tok::EqEq
                    | Tok::Ne
                    | Tok::Lt
                    | Tok::Le
                    | Tok::Gt
                    | Tok::Ge
                    | Tok::Store
                    | Tok::LBracket
                    | Tok::Squared
                    | Tok::Cubed
                    | Tok::Bang
            )
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.advance();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: K) -> bool {
        if self.at_kw(k) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::error(Category::Parser, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    fn warn(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::warning(Category::Parser, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    fn ident(&mut self, what: &str) -> Option<Ident> {
        self.soft_to_ident();
        match self.peek().clone() {
            Tok::Ident(name) => {
                let span = self.advance().span;
                Some(Ident { name, span })
            }
            Tok::Number(n) => {
                // `EXPORT 2TEST()`: digits glued to a name.
                let span = self.span();
                let mut name = n;
                self.advance();
                if let Tok::Ident(rest) = self.peek().clone() {
                    name.push_str(&rest);
                    self.advance();
                }
                self.diags.push(
                    Diagnostic::error(
                        Category::Lexer,
                        "invalid-identifier",
                        span.to(self.prev_span()),
                        format!("invalid identifier `{name}`"),
                    )
                    .with_hint("identifiers must start with a letter or `_`"),
                );
                Some(Ident { name, span })
            }
            other => {
                let span = self.span();
                self.error(
                    "expected-identifier",
                    span,
                    format!("expected {what}, found {}", other.describe()),
                );
                None
            }
        }
    }

    /// Skips to the next `;` (consumed) or block keyword.
    fn recover(&mut self) {
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Semi => {
                    self.advance();
                    return;
                }
                Tok::Kw(k) if BLOCK_END.contains(k) || *k == K::Export => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // ---- program structure -------------------------------------------

    fn program(&mut self) -> Program {
        let mut items = Vec::new();
        let mut pragmas = Vec::new();
        loop {
            let start = self.span();
            match self.peek().clone() {
                Tok::Eof => break,
                Tok::Semi => {
                    self.advance();
                }
                Tok::Pragma(text) => {
                    self.advance();
                    pragmas.push((text, start));
                }
                Tok::CasBlock(text) => {
                    self.advance();
                    items.push(Item::CasBlock { text, span: start });
                }
                Tok::Kw(K::Export) => {
                    self.advance();
                    self.header_or_globals(true, false, start, &mut items);
                }
                Tok::Kw(K::Local) => {
                    self.advance();
                    self.header_or_globals(false, true, start, &mut items);
                }
                Tok::Kw(K::Key) => {
                    self.advance();
                    if let Some(name) = self.ident("key handler name") {
                        let params = self.param_list();
                        if let Some(f) = self.function_body(name, params, false, false, start) {
                            items.push(Item::Function(FunctionDef { is_key: true, ..f }));
                        }
                    }
                }
                Tok::Ident(_) if *self.peek_at(1) == Tok::LParen => {
                    let name = self.ident("function name").unwrap();
                    let params = self.param_list();
                    self.after_header(name, params, false, false, start, &mut items);
                }
                Tok::Kw(K::Return) => {
                    let span = self.span();
                    self.error("return-outside-function", span, "RETURN outside a function");
                    self.advance();
                    self.recover();
                }
                Tok::Kw(K::Begin) => {
                    let span = self.span();
                    self.error("invalid-top-level", span, "BEGIN block outside a function")
                        .hint = Some(
                        "a program needs a header such as `EXPORT Name()` before BEGIN".into(),
                    );
                    self.advance();
                    let _ = self.stmts_until(&[K::End]);
                    self.eat_kw(K::End);
                }
                Tok::Kw(K::End) => {
                    // Firmware consumes an unclaimed END before reporting
                    // its error (usually at the following semicolon).
                    self.advance();
                    self.error(
                        "invalid-top-level",
                        self.span(),
                        "unexpected END outside a block",
                    );
                    self.recover();
                }
                other => {
                    let span = self.span();
                    self.error("invalid-top-level", span, format!("invalid top-level token: {}", other.describe()))
                        .hint = Some("only function definitions and LOCAL/EXPORT variables may appear at file level".into());
                    // Progress guarantee: `recover()` returns without
                    // consuming at block-end keywords (an enclosing
                    // construct normally eats them), but at top level
                    // there is no enclosing construct — an END token here
                    // would otherwise loop forever, pushing one diagnostic
                    // per iteration (found via a real hpcalc.org program).
                    if matches!(self.peek(), Tok::Eof) {
                        break;
                    }
                    self.advance();
                    self.recover();
                }
            }
        }
        Program {
            items,
            pragmas,
            punctuation: Vec::new(),
        }
    }

    fn header_or_globals(
        &mut self,
        exported: bool,
        local: bool,
        start: Span,
        items: &mut Vec<Item>,
    ) {
        self.soft_to_ident();
        let is_header = matches!(self.peek(), Tok::Ident(_) | Tok::Number(_))
            && (*self.peek_at(1) == Tok::LParen
                || (matches!(self.peek(), Tok::Number(_))
                    && matches!(self.peek_at(1), Tok::Ident(_))));
        if is_header {
            let Some(name) = self.ident("function name") else {
                return self.recover();
            };
            if *self.peek() != Tok::LParen {
                // `EXPORT 2TEST()` lexes as number + identifier.
                if !self.eat(&Tok::LParen) {
                    return self.recover();
                }
            }
            let params = self.param_list();
            self.after_header(name, params, exported, local, start, items);
            return;
        }
        // Variables: `EXPORT a:=1, b;`
        let mut vars = Vec::new();
        loop {
            let Some(name) = self.ident("variable name") else {
                return self.recover();
            };
            if self.at_kw(K::Begin) && vars.is_empty() && exported {
                self.error(
                    "export-header-parens",
                    name.span,
                    format!(
                        "EXPORT header `{}` is missing its parameter list",
                        name.name
                    ),
                )
                .hint = Some(format!("write `EXPORT {}()`", name.name));
                if let Some(f) = self.function_body(name, Vec::new(), exported, local, start) {
                    items.push(Item::Function(f));
                }
                return;
            }
            let init = if self.eat(&Tok::Assign) {
                Some(self.expr())
            } else {
                None
            };
            vars.push(VarDecl { name, init });
            if self.eat(&Tok::Comma) {
                continue;
            }
            if !self.eat(&Tok::Semi) {
                let span = self.span();
                self.error(
                    "missing-semicolon",
                    span,
                    format!(
                        "expected `;` after variable declaration, found {}",
                        self.found()
                    ),
                );
                self.recover();
            }
            break;
        }
        items.push(Item::Globals {
            exported,
            vars,
            span: start.to(self.prev_span()),
        });
    }

    fn after_header(
        &mut self,
        name: Ident,
        params: Vec<Ident>,
        exported: bool,
        local: bool,
        start: Span,
        items: &mut Vec<Item>,
    ) {
        if self.at_kw(K::Begin) {
            if let Some(f) = self.function_body(name, params, exported, local, start) {
                items.push(Item::Function(f));
            }
            return;
        }
        if *self.peek() == Tok::Semi {
            let semi = self.advance().span;
            if self.at_kw(K::Begin) {
                self.error(
                    "export-header-semicolon",
                    semi,
                    "unexpected `;` after function header",
                )
                .hint = Some("remove the `;` between the header and BEGIN".into());
                if let Some(f) = self.function_body(name, params, exported, local, start) {
                    items.push(Item::Function(f));
                }
                return;
            }
            items.push(Item::ForwardDecl {
                name,
                params,
                span: start.to(semi),
            });
            return;
        }
        let span = self.span();
        self.error(
            "expected-begin",
            span,
            format!(
                "expected BEGIN after function header, found {}",
                self.found()
            ),
        );
        self.recover();
    }

    fn param_list(&mut self) -> Vec<Ident> {
        let mut params = Vec::new();
        if !self.eat(&Tok::LParen) {
            let span = self.span();
            self.error("expected-lparen", span, "expected `(` after function name");
            return params;
        }
        if self.eat(&Tok::RParen) {
            return params;
        }
        while let Some(p) = self.ident("parameter name") {
            params.push(p);
            if self.eat(&Tok::Comma) {
                continue;
            }
            if !self.eat(&Tok::RParen) {
                let span = self.span();
                self.error(
                    "expected-rparen",
                    span,
                    "expected `)` to close the parameter list",
                );
            }
            break;
        }
        params
    }

    fn function_body(
        &mut self,
        name: Ident,
        params: Vec<Ident>,
        exported: bool,
        local: bool,
        start: Span,
    ) -> Option<FunctionDef> {
        let begin = self.span();
        if !self.eat_kw(K::Begin) {
            self.error("expected-begin", begin, "expected BEGIN");
            return None;
        }
        let body = self.stmts_until(&[K::End]);
        if self.eat_kw(K::End) {
            // The original firmware rejects a function END that is not
            // followed by `;`, at the next token (diagnostic corpus h09, h10).
            if *self.peek() != Tok::Semi {
                let at = self.span();
                self.error(
                    "missing-semicolon",
                    at,
                    format!(
                        "missing semicolon: expected `;` after the END of `{}`",
                        name.name
                    ),
                )
                .hint = Some("write END; to close a function".into());
            }
        } else {
            let span = self.span();
            self.unclosed(begin, "BEGIN", span);
        }
        Some(FunctionDef {
            name,
            params,
            body,
            exported,
            is_key: false,
            is_local: local,
            span: start.to(self.prev_span()),
        })
    }

    fn unclosed(&mut self, open: Span, what: &str, at: Span) {
        let found = self.found();
        self.error(
            "unclosed-block",
            at,
            format!("missing END: unclosed {what} block (found {found})"),
        )
        .hint = Some(format!("add END; to close the {what} at {}", open.start));
    }

    // ---- statements ---------------------------------------------------

    fn stmts_until(&mut self, terms: &[K]) -> Vec<Stmt> {
        let mut out = Vec::new();
        loop {
            while self.eat(&Tok::Semi) {}
            if self.soft_operand() {
                self.soft_to_ident();
            }
            match self.peek() {
                Tok::Eof => break,
                Tok::Kw(k) if terms.contains(k) || BLOCK_END.contains(k) => break,
                Tok::Kw(K::Export) => break,
                _ => {}
            }
            let before = self.pos;
            let stmt = self.statement();
            let simple = matches!(
                stmt.kind,
                StmtKind::Expr(_)
                    | StmtKind::Local(_)
                    | StmtKind::Return(_)
                    | StmtKind::Break(_)
                    | StmtKind::Continue
                    | StmtKind::Kill
            );
            let stmt_span = stmt.span;
            let return_with_value = matches!(stmt.kind, StmtKind::Return(Some(_)));
            out.push(stmt);
            if self.pos == before {
                // No progress: skip the offending token.
                self.advance();
                continue;
            }
            match self.peek() {
                Tok::Semi => {}
                // A value RETURN needs `;` before END (h03, h13), but an
                // empty RETURN may omit it (location probes r01, r05).
                Tok::Kw(K::End) if return_with_value => {
                    let at = self.span();
                    self.error(
                        "missing-semicolon",
                        at,
                        "missing semicolon: expected `;` after RETURN before `END`",
                    );
                }
                Tok::Kw(k) if terms.contains(k) || BLOCK_END.contains(k) => {
                    if simple {
                        let span = Span {
                            start: stmt_span.end,
                            end: stmt_span.end,
                        };
                        self.warn(
                            "missing-semicolon-before-end",
                            span,
                            format!("missing semicolon before {}", self.found()),
                        )
                        .unverified = true;
                    }
                }
                Tok::Eof => {}
                other => {
                    let msg = format!("missing semicolon: expected `;` before {}", self.found());
                    match other {
                        Tok::RParen | Tok::RBrace | Tok::RBracket => {
                            let s = self.span();
                            let d = other.describe();
                            self.error(
                                "unexpected-token",
                                s,
                                format!("extra {d}: no matching opening delimiter"),
                            );
                        }
                        _ => {
                            self.error("missing-semicolon", self.span(), msg);
                        }
                    }
                    self.recover();
                }
            }
        }
        out
    }

    fn statement(&mut self) -> Stmt {
        // `end:=5`: a soft keyword being assigned is a name (k36).
        if *self.peek_at(1) == Tok::Assign {
            self.soft_to_ident();
        }
        let start = self.span();
        let kind = match self.peek().clone() {
            Tok::Kw(K::Begin) => {
                self.advance();
                let body = self.stmts_until(&[K::End]);
                if !self.eat_kw(K::End) {
                    let at = self.span();
                    self.unclosed(start, "BEGIN", at);
                }
                StmtKind::Block(body)
            }
            Tok::Kw(K::Local) => {
                self.advance();
                self.local_decl(start)
            }
            Tok::Kw(K::If) => self.if_stmt(start),
            Tok::Kw(K::Case) => self.case_stmt(start),
            Tok::Kw(K::Iferr) => self.iferr_stmt(start),
            Tok::Kw(K::For) => self.for_stmt(start),
            Tok::Kw(K::While) => {
                self.advance();
                let cond = self.expr();
                if !self.eat_kw(K::Do) {
                    let span = self.span();
                    self.error(
                        "expected-do",
                        span,
                        "malformed WHILE: expected DO after the condition",
                    );
                }
                let body = self.stmts_until(&[K::End]);
                self.close_end(start, "WHILE");
                StmtKind::While { cond, body }
            }
            Tok::Kw(K::Repeat) => {
                self.advance();
                let body = self.stmts_until(&[K::Until]);
                if !self.eat_kw(K::Until) {
                    let span = self.span();
                    let found = self.found();
                    self.error(
                        "expected-until",
                        span,
                        format!("malformed REPEAT: expected UNTIL, found {found}"),
                    )
                    .hint = Some("REPEAT … UNTIL condition; is closed by UNTIL, not END".into());
                    if self.at_kw(K::End) {
                        self.advance();
                    }
                    return Stmt {
                        kind: StmtKind::Repeat {
                            body,
                            until: Expr {
                                kind: ExprKind::Error,
                                span,
                            },
                        },
                        span: start,
                    };
                }
                let until = self.expr();
                StmtKind::Repeat { body, until }
            }
            Tok::Kw(K::Return) => {
                self.advance();
                if self.stmt_ends() {
                    StmtKind::Return(None)
                } else {
                    StmtKind::Return(Some(self.expr()))
                }
            }
            Tok::Kw(K::Break) => {
                self.advance();
                if self.stmt_ends() {
                    StmtKind::Break(None)
                } else {
                    StmtKind::Break(Some(self.expr()))
                }
            }
            Tok::Kw(K::Continue) => {
                self.advance();
                StmtKind::Continue
            }
            Tok::Kw(K::Kill) => {
                self.advance();
                StmtKind::Kill
            }
            Tok::Kw(k @ (K::Then | K::Do | K::Step | K::To | K::From | K::Downto)) => {
                let span = self.span();
                self.error(
                    "unexpected-keyword",
                    span,
                    format!("unexpected `{}`", k.as_str()),
                );
                self.advance();
                StmtKind::Expr(Expr {
                    kind: ExprKind::Error,
                    span,
                })
            }
            _ => {
                let e = self.expr();
                if let ExprKind::Binary(BinOp::Eq, lhs, _) = &e.kind {
                    if matches!(
                        lhs.kind,
                        ExprKind::Ident(_) | ExprKind::Call(..) | ExprKind::Index(..)
                    ) {
                        self.diags.push(
                            Diagnostic::warning(Category::Parser, "equals-statement", e.span, "invalid assignment: `=` compares values and the result is discarded")
                                .with_hint("did you mean ':='?")
                                .unverified(),
                        );
                    }
                }
                StmtKind::Expr(e)
            }
        };
        Stmt {
            kind,
            span: start.to(self.prev_span()),
        }
    }

    fn stmt_ends(&self) -> bool {
        matches!(self.peek(), Tok::Semi | Tok::Eof)
            || matches!(self.peek(), Tok::Kw(k) if BLOCK_END.contains(k)) && !self.soft_operand()
    }

    fn close_end(&mut self, open: Span, what: &str) {
        if !self.eat_kw(K::End) {
            let at = self.span();
            self.unclosed(open, what, at);
        }
    }

    fn local_decl(&mut self, start: Span) -> StmtKind {
        let mut vars = Vec::new();
        loop {
            // MOD may be declared even in upper case, but expression
            // grammar still treats it as an operator (probes m01-m06).
            if self.at_kw(K::Mod) && !self.soft_at(0) {
                self.toks[self.pos].tok = Tok::Ident(K::Mod.as_str().into());
            }
            let Some(name) = self.ident("variable name") else {
                self.recover();
                break;
            };
            if *self.peek() == Tok::LParen && vars.is_empty() {
                self.error(
                    "nested-function",
                    name.span,
                    "functions cannot be declared inside another function",
                )
                .hint = Some("move this LOCAL function to file level".into());
                self.param_list();
                if self.at_kw(K::Begin) {
                    self.advance();
                    let _ = self.stmts_until(&[K::End]);
                    self.eat_kw(K::End);
                }
                return StmtKind::Local(vars);
            }
            let init = if self.eat(&Tok::Assign) {
                Some(self.expr())
            } else {
                None
            };
            vars.push(VarDecl { name, init });
            if self.eat(&Tok::Comma) {
                continue;
            }
            if let Tok::Ident(_) = self.peek() {
                let span = self.span();
                self.error(
                    "local-missing-comma",
                    span,
                    "malformed LOCAL declaration: expected `,` between variable names",
                );
                continue;
            }
            break;
        }
        let _ = start;
        StmtKind::Local(vars)
    }

    fn if_stmt(&mut self, start: Span) -> StmtKind {
        self.advance();
        let cond = self.expr();
        if !self.eat_kw(K::Then) {
            let span = self.span();
            self.error(
                "expected-then",
                span,
                format!("malformed IF: expected THEN, found {}", self.found()),
            );
        }
        let then_body = self.stmts_until(&[K::Else, K::End]);
        let else_body = if self.eat_kw(K::Else) {
            Some(self.stmts_until(&[K::End]))
        } else {
            None
        };
        self.close_end(start, "IF");
        StmtKind::If {
            cond,
            then_body,
            else_body,
        }
    }

    fn case_stmt(&mut self, start: Span) -> StmtKind {
        self.advance();
        let mut arms = Vec::new();
        let mut default = None;
        loop {
            while self.eat(&Tok::Semi) {}
            if self.at_kw(K::If) {
                let arm_start = self.span();
                self.advance();
                let cond = self.expr();
                if !self.eat_kw(K::Then) {
                    let span = self.span();
                    self.error("expected-then", span, "malformed CASE arm: expected THEN");
                }
                let body = self.stmts_until(&[K::End]);
                self.close_end(arm_start, "IF");
                arms.push((cond, body));
            } else if self.eat_kw(K::Default) {
                default = Some(self.stmts_until(&[K::End]));
                break;
            } else {
                break;
            }
        }
        self.close_end(start, "CASE");
        StmtKind::Case { arms, default }
    }

    fn iferr_stmt(&mut self, start: Span) -> StmtKind {
        self.advance();
        let body = self.stmts_until(&[K::Then]);
        if !self.eat_kw(K::Then) {
            let span = self.span();
            self.error("expected-then", span, "malformed IFERR: expected THEN");
        }
        let handler = self.stmts_until(&[K::Else, K::End]);
        let else_body = if self.eat_kw(K::Else) {
            Some(self.stmts_until(&[K::End]))
        } else {
            None
        };
        self.close_end(start, "IFERR");
        StmtKind::IfErr {
            body,
            handler,
            else_body,
        }
    }

    fn for_stmt(&mut self, start: Span) -> StmtKind {
        self.advance();
        let var = self.ident("loop variable").unwrap_or(Ident {
            name: String::new(),
            span: start,
        });
        if self.eat(&Tok::Assign) {
            self.warn("for-assign-form", var.span, "`FOR var := start` form")
                .unverified = true;
        } else if !self.eat_kw(K::From) {
            let span = self.span();
            self.error(
                "expected-from",
                span,
                "malformed FOR: expected FROM after the loop variable",
            );
        }
        let from = self.expr();
        let mut step = None;
        if self.eat_kw(K::Step) {
            step = Some(self.expr());
        }
        let down = if self.eat_kw(K::To) {
            false
        } else if self.eat_kw(K::Downto) {
            true
        } else {
            let span = self.span();
            self.error("expected-to", span, "malformed FOR: expected TO or DOWNTO");
            false
        };
        let to = self.expr();
        if step.is_none() && self.eat_kw(K::Step) {
            step = Some(self.expr());
        }
        if !self.eat_kw(K::Do) {
            let span = self.span();
            let found = self.found();
            self.error(
                "expected-do",
                span,
                format!("malformed FOR: expected DO, found {found}"),
            );
            self.eat_kw(K::Begin);
        }
        let body = self.stmts_until(&[K::End]);
        self.close_end(start, "FOR");
        StmtKind::For {
            var,
            from,
            to,
            step,
            down,
            body,
        }
    }

    // ---- expressions --------------------------------------------------

    pub fn expr(&mut self) -> Expr {
        let lhs = self.or_expr();
        if *self.peek() == Tok::Assign {
            self.advance();
            let rhs = self.expr();
            let span = lhs.span.to(rhs.span);
            return Expr {
                kind: ExprKind::Assign(Box::new(lhs), Box::new(rhs)),
                span,
            };
        }
        let mut e = lhs;
        while *self.peek() == Tok::Store {
            self.advance();
            let target = self.or_expr();
            let span = e.span.to(target.span);
            e = Expr {
                kind: ExprKind::Store(Box::new(e), Box::new(target)),
                span,
            };
        }
        e
    }

    fn bin(l: Expr, op: BinOp, r: Expr) -> Expr {
        let span = l.span.to(r.span);
        Expr {
            kind: ExprKind::Binary(op, Box::new(l), Box::new(r)),
            span,
        }
    }

    fn or_expr(&mut self) -> Expr {
        let mut l = self.and_expr();
        loop {
            let op = match self.peek() {
                Tok::Kw(K::Or) => BinOp::Or,
                Tok::Kw(K::Xor) => BinOp::Xor,
                _ => return l,
            };
            self.advance();
            let r = self.and_expr();
            l = Self::bin(l, op, r);
        }
    }

    fn and_expr(&mut self) -> Expr {
        let mut l = self.not_expr();
        while self.at_kw(K::And) {
            self.advance();
            let r = self.not_expr();
            l = Self::bin(l, BinOp::And, r);
        }
        l
    }

    fn not_expr(&mut self) -> Expr {
        // Only upper-case NOT is the operator: the firmware reads `not` and
        // `Not` as names (k07, k16, k34).
        if self.at_kw(K::Not) && !self.soft_at(0) {
            let start = self.advance().span;
            let e = self.not_expr();
            let span = start.to(e.span);
            return Expr {
                kind: ExprKind::Unary(UnOp::Not, Box::new(e)),
                span,
            };
        }
        self.cmp_expr()
    }

    fn cmp_expr(&mut self) -> Expr {
        let mut l = self.add_expr();
        loop {
            let op = match self.peek() {
                Tok::Eq | Tok::EqEq => BinOp::Eq,
                Tok::Ne => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                _ => return l,
            };
            self.advance();
            let r = self.add_expr();
            l = Self::bin(l, op, r);
        }
    }

    fn add_expr(&mut self) -> Expr {
        let mut l = self.mul_expr();
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return l,
            };
            self.advance();
            let r = self.mul_expr();
            l = Self::bin(l, op, r);
        }
    }

    fn mul_expr(&mut self) -> Expr {
        let mut l = self.unary_expr();
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                // MOD, AND, OR and XOR are operators in any letter case
                // (k06, k21, k25-k29); DIV only in upper case (k32, k33).
                Tok::Kw(K::Mod) => BinOp::Mod,
                Tok::Kw(K::Div) if !self.soft_at(0) => BinOp::IntDiv,
                _ => return l,
            };
            self.advance();
            let r = self.unary_expr();
            l = Self::bin(l, op, r);
        }
    }

    fn unary_expr(&mut self) -> Expr {
        match self.peek() {
            Tok::Minus => {
                let start = self.advance().span;
                let e = self.unary_expr();
                let span = start.to(e.span);
                Expr {
                    kind: ExprKind::Unary(UnOp::Neg, Box::new(e)),
                    span,
                }
            }
            Tok::Plus => {
                self.advance();
                self.unary_expr()
            }
            _ => self.pow_expr(),
        }
    }

    fn pow_expr(&mut self) -> Expr {
        let base = self.postfix_expr();
        if *self.peek() == Tok::Caret {
            self.advance();
            let exp = self.unary_expr();
            return Self::bin(base, BinOp::Pow, exp);
        }
        base
    }

    fn postfix_expr(&mut self) -> Expr {
        let mut e = self.primary();
        loop {
            match self.peek() {
                Tok::LParen
                    if matches!(
                        e.kind,
                        ExprKind::Ident(_) | ExprKind::Call(..) | ExprKind::Index(..)
                    ) =>
                {
                    self.advance();
                    let args = self.args(Tok::RParen, "(");
                    let span = e.span.to(self.prev_span());
                    e = Expr {
                        kind: ExprKind::Call(Box::new(e), args),
                        span,
                    };
                }
                Tok::LBracket
                    if matches!(
                        e.kind,
                        ExprKind::Ident(_) | ExprKind::Call(..) | ExprKind::Index(..)
                    ) =>
                {
                    self.advance();
                    let args = self.args(Tok::RBracket, "[");
                    let span = e.span.to(self.prev_span());
                    e = Expr {
                        kind: ExprKind::Index(Box::new(e), args),
                        span,
                    };
                }
                Tok::Bang | Tok::Squared | Tok::Cubed => {
                    let op = match self.advance().tok {
                        Tok::Bang => PostOp::Factorial,
                        Tok::Squared => PostOp::Square,
                        _ => PostOp::Cube,
                    };
                    let span = e.span.to(self.prev_span());
                    e = Expr {
                        kind: ExprKind::Postfix(op, Box::new(e)),
                        span,
                    };
                }
                _ => return e,
            }
        }
    }

    /// Comma-separated arguments up to `close` (already past the opener).
    fn args(&mut self, close: Tok, open: &str) -> Vec<Expr> {
        let mut out = Vec::new();
        if self.eat(&close) {
            return out;
        }
        loop {
            out.push(self.expr());
            if self.eat(&Tok::Comma) {
                if *self.peek() == close {
                    let span = self.prev_span();
                    self.error("trailing-comma", span, "trailing comma in argument list");
                    self.advance();
                    return out;
                }
                continue;
            }
            if !self.eat(&close) {
                let span = self.span();
                let found = self.found();
                self.error(
                    "unclosed-delimiter",
                    span,
                    format!(
                        "unclosed `{open}`: expected `{}` but found {found}",
                        close_symbol(&close)
                    ),
                );
            }
            return out;
        }
    }

    fn primary(&mut self) -> Expr {
        self.soft_to_ident();
        let t = self.advance();
        let span = t.span;
        let kind = match t.tok {
            Tok::Number(n) => ExprKind::Number(n),
            Tok::HashInt(h) => ExprKind::HashInt(h),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Ident(i) => ExprKind::Ident(i),
            Tok::Kw(K::Mod) => {
                self.error(
                    "expected-expression",
                    self.span(),
                    "MOD needs a left operand",
                );
                ExprKind::Error
            }
            Tok::Quoted(text) => {
                let (inner, d) = parse_expression(&text);
                for mut x in d.items {
                    // Re-anchor at the quote: inner spans are relative.
                    x.span = span;
                    self.diags.push(x);
                }
                ExprKind::Quoted(Box::new(inner))
            }
            Tok::Sqrt => {
                let e = self.postfix_expr();
                let span = span.to(e.span);
                return Expr {
                    kind: ExprKind::Unary(UnOp::Sqrt, Box::new(e)),
                    span,
                };
            }
            Tok::LParen => {
                let e = self.expr();
                if self.eat(&Tok::Comma) {
                    let im = self.expr();
                    if !self.eat(&Tok::RParen) {
                        self.error(
                            "unclosed-delimiter",
                            self.span(),
                            "unclosed `(`: expected `)`",
                        );
                    }
                    let full = span.to(self.prev_span());
                    return Expr {
                        kind: ExprKind::ComplexPair(Box::new(e), Box::new(im)),
                        span: full,
                    };
                }
                if !self.eat(&Tok::RParen) {
                    let found = self.found();
                    self.error(
                        "unclosed-delimiter",
                        self.span(),
                        format!("unclosed `(`: expected `)` but found {found}"),
                    );
                }
                return Expr {
                    kind: e.kind,
                    span: span.to(self.prev_span()),
                };
            }
            Tok::LBrace => {
                let items = self.args(Tok::RBrace, "{");
                ExprKind::List(items)
            }
            Tok::LBracket => {
                let items = self.args(Tok::RBracket, "[");
                ExprKind::Bracket(items)
            }
            other => {
                let msg = format!("expected an expression, found {}", other.describe());
                let code = match other {
                    Tok::RParen | Tok::RBrace | Tok::RBracket => "unexpected-token",
                    _ => "expected-expression",
                };
                self.error(code, span, msg);
                // Do not swallow structural tokens.
                // advance() does not consume EOF. Rewinding it underflows
                // for empty input and revisits the previous token otherwise.
                if matches!(other, Tok::Semi | Tok::Kw(_)) {
                    self.pos -= 1;
                }
                ExprKind::Error
            }
        };
        Expr {
            kind,
            span: span.to(self.prev_span()),
        }
    }
}

fn close_symbol(t: &Tok) -> &'static str {
    match t {
        Tok::RParen => ")",
        Tok::RBrace => "}",
        Tok::RBracket => "]",
        _ => "?",
    }
}

/// Parses a standalone expression (quoted expressions, `EXPR` strings).
pub fn parse_expression(src: &str) -> (Expr, Diagnostics) {
    let mut diags = Diagnostics::default();
    let toks = lex(src, &mut diags);
    let mut p = Parser {
        toks,
        pos: 0,
        diags,
        prev_end: 0,
    };
    let e = p.expr();
    if *p.peek() != Tok::Eof {
        let span = p.span();
        p.error(
            "unexpected-token",
            span,
            format!("unexpected {} after expression", p.peek().describe()),
        );
    }
    (e, p.diags)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> Program {
        let (p, d) = parse(src);
        assert!(!d.has_errors(), "unexpected errors: {:?}", d.items);
        p
    }

    #[test]
    fn success_example() {
        let p =
            ok("EXPORT TEST()\nBEGIN\n  LOCAL x := 10;\n  LOCAL y := 20;\n  PRINT(x + y);\nEND;\n");
        let Item::Function(f) = &p.items[0] else {
            panic!()
        };
        assert_eq!(f.name.name, "TEST");
        assert!(f.exported);
        assert_eq!(f.body.len(), 3);
    }

    #[test]
    fn precedence() {
        let (e, d) = parse_expression("-2^2+3*4 MOD 5");
        assert!(!d.has_errors());
        let ExprKind::Binary(BinOp::Add, l, r) = e.kind else {
            panic!()
        };
        assert!(matches!(l.kind, ExprKind::Unary(UnOp::Neg, _)));
        assert!(matches!(r.kind, ExprKind::Binary(BinOp::Mod, _, _)));
    }

    #[test]
    fn control_flow() {
        ok(
            "EXPORT T()\nBEGIN\nLOCAL I,S:=0;\nFOR I FROM 1 TO 4 DO S:=S+I; END;\nIF S=10 THEN PRINT(\"OK\"); ELSE PRINT(1); END;\n\
            WHILE S>0 DO S:=S-1; END;\nREPEAT S:=S+1; UNTIL S>=5;\nCASE IF S=1 THEN PRINT(1); END; DEFAULT PRINT(2); END;\n\
            IFERR S:=1/0; THEN PRINT(\"e\"); END;\nFOR I FROM 10 DOWNTO 1 STEP 2 DO END;\nEND;",
        );
    }

    #[test]
    fn forward_decls_and_globals() {
        let p = ok(
            "SUB(a);\nEXPORT G:=5;\nLOCAL h;\nEXPORT T()\nBEGIN\nRETURN SUB(1);\nEND;\nSUB(a)\nBEGIN\nRETURN a*G;\nEND;",
        );
        assert_eq!(p.items.len(), 5);
    }

    #[test]
    fn store_and_index() {
        ok(
            "EXPORT T()\nBEGIN\nLOCAL L:={0,0,0};\n7▶L(2);\nL[3]:=1;\nLOCAL M:=[[1,2],[3,4]];\nM(1,2):=5;\nPRINT(L(2)+M(1,1));\nEND;",
        );
    }
}

#[cfg(test)]
mod robustness_tests {
    use super::*;
    use crate::SourceFile;

    /// Regression (found via a real hpcalc.org program, `Benchmark`):
    /// an unclaimed block-end token at top level used to make the
    /// `invalid-top-level` recovery loop forever, pushing one diagnostic
    /// per iteration (20 GB of memory from a 13 KB input).
    #[test]
    fn top_level_block_end_token_terminates() {
        let src = "VIEW \"S\", N()\nBEGIN\nEND;\n\nXD\n";
        let file = SourceFile::new("<repro>", src.to_string());
        let (_, diags) = parse(&file.text);
        assert!(diags.has_errors(), "expected diagnostics for stray END");
        assert!(
            diags.items.len() < 50,
            "recovery must not loop: got {} diagnostics",
            diags.items.len()
        );
    }

    /// A lone END at top level must terminate as well.
    #[test]
    fn lone_top_level_end_terminates() {
        let file = SourceFile::new("<repro>", "END;\n".to_string());
        let (_, diags) = parse(&file.text);
        assert!(diags.has_errors());
        assert!(diags.items.len() < 20, "{}", diags.items.len());
    }

    /// Garbage tails (binary container data decoded as text) must produce
    /// a bounded number of diagnostics, not one runaway error stream.
    #[test]
    fn garbage_tail_is_bounded() {
        let mut src = String::from("EXPORT T()\nBEGIN\n  RETURN 1;\nEND;\n");
        for _ in 0..2000 {
            src.push('\u{FFFD}');
        }
        let file = SourceFile::new("<repro>", src);
        let (_, diags) = parse(&file.text);
        assert!(diags.items.len() < 40_000, "got {}", diags.items.len());
    }
}

#[cfg(test)]
mod expression_input_tests {
    #[test]
    fn empty_and_truncated_expressions_report_errors_without_rewinding_eof() {
        for input in ["", " ", "(", "1+", "{", "f("] {
            let (_, diagnostics) = super::parse_expression(input);
            assert!(diagnostics.has_errors(), "{input:?}");
        }
    }
}
