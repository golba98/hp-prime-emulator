//! Textbook notation for the real math-palette fields. Decorations never claim
//! source byte positions; only editable fields and the enclosing node do.
use super::*;
use crate::ide::native::expression::palette::Form;
pub(super) fn mark(s: &str, px: f32) -> MathLayout {
    if s.is_empty() {
        return MathLayout::default();
    }
    let mut out = literal(s, 0, px);
    out.carets.clear();
    for glyph in &mut out.glyphs {
        glyph.source = 0..0;
    }
    out
}
fn fraction(upper: MathLayout, lower: MathLayout) -> MathLayout {
    let width = upper.width.max(lower.width) + 4;
    let uh = upper.height;
    let mut out = MathLayout {
        width,
        height: uh + lower.height + 5,
        baseline: uh + 3,
        ..Default::default()
    };
    let ux = (width - upper.width) / 2;
    let lx = (width - lower.width) / 2;
    out.append(upper, ux, 0);
    out.append(lower, lx, uh + 5);
    out.rules.push(Rule {
        x: 0,
        y: uh + 2,
        width,
        height: 0,
    });
    out
}
pub(super) fn brackets(parts: Vec<MathLayout>, rows: usize, cols: usize) -> MathLayout {
    matrix_grid(
        parts,
        rows,
        cols,
        MatrixSpacing {
            column_gap: 5,
            side_insets: (5, 5),
            row_gap: 3,
            top_inset: i64::from(rows != 1),
            bracket_cap: 3,
            paired_rails: false,
            round_cell_centers: false,
        },
    )
}

pub(super) fn static_brackets(
    mut parts: Vec<MathLayout>,
    rows: usize,
    cols: usize,
    native_result: bool,
    style: MatrixStyle,
) -> MathLayout {
    // Medium Prime Sans digits use a 16px cell in History and Show. Trim the
    // trailing layout descent, so the glyphs and the 4px grid gap stay put.
    let compact = match style {
        MatrixStyle::Standard => false,
        MatrixStyle::CompactHistory => true,
        MatrixStyle::CompactShow => cols == 2,
    } && parts.iter().all(|part| {
        part.has_plain_descent()
            && !part.glyphs.is_empty()
            && part.glyphs.iter().all(|glyph| {
                glyph.px == 14. && glyph.y == 0 && glyph.text.bytes().all(|c| c.is_ascii_digit())
            })
    });
    if compact {
        for part in &mut parts {
            part.trim_plain_descent();
        }
    }
    let mut layout = if compact && style == MatrixStyle::CompactShow {
        matrix_grid(
            parts,
            rows,
            cols,
            MatrixSpacing {
                column_gap: 8,
                side_insets: (6, 5),
                row_gap: 4,
                top_inset: 0,
                bracket_cap: 4,
                paired_rails: true,
                round_cell_centers: false,
            },
        )
    } else {
        static_matrix(parts, rows, cols, native_result, false)
    };
    layout.compact_numeric_grid = compact;
    layout
}

pub(super) fn history_fraction_brackets(
    parts: Vec<MathLayout>,
    rows: usize,
    cols: usize,
) -> MathLayout {
    static_matrix(parts, rows, cols, false, true)
}

fn static_matrix(
    parts: Vec<MathLayout>,
    rows: usize,
    cols: usize,
    native_result: bool,
    round_cell_centers: bool,
) -> MathLayout {
    let (column_gap, side_inset, bracket_cap) = if native_result { (12, 8, 4) } else { (5, 5, 3) };
    matrix_grid(
        parts,
        rows,
        cols,
        MatrixSpacing {
            column_gap,
            side_insets: (side_inset, side_inset),
            row_gap: 4,
            top_inset: 0,
            bracket_cap,
            paired_rails: false,
            round_cell_centers,
        },
    )
}

struct MatrixSpacing {
    column_gap: i64,
    side_insets: (i64, i64),
    row_gap: i64,
    top_inset: i64,
    bracket_cap: i64,
    paired_rails: bool,
    round_cell_centers: bool,
}

fn matrix_grid(
    parts: Vec<MathLayout>,
    rows: usize,
    cols: usize,
    spacing: MatrixSpacing,
) -> MathLayout {
    let MatrixSpacing {
        column_gap,
        side_insets: (left_inset, right_inset),
        row_gap,
        top_inset,
        bracket_cap,
        paired_rails,
        round_cell_centers,
    } = spacing;
    let widths = (0..cols)
        .map(|c| {
            parts
                .iter()
                .skip(c)
                .step_by(cols)
                .map(|p| p.width)
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let heights = (0..rows)
        .map(|r| {
            parts
                .iter()
                .skip(r * cols)
                .take(cols)
                .map(|p| p.height)
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let width =
        widths.iter().sum::<i64>() + (cols as i64 - 1) * column_gap + left_inset + right_inset;
    let height = heights.iter().sum::<i64>() + (rows as i64 - 1) * row_gap + 2 * top_inset;
    let mut out = MathLayout {
        width,
        height,
        baseline: height / 2,
        includes_matrix: true,
        is_matrix_grid: true,
        ..Default::default()
    };
    let mut parts = parts.into_iter();
    let mut y = top_inset;
    for h in heights {
        let mut x = left_inset;
        for &w in &widths {
            if let Some(part) = parts.next() {
                let ox = (w - part.width) / 2;
                let oy = (h - part.height + i64::from(round_cell_centers)) / 2;
                out.append(part, x + ox, y + oy);
            }
            x += w + column_gap;
        }
        y += h + row_gap;
    }
    let rail_positions = [0, width - 1, 2, width - 3];
    let rail_count = if paired_rails { 4 } else { 2 };
    for &x in &rail_positions[..rail_count] {
        out.rules.push(Rule {
            x,
            y: 0,
            width: 0,
            height: height - 1,
        });
    }
    for x in [0, width - 1 - bracket_cap] {
        for y in [0, height - 1] {
            out.rules.push(Rule {
                x,
                y,
                width: bracket_cap,
                height: 0,
            });
        }
    }
    out
}
pub(super) fn layout(editor: &ExpressionEditor, t: &Template, form: Form, px: f32) -> MathLayout {
    let f = |i: usize, scale: f32| {
        t.fields.get(i).map_or_else(
            || mark("", px * scale),
            |r| range(editor, r.clone(), px * scale),
        )
    };
    match form {
        Form::Index => {
            let mut base = f(0, 1.);
            if let Some(field) = t.fields.first() {
                let (expr, diagnostics) =
                    ppl_syntax::parse_expression(&editor.text()[field.clone()]);
                if !diagnostics.has_errors()
                    && !matches!(
                        expr.kind,
                        ppl_syntax::ast::ExprKind::Ident(_)
                            | ppl_syntax::ast::ExprKind::Number(_)
                            | ppl_syntax::ast::ExprKind::Call(..)
                            | ppl_syntax::ast::ExprKind::Index(..)
                            | ppl_syntax::ast::ExprKind::Bracket(_)
                    )
                {
                    base = sequence(vec![mark("(", px), base, mark(")", px)]);
                }
            }
            let index = f(1, 0.65);
            let bx = base.width;
            let y = base.height / 2;
            let mut out = MathLayout {
                width: bx + index.width + 1,
                height: base.height.max(y + index.height),
                baseline: base.baseline,
                ..Default::default()
            };
            out.append(base, 0, 0);
            out.append(index, bx + 1, y);
            out
        }
        Form::Derivative => fraction(
            sequence(vec![mark("∂", px * 0.85), f(0, 0.85)]),
            sequence(vec![mark("∂", px * 0.85), f(1, 0.85)]),
        ),
        Form::LogBase => {
            let log = mark("log", px);
            let base = f(1, 0.62);
            let body = f(0, 1.);
            let bx = log.width;
            let x = bx + base.width + 2;
            let by = log.height / 2;
            let tail = sequence(vec![mark("(", px), body, mark(")", px)]);
            let mut out = MathLayout {
                width: x + tail.width,
                height: tail.height.max(by + base.height),
                baseline: tail.baseline,
                ..Default::default()
            };
            out.append(log, 0, 0);
            out.append(base, bx, by);
            out.append(tail, x, 0);
            out
        }
        Form::Matrix { rows, cols } => {
            brackets((0..rows * cols).map(|i| f(i, 1.)).collect(), rows, cols)
        }
        Form::RowVector => brackets(
            (0..t.fields.len()).map(|i| f(i, 1.)).collect(),
            1,
            t.fields.len(),
        ),
        Form::ColumnVector => brackets((0..2).map(|i| f(i, 1.)).collect(), 2, 1),
        Form::MixedFraction => sequence(vec![
            mark("(", px),
            f(0, 1.),
            mark("+", px),
            fraction(f(1, 0.85), f(2, 0.85)),
            mark(")", px),
        ]),
        Form::Sexagesimal => sequence(vec![
            f(0, 1.),
            mark("°", px),
            f(1, 1.),
            mark("′", px),
            f(2, 1.),
            mark("″", px),
        ]),
        Form::Piecewise => {
            let first = sequence(vec![f(1, 1.), mark(" if ", px), f(0, 1.)]);
            let second = sequence(vec![f(3, 1.), mark(" if ", px), f(2, 1.)]);
            let y = first.height + 2;
            let height = y + second.height;
            let brace = mark("{", height as f32 * 0.8);
            let bx = brace.width;
            let mut out = MathLayout {
                width: bx + first.width.max(second.width),
                height,
                baseline: height / 2,
                ..Default::default()
            };
            out.append(brace, 0, 0);
            out.append(first, bx, 0);
            out.append(second, bx, y);
            out
        }
        Form::Limit => {
            let label = mark("lim", px);
            let base = sequence(vec![f(1, 0.85), mark("→", px * 0.85), f(2, 0.85)]);
            let direction = f(3, 0.55);
            let offset = direction.height / 2;
            let mut lower = MathLayout {
                width: base.width + direction.width,
                height: base.height + offset,
                baseline: base.baseline + offset,
                ..Default::default()
            };
            let bx = base.width;
            lower.append(base, 0, offset);
            lower.append(direction, bx, 0);
            let left = label.width.max(lower.width);
            let y = label.height + 3;
            let height = y + lower.height;
            let body = f(0, 1.);
            let mut out = MathLayout {
                width: left + body.width + 2,
                height: height.max(body.height),
                baseline: label.baseline,
                ..Default::default()
            };
            let lx = (left - label.width) / 2;
            let dx = (left - lower.width) / 2;
            out.append(label, lx, 0);
            out.append(lower, dx, y);
            out.append(body, left + 2, 0);
            out
        }
        Form::Integral => {
            let symbol = mark("∫", px * 2.1);
            let upper = f(3, 0.65);
            let lower = f(2, 0.65);
            let tail = sequence(vec![f(0, 1.), mark("d", px), f(1, 1.)]);
            let height = symbol.height.max(tail.height * 2 + 1);
            let limit_x = symbol.width / 2;
            let tail_x = limit_x + upper.width.max(lower.width) + 2;
            let lower_y = height - lower.height;
            let middle = (height - tail.height) / 2;
            let mut out = MathLayout {
                width: tail_x + tail.width,
                height,
                baseline: middle + tail.baseline,
                ..Default::default()
            };
            out.append(symbol, 0, 0);
            out.append(upper, limit_x, 0);
            out.append(lower, limit_x, lower_y);
            out.append(tail, tail_x, middle);
            out
        }
        Form::Sum | Form::Product => {
            let symbol = mark(if form == Form::Sum { "∑" } else { "∏" }, px * 2.35);
            let lower = sequence(vec![f(1, 0.65), mark("=", px * 0.65), f(2, 0.65)]);
            let upper = f(3, 0.65);
            let width = lower.width.max(upper.width).max(symbol.width);
            let uy = upper.height;
            let ly = uy + symbol.height;
            let height = ly + lower.height;
            let tail = sequence(vec![mark("(", px), f(0, 1.), mark(")", px)]);
            let tail_y = uy + (symbol.height - tail.height) / 2;
            let mut out = MathLayout {
                width: width + 2 + tail.width,
                height,
                baseline: tail_y + tail.baseline,
                ..Default::default()
            };
            let sx = (width - symbol.width) / 2;
            let ux = (width - upper.width) / 2;
            let lx = (width - lower.width) / 2;
            out.append(upper, ux, 0);
            out.append(symbol, sx, uy);
            out.append(lower, lx, ly);
            out.append(tail, width + 2, tail_y);
            out
        }
    }
}
