//! Canonical unevaluated PPL source for application definitions and commands.
//! This preserves calculator list, complex-pair and identifier syntax; CAS
//! translation has a separate printer.
use ppl_syntax::ast::{BinOp, Expr, ExprKind as E, PostOp, UnOp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    NestingLimit,
    Sexagesimal,
    Assignment,
    Invalid,
}
impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NestingLimit => "Expression nesting limit exceeded",
            Self::Sexagesimal => "Sexagesimal definition source is unavailable",
            Self::Assignment => "Assignment definition source is unavailable",
            Self::Invalid => "Invalid expression source",
        })
    }
}
impl std::error::Error for SourceError {}

pub fn ppl_expression_source(expression: &Expr) -> Result<String, SourceError> {
    render(expression, 0, 0)
}
fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or | BinOp::Xor => 2,
        BinOp::And => 3,
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 5,
        BinOp::Add | BinOp::Sub => 6,
        BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::IntDiv | BinOp::Polar => 7,
        BinOp::Pow => 9,
    }
}
fn render(expression: &Expr, parent: u8, depth: usize) -> Result<String, SourceError> {
    if depth > 128 {
        return Err(SourceError::NestingLimit);
    }
    let child = |e: &Expr, p| render(e, p, depth + 1);
    let list = |items: &[Expr]| -> Result<String, SourceError> {
        items
            .iter()
            .map(|e| child(e, 0))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join(","))
    };
    let (text, level) = match &expression.kind {
        E::Number(number) | E::Ident(number) => (number.clone(), 11),
        E::HashInt(number) => (format!("#{number}"), 11),
        E::Str(text) => (
            crate::cas_entry_source(&Expr {
                kind: E::Str(text.clone()),
                span: expression.span,
            }),
            11,
        ),
        E::List(items) => (format!("{{{}}}", list(items)?), 11),
        E::Bracket(items) => (format!("[{}]", list(items)?), 11),
        E::ComplexPair(a, b) => (format!("({},{})", child(a, 0)?, child(b, 0)?), 11),
        E::Unary(UnOp::Neg, inner) => (format!("-{}", child(inner, 8)?), 8),
        E::Unary(UnOp::Not, inner) => (format!("NOT {}", child(inner, 4)?), 4),
        E::Unary(UnOp::Sqrt, inner) => (format!("√({})", child(inner, 0)?), 10),
        E::Unary(UnOp::Sexagesimal, _) => {
            return Err(SourceError::Sexagesimal);
        }
        E::Postfix(op, inner) => (
            format!(
                "{}{}",
                child(inner, 10)?,
                match op {
                    PostOp::Factorial => "!",
                    PostOp::Square => "²",
                    PostOp::Cube => "³",
                }
            ),
            10,
        ),
        E::Binary(op, a, b) => {
            let p = precedence(*op);
            let (left, right) = if *op == BinOp::Pow {
                (p + 1, 8)
            } else {
                (p, p + 1)
            };
            let symbol = if matches!(
                op,
                BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Mod | BinOp::IntDiv
            ) {
                format!(" {} ", op.symbol())
            } else {
                op.symbol().into()
            };
            (
                format!("{}{symbol}{}", child(a, left)?, child(b, right)?),
                p,
            )
        }
        E::Call(callee, args) => (format!("{}({})", child(callee, 10)?, list(args)?), 10),
        E::Index(callee, args) => (format!("{}[{}]", child(callee, 10)?, list(args)?), 10),
        E::Quoted(inner) => return child(inner, parent),
        E::Assign(..) | E::Store(..) => {
            return Err(SourceError::Assignment);
        }
        E::Error => return Err(SourceError::Invalid),
    };
    Ok(if level < parent {
        format!("({text})")
    } else {
        text
    })
}
