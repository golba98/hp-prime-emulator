//! Tokenizer for HP Prime PPL source.

use crate::diag::{Category, Diagnostic, Diagnostics};
use crate::source::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Keyword {
    Export,
    Local,
    Begin,
    End,
    If,
    Then,
    Else,
    Case,
    Default,
    Iferr,
    For,
    From,
    To,
    Downto,
    Step,
    Do,
    While,
    Repeat,
    Until,
    Return,
    Break,
    Continue,
    Kill,
    Key,
    And,
    Or,
    Xor,
    Not,
    Mod,
    Div,
}

impl Keyword {
    pub fn from_upper(s: &str) -> Option<Keyword> {
        use Keyword::*;
        Some(match s {
            "EXPORT" => Export,
            "LOCAL" => Local,
            "BEGIN" => Begin,
            "END" => End,
            "IF" => If,
            "THEN" => Then,
            "ELSE" => Else,
            "CASE" => Case,
            "DEFAULT" => Default,
            "IFERR" => Iferr,
            "FOR" => For,
            "FROM" => From,
            "TO" => To,
            "DOWNTO" => Downto,
            "STEP" => Step,
            "DO" => Do,
            "WHILE" => While,
            "REPEAT" => Repeat,
            "UNTIL" => Until,
            "RETURN" => Return,
            "BREAK" => Break,
            "CONTINUE" => Continue,
            "KILL" => Kill,
            "KEY" => Key,
            "AND" => And,
            "OR" => Or,
            "XOR" => Xor,
            "NOT" => Not,
            "MOD" => Mod,
            "DIV" => Div,
            _ => return None,
        })
    }

    /// The keyword spelled `s` in any ASCII letter case (`END`, `end`,
    /// `End`). The text itself is only compared, never rewritten.
    pub fn from_any_case(s: &str) -> Option<Keyword> {
        if !s.is_ascii() || s.len() > 8 {
            return None;
        }
        Keyword::from_upper(&s.to_ascii_uppercase())
    }

    pub fn as_str(self) -> &'static str {
        use Keyword::*;
        match self {
            Export => "EXPORT",
            Local => "LOCAL",
            Begin => "BEGIN",
            End => "END",
            If => "IF",
            Then => "THEN",
            Else => "ELSE",
            Case => "CASE",
            Default => "DEFAULT",
            Iferr => "IFERR",
            For => "FOR",
            From => "FROM",
            To => "TO",
            Downto => "DOWNTO",
            Step => "STEP",
            Do => "DO",
            While => "WHILE",
            Repeat => "REPEAT",
            Until => "UNTIL",
            Return => "RETURN",
            Break => "BREAK",
            Continue => "CONTINUE",
            Kill => "KILL",
            Key => "KEY",
            And => "AND",
            Or => "OR",
            Xor => "XOR",
            Not => "NOT",
            Mod => "MOD",
            Div => "DIV",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Number(String),
    /// Text after `#`, e.g. `FFh` or `AF:16h`.
    HashInt(String),
    Str(String),
    /// Text between single quotes: a symbolic expression.
    Quoted(String),
    Kw(Keyword),
    Pragma(String),
    CasBlock(String),
    /// `// ...` up to (not including) the newline. Only produced by
    /// [`lex_with_trivia`]; the parser never sees comments.
    Comment,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semi,
    Assign,
    Store,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Bang,
    Squared,
    Cubed,
    Sqrt,
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("identifier `{s}`"),
            Tok::Number(s) => format!("number `{s}`"),
            Tok::HashInt(s) => format!("integer `#{s}`"),
            Tok::Str(_) => "string".into(),
            Tok::Quoted(_) => "quoted expression".into(),
            Tok::Kw(k) => format!("`{}`", k.as_str()),
            Tok::Pragma(_) => "`#pragma`".into(),
            Tok::CasBlock(_) => "`#cas` block".into(),
            Tok::Comment => "comment".into(),
            Tok::Eof => "end of file".into(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Comma => ",",
            Tok::Semi => ";",
            Tok::Assign => ":=",
            Tok::Store => "▶",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::Slash => "/",
            Tok::Caret => "^",
            Tok::Eq => "=",
            Tok::EqEq => "==",
            Tok::Ne => "≠",
            Tok::Lt => "<",
            Tok::Le => "≤",
            Tok::Gt => ">",
            Tok::Ge => "≥",
            Tok::Bang => "!",
            Tok::Squared => "²",
            Tok::Cubed => "³",
            Tok::Sqrt => "√",
            _ => "?",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// For a keyword written in other than upper case (`end`, `Begin`),
    /// its spelling. The firmware reads such a word as the keyword where
    /// the grammar expects one and as a name elsewhere, so the parser may
    /// turn it back into an identifier; upper-case keywords are reserved.
    pub soft: Option<String>,
}

/// ASCII case-insensitive substring search (byte offset of the match).
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let n = needle.as_bytes();
    hay.as_bytes()
        .windows(n.len())
        .position(|w| w.eq_ignore_ascii_case(n))
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic() && !matches!(c, 'π' | 'ⅈ' | 'ℯ')
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric() && !matches!(c, '²' | '³')
}

pub fn lex(src: &str, diags: &mut Diagnostics) -> Vec<Token> {
    lex_inner(src, diags, false)
}

/// Like [`lex`], but also emits a [`Tok::Comment`] token for every `//`
/// comment, so editors can classify every byte from one token stream.
pub fn lex_with_trivia(src: &str, diags: &mut Diagnostics) -> Vec<Token> {
    lex_inner(src, diags, true)
}

fn lex_inner(src: &str, diags: &mut Diagnostics, trivia: bool) -> Vec<Token> {
    let mut lx = Lexer {
        src,
        chars: src.char_indices().collect(),
        pos: 0,
        out: Vec::new(),
        trivia,
    };
    lx.run(diags);
    lx.out
}

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    pos: usize,
    out: Vec<Token>,
    trivia: bool,
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.pos + k).map(|&(_, c)| c)
    }

    fn offset(&self) -> usize {
        self.chars
            .get(self.pos)
            .map(|&(o, _)| o)
            .unwrap_or(self.src.len())
    }

    fn push(&mut self, tok: Tok, start: usize) {
        let end = self.offset();
        self.out.push(Token {
            tok,
            span: Span::new(start, end),
            soft: None,
        });
    }

    fn run(&mut self, diags: &mut Diagnostics) {
        while let Some(c) = self.peek(0) {
            let start = self.offset();
            if c.is_whitespace() {
                self.pos += 1;
                continue;
            }
            if c == '/' && self.peek(1) == Some('/') {
                while let Some(c) = self.peek(0) {
                    if c == '\n' {
                        break;
                    }
                    self.pos += 1;
                }
                if self.trivia {
                    self.push(Tok::Comment, start);
                }
                continue;
            }
            if c.is_ascii_digit() || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit()))
            {
                self.number(start);
                continue;
            }
            if is_ident_start(c) {
                self.ident(start);
                continue;
            }
            // `%(x, y)`, `%CHANGE(x, y)`, `%TOTAL(x, y)`: documented
            // percentage functions whose names begin with `%`.
            if c == '%'
                && self
                    .peek(1)
                    .is_some_and(|n| n == '(' || n.is_ascii_uppercase())
            {
                self.pos += 1;
                while self.peek(0).is_some_and(is_ident_continue) {
                    self.pos += 1;
                }
                let text = self.src[start..self.offset()].to_string();
                self.push(Tok::Ident(text), start);
                continue;
            }
            match c {
                '"' => self.string(start, diags),
                '\'' => self.quoted(start, diags),
                '#' => self.hash(start, diags),
                _ => self.punct(c, start, diags),
            }
        }
        let end = self.src.len();
        self.out.push(Token {
            tok: Tok::Eof,
            span: Span::new(end, end),
            soft: None,
        });
    }

    fn number(&mut self, start: usize) {
        while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek(0) == Some('.') {
            self.pos += 1;
            while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if let Some(e) = self.peek(0) {
            if matches!(e, 'E' | 'e' | 'ᴇ') {
                let sign = matches!(self.peek(1), Some('+' | '-' | '−'));
                let digit_at = if sign { 2 } else { 1 };
                if self.peek(digit_at).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += digit_at;
                    while self.peek(0).is_some_and(|c| c.is_ascii_digit()) {
                        self.pos += 1;
                    }
                }
            }
        }
        let text = self.src[start..self.offset()].to_string();
        self.push(Tok::Number(text), start);
    }

    fn ident(&mut self, start: usize) {
        loop {
            match self.peek(0) {
                Some(c) if is_ident_continue(c) => self.pos += 1,
                // `B→R`, `R→B`: arrow inside a name.
                Some('→') if self.peek(1).is_some_and(is_ident_start) => self.pos += 1,
                // `CAS.diff`, `Finance.TVM`: qualified names.
                Some('.') if self.peek(1).is_some_and(is_ident_start) => self.pos += 1,
                _ => break,
            }
        }
        let text = &self.src[start..self.offset()];
        // The original firmware accepts keywords in any letter case
        // (diagnostic corpus h01, h07, k01-k05) but still accepts the
        // lower- and mixed-case words as names (h08, k08-k23): those are
        // soft keywords, which the parser resolves by position.
        let (tok, soft) = match Keyword::from_upper(text) {
            Some(k) => (Tok::Kw(k), None),
            None => match Keyword::from_any_case(text) {
                Some(k) => (Tok::Kw(k), Some(text.to_string())),
                None => (Tok::Ident(text.to_string()), None),
            },
        };
        self.push(tok, start);
        self.out.last_mut().unwrap().soft = soft;
    }

    fn string(&mut self, start: usize, diags: &mut Diagnostics) {
        let open_pos = self.pos;
        self.pos += 1;
        let mut s = String::new();
        let mut newlines = 0usize;
        loop {
            let Some(c) = self.peek(0) else {
                // Unclosed at end of file. Multi-line strings are legal, so
                // the literal formally runs to EOF, but that would swallow
                // every later statement. Recover at the end of the opening
                // line instead: the error points at the literal and the
                // rest of the program is still lexed, parsed and
                // highlighted normally.
                self.pos = open_pos + 1;
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.pos += 1;
                }
                let end = self.offset();
                diags.push(
                    Diagnostic::error(
                        Category::Lexer,
                        "unclosed-string",
                        Span::new(start, end.max(start + 1)),
                        "unclosed string literal",
                    )
                    .with_hint("add the closing `\"`"),
                );
                let body = self.src[start + 1..end].to_string();
                self.out.push(Token {
                    tok: Tok::Str(body),
                    span: Span::new(start, end),
                    soft: None,
                });
                return;
            };
            match c {
                '"' => {
                    self.pos += 1;
                    break;
                }
                '\\' => {
                    let esc_start = self.offset();
                    self.pos += 1;
                    match self.peek(0) {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('"') => s.push('"'),
                        Some('\\') => s.push('\\'),
                        Some(other) => {
                            diags.push(
                                Diagnostic::warning(
                                    Category::Lexer,
                                    "unknown-escape",
                                    Span::new(esc_start, esc_start + 1 + other.len_utf8()),
                                    format!("unknown escape `\\{other}` is kept literally"),
                                )
                                .unverified(),
                            );
                            s.push('\\');
                            s.push(other);
                        }
                        None => continue,
                    }
                    self.pos += 1;
                }
                '\n' => {
                    // Multi-line strings are legal; keep the newline.
                    s.push('\n');
                    newlines += 1;
                    self.pos += 1;
                }
                _ => {
                    s.push(c);
                    self.pos += 1;
                }
            }
        }
        if newlines > 0 {
            // Legal, but a far more common cause is a missing quote that
            // inverts every later string; say so without failing.
            diags.push(
                Diagnostic::note(
                    Category::Lexer,
                    "multiline-string",
                    Span::new(start, start + 1),
                    format!("string literal spans {} lines", newlines + 1),
                )
                .with_hint("if this is not intended, check for a missing closing `\"`")
                .unverified(),
            );
        }
        self.push(Tok::Str(s), start);
    }

    fn quoted(&mut self, start: usize, diags: &mut Diagnostics) {
        self.pos += 1;
        let body_start = self.offset();
        while let Some(c) = self.peek(0) {
            if c == '\'' || c == '\n' {
                break;
            }
            self.pos += 1;
        }
        let body = self.src[body_start..self.offset()].to_string();
        if self.peek(0) == Some('\'') {
            self.pos += 1;
        } else {
            diags.push(Diagnostic::error(
                Category::Lexer,
                "unclosed-quote",
                Span::new(start, start + 1),
                "unclosed quoted expression",
            ));
        }
        self.push(Tok::Quoted(body), start);
    }

    fn hash(&mut self, start: usize, diags: &mut Diagnostics) {
        let rest = &self.src[start..];
        let starts_with_ci = |p: &str| {
            rest.len() >= p.len()
                && rest.is_char_boundary(p.len())
                && rest[..p.len()].eq_ignore_ascii_case(p)
        };
        if starts_with_ci("#pragma") {
            let line_end = rest.find('\n').unwrap_or(rest.len());
            let text = rest[..line_end].to_string();
            self.advance_bytes(start + line_end);
            self.push(Tok::Pragma(text), start);
            return;
        }
        if starts_with_ci("#cas") {
            let end = find_ci(rest, "#end").map(|i| i + 4).unwrap_or(rest.len());
            let text = rest[..end].to_string();
            self.advance_bytes(start + end);
            self.push(Tok::CasBlock(text), start);
            return;
        }
        self.pos += 1;
        let body_start = self.offset();
        while self
            .peek(0)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == ':')
        {
            self.pos += 1;
        }
        let body = self.src[body_start..self.offset()].to_string();
        if body.is_empty() {
            diags.push(Diagnostic::error(
                Category::Lexer,
                "malformed-integer",
                Span::new(start, start + 1),
                "`#` must be followed by integer digits",
            ));
        }
        self.push(Tok::HashInt(body), start);
    }

    fn advance_bytes(&mut self, target: usize) {
        // Binary search: `chars` is sorted by byte offset.
        let idx = self.chars.partition_point(|&(o, _)| o < target);
        self.pos = self.pos.max(idx);
    }

    fn punct(&mut self, c: char, start: usize, diags: &mut Diagnostics) {
        let two = |l: &Self, a: char| l.peek(1) == Some(a);
        let (tok, len) = match c {
            '(' => (Tok::LParen, 1),
            ')' => (Tok::RParen, 1),
            '{' => (Tok::LBrace, 1),
            '}' => (Tok::RBrace, 1),
            '[' => (Tok::LBracket, 1),
            ']' => (Tok::RBracket, 1),
            ',' => (Tok::Comma, 1),
            ';' => (Tok::Semi, 1),
            ':' if two(self, '=') => (Tok::Assign, 2),
            '▶' => (Tok::Store, 1),
            '+' => (Tok::Plus, 1),
            '-' | '−' => (Tok::Minus, 1),
            '*' | '×' | '·' => (Tok::Star, 1),
            '/' | '÷' => (Tok::Slash, 1),
            '^' => (Tok::Caret, 1),
            '=' if two(self, '=') => (Tok::EqEq, 2),
            '=' => (Tok::Eq, 1),
            '≠' => (Tok::Ne, 1),
            '!' if two(self, '=') => (Tok::Ne, 2),
            '!' => (Tok::Bang, 1),
            '<' if two(self, '>') => (Tok::Ne, 2),
            '<' if two(self, '=') => (Tok::Le, 2),
            '<' => (Tok::Lt, 1),
            '≤' => (Tok::Le, 1),
            '>' if two(self, '=') => (Tok::Ge, 2),
            '>' => (Tok::Gt, 1),
            '≥' => (Tok::Ge, 1),
            '²' => (Tok::Squared, 1),
            '³' => (Tok::Cubed, 1),
            '√' => (Tok::Sqrt, 1),
            'π' | 'ⅈ' | 'ℯ' | '∞' | '\u{E003}' => (Tok::Ident(c.to_string()), 1),
            _ => {
                self.pos += 1;
                diags.push(Diagnostic::error(
                    Category::Lexer,
                    "invalid-character",
                    Span::new(start, self.offset()),
                    format!("unexpected character `{c}`"),
                ));
                return;
            }
        };
        self.pos += len;
        self.push(tok, start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        let mut d = Diagnostics::default();
        lex(s, &mut d).into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn basic_tokens() {
        assert_eq!(
            toks("LOCAL x:=1.5E3; // hi\nx▶B→R"),
            vec![
                Tok::Kw(Keyword::Local),
                Tok::Ident("x".into()),
                Tok::Assign,
                Tok::Number("1.5E3".into()),
                Tok::Semi,
                Tok::Ident("x".into()),
                Tok::Store,
                Tok::Ident("B→R".into()),
                Tok::Eof
            ]
        );
    }

    #[test]
    fn keywords_in_any_case_keep_their_spelling_and_span() {
        let src = "θ begin End MOD mod BEGINNER";
        let mut d = Diagnostics::default();
        let t = lex(src, &mut d);
        let at = |i: usize| &src[t[i].span.start as usize..t[i].span.end as usize];
        assert_eq!(t[1].tok, Tok::Kw(Keyword::Begin));
        assert_eq!((t[1].soft.as_deref(), at(1)), (Some("begin"), "begin"));
        assert_eq!((t[2].soft.as_deref(), at(2)), (Some("End"), "End"));
        // Upper-case keywords are reserved, not soft.
        assert_eq!(
            (t[3].tok.clone(), t[3].soft.clone()),
            (Tok::Kw(Keyword::Mod), None)
        );
        assert_eq!(t[4].soft.as_deref(), Some("mod"));
        assert_eq!(t[5].tok, Tok::Ident("BEGINNER".into()));
        // Non-ASCII letters never fold onto a keyword.
        assert_eq!(Keyword::from_any_case("ıf"), None);
    }

    #[test]
    fn strings_and_hash() {
        assert_eq!(toks(r#""a\"b""#), vec![Tok::Str("a\"b".into()), Tok::Eof]);
        assert_eq!(toks("#FFh"), vec![Tok::HashInt("FFh".into()), Tok::Eof]);
        assert_eq!(
            toks("a<>b"),
            vec![
                Tok::Ident("a".into()),
                Tok::Ne,
                Tok::Ident("b".into()),
                Tok::Eof
            ]
        );
        assert_eq!(
            toks("CAS.diff"),
            vec![Tok::Ident("CAS.diff".into()), Tok::Eof]
        );
    }

    #[test]
    fn unclosed_string_is_error() {
        let mut d = Diagnostics::default();
        lex("PRINT(\"hello);", &mut d);
        assert!(d.items.iter().any(|x| x.code == "unclosed-string"));
    }
}
