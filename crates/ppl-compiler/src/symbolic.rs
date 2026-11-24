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
        ExprKind::List(items) => format!("list[{}]", list(items)),
        ExprKind::Bracket(items) => format!("[{}]", list(items)),
        ExprKind::ComplexPair(a, b) => format!("({})+i*({})", source(a), source(b)),
        ExprKind::Unary(op, a) => match op {
            UnOp::Neg => format!("-({})", source(a)),
            UnOp::Not => format!("not({})", source(a)),
            UnOp::Sqrt => format!("sqrt({})", source(a)),
            // Giac receives the numeric expression; this is a Home numeric
            // representation flag, not a different algebraic operation.
            UnOp::Sexagesimal => source(a),
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

/// Assignment is statement syntax in a quoted algebra argument. The installed
/// G1 refuses the complete quoted tree before evaluating any of its children.
pub(crate) fn contains_assignment(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Assign(..) | ExprKind::Store(..) => true,
        ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => {
            contains_assignment(a)
        }
        ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => {
            contains_assignment(a) || contains_assignment(b)
        }
        ExprKind::Call(callee, args) | ExprKind::Index(callee, args) => {
            contains_assignment(callee) || args.iter().any(contains_assignment)
        }
        ExprKind::List(items) | ExprKind::Bracket(items) => items.iter().any(contains_assignment),
        _ => false,
    }
}

/// Replace only declared PPL calls with typed runtime operands. A marker is
/// chosen outside the complete original source, including string contents;
/// runtime substitution scans the template once and never scans inserted data.
pub(crate) fn dynamic_source(
    expr: &Expr,
    native: impl Fn(&Expr) -> bool,
) -> (String, String, Vec<Expr>) {
    let original = source(expr);
    let mut marker = "__PPL_CAS_ARGUMENT_".to_string();
    while original.contains(&marker) {
        marker.push('_');
    }
    fn visit(
        expr: &Expr,
        native: &impl Fn(&Expr) -> bool,
        marker: &str,
        values: &mut Vec<Expr>,
    ) -> Expr {
        if native(expr) {
            let index = values.len();
            values.push(expr.clone());
            return Expr {
                kind: ExprKind::Ident(format!("{marker}{index}__")),
                span: expr.span,
            };
        }
        let mut mapped = expr.clone();
        let map = |e: &mut Expr, values: &mut Vec<Expr>| *e = visit(e, native, marker, values);
        match &mut mapped.kind {
            ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) => map(a, values),
            ExprKind::Binary(_, a, b)
            | ExprKind::ComplexPair(a, b)
            | ExprKind::Assign(a, b)
            | ExprKind::Store(a, b) => {
                map(a, values);
                map(b, values);
            }
            ExprKind::Call(callee, args) | ExprKind::Index(callee, args) => {
                map(callee, values);
                for arg in args {
                    map(arg, values);
                }
            }
            ExprKind::List(items) | ExprKind::Bracket(items) => {
                for item in items {
                    map(item, values);
                }
            }
            // Quoted algebra retains its own unevaluated expression boundary.
            _ => {}
        }
        mapped
    }
    let mut values = Vec::new();
    let mapped = visit(expr, &native, &marker, &mut values);
    (source(&mapped), marker, values)
}
