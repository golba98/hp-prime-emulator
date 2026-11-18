//! Static textbook geometry follows parsed precedence without changing source.
use super::*;
use ppl_syntax::ast::{BinOp, Expr, ExprKind, PostOp};

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
    if let ExprKind::Bracket(items) = &e.kind
        && !items.is_empty()
    {
        let layout = |item: &Expr| node(item, source, px, depth + 1);
        let rows: Option<Vec<_>> = items
            .iter()
            .map(|item| match &item.kind {
                ExprKind::Bracket(cells) if !cells.is_empty() => Some(cells.as_slice()),
                _ => None,
            })
            .collect();
        if let Some(rows) = rows {
            let cols = rows[0].len();
            if rows.iter().all(|row| row.len() == cols) {
                let cells = rows.iter().flat_map(|row| row.iter().map(layout)).collect();
                return palette::brackets(cells, rows.len(), cols);
            }
        }
        return palette::brackets(items.iter().map(layout).collect(), 1, items.len());
    }
    let editor = ExpressionEditor::from_text(raw);
    if editor
        .templates()
        .iter()
        .any(|t| t.span == (0..raw.len()) && matches!(t.kind, TemplateKind::Palette(_)))
    {
        return range(&editor, 0..raw.len(), px);
    }
    if let ExprKind::List(items) = &e.kind
        && let Some(layout) = list(e, items, source, px, depth)
    {
        return layout;
    }
    // Static math can scroll in Show and Help. Keep deeply nested glyphs
    // legible without enlarging a layout requested at a smaller scale.
    let smaller = |scale: f32| (px * scale).max(px.min(7.));
    match &e.kind {
        ExprKind::Binary(BinOp::Div, a, b) => {
            let upper = node(a, source, smaller(0.86), depth + 1);
            let lower = node(b, source, smaller(0.86), depth + 1);
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
            let base = grouped_operand(a, source, px, depth + 1);
            let exponent = node(b, source, smaller(0.72), depth + 1);
            power(base, exponent)
        }
        ExprKind::Postfix(PostOp::Factorial, operand) => {
            let body = grouped_operand(operand, source, px, depth + 1);
            let suffix = &source[operand.span.end as usize..e.span.end as usize];
            sequence(vec![body, text(suffix, px)])
        }
        ExprKind::Call(f, args)
            if matches!(&f.kind, ExprKind::Ident(name) if name.eq_ignore_ascii_case("ABS"))
                && args.len() == 1 =>
        {
            absolute(node(&args[0], source, px, depth + 1))
        }
        ExprKind::Call(f, args)
            if matches!(&f.kind, ExprKind::Ident(name) if name.eq_ignore_ascii_case("SQRT"))
                && args.len() == 1 =>
        {
            radical(None, node(&args[0], source, px, depth + 1), px)
        }
        ExprKind::Call(f, args)
            if matches!(&f.kind, ExprKind::Ident(name) if name.eq_ignore_ascii_case("NTHROOT"))
                && args.len() == 2 =>
        {
            let degree = node(&args[0], source, px, depth + 1);
            let body = node(&args[1], source, px, depth + 1);
            radical(Some(degree), body, px)
        }
        ExprKind::Call(callee, args) => named_call(e, callee, args, source, px, depth)
            .unwrap_or_else(|| {
                sequence(compose_children(
                    source,
                    e.span.start as usize..e.span.end as usize,
                    std::iter::once(callee.as_ref()).chain(args),
                    px,
                    depth,
                ))
            }),
        ExprKind::Str(_) => text(raw, px),
        _ => {
            let children: Vec<&Expr> = match &e.kind {
                ExprKind::Binary(_, a, b)
                | ExprKind::Assign(a, b)
                | ExprKind::Store(a, b)
                | ExprKind::ComplexPair(a, b) => vec![a, b],
                ExprKind::Unary(_, a) | ExprKind::Postfix(_, a) | ExprKind::Quoted(a) => vec![a],
                ExprKind::Index(a, args) => {
                    std::iter::once(a.as_ref()).chain(args.iter()).collect()
                }
                ExprKind::List(items) | ExprKind::Bracket(items) => items.iter().collect(),
                _ => vec![],
            };
            if children.is_empty() {
                return flat(raw, px);
            }
            sequence(compose_children(
                source,
                e.span.start as usize..e.span.end as usize,
                children,
                px,
                depth,
            ))
        }
    }
}

fn list(expr: &Expr, items: &[Expr], source: &str, px: f32, depth: usize) -> Option<MathLayout> {
    let start = expr.span.start as usize;
    let end = expr.span.end as usize;
    let first = items.first()?.span.start as usize;
    let last = items.last()?.span.end as usize;
    // Parenthesized lists include the enclosing parentheses in their span.
    // Identify the braces from the gaps around the items and keep any
    // grouping outside those braces.
    let leading = source
        .get(start..first)?
        .trim_start_matches(|c: char| c == '(' || c.is_whitespace());
    let trailing = source.get(last..end)?.trim_start();
    let open = first - leading.len();
    let close = end - trailing.len();
    leading.strip_prefix('{')?;
    trailing.strip_prefix('}')?;
    let body = list_delimiters(
        sequence(compose_children(source, open + 1..close, items, px, depth)),
        px,
    );
    if open == start && close + 1 == end {
        return Some(body);
    }
    let mut parts = Vec::with_capacity(3);
    if start < open {
        parts.push(text(&source[start..open], px));
    }
    parts.push(body);
    if close + 1 < end {
        parts.push(text(&source[close + 1..end], px));
    }
    Some(sequence(parts))
}
fn compose_children<'a>(
    source: &str,
    span: Range<usize>,
    children: impl IntoIterator<Item = &'a Expr>,
    px: f32,
    depth: usize,
) -> Vec<MathLayout> {
    let mut parts = Vec::new();
    let mut pos = span.start;
    for child in children {
        let start = child.span.start as usize;
        if start > pos {
            parts.push(text(&source[pos..start], px));
        }
        parts.push(node(child, source, px, depth + 1));
        pos = child.span.end as usize;
    }
    if pos < span.end {
        parts.push(text(&source[pos..span.end], px));
    }
    parts
}

fn named_call(
    call: &Expr,
    callee: &Expr,
    args: &[Expr],
    source: &str,
    px: f32,
    depth: usize,
) -> Option<MathLayout> {
    if !matches!(callee.kind, ExprKind::Ident(_)) {
        return None;
    }
    let after_name = callee.span.end as usize;
    let first = args.first()?.span.start as usize;
    let before_first = source.get(after_name..first)?.trim_start();
    let open = first - before_first.strip_prefix('(')?.len() - 1;
    let after_args = args.last()?.span.end as usize;
    let end = call.span.end as usize;
    let after_last = source.get(after_args..end)?.trim_start();
    let close = end - after_last.strip_prefix(')')?.len() - 1;
    // Only the parentheses that belong to this call are replaced. Other
    // source delimiters, commas and spacing keep their usual layout.
    let mut parts = compose_children(source, call.span.start as usize..open, [callee], px, depth);
    let body = sequence(compose_children(source, open + 1..close, args, px, depth));
    if body.height > px.ceil() as i64 + 3 {
        parts.push(tall_parentheses(body, px));
    } else {
        parts.extend([text("(", px), body, text(")", px)]);
    }
    if close + 1 < end {
        parts.push(text(&source[close + 1..end], px));
    }
    Some(sequence(parts))
}

fn tall_parentheses(body: MathLayout, px: f32) -> MathLayout {
    let shoulder = (0.28 * px).ceil().max(2.) as i64;
    let inset = shoulder + 2;
    let mut out = MathLayout {
        width: body.width + 2 * inset,
        height: body.height,
        baseline: body.baseline,
        ..Default::default()
    };
    out.append(body, inset, 0);
    let bottom = out.height - 1;
    let right = out.width - 1;
    // Join samples from a half-ellipse on each side. The horizontal size
    // stays proportional to the text while the vertical size follows the
    // argument block, so even a nested fraction has room between the marks.
    const STEPS: i64 = 12;
    let mut previous = (shoulder, 0);
    for i in 1..=STEPS {
        let t = 2. * i as f64 / STEPS as f64 - 1.;
        let x = (shoulder as f64 * (1. - (1. - t * t).sqrt())).round() as i64;
        let y = bottom * i / STEPS;
        if y == previous.1 {
            continue;
        }
        let (sx, sy) = previous;
        for (x, dx) in [(sx, x - sx), (right - sx, sx - x)] {
            out.rules.push(Rule {
                x,
                y: sy,
                width: dx,
                height: y - sy,
            });
        }
        previous = (x, y);
    }
    out
}

fn grouped_operand(operand: &Expr, source: &str, px: f32, depth: usize) -> MathLayout {
    let body = node(operand, source, px, depth);
    let raw = source[operand.span.start as usize..operand.span.end as usize].trim();
    if depth < 64
        && matches!(operand.kind, ExprKind::Binary(BinOp::Pow | BinOp::Div, ..))
        && raw.starts_with('(')
        && raw.ends_with(')')
    {
        // Power and fraction layouts consume their source delimiters. Restore
        // the group when another operator applies to the entire expression.
        sequence(vec![text("(", px), body, text(")", px)])
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_calls_enclose_tall_arguments_and_keep_ordinary_parentheses() {
        let short = expression("sin(x)", 17.);
        assert!(short.rules.is_empty());
        assert_eq!(
            short
                .glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<String>(),
            "sin(x)"
        );

        for source in [
            "sin(x^2)",
            "sin((x+1)/(x^2+2))",
            "f((x+1)/(y+2),(m+1)/(n+2))",
        ] {
            let call = expression(source, 17.);
            let vertical_segments: Vec<_> =
                call.rules.iter().filter(|rule| rule.height > 0).collect();
            assert!(vertical_segments.iter().filter(|rule| rule.y == 0).count() >= 2);
            assert_eq!(
                vertical_segments
                    .iter()
                    .map(|rule| rule.y + rule.height)
                    .max(),
                Some(call.height - 1)
            );
            assert!(call.glyphs.iter().all(|glyph| glyph.y >= 0));
        }
        let multiple = expression("f((x+1)/(y+2),(m+1)/(n+2))", 17.);
        assert_eq!(
            multiple
                .rules
                .iter()
                .filter(|rule| rule.height == 0)
                .count(),
            2
        );
        assert!(multiple.glyphs.iter().any(|g| g.text == ","));
    }

    #[test]
    fn a_nested_call_preserves_its_own_delimiters_inside_the_outer_call() {
        let inner = expression("sin(x/(y+1))", 17.);
        let outer = expression("atan(sin(x/(y+1))/(z+2))", 17.);
        let sides =
            |layout: &MathLayout| layout.rules.iter().filter(|rule| rule.height > 0).count();
        assert!(sides(&inner) > 0);
        assert!(sides(&outer) > sides(&inner));
        assert!(outer.height > inner.height);
        assert_eq!(
            outer.rules.iter().filter(|rule| rule.height == 0).count(),
            2
        );
        assert!(
            outer
                .rules
                .iter()
                .all(|rule| rule.y >= 0 && rule.y + rule.height < outer.height)
        );
    }

    #[test]
    fn textbook_absolute_value_encloses_every_row_of_its_fraction() {
        let layout = expression("abs(-3/(2+sqrt(5/7)))", 17.);
        let bars: Vec<_> = layout
            .rules
            .iter()
            .filter(|rule| rule.width == 0 && rule.height >= layout.height - 2)
            .map(|rule| rule.x)
            .collect();
        assert_eq!(bars, [0, layout.width - 1]);
        assert_eq!(
            layout.rules.iter().filter(|rule| rule.height == 0).count(),
            3
        );
        assert!(!layout.glyphs.iter().any(|glyph| glyph.text == "/"));
    }

    #[test]
    fn textbook_factorial_preserves_its_grouped_power_operand() {
        let layout = expression("(x^2)!", 17.);
        assert_eq!(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<String>(),
            "(x2)!"
        );
        let glyph = |text| {
            layout
                .glyphs
                .iter()
                .find(|glyph| glyph.text == text)
                .unwrap()
        };
        assert!(glyph("2").y < glyph("x").y);
        assert!(glyph("!").x > glyph(")").x);
    }

    #[test]
    fn cas_fraction_matrix_source_uses_stacked_fractions() {
        let layout = expression("[[1/2,2/3],[3/4,4/5]]", 17.);
        assert_eq!(
            layout
                .rules
                .iter()
                .filter(|r| r.height == 0 && r.width > 5)
                .count(),
            4
        );
        assert!(!layout.glyphs.iter().any(|glyph| glyph.text == "/"));
        assert!(layout.height > 60);
    }

    #[test]
    fn textbook_radicals_stack_and_enclose_fraction_fields() {
        for (source, numerator, denominator) in
            [("SQRT(9/4)", "9", "4"), ("NTHROOT(3,27/8)", "7", "8")]
        {
            let layout = expression(source, 17.);
            assert!(
                !layout.glyphs.iter().any(|glyph| glyph.text == "/"),
                "inline slash under {source}"
            );
            let digit = |text| {
                layout
                    .glyphs
                    .iter()
                    .find(|glyph| glyph.text == text)
                    .unwrap()
            };
            let (upper, lower) = (digit(numerator), digit(denominator));
            let fraction = layout
                .rules
                .iter()
                .find(|rule| rule.height == 0 && upper.y < rule.y && rule.y < lower.y)
                .unwrap();
            let bar = layout
                .rules
                .iter()
                .find(|rule| rule.height == 0 && rule.y < upper.y)
                .unwrap();
            let stem = layout.rules.iter().find(|rule| rule.height < -1).unwrap();
            assert_eq!((stem.x + stem.width, stem.y + stem.height), (bar.x, bar.y));
            assert!(stem.y >= lower.y);
            assert!(bar.x <= fraction.x && bar.x + bar.width >= lower.x);
            assert!(bar.x + bar.width < layout.width);
            assert!(lower.y + lower.px.ceil() as i64 <= layout.height);
        }
    }

    #[test]
    fn textbook_nested_radicals_leave_clearance_between_bars() {
        let layout = expression("SQRT(SQRT(81/16))", 17.);
        let mut bar_rows: Vec<_> = layout
            .rules
            .iter()
            .filter(|rule| rule.height == 0)
            .map(|rule| rule.y)
            .collect();
        bar_rows.sort_unstable();
        assert_eq!(bar_rows.len(), 3);
        assert!(bar_rows.windows(2).all(|rows| rows[1] - rows[0] >= 2));
    }

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
    fn textbook_braces_expand_inside_a_grouped_list() {
        let layout = expression("({1/2,3})", 17.);
        let glyphs = layout
            .glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>();
        assert_eq!(glyphs, "(12,3)");
        assert!(layout.rules.iter().any(|rule| rule.y == 0));
        assert!(
            layout
                .rules
                .iter()
                .any(|rule| rule.y + rule.height == layout.height - 1)
        );
    }
    #[test]
    fn textbook_power_contains_every_fraction_line_in_a_tall_exponent() {
        let exponent = (0..18).fold("z".to_owned(), |source, _| format!("1/({source})"));
        let layout = expression(&format!("x^({exponent})"), 17.);
        assert_eq!(layout.rules.len(), 18);
        for rule in &layout.rules {
            assert!(
                rule.y + rule.height < layout.height,
                "rule {rule:?}, layout {}",
                layout.height
            );
        }
        let last = layout
            .glyphs
            .iter()
            .find(|glyph| glyph.text == "z")
            .unwrap();
        assert!(last.px >= 7.);
        assert!(last.y + last.px.ceil() as i64 <= layout.height);
    }

    #[test]
    fn textbook_power_displays_parentheses_around_a_power_base() {
        let layout = expression("(x^2)^3", 17.);
        let glyphs = layout
            .glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>();
        assert_eq!(glyphs, "(x2)3");
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
