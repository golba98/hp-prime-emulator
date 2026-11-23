//! One geometry model for textbook entry painting and pointer caret placement.
use super::expression::{ExpressionEditor, Template, TemplateKind, parentheses_follow_name};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use std::ops::Range;
mod palette;
mod results;
pub(super) use results::{history_fraction_grids, history_result, result_with_matrix_font};
pub use results::{result, text};
mod symbolic;
pub use symbolic::expression;
pub(super) use symbolic::{history_expression, history_row_expression};

#[derive(Clone, Debug)]
struct Glyph {
    x: i64,
    y: i64,
    text: String,
    px: f32,
    source: Range<usize>,
}
#[derive(Clone, Debug)]
struct Rule {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}
#[derive(Clone, Debug)]
struct SelectionBounds {
    source: Range<usize>,
    whole: bool,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaretKind {
    Text,
    Boundary,
    Delimiter,
}
#[derive(Clone, Debug)]
struct Caret {
    byte: usize,
    x: i64,
    y: i64,
    height: i64,
    kind: CaretKind,
}
#[derive(Clone, Debug, Default)]
pub struct MathLayout {
    glyphs: Vec<Glyph>,
    rules: Vec<Rule>,
    carets: Vec<Caret>,
    selection_bounds: Vec<SelectionBounds>,
    pub width: i64,
    pub height: i64,
    baseline: i64,
    entry_padding: i64,
    pub(super) includes_matrix: bool,
    pub(super) centers_in_show: bool,
    pub(super) is_matrix_grid: bool,
    pub(super) compact_numeric_grid: bool,
    align_result_with_power_base: bool,
    compact_rational_result: bool,
}
impl MathLayout {
    pub(super) fn history_result_offset(&self, result: &Self) -> i64 {
        if !self.align_result_with_power_base {
            return 0;
        }
        if result.compact_rational_result {
            // The lower field and the power base both use the same trimmed
            // descent. Align their lower edges while keeping the row bounds.
            return (self.height - result.height).max(0);
        }
        if result.has_plain_descent() {
            (self.baseline - result.baseline).max(0)
        } else {
            0
        }
    }

    pub(super) fn history_power_baseline(&self, result: &Self) -> Option<i64> {
        (self.align_result_with_power_base
            && (result.compact_rational_result || result.has_plain_descent()))
        .then_some(self.baseline)
    }

    fn has_plain_descent(&self) -> bool {
        self.rules.is_empty() && self.height == self.baseline + 3
    }

    fn translate(&mut self, x: i64, y: i64) {
        for g in &mut self.glyphs {
            g.x += x;
            g.y += y;
        }
        for r in &mut self.rules {
            r.x += x;
            r.y += y;
        }
        for bounds in &mut self.selection_bounds {
            bounds.x += x;
            bounds.y += y;
        }
        for c in &mut self.carets {
            c.x += x;
            c.y += y;
        }
    }
    fn append(&mut self, mut other: Self, x: i64, y: i64) {
        other.translate(x, y);
        self.includes_matrix |= other.includes_matrix;
        self.entry_padding = self.entry_padding.max(other.entry_padding);
        self.glyphs.extend(other.glyphs);
        self.rules.extend(other.rules);
        self.carets.extend(other.carets);
        self.selection_bounds.extend(other.selection_bounds);
    }
    fn mark_template_boundaries(&mut self, span: &Range<usize>) {
        for (byte, x) in [(span.start, 0), (span.end, self.width)] {
            self.carets.push(Caret {
                byte,
                x,
                y: 0,
                height: self.height,
                kind: CaretKind::Boundary,
            });
        }
    }
    fn trim_plain_descent(&mut self) {
        if self.has_plain_descent() {
            self.height -= 1;
            for caret in &mut self.carets {
                caret.height = caret.height.min(self.height - caret.y);
            }
        }
    }
    fn mark_delimiter(&mut self, source: Range<usize>, x: i64, inset: i64) {
        self.carets.extend([
            Caret {
                byte: source.start,
                x,
                y: 0,
                height: self.height,
                kind: CaretKind::Delimiter,
            },
            Caret {
                byte: source.end,
                x: x + inset,
                y: 0,
                height: self.height,
                kind: CaretKind::Delimiter,
            },
        ]);
        self.selection_bounds.push(SelectionBounds {
            source,
            whole: false,
            x,
            y: 0,
            width: inset,
            height: self.height,
        });
    }
    fn mark_delimiters(&mut self, (open, close): (usize, usize), inset: i64) {
        self.mark_delimiter(open..open + 1, 0, inset);
        self.mark_delimiter(close..close + 1, self.width - inset, inset);
    }
    pub fn cursor(&self, byte: usize) -> (i64, i64, i64) {
        self.carets
            .iter()
            .filter(|c| c.byte == byte)
            .min_by_key(|c| c.kind != CaretKind::Delimiter)
            .map_or((self.width, 0, self.height), |c| (c.x, c.y, c.height))
    }
    pub fn hit_test(&self, x: i64, y: i64) -> usize {
        self.carets
            .iter()
            .min_by_key(|c| {
                let dx = c.x - x;
                let dy = if c.kind == CaretKind::Boundary {
                    (y - c.y - c.height / 2).abs()
                } else if y < c.y {
                    c.y - y
                } else if y > c.y + c.height {
                    y - c.y - c.height
                } else {
                    0
                };
                dx * dx + dy * dy * 4
            })
            .map_or(0, |c| c.byte)
    }
    pub fn paint_static(&self, grob: &mut Grob, x: i64, y: i64, color: u32) {
        for g in &self.glyphs {
            draw_text(grob, &g.text, x + g.x, y + g.y, g.px, color, None, None);
        }
        for r in &self.rules {
            grob.line(
                x + r.x,
                y + r.y,
                x + r.x + r.width,
                y + r.y + r.height,
                color,
            );
        }
    }
    pub fn paint(&self, grob: &mut Grob, x: i64, y: i64, selection: Range<usize>, cursor: usize) {
        const SELECTION_COLOR: u32 = 0xb7d4f6;
        let selected =
            |source: &Range<usize>| source.start < selection.end && source.end > selection.start;
        for bounds in &self.selection_bounds {
            let left = (x + bounds.x).max(1);
            let right = (x + bounds.x + bounds.width - 1).min(318);
            let highlighted = if bounds.whole {
                selection.start <= bounds.source.start && selection.end >= bounds.source.end
            } else {
                selected(&bounds.source)
            };
            if left <= right && highlighted {
                grob.fill_rect(
                    left,
                    y + bounds.y,
                    right,
                    y + bounds.y + bounds.height - 1,
                    SELECTION_COLOR,
                );
            }
        }
        for g in &self.glyphs {
            if selected(&g.source) {
                grob.fill_rect(
                    (x + g.x).max(1),
                    y + g.y,
                    (x + g.x + measure(&g.text, g.px)).min(318),
                    y + g.y + (g.px.ceil() as i64) + 2,
                    SELECTION_COLOR,
                );
            }
            draw_text(grob, &g.text, x + g.x, y + g.y, g.px, 0, None, None);
        }
        for r in &self.rules {
            grob.line(x + r.x, y + r.y, x + r.x + r.width, y + r.y + r.height, 0);
        }
        let (cx, cy, h) = self.cursor(cursor);
        grob.line(x + cx, y + cy, x + cx, y + cy + h, 0);
    }
}
fn literal(text: &str, start: usize, px: f32) -> MathLayout {
    let height = px.ceil() as i64 + 3;
    let mut out = MathLayout {
        height,
        baseline: height - 3,
        ..Default::default()
    };
    out.carets.push(Caret {
        byte: start,
        x: 0,
        y: 0,
        height,
        kind: CaretKind::Text,
    });
    if text.is_empty() {
        // A math slot is a geometric outline, not a thin font glyph whose
        // strokes may disappear when a small palette icon is rasterized.
        let width = (px * 0.6).ceil().max(3.) as i64;
        let height = (px * 0.72).ceil().max(4.) as i64;
        let top = ((out.height - height) / 2).max(0);
        out.width = width;
        for y in [top, top + height - 1] {
            out.rules.push(Rule {
                x: 0,
                y,
                width: width - 1,
                height: 0,
            });
        }
        for x in [0, width - 1] {
            out.rules.push(Rule {
                x,
                y: top,
                width: 0,
                height: height - 1,
            });
        }
        return out;
    }
    for (byte, ch) in text.char_indices() {
        // Prime Sans keeps the imaginary unit at its original private index.
        let display = if ch == 'ⅈ' && ppl_graphics::text::prime::is_selected() {
            '\u{e003}'
        } else {
            ch
        };
        let text = display.to_string();
        let width = measure(&text, px);
        out.glyphs.push(Glyph {
            x: out.width,
            y: 0,
            text,
            px,
            source: start + byte..start + byte + ch.len_utf8(),
        });
        out.width += width;
        out.carets.push(Caret {
            byte: start + byte + ch.len_utf8(),
            x: out.width,
            y: 0,
            height,
            kind: CaretKind::Text,
        });
    }
    out
}
fn sequence(parts: Vec<MathLayout>) -> MathLayout {
    let baseline = parts.iter().map(|p| p.baseline).max().unwrap_or(0);
    let mut out = MathLayout {
        baseline,
        ..Default::default()
    };
    for part in parts {
        let x = out.width;
        let y = baseline - part.baseline;
        out.width += part.width;
        out.height = out.height.max(y + part.height);
        out.append(part, x, y);
    }
    out
}
fn list_delimiters(
    mut px: f32,
    matrix_px: f32,
    layout_parts: impl Fn(f32) -> Vec<MathLayout>,
) -> MathLayout {
    let mut parts = layout_parts(px);
    let encloses_matrix = parts.iter().any(|part| part.includes_matrix);
    if encloses_matrix {
        // Lists containing a matrix share its text scale and vertical center.
        if matrix_px < px {
            px = matrix_px;
            parts = layout_parts(px);
        }
        for part in &mut parts {
            part.baseline = part.height / 2;
        }
    }
    let body = sequence(parts);
    let mut layout = if !encloses_matrix && body.height <= px.ceil() as i64 + 3 {
        sequence(vec![text("{", px), body, text("}", px)])
    } else {
        stretched_braces(body, px, i64::from(encloses_matrix), None)
    };
    layout.centers_in_show = true;
    layout
}
fn stretched_braces(
    body: MathLayout,
    px: f32,
    padding: i64,
    delimiters: Option<(usize, usize)>,
) -> MathLayout {
    let bend = (px * 0.08).round().max(1.) as i64;
    let inset = 3 * bend + 2;
    let mut out = MathLayout {
        width: body.width + 2 * inset,
        height: body.height + 2 * padding,
        baseline: body.baseline + padding,
        ..Default::default()
    };
    if let Some(delimiters) = delimiters {
        out.mark_delimiters(delimiters, inset);
        out.entry_padding = body.entry_padding + 2 * padding;
    }
    out.append(body, inset, padding);
    let bottom = out.height - 1;
    let middle = bottom / 2;
    let brace = [
        (2 * bend, 0),
        (bend, bend),
        (bend, middle - bend),
        (0, middle),
        (bend, middle + bend),
        (bend, bottom - bend),
        (2 * bend, bottom),
    ];
    let right = out.width - 1;
    for (&(x, y), &(next_x, next_y)) in brace.iter().zip(&brace[1..]) {
        let dx = next_x - x;
        for (x, width) in [(x, dx), (right - x, -dx)] {
            out.rules.push(Rule {
                x,
                y,
                width,
                height: next_y - y,
            });
        }
    }
    out
}
fn tall_parentheses(body: MathLayout, px: f32, delimiters: Option<(usize, usize)>) -> MathLayout {
    let shoulder = (0.28 * px).ceil().max(2.) as i64;
    let inset = shoulder + 2;
    let mut out = MathLayout {
        width: body.width + 2 * inset,
        height: body.height,
        baseline: body.baseline,
        ..Default::default()
    };
    if let Some(delimiters) = delimiters {
        out.mark_delimiters(delimiters, inset);
    }
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

fn range(editor: &ExpressionEditor, source: Range<usize>, px: f32) -> MathLayout {
    if source.is_empty() {
        return literal("", source.start, px);
    }
    let mut parts = Vec::new();
    let mut pos = source.start;
    while pos < source.end {
        if let Some(template) = editor
            .templates()
            .iter()
            .filter(|t| t.span.start == pos && t.span.end <= source.end)
            .max_by_key(|t| t.span.end)
        {
            pos = template.span.end;
            if pos < source.end
                && let Some(fraction) = editor.fraction_factorial_operand(template)
            {
                parts.push(grouped_fraction_factorial(editor, template, fraction, px));
                pos += 1;
            } else if template.kind == TemplateKind::Parentheses
                && pos < source.end
                && editor.text().as_bytes()[pos] == b'!'
                && let [field] = template.fields.as_slice()
                && call_delimiters(editor.text(), template).is_none()
                && editor
                    .templates()
                    .iter()
                    .any(|child| child.kind == TemplateKind::Power && child.span == *field)
            {
                parts.push(grouped_power_factorial(editor, field, px));
                pos += 1;
            } else {
                parts.push(template_layout(editor, template, px));
            }
        } else {
            let ch = editor.text()[pos..source.end].chars().next().unwrap();
            parts.push(literal(&ch.to_string(), pos, px));
            pos += ch.len_utf8();
        }
    }
    sequence(parts)
}

fn grouped_fraction_factorial(
    editor: &ExpressionEditor,
    group: &Template,
    fraction: &Template,
    px: f32,
) -> MathLayout {
    let mut out = template_layout(editor, fraction, px);
    // The fraction carries the visible grouping. Its outside carets still
    // enclose the parentheses in the editable source.
    for caret in &mut out.carets {
        if caret.kind == CaretKind::Boundary {
            if caret.byte == fraction.span.start {
                caret.byte = group.span.start;
                caret.kind = CaretKind::Delimiter;
            } else if caret.byte == fraction.span.end {
                caret.byte = group.span.end;
                caret.kind = CaretKind::Delimiter;
            }
        }
    }
    out.selection_bounds.push(SelectionBounds {
        source: group.span.clone(),
        whole: true,
        x: 0,
        y: 0,
        width: out.width,
        height: out.height,
    });
    append_tall_factorial(&mut out, group.span.end, px);
    out
}

fn grouped_power_factorial(editor: &ExpressionEditor, field: &Range<usize>, px: f32) -> MathLayout {
    let body = range(editor, field.clone(), px);
    let mut out = grouped_power(body, px, Some((field.start - 1, field.end)));
    append_tall_factorial(&mut out, field.end + 1, px);
    out
}

fn grouped_power(mut body: MathLayout, px: f32, delimiters: Option<(usize, usize)>) -> MathLayout {
    // Grouping extends one pixel above and below the power.
    body.translate(0, 1);
    body.height += 2;
    body.baseline += 1;
    body.entry_padding += 2;
    tall_parentheses(body, px, delimiters)
}

fn power_on_grouped_base(
    body: MathLayout,
    exponent: MathLayout,
    px: f32,
    delimiters: Option<(usize, usize)>,
) -> MathLayout {
    let body_width = body.width;
    let mut base = grouped_power(body, px, None);
    let base_width = base.width;
    let inset = (base_width - body_width) / 2;
    let close_x = base_width - inset;
    // Separate the right side of the group from the start of the superscript.
    base.width += 1;
    let mut out = superscript_power(base, exponent, px);
    if let Some((open, close)) = delimiters {
        // The grouping marks cover the base. Their editing stops and
        // selection cover the whole power, including when entered from above.
        let previous = out.carets.len();
        out.mark_delimiter(open..open + 1, 0, inset);
        out.mark_delimiter(close..close + 1, close_x, inset);
        let added = out.carets.len() - previous;
        out.carets.rotate_right(added);
        out.selection_bounds.push(SelectionBounds {
            source: open..close + 1,
            whole: true,
            x: 0,
            y: 0,
            width: base_width,
            height: out.height,
        });
    }
    out
}

fn append_tall_factorial(out: &mut MathLayout, source: usize, px: f32) {
    let postfix = literal("!", source, px);
    let x = out.width;
    let y = out.baseline - postfix.baseline;
    out.width += postfix.width;
    out.mark_delimiter(source..source + 1, x, postfix.width);
    out.append(postfix, x, y);
}

fn stacked_fraction(
    upper: MathLayout,
    lower: MathLayout,
    side_margin: i64,
    gap_below_rule: i64,
) -> MathLayout {
    let width = upper.width.max(lower.width) + 2 * side_margin;
    let rule_y = upper.height + 2;
    let lower_y = rule_y + gap_below_rule;
    let mut out = MathLayout {
        width,
        height: lower_y + lower.height,
        baseline: rule_y + 2,
        ..Default::default()
    };
    let ux = (width - upper.width) / 2;
    let lx = (width - lower.width) / 2;
    out.append(upper, ux, 0);
    out.append(lower, lx, lower_y);
    out.rules.push(Rule {
        x: 0,
        y: rule_y,
        width,
        height: 0,
    });
    out
}

fn indexed_fraction(
    editor: &ExpressionEditor,
    source: &Range<usize>,
    degree: &MathLayout,
    px: f32,
) -> Option<MathLayout> {
    let template = editor
        .templates()
        .iter()
        .find(|t| t.kind == TemplateKind::Fraction && t.span == *source)?;
    let [numerator, denominator] = template.fields.as_slice() else {
        return None;
    };
    if !degree.rules.is_empty()
        || degree.height != px.ceil() as i64 + 3
        || [numerator, denominator].into_iter().any(|field| {
            field.is_empty()
                || editor
                    .templates()
                    .iter()
                    .any(|child| child.span.start >= field.start && child.span.end <= field.end)
        })
    {
        return None;
    }

    // Tighten the insets so both plain fields fit at the same size as
    // the index. Nested or empty fields use the ordinary fraction layout.
    let mut upper = range(editor, numerator.clone(), px);
    let mut lower = range(editor, denominator.clone(), px);
    upper.trim_plain_descent();
    lower.trim_plain_descent();
    let mut fraction = stacked_fraction(upper, lower, 1, 1);
    fraction.entry_padding = fraction.entry_padding.max(1);
    fraction.mark_template_boundaries(&template.span);
    Some(fraction)
}

fn superscript_rise(px: f32) -> i64 {
    (px * 0.7).round() as i64
}

fn fractional_exponent(
    editor: &ExpressionEditor,
    source: &Range<usize>,
    px: f32,
) -> Option<MathLayout> {
    let template = editor
        .templates()
        .iter()
        .find(|t| t.kind == TemplateKind::Fraction && t.span == *source)?;
    let [numerator, denominator] = template.fields.as_slice() else {
        return None;
    };
    let upper = range(editor, numerator.clone(), px);
    let lower = range(editor, denominator.clone(), px);
    let mut out = fractional_exponent_layout(upper, lower, px);
    out.mark_template_boundaries(&template.span);
    Some(out)
}

fn fractional_exponent_layout(mut upper: MathLayout, mut lower: MathLayout, px: f32) -> MathLayout {
    upper.trim_plain_descent();
    lower.trim_plain_descent();
    // Raise the whole fraction. The denominator sits on the superscript
    // baseline, so the base stays on the ordinary line.
    let baseline = upper.height + 3 + lower.baseline;
    let mut out = stacked_fraction(upper, lower, 1, 1);
    out.baseline = baseline;
    out.entry_padding = out.entry_padding.max(superscript_rise(px));
    out
}

fn power(base: MathLayout, exponent: MathLayout) -> MathLayout {
    let x = base.width + 2;
    let y = exponent.height / 2;
    let mut out = MathLayout {
        width: x + exponent.width,
        height: (base.height + y).max(exponent.height),
        baseline: base.baseline + y,
        ..Default::default()
    };
    out.append(base, 0, y);
    out.append(exponent, x, 0);
    out
}

fn superscript_power(mut base: MathLayout, mut exponent: MathLayout, px: f32) -> MathLayout {
    // Place each exponent relative to its baseline so a nested exponent
    // climbs by the same step as the outer one. A grouped base adds its ascent.
    let base_ascent = (base.baseline - px.ceil() as i64).max(0);
    exponent.baseline += superscript_rise(px) + base_ascent;
    base.trim_plain_descent();
    sequence(vec![base, exponent])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RadicalPresentation {
    Ordinary,
    CompactIndexedFraction,
    SquareFraction { compact: bool },
}

fn radical(
    degree: Option<MathLayout>,
    mut body: MathLayout,
    px: f32,
    presentation: RadicalPresentation,
) -> MathLayout {
    let indexed = degree.is_some();
    let compact_fraction = presentation == RadicalPresentation::CompactIndexedFraction;
    let square_fraction = matches!(presentation, RadicalPresentation::SquareFraction { .. });
    let compact_square = matches!(
        presentation,
        RadicalPresentation::SquareFraction { compact: true }
    );
    let bar_offset = i64::from(compact_fraction);
    let mut degree = degree.unwrap_or_default();
    let mut sign = palette::mark("√", px);
    let tall = body.height > sign.height;
    if tall {
        // Leave room under the overbar and keep the hook beside the lowest
        // line of the radicand.
        let overbar_gap = if compact_square { 1 } else { 2 };
        body.translate(0, overbar_gap);
        body.height += overbar_gap;
        body.baseline += overbar_gap;
        if degree.height > 0 && degree.rules.is_empty() {
            // A plain index sits beside the lower edge of a stretched root.
            let radicand_descent = body.height - body.baseline;
            let degree_descent = degree.height - degree.baseline;
            let margin = if compact_fraction { 1 } else { 3 };
            degree.baseline -= (radicand_descent - degree_descent - margin).max(0);
        }
        sign.glyphs.clear();
        sign.height = body.height;
        sign.baseline = body.baseline;
        if indexed || square_fraction {
            // Leave a pixel between the stem and the fraction line, with
            // room at the right for the overbar to cover the full fraction.
            body.translate(1, 0);
            body.width += if square_fraction { 4 } else { 3 };
            // The descending hook joins the bottom of the vertical stem.
            let sign_scale = if compact_fraction || square_fraction {
                0.35
            } else {
                0.5
            };
            sign.width = ((px * sign_scale).round() as i64).max(4);
            let stem = sign.width - 1;
            let bottom = body.height - 1;
            let hook = ((px * 0.3).round() as i64).clamp(2, stem);
            sign.rules.extend([
                Rule {
                    x: stem - hook,
                    y: bottom - hook,
                    width: hook,
                    height: hook,
                },
                Rule {
                    x: stem,
                    y: bottom,
                    width: 0,
                    height: bar_offset - bottom,
                },
            ]);
        } else {
            sign.width = sign.width.max(5);
            let bottom = body.height - 2;
            let knee = sign.width / 2;
            let hook = knee / 2;
            let rise = (px * 0.25).ceil().max(2.) as i64;
            sign.rules.extend([
                Rule {
                    x: 0,
                    y: bottom - rise + 1,
                    width: hook,
                    height: -1,
                },
                Rule {
                    x: hook,
                    y: bottom - rise,
                    width: knee - hook,
                    height: rise,
                },
                Rule {
                    x: knee,
                    y: bottom,
                    width: sign.width - 1 - knee,
                    height: -bottom,
                },
            ]);
        }
    }
    let x = degree.width + sign.width - i64::from(tall);
    let width = body.width;
    let baseline = body.baseline;
    let mut out = sequence(vec![degree, sign, body]);
    let y = out.baseline - baseline + bar_offset;
    out.rules.push(Rule {
        x,
        y,
        width,
        height: 0,
    });
    if tall && (indexed || square_fraction) {
        out.rules.push(Rule {
            x: x + width,
            y,
            width: 0,
            height: 1,
        });
    }
    out
}

fn absolute(body: MathLayout) -> MathLayout {
    absolute_with_selection(body, None)
}

fn absolute_with_selection(
    body: MathLayout,
    delimiters: Option<(Range<usize>, Range<usize>)>,
) -> MathLayout {
    let inset = 3;
    let padding = i64::from(delimiters.is_some());
    let mut out = MathLayout {
        width: body.width + 2 * inset,
        height: body.height + 2 * padding,
        baseline: body.baseline + padding,
        ..Default::default()
    };
    if let Some((open, close)) = delimiters {
        out.mark_delimiter(open, 0, inset);
        out.mark_delimiter(close, out.width - inset, inset);
        out.entry_padding = body.entry_padding + 2 * padding;
    }
    out.append(body, inset, padding);
    for x in [padding, out.width - 1] {
        out.rules.push(Rule {
            x,
            y: 1 - padding,
            width: 0,
            height: out.height - 2 + padding,
        });
    }
    out
}

fn call_delimiters(source: &str, template: &Template) -> Option<(usize, usize)> {
    let [field] = template.fields.as_slice() else {
        return None;
    };
    let open = field.start.checked_sub(1)?;
    let close = field.end;
    if open < template.span.start
        || close >= template.span.end
        || source.as_bytes().get(open) != Some(&b'(')
        || source.as_bytes().get(close) != Some(&b')')
    {
        return None;
    }
    match template.kind {
        TemplateKind::Function(_) => Some((open, close)),
        // Unknown function names retain their normal grouping template.
        TemplateKind::Parentheses => parentheses_follow_name(source, open).then_some((open, close)),
        _ => None,
    }
}

fn editable_delimiters(
    editor: &ExpressionEditor,
    span: Range<usize>,
    (open, close): (usize, usize),
    px: f32,
    stretch: impl FnOnce(MathLayout, f32, (usize, usize)) -> MathLayout,
) -> MathLayout {
    let source = editor.text();
    let body = range(editor, open + 1..close, px);
    if body.height <= px.ceil() as i64 + 3 {
        return sequence(vec![
            literal(&source[span.start..open + 1], span.start, px),
            body,
            literal(&source[close..span.end], close, px),
        ]);
    }

    let mut parts = Vec::new();
    if span.start < open {
        parts.push(literal(&source[span.start..open], span.start, px));
    }
    parts.push(stretch(body, px, (open, close)));
    if close + 1 < span.end {
        parts.push(literal(&source[close + 1..span.end], close + 1, px));
    }
    sequence(parts)
}

fn template_layout(editor: &ExpressionEditor, t: &Template, px: f32) -> MathLayout {
    use TemplateKind::*;
    let mut out = match t.kind {
        Palette(form) => palette::layout(editor, t, form, px),
        Fraction if t.fields.len() == 2 => {
            let upper = range(editor, t.fields[0].clone(), px * 0.86);
            let lower = range(editor, t.fields[1].clone(), px * 0.86);
            stacked_fraction(upper, lower, 3, 3)
        }
        Power if t.fields.len() == 2 => {
            let base = range(editor, t.fields[0].clone(), px);
            if let Some(field) = editor.powered_base(t) {
                let exponent = range(editor, t.fields[1].clone(), px);
                power_on_grouped_base(base, exponent, px, Some((field.start - 1, field.end)))
            } else if base.baseline > px.ceil() as i64 {
                let exponent = range(editor, t.fields[1].clone(), px * 0.72);
                power(base, exponent)
            } else {
                let field = &t.fields[1];
                let exponent = fractional_exponent(editor, field, px)
                    .unwrap_or_else(|| range(editor, field.clone(), px));
                superscript_power(base, exponent, px)
            }
        }
        NthRoot if t.fields.len() == 2 => {
            let degree = range(editor, t.fields[0].clone(), px);
            let radicand = &t.fields[1];
            let fraction = indexed_fraction(editor, radicand, &degree, px);
            let presentation = if fraction.is_some() {
                RadicalPresentation::CompactIndexedFraction
            } else {
                RadicalPresentation::Ordinary
            };
            let body = fraction.unwrap_or_else(|| range(editor, radicand.clone(), px));
            radical(Some(degree), body, px, presentation)
        }
        Root => {
            let body = range(editor, t.fields[0].clone(), px);
            radical(None, body, px, RadicalPresentation::Ordinary)
        }
        Function("ABS") if t.fields.len() == 1 => {
            let field = &t.fields[0];
            let delimiters = editor
                .stacked_absolute_field(t)
                .map(|field| (t.span.start..field.start, field.end..t.span.end));
            absolute_with_selection(range(editor, field.clone(), px), delimiters)
        }
        List if let [field] = t.fields.as_slice()
            && field.start > t.span.start
            && field.end < t.span.end
            && editor.text().as_bytes().get(field.start - 1) == Some(&b'{')
            && editor.text().as_bytes().get(field.end) == Some(&b'}') =>
        {
            editable_delimiters(
                editor,
                t.span.clone(),
                (field.start - 1, field.end),
                px,
                |body, px, delimiters| stretched_braces(body, px, 1, Some(delimiters)),
            )
        }
        _ if let Some(delimiters) = call_delimiters(editor.text(), t) => editable_delimiters(
            editor,
            t.span.clone(),
            delimiters,
            px,
            |body, px, delimiters| tall_parentheses(body, px, Some(delimiters)),
        ),
        _ => {
            // Keep source syntax around fields with ordinary delimiters.
            let mut parts = Vec::new();
            let mut pos = t.span.start;
            for f in &t.fields {
                if f.start > pos {
                    parts.push(literal(&editor.text()[pos..f.start], pos, px));
                }
                parts.push(range(editor, f.clone(), px));
                pos = f.end;
            }
            if pos < t.span.end {
                parts.push(literal(&editor.text()[pos..t.span.end], pos, px));
            }
            sequence(parts)
        }
    };
    out.mark_template_boundaries(&t.span);
    out
}
pub fn linear(editor: &ExpressionEditor) -> MathLayout {
    let mut result = literal(editor.text(), 0, 14.);
    if editor.text().is_empty() {
        result.glyphs.clear();
        result.rules.clear();
        result.width = 0;
    }
    result
}
pub fn layout(editor: &ExpressionEditor, max_height: i64) -> MathLayout {
    fit(editor, max_height, false)
}
pub(super) fn entry(editor: &ExpressionEditor, max_height: i64) -> MathLayout {
    fit(editor, max_height, true)
}
fn fit(editor: &ExpressionEditor, max_height: i64, allow_entry_padding: bool) -> MathLayout {
    if editor.text().is_empty() {
        let mut empty = literal("", 0, 14.);
        empty.glyphs.clear();
        empty.rules.clear();
        empty.width = 0;
        return empty;
    }
    let mut px = 14.0;
    loop {
        let result = range(editor, 0..editor.text().len(), px);
        // Grouping marks and fractional superscripts need extra vertical
        // space without shrinking the editable fields.
        let limit = if allow_entry_padding {
            (max_height + result.entry_padding).min(96)
        } else {
            max_height
        };
        if result.height <= limit || px <= 4.0 {
            return result;
        }
        px -= 1.0;
    }
}
/// The palette uses the same notation and font as entry, fitted to its cell.
pub fn palette_icon(kind: TemplateKind, width: i64, height: i64) -> ppl_graphics::Grob {
    let mut editor = ExpressionEditor::default();
    editor.insert_template(kind);
    let mut px = 14.;
    let layout = loop {
        let layout = range(&editor, 0..editor.text().len(), px);
        if (layout.width <= width - 4 && layout.height <= height - 4) || px <= 4. {
            break layout;
        }
        px -= 0.5;
    };
    let mut image = ppl_graphics::Grob::new(width as usize, height as usize, 0xffffff);
    layout.paint_static(
        &mut image,
        (width - layout.width) / 2,
        (height - layout.height) / 2,
        0,
    );
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textbook_call_delimiters_keep_editable_source_and_fraction_stops() {
        use super::super::expression::EditCommand;

        for source in ["sin((0)/(2))", "f((0)/(2))", "sin(cos((0)/(2)))"] {
            let mut editor = ExpressionEditor::from_text(source);
            let layout = layout(&editor, 34);
            let open = source.find('(').unwrap();
            let close = source.len() - 1;
            let xs = [open, open + 1, close, close + 1].map(|byte| {
                let (x, y, height) = layout.cursor(byte);
                assert_eq!((y, height), (0, layout.height), "{source} at {byte}");
                assert_eq!(layout.hit_test(x, y + 1), byte);
                assert_eq!(layout.hit_test(x, y + height - 2), byte);
                x
            });
            assert!(xs.windows(2).all(|pair| pair[0] < pair[1]));

            let denominator = source.find('2').unwrap();
            let (x, y, height) = layout.cursor(denominator);
            assert!(height < layout.height && y > 0);
            assert_eq!(layout.hit_test(x, y + height / 2), denominator);

            editor.set_cursor(open + 1, false);
            editor.apply(EditCommand::Left(false));
            assert_eq!(editor.cursor(), open);
            editor.apply(EditCommand::Right(false));
            assert_eq!(editor.cursor(), open + 1);

            editor.set_cursor(close + 1, false);
            editor.apply(EditCommand::Left(true));
            assert_eq!(editor.selected_text(), ")");
            let mut image = Grob::new(
                layout.width as usize + 4,
                layout.height as usize + 4,
                0xffffff,
            );
            layout.paint(&mut image, 1, 1, editor.selection(), editor.cursor());
            assert_eq!(image.get(xs[2] + 3, 1), Some(0xb7d4f6));
            assert_eq!(image.get(xs[2] + 3, layout.height), Some(0xb7d4f6));
            assert_ne!(image.get(xs[0] + 3, 1), Some(0xb7d4f6));
            assert_eq!(editor.text(), source);
        }
    }

    #[test]
    fn only_tall_named_drafts_stretch_and_highlight_parentheses() {
        for source in ["sin(2)", "((0)/(2))", "\"f((0)/(2))\""] {
            let editor = ExpressionEditor::from_text(source);
            assert!(layout(&editor, 34).selection_bounds.is_empty(), "{source}");
        }
        let mut editor = ExpressionEditor::from_text("sin((0)/(2))");
        assert!(linear(&editor).selection_bounds.is_empty());
        editor.set_cursor(editor.text().len() - 1, true);
        let layout = layout(&editor, 34);
        for x in [-layout.width - 10, 330] {
            let mut image = Grob::new(320, 40, 0xffffff);
            layout.paint(&mut image, x, 1, editor.selection(), editor.cursor());
            assert!(image.pixels().iter().all(|&pixel| pixel == 0xffffff));
        }
    }

    #[test]
    fn textbook_grouped_power_factorial_has_full_height_hit_targets() {
        let source = "((2)^(2))!";
        let editor = ExpressionEditor::from_text(source);
        let layout = entry(&editor, 34);
        let power = ExpressionEditor::from_text("(2)^(2)");
        assert_eq!(layout.height, entry(&power, 34).height + 2);
        let ranges = layout
            .selection_bounds
            .iter()
            .map(|bounds| {
                assert_eq!((bounds.y, bounds.height), (0, layout.height));
                assert_eq!(layout.hit_test(bounds.x, 1), bounds.source.start);
                assert_eq!(
                    layout.hit_test(bounds.x + bounds.width, layout.height - 1),
                    bounds.source.end
                );
                bounds.source.clone()
            })
            .collect::<Vec<_>>();
        assert_eq!(ranges, [0..1, 8..9, 9..10]);
        let (x, y, height) = layout.cursor(6);
        assert!(height < layout.height);
        assert_eq!(layout.hit_test(x, y + height / 2), 6);

        for source in ["(3)!", "3!", "((2)^(2))"] {
            let editor = ExpressionEditor::from_text(source);
            assert!(entry(&editor, 34).selection_bounds.is_empty(), "{source}");
        }
        let call = ExpressionEditor::from_text("f((2)^(2))!");
        assert_eq!(entry(&call, 34).selection_bounds.len(), 2);
        assert!(linear(&editor).selection_bounds.is_empty());
    }

    #[test]
    fn textbook_fractional_exponent_raises_the_whole_stack() {
        let editor = ExpressionEditor::from_text("(16)^((1)/(2))");
        let layout = entry(&editor, 36);
        assert_eq!(layout.height, 45);
        assert_eq!(layout.rules[0].y, 18);
        let glyphs = layout
            .glyphs
            .iter()
            .map(|glyph| (glyph.source.start, glyph.px, glyph.y))
            .collect::<Vec<_>>();
        assert_eq!(
            glyphs,
            [(1, 14., 29), (2, 14., 29), (7, 14., 0), (11, 14., 19)]
        );
        for byte in [7, 11] {
            let (x, y, height) = layout.cursor(byte);
            assert_eq!(layout.hit_test(x, y + height / 2), byte);
        }

        let fraction = ExpressionEditor::from_text("(1)/(2)");
        assert_eq!(entry(&fraction, 36).height, 35);
    }

    #[test]
    fn textbook_editable_powers_keep_the_text_size_at_each_level() {
        let source = "(2)^((2)^(2))";
        let editor = ExpressionEditor::from_text(source);
        let layout = entry(&editor, 36);
        let sizes_and_rows = layout
            .glyphs
            .iter()
            .map(|glyph| (glyph.px, glyph.y))
            .collect::<Vec<_>>();
        assert_eq!(sizes_and_rows, [(14., 20), (14., 10), (14., 0)]);
        assert_eq!(layout.height, 36);
        for (byte, _) in source.match_indices('2') {
            let (x, y, height) = layout.cursor(byte);
            assert_eq!(layout.hit_test(x, y + height / 2), byte);
        }
        let grouped_base = ExpressionEditor::from_text("((2)^(2))^(2)");
        let grouped_base = entry(&grouped_base, 36);
        assert!(grouped_base.glyphs.iter().all(|glyph| glyph.px == 14.));
        assert_eq!(grouped_base.height, 38);
        assert_eq!(grouped_base.selection_bounds.len(), 3);
        for (bounds, source) in grouped_base.selection_bounds.iter().zip([0..1, 8..9]) {
            assert_eq!(bounds.source, source);
            assert_eq!((bounds.y, bounds.height), (0, 38));
            let left = grouped_base.hit_test(bounds.x, bounds.y + bounds.height - 2);
            let right =
                grouped_base.hit_test(bounds.x + bounds.width, bounds.y + bounds.height - 2);
            assert_eq!((left, right), (source.start, source.end));
        }
        assert_eq!(grouped_base.selection_bounds[2].source, 0..9);
        assert!(grouped_base.selection_bounds[2].whole);
        let (x, y, h) = grouped_base.cursor(11);
        assert_eq!(x, grouped_base.cursor(9).0 + 1);
        assert_eq!(grouped_base.hit_test(x, y + h / 2), 11);
    }

    #[test]
    fn textbook_fraction_groups_factorial_without_visible_parentheses() {
        let source = "((6)/(2))!";
        let editor = ExpressionEditor::from_text(source);
        let layout = entry(&editor, 34);
        assert_eq!(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<Vec<_>>(),
            ["6", "2", "!"]
        );
        assert_eq!(layout.selection_bounds.len(), 2);
        assert!(layout.selection_bounds[0].whole);
        assert_eq!(layout.selection_bounds[0].source, 0..9);
        assert_eq!(layout.selection_bounds[1].source, 9..10);
        for byte in [0, 9, 10] {
            let (x, y, height) = layout.cursor(byte);
            assert_eq!((y, height), (0, layout.height));
            assert_eq!(layout.hit_test(x, height - 1), byte);
        }
        let (x, y, height) = layout.cursor(6);
        assert!(height < layout.height);
        assert_eq!(layout.hit_test(x, y + height / 2), 6);
        for source in ["((6)/(2))", "(3)!", "ABS((-3)/(2))", "f((6)/(2))!"] {
            let control = ExpressionEditor::from_text(source);
            assert!(
                entry(&control, 34)
                    .selection_bounds
                    .iter()
                    .all(|bounds| !bounds.whole),
                "{source}"
            );
        }
    }

    #[test]
    fn tall_editable_list_braces_keep_source_carets_and_allow_padding() {
        use super::super::expression::EditCommand;

        for (source, expected) in [("{1,(1)/(2)}", 37), ("{0,{(1)/(2),3}}", 39)] {
            let mut editor = ExpressionEditor::from_text(source);
            let layout = entry(&editor, 36);
            assert_eq!(layout.height, expected);
            let close = source.len() - 1;
            let bounds = [0, 1, close, close + 1].map(|byte| {
                let (x, y, height) = layout.cursor(byte);
                assert_eq!((y, height), (0, expected));
                assert_eq!(layout.hit_test(x, expected - 2), byte);
                x
            });
            assert!(bounds.windows(2).all(|pair| pair[0] < pair[1]));
            editor.apply(EditCommand::Left(true));
            assert_eq!(editor.selected_text(), "}");
            let right = bounds[2] + 2;
            let mut image = Grob::new(layout.width as usize + 4, expected as usize + 4, 0xffffff);
            layout.paint(&mut image, 1, 1, editor.selection(), editor.cursor());
            assert_eq!(image.get(right, 1), Some(0xb7d4f6));
            assert_eq!(image.get(right, expected), Some(0xb7d4f6));
            assert_eq!(editor.text(), source);
            assert!(linear(&editor).selection_bounds.is_empty());
        }
        let short = ExpressionEditor::from_text("{1,2}");
        assert!(entry(&short, 36).selection_bounds.is_empty());
        assert_eq!(entry(&short, 36).height, 17);
    }

    #[test]
    fn textbook_radical_keeps_pointer_positions_inside_fraction_fields() {
        let source = "NTHROOT(3,(27)/(8))";
        let editor = ExpressionEditor::from_text(source);
        let layout = layout(&editor, 96);
        let (upper_x, upper_y, upper_h) = layout.cursor(source.find("27").unwrap());
        let (lower_x, lower_y, lower_h) = layout.cursor(source.find('8').unwrap());
        assert!(upper_y + upper_h < lower_y);
        assert!(lower_y + lower_h <= layout.height);
        assert_eq!(
            layout.hit_test(upper_x, upper_y + upper_h / 2),
            source.find("27").unwrap()
        );
        assert_eq!(
            layout.hit_test(lower_x, lower_y + lower_h / 2),
            source.find('8').unwrap()
        );
    }

    #[test]
    fn textbook_power_editor_keeps_the_nested_exponent_caret_in_bounds() {
        let mut editor = ExpressionEditor::default();
        editor.insert_template(TemplateKind::Power);
        editor.insert("x");
        assert!(editor.vertical_field(false));
        for _ in 0..12 {
            editor.insert_template(TemplateKind::Fraction);
            editor.insert("1");
            assert!(editor.vertical_field(true));
        }
        editor.insert("z");
        let render = layout(&editor, 400);
        let (x, y, height) = render.cursor(editor.cursor());
        assert!(
            y + height <= render.height,
            "caret bottom {y}+{height}, layout {}",
            render.height
        );
        assert_eq!(render.hit_test(x, y + height / 2), editor.cursor());
    }

    #[test]
    fn fraction_fields_have_distinct_rows_and_pointer_positions() {
        let mut e = ExpressionEditor::default();
        e.insert_template(TemplateKind::Fraction);
        e.insert("1");
        assert!(e.vertical_field(true));
        e.insert("2");
        let l = layout(&e, 34);
        let n = l.cursor(1);
        let d = l.cursor(5);
        assert!(n.1 < d.1);
        assert_eq!(l.hit_test(n.0, n.1), 1);
        assert_eq!(l.hit_test(n.0 - 3, n.1 + n.2 / 2), 1);
        assert_eq!(l.hit_test(d.0 - 3, d.1 + d.2 / 2), 5);
        assert_eq!(l.hit_test(d.0, d.1 + d.2), 5);
        assert!(l.height <= 34);
        assert_eq!(l.rules.len(), 1);
    }
    #[test]
    fn nth_root_degree_and_body_share_pointer_and_keyboard_geometry() {
        let e = ExpressionEditor::from_text("NTHROOT(3,8)");
        let l = layout(&e, 34);
        let degree = l.cursor(9);
        let body = l.cursor(11);
        assert!(degree.0 < body.0);
        assert_eq!(degree.1, body.1);
        assert_eq!(l.hit_test(degree.0, degree.1), 9);
        assert_eq!(l.hit_test(body.0, body.1), 11);
        assert_eq!(l.rules.len(), 1);
        let field = e
            .templates()
            .iter()
            .find(|t| t.kind == TemplateKind::NthRoot)
            .unwrap()
            .fields[1]
            .clone();
        let beginning = l.cursor(field.start);
        assert_eq!(l.hit_test(beginning.0, beginning.1), field.start);
        assert_eq!(l.height, 17);
    }
    #[test]
    fn nth_root_index_stays_beside_the_lower_fraction_field() {
        let source = "NTHROOT(3,(54)/(2))";
        let draft = layout(&ExpressionEditor::from_text(source), 34);
        for rendered in [&draft, &expression(source, 12.)] {
            let glyph = |digit| {
                rendered
                    .glyphs
                    .iter()
                    .find(|glyph| glyph.text == digit)
                    .unwrap()
            };
            let degree = glyph("3");
            let numerator = glyph("5");
            let denominator = glyph("2");
            assert!(degree.y > numerator.y + 8);
            assert!((degree.y - denominator.y).abs() <= 4);
        }
        let (x, y, height) = draft.cursor(8);
        assert_eq!(draft.hit_test(x, y + height / 2), 8);
    }
    #[test]
    fn stretched_indexed_root_joins_the_hook_stem_and_overbar() {
        let source = "NTHROOT(3,(54)/(2))";
        let draft = layout(&ExpressionEditor::from_text(source), 34);
        for rendered in [&draft, &expression(source, 12.)] {
            let stem = rendered
                .rules
                .iter()
                .find(|rule| rule.width == 0 && rule.height < -1)
                .unwrap();
            let top = stem.y + stem.height;
            assert_eq!(stem.y, rendered.height - 1);
            let hook = rendered
                .rules
                .iter()
                .find(|rule| rule.width == rule.height && rule.width > 0)
                .unwrap();
            assert_eq!(
                (hook.x + hook.width, hook.y + hook.height),
                (stem.x, stem.y)
            );
            let bar = rendered
                .rules
                .iter()
                .find(|rule| rule.x == stem.x && rule.y == top && rule.height == 0)
                .unwrap();
            assert!(rendered.rules.iter().any(|rule| rule.x == bar.x + bar.width
                && rule.y == top
                && rule.width == 0
                && rule.height == 1));
        }
    }
    #[test]
    fn indexed_fraction_draft_keeps_degree_and_both_fields_at_full_size() {
        let source = "NTHROOT(12,(8192)/(2))";
        let draft = entry(&ExpressionEditor::from_text(source), 36);
        assert_eq!(draft.height, 37);
        assert!(draft.glyphs.iter().all(|glyph| glyph.px == 14.));
        let glyph_at = |byte| {
            draft
                .glyphs
                .iter()
                .find(|glyph| glyph.source.start == byte)
                .unwrap()
        };
        assert_eq!(glyph_at(source.find("12").unwrap()).y, 19);
        assert_eq!(glyph_at(source.find("8192").unwrap()).y, 2);
        assert_eq!(glyph_at(source.rfind('2').unwrap()).y, 21);
        assert!(
            draft
                .rules
                .iter()
                .any(|rule| rule.y == 20 && rule.height == 0)
        );
    }

    #[test]
    fn indexed_fraction_preserves_nested_and_empty_field_layout() {
        for source in [
            "NTHROOT(3,((1)/(2))/(2))",
            "NTHROOT(3,(5)/())",
            "NTHROOT((1)/(2),(8)/(2))",
        ] {
            let editor = ExpressionEditor::from_text(source);
            let root = editor
                .templates()
                .iter()
                .find(|template| template.kind == TemplateKind::NthRoot)
                .unwrap();
            let degree = range(&editor, root.fields[0].clone(), 14.);
            assert!(indexed_fraction(&editor, &root.fields[1], &degree, 14.).is_none());
        }
    }

    #[test]
    fn nested_fraction_layout_stays_inside_entry_height() {
        let mut e = ExpressionEditor::default();
        e.insert_template(TemplateKind::Fraction);
        e.insert_template(TemplateKind::Fraction);
        e.insert("12");
        let l = layout(&e, 34);
        assert!(l.height <= 34);
        assert!(l.width > 0);
        let mut image = ppl_graphics::Grob::new(l.width as usize, l.height as usize, 0xffffff);
        l.paint_static(&mut image, 0, 0, 0);
        assert!(image.pixels().iter().any(|&pixel| pixel != 0xffffff));
    }
}
