//! Typed abstract syntax tree for PPL programs.

use crate::source::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Program {
    /// Lexer punctuation spans in source order, for native diagnostics.
    pub punctuation: Vec<Punctuation>,
    pub items: Vec<Item>,
    pub pragmas: Vec<(String, Span)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PunctuationKind {
    Open,
    Close,
    Comma,
}

#[derive(Clone, Copy, Debug)]
pub struct Punctuation {
    pub kind: PunctuationKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Function(FunctionDef),
    /// `name(params);` before its definition.
    ForwardDecl {
        name: Ident,
        params: Vec<Ident>,
        span: Span,
    },
    /// File-level `LOCAL` or `EXPORT` variables.
    Globals {
        exported: bool,
        vars: Vec<VarDecl>,
        span: Span,
    },
    /// `#cas … #end` section (not executable by this emulator yet).
    CasBlock {
        text: String,
        span: Span,
    },
}

#[derive(Clone, Debug)]
pub struct FunctionDef {
    pub name: Ident,
    pub params: Vec<Ident>,
    pub body: Vec<Stmt>,
    pub exported: bool,
    /// `KEY K_Name()` key handler.
    pub is_key: bool,
    /// Declared with `LOCAL name(...)` at file level.
    pub is_local: bool,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub name: Ident,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Expr(Expr),
    Local(Vec<VarDecl>),
    Block(Vec<Stmt>),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    Case {
        arms: Vec<(Expr, Vec<Stmt>)>,
        default: Option<Vec<Stmt>>,
    },
    IfErr {
        body: Vec<Stmt>,
        handler: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    For {
        var: Ident,
        from: Expr,
        to: Expr,
        step: Option<Expr>,
        down: bool,
        body: Vec<Stmt>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Repeat {
        body: Vec<Stmt>,
        until: Expr,
    },
    Return(Option<Expr>),
    Break(Option<Expr>),
    Continue,
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
    IntDiv,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Xor,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
            BinOp::Mod => "MOD",
            BinOp::IntDiv => "DIV",
            BinOp::Eq => "==",
            BinOp::Ne => "≠",
            BinOp::Lt => "<",
            BinOp::Le => "≤",
            BinOp::Gt => ">",
            BinOp::Ge => "≥",
            BinOp::And => "AND",
            BinOp::Or => "OR",
            BinOp::Xor => "XOR",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Sqrt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostOp {
    Factorial,
    Square,
    Cube,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Number(String),
    HashInt(String),
    Str(String),
    Ident(String),
    /// `{a, b}`
    List(Vec<Expr>),
    /// `[a, b]` (vector) or `[[a,b],[c,d]]` (matrix rows are nested Brackets).
    Bracket(Vec<Expr>),
    /// `(re, im)` complex constructor.
    ComplexPair(Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Postfix(PostOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `f(args)` or `var(index…)`: resolved during semantic analysis.
    Call(Box<Expr>, Vec<Expr>),
    /// `var[index…]`
    Index(Box<Expr>, Vec<Expr>),
    /// `target := value`
    Assign(Box<Expr>, Box<Expr>),
    /// `value ▶ target`
    Store(Box<Expr>, Box<Expr>),
    /// `'expr'`
    Quoted(Box<Expr>),
    /// Unparseable piece already reported as an error.
    Error,
}
