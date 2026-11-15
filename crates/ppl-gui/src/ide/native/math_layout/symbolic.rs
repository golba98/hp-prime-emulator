//! Static textbook geometry follows parsed precedence without changing source.
use super::*;
use ppl_syntax::ast::{BinOp, Expr, ExprKind};

pub fn expression(source: &str, px: f32) -> MathLayout {
    if source.is_empty() {
        return text("", px);
    }
    if let Ok(normalized) = ExpressionEditor::from_text(source).evaluation_text(false)
        && normalized != source
    {
        return expression(&normalized, px);
    }
    if source.len() <= 8192 {
        let (expr, diags) = ppl_syntax::parse_expression(source);
        if !diags.has_errors() {
            return node(&expr, source, px, 0);
        }
    }
    flat(source, px)
}
fn flat(source: &str, px: f32) -> MathLayout {
    range(&ExpressionEditor::from_text(source), 0..source.len(), px)
}
fn node(e: &Expr, source: &str, px: f32, depth: usize) -> MathLayout {
    let raw = &source[e.span.start as usize..e.span.end as usize];
    if depth >= 64 {
        return text(raw, px);
    }
    let editor = ExpressionEditor::from_text(raw);
    if editor.templates().iter().any(|t| {
        t.span == (0..raw.len())
            && matches!(
                t.kind,
                TemplateKind::Palette(_) | TemplateKind::Function("ABS")
            )
    }) {
        return range(&editor, 0..raw.len(), px);
    }
    match &e.kind {
        ExprKind::Binary(BinOp::Div, a, b) => {
            let upper = node(a, source, px * 0.86, depth + 1);
            let lower = node(b, source, px * 0.86, depth + 1);
            let width = upper.width.max(lower.width) + 6;
            let upper_h = upper.height;
            let mut out = MathLayout {
                width,
                height: upper_h + lower.height + 5,
                baseline: upper_h + 4,
                ..Default::default()
            };
            let ux = (width - upper.width) / 2;
            let lx = (width - lower.width) / 2;
            out.append(upper, ux, 0);
            out.append(lower, lx, upper_h + 5);
            out.rules.push(Rule {
                x: 0,
                y: upper_h + 2,
                width,
                height: 0,
            });
            out
        }
        ExprKind::Binary(BinOp::Pow, a, b) => {
            let base = node(a, source, px, depth + 1);
            let exp = node(b, source, px * 0.72, depth + 1);
            let y = exp.height / 2;
            let mut out = MathLayout {
                width: base.width + exp.width + 2,
                height: base.height + y,
                baseline: base.baseline + y,
                ..Default::default()
            };
            let x = base.width + 2;
            out.append(base, 0, y);
            out.append(exp, x, 0);
            out
        }
        ExprKind::Str(_) => text(raw, px),
        _ => {
            let children: Vec<&Expr> = match &e.kind {
                ExprKind::Binary(_, a, b)
                | ExprKind::Assign(a, b)
                | ExprKind::Store(a, b)
                | ExprKind::ComplexPair(a, b) => vec![a, b],
                ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => vec![a],
                ExprKind::Call(a, args) | ExprKind::Index(a, args) => {
                    std::iter::once(a.as_ref()).chain(args.iter()).collect()
                }
                ExprKind::List(items) | ExprKind::Bracket(items) => items.iter().collect(),
                _ => vec![],
            };
            // The existing radical geometry also covers empty and recovered
            // root templates. Keep it for calls until result nodes need richer
            // function-specific notation.
            if children.is_empty()
                || matches!(&e.kind,ExprKind::Call(f,_) if matches!(&f.kind,ExprKind::Ident(name) if name.eq_ignore_ascii_case("SQRT")||name.eq_ignore_ascii_case("NTHROOT")))
            {
                return flat(raw, px);
            }
            let mut parts = Vec::new();
            let mut pos = e.span.start as usize;
            for child in children {
                let start = child.span.start as usize;
                if start > pos {
                    parts.push(text(&source[pos..start], px));
                }
                parts.push(node(child, source, px, depth + 1));
                pos = child.span.end as usize;
            }
            if pos < e.span.end as usize {
                parts.push(text(&source[pos..e.span.end as usize], px));
            }
            sequence(parts)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rational_and_power_geometry_preserves_precedence_and_literal_strings() {
        let fraction = expression("(x^2+1)/(2*x)", 17.);
        assert_eq!(fraction.rules.len(), 1);
        assert!(fraction.height > 30);
        let p = expression("x^(1/2)", 17.);
        assert_eq!(p.rules.len(), 1);
        assert!(p.width < fraction.width);
        let string = expression("\"1/2\"", 17.);
        assert!(string.rules.is_empty());
        assert_eq!(
            string
                .glyphs
                .iter()
                .map(|g| g.text.as_str())
                .collect::<String>(),
            "\"1/2\""
        );
    }
    #[test]
    fn history_and_show_use_calculus_notation_and_multiline_collections() {
        for source in [
            "CAS.diff(x^2,x)",
            "CAS.integrate(x,x,,)",
            "CAS.sum(k,k,1,5)",
            "CAS.piecewise(1>0,2,1<0,3)",
            "CAS.limit(SIN(x)/x,x,0,)",
        ] {
            let layout = expression(source, 14.);
            let ink = layout
                .glyphs
                .iter()
                .map(|g| g.text.as_str())
                .collect::<String>();
            assert!(!ink.contains("CAS."), "{source}: {ink}");
            assert!(!ink.contains("integrate"), "{source}: {ink}");
            assert!(layout.height > 20, "{source}: {}", layout.height);
        }
        let row = expression("[1,2]", 14.);
        let matrix = expression("[[1,2],[3,4]]", 14.);
        assert!(matrix.height > row.height * 2);
    }
}
