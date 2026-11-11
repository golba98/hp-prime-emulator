//! Canonical algebra source for quoted expressions. Symbols remain unevaluated.
use ppl_syntax::ast::{BinOp, Expr, ExprKind, PostOp, UnOp};
pub fn source(expr: &Expr) -> String {
    render(expr, false)
}
pub fn entry_source(expr: &Expr) -> String {
    render(expr, true)
}
fn render(expr: &Expr, preserve_quotes: bool) -> String {
    let source = |e: &Expr| render(e, preserve_quotes);
    let list = |items: &[Expr]| items.iter().map(source).collect::<Vec<_>>().join(",");
    match &expr.kind {
        ExprKind::Number(s) => s.clone(),
        ExprKind::HashInt(s) => format!("#{s}"),
        ExprKind::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        ExprKind::Ident(s) => match s.as_str() {
            "π" => "pi".into(),
            "ⅈ" | "\u{e003}" => "i".into(),
            "ℯ" => "exp(1)".into(),
            _ => s.clone(),
        },
        ExprKind::List(items) | ExprKind::Bracket(items) => format!("[{}]", list(items)),
        ExprKind::ComplexPair(a, b) => format!("({})+i*({})", source(a), source(b)),
        ExprKind::Unary(op, a) => match op {
            UnOp::Neg => format!("-({})", source(a)),
            UnOp::Not => format!("not({})", source(a)),
            UnOp::Sqrt => format!("sqrt({})", source(a)),
        },
        ExprKind::Postfix(op, a) => match op {
            PostOp::Factorial => format!("({})!", source(a)),
            PostOp::Square => format!("({})^2", source(a)),
            PostOp::Cube => format!("({})^3", source(a)),
        },
        ExprKind::Binary(op, a, b) => {
            let op = match op {
                BinOp::Eq => "=",
                BinOp::Ne => "!=",
                BinOp::Le => "<=",
                BinOp::Ge => ">=",
                BinOp::Mod => "%",
                BinOp::IntDiv => "/",
                BinOp::And => " and ",
                BinOp::Or => " or ",
                BinOp::Xor => " xor ",
                _ => op.symbol(),
            };
            format!("({}){op}({})", source(a), source(b))
        }
        ExprKind::Call(c, args) => {
            let name = source(c);
            let name = if name.chars().all(|c| c.is_ascii_uppercase()) {
                name.to_ascii_lowercase()
            } else {
                name
            };
            if name == "nthroot" && args.len() == 2 {
                return format!("surd({},{})", source(&args[1]), source(&args[0]));
            }
            if name == "alog" && args.len() == 1 {
                return format!("10^({})", source(&args[0]));
            }
            let name = match name.as_str() {
                "log" if matches!(&c.kind,ExprKind::Ident(n) if n=="LOG") => "log10".into(),
                "approx" if matches!(&c.kind,ExprKind::Ident(n) if n=="APPROX") => "evalf".into(),
                _ => name,
            };
            format!("{name}({})", list(args))
        }
        ExprKind::Index(c, args) => format!("{}[{}]", source(c), list(args)),
        ExprKind::Assign(a, b) => format!("{}:=({})", source(a), source(b)),
        ExprKind::Store(a, b) => format!("{}:=({})", source(b), source(a)),
        ExprKind::Quoted(a) => {
            if preserve_quotes {
                format!("'{}'", source(a))
            } else {
                source(a)
            }
        }
        ExprKind::Error => "undef".into(),
    }
}
