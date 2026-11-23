//! Static textbook geometry follows parsed precedence without changing source.
use super::*;
use ppl_math::DigitGrouping;
use ppl_syntax::ast::{BinOp, Expr, ExprKind, PostOp, UnOp};

pub fn expression(source: &str, px: f32) -> MathLayout {
    expression_with_matrix_font(source, px, px)
}

pub(in crate::ide::native) fn expression_with_matrix_font(
    source: &str,
    px: f32,
    matrix_px: f32,
) -> MathLayout {
    expression_layout(source, px, matrix_px, SourcePresentation::Ordinary)
}

pub(in crate::ide::native) fn history_expression(
    source: &str,
    px: f32,
    matrix_px: f32,
    grouping: Option<DigitGrouping>,
) -> MathLayout {
    history_row_expression(source, px, matrix_px, grouping, false)
}

pub(in crate::ide::native) fn history_row_expression(
    source: &str,
    px: f32,
    matrix_px: f32,
    grouping: Option<DigitGrouping>,
    compact_matrices: bool,
) -> MathLayout {
    expression_layout(
        source,
        px,
        matrix_px,
        SourcePresentation::History(grouping, MatrixStyle::history(compact_matrices)),
    )
}

pub(in crate::ide::native) fn show_matrix_expression(source: &str, matrix_px: f32) -> MathLayout {
    expression_layout(
        source,
        17.,
        matrix_px,
        SourcePresentation::History(None, MatrixStyle::CompactShow),
    )
}

pub(super) fn rational_result(source: &str, px: f32, matrix_px: f32) -> MathLayout {
    expression_layout(source, px, matrix_px, SourcePresentation::RationalResult)
}

enum SourcePresentation {
    Ordinary,
    History(Option<DigitGrouping>, MatrixStyle),
    RationalResult,
}

fn expression_layout(
    source: &str,
    px: f32,
    matrix_px: f32,
    presentation: SourcePresentation,
) -> MathLayout {
    if source.is_empty() {
        return text("", px);
    }
    if let Ok(normalized) = ExpressionEditor::from_text(source).evaluation_text(false)
        && normalized != source
    {
        return expression_layout(&normalized, px, matrix_px, presentation);
    }
    if source.len() <= 8192 {
        let (expr, diags) = ppl_syntax::parse_expression(source);
        if !diags.has_errors() {
            if matches!(
                presentation,
                SourcePresentation::History(..) | SourcePresentation::RationalResult
            ) && let Some((numerator, denominator)) = numeric_fraction_fields(&expr)
            {
                let mut layout = compact_numeric_fraction(numerator, denominator, source, px);
                layout.compact_rational_result =
                    matches!(presentation, SourcePresentation::RationalResult);
                return layout;
            }
            if let SourcePresentation::History(grouping, matrix_style) = presentation {
                if grouping.is_some()
                    && let Some(args) = indexed_root_fields(&expr)
                {
                    return indexed_root(args, source, px, 0, matrix_px, grouping);
                }
                if let Some(layout) = history_numeric_power(&expr, source, px) {
                    return layout;
                }
                return node_with_matrix_style(&expr, source, px, 0, matrix_px, matrix_style);
            }
            return node(&expr, source, px, 0, matrix_px);
        }
    }
    flat(source, px)
}
fn flat(source: &str, px: f32) -> MathLayout {
    range(&ExpressionEditor::from_text(source), 0..source.len(), px)
}

fn numeric_source<'a>(operand: &Expr, source: &'a str) -> &'a str {
    source[operand.span.start as usize..operand.span.end as usize]
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '(' | ')'))
}

fn numeric_fraction_fields(expr: &Expr) -> Option<(&Expr, &Expr)> {
    let ExprKind::Binary(BinOp::Div, numerator, denominator) = &expr.kind else {
        return None;
    };
    (matches!(numerator.kind, ExprKind::Number(_))
        && matches!(denominator.kind, ExprKind::Number(_)))
    .then_some((numerator.as_ref(), denominator.as_ref()))
}

fn compact_numeric_fraction(
    numerator: &Expr,
    denominator: &Expr,
    source: &str,
    px: f32,
) -> MathLayout {
    let field = |operand| {
        let mut layout = flat(numeric_source(operand, source), px);
        layout.trim_plain_descent();
        layout
    };
    let mut layout = stacked_fraction(field(numerator), field(denominator), 1, 1);
    // The line includes both endpoints. Keep the final pixel within the row.
    layout.rules.last_mut().expect("fraction rule").width -= 1;
    layout
}

pub(super) fn history_fraction_grid(
    source: &str,
    px: f32,
    rows: usize,
    cols: usize,
    typed_result: bool,
) -> Option<(MathLayout, Vec<bool>)> {
    let (expr, diagnostics) = ppl_syntax::parse_expression(source);
    if diagnostics.has_errors() {
        return None;
    }
    let ExprKind::Bracket(contents) = &expr.kind else {
        return None;
    };
    if contents.len() != rows {
        return None;
    }

    let digit = |expr: &Expr| {
        matches!(expr.kind, ExprKind::Number(_))
            && matches!(numeric_source(expr, source).as_bytes(), [b'0'..=b'9'])
    };
    let mut cells = Vec::with_capacity(rows.checked_mul(cols)?);
    let mut fraction_rows = Vec::with_capacity(rows);
    for row in contents {
        let ExprKind::Bracket(fields) = &row.kind else {
            return None;
        };
        if fields.len() != cols {
            return None;
        }
        let mut has_fraction = false;
        for field in fields {
            let layout = if digit(field) {
                let mut layout = flat(numeric_source(field, source), px);
                layout.trim_plain_descent();
                layout
            } else {
                let (upper, lower) = numeric_fraction_fields(field)?;
                if ![upper, lower]
                    .into_iter()
                    .all(|expr| digit(expr) && numeric_source(expr, source) != "0")
                {
                    return None;
                }
                has_fraction = true;
                let mut compact = compact_numeric_fraction(upper, lower, source, px);
                // Keep the original column allotment while centering the
                // shorter rule and full-size digits inside it.
                let standard_width = if typed_result {
                    // CAS rational cells display plain operands while their
                    // stored expression groups both fields.
                    let fraction = format!(
                        "{}/{}",
                        numeric_source(upper, source),
                        numeric_source(lower, source)
                    );
                    expression(&fraction, px).width
                } else {
                    node(field, source, px, 1, px).width
                };
                if compact.width > standard_width {
                    return None;
                }
                compact.translate((standard_width - compact.width) / 2, 0);
                compact.width = standard_width;
                compact
            };
            cells.push(layout);
        }
        fraction_rows.push(has_fraction);
    }
    if !fraction_rows.iter().any(|&has_fraction| has_fraction) {
        return None;
    }
    Some((
        palette::history_fraction_brackets(cells, rows, cols),
        fraction_rows,
    ))
}

fn power_on_fractional_base(
    base: &Expr,
    exponent: &Expr,
    source: &str,
    px: f32,
) -> Option<MathLayout> {
    let raw_base = source[base.span.start as usize..base.span.end as usize].trim();
    if !raw_base.starts_with('(') || !raw_base.ends_with(')') {
        return None;
    }
    let (upper, lower) = numeric_fraction_fields(base)?;
    let (numerator, denominator) = numeric_fraction_fields(exponent)?;
    let number = |field| flat(numeric_source(field, source), px);
    // The surrounding parentheses group the two rows of the base; the
    // fraction bars themselves already group the stored numeric fields.
    let mut upper = number(upper);
    upper.translate(0, 1);
    let base = stacked_fraction(upper, number(lower), 0, 1);
    let mut base = tall_parentheses(base, px, None);
    // Keep the first pixel of the exponent clear of the closing mark.
    base.width += 1;
    let exponent = fractional_exponent_layout(number(numerator), number(denominator), px);
    Some(superscript_power(base, exponent, px))
}

fn history_numeric_power(expr: &Expr, source: &str, px: f32) -> Option<MathLayout> {
    let ExprKind::Binary(BinOp::Pow, base, exponent) = &expr.kind else {
        return None;
    };
    if !matches!(base.kind, ExprKind::Number(_)) {
        return None;
    }
    let ExprKind::Binary(operator @ (BinOp::Div | BinOp::Pow), first, second) = &exponent.kind
    else {
        return None;
    };
    if !matches!(second.kind, ExprKind::Number(_)) {
        return None;
    }
    // The fraction bar or successive rises group the stored fields the
    // same way as calculator entry. Keep the complete source for Copy.
    let number = |field| flat(numeric_source(field, source), px);
    let exponent = match operator {
        BinOp::Div => {
            let numerator = match &first.kind {
                ExprKind::Number(_) => number(first),
                ExprKind::Unary(UnOp::Neg, magnitude)
                    if matches!(magnitude.kind, ExprKind::Number(_)) =>
                {
                    sequence(vec![text("-", px), number(magnitude)])
                }
                _ => return None,
            };
            fractional_exponent_layout(numerator, number(second), px)
        }
        BinOp::Pow if matches!(first.kind, ExprKind::Number(_)) => {
            superscript_power(number(first), number(second), px)
        }
        _ => return None,
    };
    let mut layout = superscript_power(number(base), exponent, px);
    layout.align_result_with_power_base = true;
    Some(layout)
}

fn indexed_root_fields(e: &Expr) -> Option<&[Expr; 2]> {
    let ExprKind::Call(callee, args) = &e.kind else {
        return None;
    };
    if matches!(&callee.kind, ExprKind::Ident(name) if name.eq_ignore_ascii_case("NTHROOT")) {
        args.as_slice().try_into().ok()
    } else {
        None
    }
}

fn indexed_root(
    args: &[Expr; 2],
    source: &str,
    px: f32,
    depth: usize,
    matrix_px: f32,
    grouping: Option<DigitGrouping>,
) -> MathLayout {
    let [degree, radicand] = args;
    let degree = node(degree, source, px, depth + 1, matrix_px);
    let body = if let ExprKind::Binary(BinOp::Div, numerator, denominator) = &radicand.kind
        && matches!(numerator.kind, ExprKind::Number(_))
        && matches!(denominator.kind, ExprKind::Number(_))
    {
        // The fraction bar already groups its stored field wrappers. Home
        // History adds any numeric separators at paint time.
        let field_px = (px * 0.86).max(px.min(7.));
        let field = |operand: &Expr| {
            let number = numeric_source(operand, source);
            if let Some(grouped) = grouping.and_then(|mode| mode.grouped_integer_literal(number)) {
                text(&grouped, field_px)
            } else {
                flat(number, field_px)
            }
        };
        stacked_fraction(field(numerator), field(denominator), 3, 3)
    } else {
        node(radicand, source, px, depth + 1, matrix_px)
    };
    radical(Some(degree), body, px, RadicalPresentation::Ordinary)
}

fn square_root_fraction(
    radicand: &Expr,
    source: &str,
    px: f32,
    depth: usize,
    matrix_px: f32,
) -> Option<MathLayout> {
    let ExprKind::Binary(BinOp::Div, numerator, denominator) = &radicand.kind else {
        return None;
    };
    let compact = ppl_graphics::text::prime::is_selected();
    let field = |expr: &Expr| {
        let mut layout = match &expr.kind {
            ExprKind::Number(_) => flat(numeric_source(expr, source), px),
            ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right)
                if matches!(left.kind, ExprKind::Number(_) | ExprKind::Ident(_))
                    && matches!(right.kind, ExprKind::Number(_) | ExprKind::Ident(_)) =>
            {
                // A simple sum already spans one field of the fraction.
                flat(
                    &source[left.span.start as usize..right.span.end as usize],
                    px,
                )
            }
            _ => node(expr, source, px, depth + 1, matrix_px),
        };
        // The installed calculator font uses a tighter fraction field.
        // Keep the bundled font's existing row leading.
        if compact {
            layout.trim_plain_descent();
        }
        layout
    };
    let body = stacked_fraction(field(numerator), field(denominator), 1, 1);
    Some(radical(
        None,
        body,
        px,
        RadicalPresentation::SquareFraction { compact },
    ))
}

fn node(e: &Expr, source: &str, px: f32, depth: usize, matrix_px: f32) -> MathLayout {
    node_with_matrix_style(e, source, px, depth, matrix_px, MatrixStyle::Standard)
}

fn node_with_matrix_style(
    e: &Expr,
    source: &str,
    px: f32,
    depth: usize,
    matrix_px: f32,
    matrix_style: MatrixStyle,
) -> MathLayout {
    let raw = &source[e.span.start as usize..e.span.end as usize];
    if depth >= 64 {
        return text(raw, px);
    }
    if let ExprKind::Bracket(items) = &e.kind
        && !items.is_empty()
    {
        let layout = |item: &Expr| node(item, source, px.min(matrix_px), depth + 1, matrix_px);
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
                return palette::static_brackets(cells, rows.len(), cols, false, matrix_style);
            }
        }
        return palette::static_brackets(
            items.iter().map(layout).collect(),
            1,
            items.len(),
            false,
            MatrixStyle::Standard,
        );
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
        && let Some(layout) = list(e, items, source, px, depth, matrix_px)
    {
        return layout;
    }
    if let Some(args) = indexed_root_fields(e) {
        return indexed_root(args, source, px, depth, matrix_px, None);
    }
    // Static math can scroll in Show and Help. Keep deeply nested glyphs
    // legible without enlarging a layout requested at a smaller scale.
    let smaller = |scale: f32| (px * scale).max(px.min(7.));
    match &e.kind {
        ExprKind::Binary(BinOp::Div, a, b) => {
            let upper = node(a, source, smaller(0.86), depth + 1, matrix_px);
            let lower = node(b, source, smaller(0.86), depth + 1, matrix_px);
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
            if let Some(layout) = power_on_fractional_base(a, b, source, px) {
                return layout;
            }
            if editor
                .templates()
                .iter()
                .any(|t| t.span == (0..raw.len()) && editor.powered_base(t).is_some())
            {
                // Calculator entry encodes each field in parentheses. Reuse
                // its grouping so a saved power has just its visible pair.
                return range(&editor, 0..raw.len(), px);
            }
            let source_base = source[a.span.start as usize..a.span.end as usize].trim();
            if matches!(a.kind, ExprKind::Binary(BinOp::Pow, ..))
                && source_base.starts_with('(')
                && source_base.ends_with(')')
            {
                let base = node(a, source, px, depth + 1, matrix_px);
                let exponent = node(b, source, px, depth + 1, matrix_px);
                return power_on_grouped_base(base, exponent, px, None);
            }
            let base = grouped_operand(a, source, px, depth + 1, matrix_px);
            let exponent = node(b, source, smaller(0.72), depth + 1, matrix_px);
            power(base, exponent)
        }
        ExprKind::Postfix(PostOp::Factorial, operand) => {
            let body = grouped_operand(operand, source, px, depth + 1, matrix_px);
            let suffix = &source[operand.span.end as usize..e.span.end as usize];
            sequence(vec![body, text(suffix, px)])
        }
        ExprKind::Call(f, args)
            if matches!(&f.kind, ExprKind::Ident(name) if name.eq_ignore_ascii_case("ABS"))
                && args.len() == 1 =>
        {
            absolute(node(&args[0], source, px, depth + 1, matrix_px))
        }
        ExprKind::Call(f, args)
            if matches!(&f.kind, ExprKind::Ident(name) if name.eq_ignore_ascii_case("SQRT"))
                && args.len() == 1 =>
        {
            if let Some(layout) = square_root_fraction(&args[0], source, px, depth + 1, matrix_px) {
                return layout;
            }
            radical(
                None,
                node(&args[0], source, px, depth + 1, matrix_px),
                px,
                RadicalPresentation::Ordinary,
            )
        }
        ExprKind::Call(callee, args) => named_call(e, callee, args, source, px, depth, matrix_px)
            .unwrap_or_else(|| {
                sequence(compose_children(
                    source,
                    e.span.start as usize..e.span.end as usize,
                    std::iter::once(callee.as_ref()).chain(args),
                    px,
                    depth,
                    matrix_px,
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
                matrix_px,
            ))
        }
    }
}

fn list(
    expr: &Expr,
    items: &[Expr],
    source: &str,
    px: f32,
    depth: usize,
    matrix_px: f32,
) -> Option<MathLayout> {
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
    let body = list_delimiters(px, matrix_px, |px| {
        compose_children(source, open + 1..close, items, px, depth, matrix_px)
    });
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
    let mut layout = sequence(parts);
    layout.centers_in_show = true;
    Some(layout)
}
fn compose_children<'a>(
    source: &str,
    span: Range<usize>,
    children: impl IntoIterator<Item = &'a Expr>,
    px: f32,
    depth: usize,
    matrix_px: f32,
) -> Vec<MathLayout> {
    let mut parts = Vec::new();
    let mut pos = span.start;
    for child in children {
        let start = child.span.start as usize;
        if start > pos {
            parts.push(text(&source[pos..start], px));
        }
        parts.push(node(child, source, px, depth + 1, matrix_px));
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
    matrix_px: f32,
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
    let mut parts = compose_children(
        source,
        call.span.start as usize..open,
        [callee],
        px,
        depth,
        matrix_px,
    );
    let body = sequence(compose_children(
        source,
        open + 1..close,
        args,
        px,
        depth,
        matrix_px,
    ));
    let stretched = body.height > px.ceil() as i64 + 3;
    if stretched {
        parts.push(tall_parentheses(body, px, None));
    } else {
        parts.extend([text("(", px), body, text(")", px)]);
    }
    if close + 1 < end {
        parts.push(text(&source[close + 1..end], px));
    }
    let mut layout = sequence(parts);
    layout.centers_in_show = stretched;
    Some(layout)
}

fn grouped_operand(
    operand: &Expr,
    source: &str,
    px: f32,
    depth: usize,
    matrix_px: f32,
) -> MathLayout {
    let body = node(operand, source, px, depth, matrix_px);
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
    fn static_numeric_fractional_power_encloses_the_base_and_retains_both_fractions() {
        let bare = expression("(16/81)^(3/4)", 14.);
        let wrapped = expression("((16)/(81))^((3)/(4))", 14.);
        let fields = |layout: &MathLayout| {
            layout
                .glyphs
                .iter()
                .map(|glyph| (glyph.text.clone(), glyph.x, glyph.y, glyph.px))
                .collect::<Vec<_>>()
        };
        assert_eq!(fields(&bare), fields(&wrapped));
        let digits = bare
            .glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>();
        assert_eq!(digits, "168134");
        assert!(bare.glyphs.iter().all(|glyph| glyph.px == 14.));
        let bars = bare
            .rules
            .iter()
            .filter(|rule| rule.height == 0)
            .collect::<Vec<_>>();
        assert_eq!(bars.len(), 2);
        assert!(bars[0].y - bars[1].y >= 28);
        assert!(bare.rules.iter().filter(|rule| rule.height > 0).count() > 2);
        let ordinary = expression("(16/81)^2", 14.);
        assert_eq!(ordinary.rules.len(), 1);
    }

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
    fn square_root_stretches_around_direct_fractions_and_groups_their_fields() {
        for (source, fields) in [
            ("SQRT(9/16)", "916"),
            ("SQRT((9)/(16))", "916"),
            ("SQRT(x/(x+1))", "xx+1"),
        ] {
            let layout = expression(source, 14.);
            let glyphs = layout
                .glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<String>();
            assert_eq!(glyphs, fields);
            let overbar = layout
                .rules
                .iter()
                .find(|rule| rule.height == 0 && rule.y == 0)
                .unwrap();
            assert!(
                layout
                    .rules
                    .iter()
                    .any(|rule| { rule.x == overbar.x && rule.width == 0 && rule.height < -24 })
            );
            assert!(layout.rules.iter().any(|rule| {
                rule.x == overbar.x + overbar.width
                    && rule.y == 0
                    && rule.width == 0
                    && rule.height == 1
            }));
        }
        assert!(
            expression("SQRT(2)", 14.)
                .glyphs
                .iter()
                .any(|glyph| glyph.text == "√")
        );
    }

    #[test]
    fn indexed_radical_fraction_hides_numeric_field_wrappers_in_static_layout() {
        let fields = |source| {
            expression(source, 17.)
                .glyphs
                .into_iter()
                .map(|glyph| glyph.text)
                .collect::<String>()
        };
        for source in ["NTHROOT(3,(54)/(2))", "NTHROOT(3,54/2)"] {
            assert_eq!(fields(source), "3542");
        }
        assert_eq!(fields("NTHROOT(12,(8.192E3)/(2))"), "128.192E32");
        assert_eq!(fields("SQRT((54)/(2))"), "542");
        assert!(fields("NTHROOT(3,(x+1)/(2))").contains("(x+1)"));
    }

    #[test]
    fn home_history_groups_only_plain_indexed_fraction_digits_using_the_live_mode() {
        let fields = |source, grouping| {
            history_expression(source, 14., 14., DigitGrouping::from_code(grouping))
                .glyphs
                .into_iter()
                .map(|glyph| glyph.text)
                .collect::<String>()
        };
        let source = "NTHROOT(12,(8192)/(2048))";
        assert_eq!(fields(source, 0), "128﹐1922﹐048");
        assert_eq!(fields(source, 3), "1281922048");
        assert_eq!(fields(source, 4), "128﹒1922﹒048");
        assert_eq!(
            fields("NTHROOT(12,(1234567890123456789)/(2))", 0),
            "121﹐234﹐567﹐890﹐123﹐456﹐7892"
        );
        assert_eq!(fields("NTHROOT(12,(8.192E3)/(2))", 0), "128.192E32");
        assert_eq!(fields("8192+1", 0), "8192+1");
    }

    #[test]
    fn standalone_numeric_fractions_keep_full_size_without_stored_wrappers() {
        let geometry = |layout: MathLayout| {
            let glyphs = layout
                .glyphs
                .into_iter()
                .map(|glyph| (glyph.text, glyph.x, glyph.y, glyph.px))
                .collect::<Vec<_>>();
            let rules = layout
                .rules
                .into_iter()
                .map(|rule| (rule.x, rule.y, rule.width, rule.height))
                .collect::<Vec<_>>();
            (glyphs, rules)
        };
        let expected = geometry(rational_result("1/3", 14., 14.));
        assert_eq!(expected.0.len(), 2);
        assert!(expected.0.iter().all(|glyph| glyph.3 == 14.));
        assert_eq!(expected.1.len(), 1);
        for source in ["1/3", "(1)/(3)"] {
            assert_eq!(
                geometry(history_expression(source, 14., 14., None)),
                expected
            );
        }
        assert_ne!(geometry(expression("1/3", 14.)), expected);
        for other in ["(-1)/(3)", "(x+1)/3", "{1/3,2/5}", "SQRT(1/3)"] {
            assert_eq!(
                geometry(history_expression(other, 14., 14., None)),
                geometry(expression(other, 14.))
            );
        }
    }

    #[test]
    fn history_uses_full_size_simple_fractional_exponents_without_source_wrappers() {
        let geometry = |layout: MathLayout| {
            let glyphs = layout
                .glyphs
                .into_iter()
                .map(|glyph| (glyph.text, glyph.x, glyph.y, glyph.px))
                .collect::<Vec<_>>();
            let rules = layout
                .rules
                .into_iter()
                .map(|rule| (rule.x, rule.y, rule.width, rule.height))
                .collect::<Vec<_>>();
            (glyphs, rules)
        };
        let wrapped = "(16)^((1)/(2))";
        let expected = geometry(range(
            &ExpressionEditor::from_text(wrapped),
            0..wrapped.len(),
            14.,
        ));
        assert_eq!(expected.1.len(), 1);
        assert_eq!(expected.0.len(), 4);
        for source in [wrapped, "16^(1/2)"] {
            let history = history_expression(source, 14., 14., None);
            assert_eq!(history.history_result_offset(&expression("4", 14.)), 29);
            assert_eq!(history.history_result_offset(&expression("1/2", 14.)), 0);
            assert_eq!(geometry(history), expected);
        }
        assert_ne!(geometry(expression(wrapped, 14.)), expected);
        for other in ["((2)^3)^(1/2)", "(16)^((x+1)/2)", "(-16)^(1/2)", "16^2"] {
            assert_eq!(
                geometry(history_expression(other, 14., 14., None)),
                geometry(expression(other, 14.))
            );
        }
    }

    #[test]
    fn history_keeps_a_numeric_minus_sign_inside_the_fractional_exponent() {
        let glyphs = |layout: MathLayout| {
            layout
                .glyphs
                .into_iter()
                .map(|glyph| (glyph.text, glyph.x, glyph.y, glyph.px))
                .collect::<Vec<_>>()
        };
        let canonical = "(81)^((-3)/(4))";
        let draft = range(
            &ExpressionEditor::from_text(canonical),
            0..canonical.len(),
            14.,
        );
        let expected = glyphs(draft);
        assert_eq!(
            expected
                .iter()
                .map(|(text, _, _, _)| text.as_str())
                .collect::<String>(),
            "81-34"
        );
        let reciprocal = rational_result("1/27", 14., 14.);
        let negative = rational_result("-1/27", 14., 14.);
        for source in [canonical, "81^(-3/4)", "81^((-(3))/4)"] {
            let history = history_expression(source, 14., 14., None);
            assert_eq!(history.history_result_offset(&reciprocal), 10);
            assert_eq!(history.history_result_offset(&negative), 0);
            assert_eq!(history.history_power_baseline(&reciprocal), Some(43));
            assert_eq!(history.history_power_baseline(&negative), None);
            assert_eq!(glyphs(history), expected);
        }
        assert_eq!(
            history_expression("1/27", 14., 14., None).history_result_offset(&reciprocal),
            0
        );
        for source in ["81^(3/(-4))", "81^(-x/4)", "81^(-2)", "81^(2^(-3))"] {
            assert_eq!(
                glyphs(history_expression(source, 14., 14., None)),
                glyphs(expression(source, 14.))
            );
        }
    }

    #[test]
    fn history_right_associated_numeric_power_matches_the_editable_stack() {
        let glyphs = |layout: MathLayout| {
            layout
                .glyphs
                .into_iter()
                .map(|glyph| (glyph.text, glyph.x, glyph.y, glyph.px))
                .collect::<Vec<_>>()
        };
        let wrapped = "(2)^((3)^(2))";
        let draft = range(&ExpressionEditor::from_text(wrapped), 0..wrapped.len(), 14.);
        let expected = glyphs(draft);
        assert_eq!(expected.len(), 3);
        for source in [wrapped, "2^(3^2)", "2^3^2"] {
            let layout = history_expression(source, 14., 14., None);
            assert_eq!(layout.history_result_offset(&expression("512", 14.)), 20);
            assert_eq!(glyphs(layout), expected);
        }
        for source in ["((2)^(3))^(2)", "2^(x^2)", "2^2", "2^(-3^2)", "2^(3^(2^2))"] {
            assert_eq!(
                glyphs(history_expression(source, 14., 14., None)),
                glyphs(expression(source, 14.))
            );
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
        for (source, expected) in [
            ("(x^2)^3", "x23"),
            ("(2^3)^2", "232"),
            ("((2)^(3))^(2)", "232"),
        ] {
            let layout = expression(source, 17.);
            assert!(!layout.rules.is_empty(), "{source}");
            let glyphs = layout
                .glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<String>();
            assert_eq!(glyphs, expected, "{source}");
        }
        assert!(expression("2^(3^2)", 17.).rules.is_empty());
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
