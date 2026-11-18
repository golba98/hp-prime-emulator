//! One geometry model for textbook entry painting and pointer caret placement.
use super::expression::{ExpressionEditor, Template, TemplateKind};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use std::ops::Range;
mod palette;
mod results;
pub use results::{result, text};
mod symbolic;
pub use symbolic::expression;

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
struct Caret {
    byte: usize,
    x: i64,
    y: i64,
    height: i64,
    boundary: bool,
}
#[derive(Clone, Debug, Default)]
pub struct MathLayout {
    glyphs: Vec<Glyph>,
    rules: Vec<Rule>,
    carets: Vec<Caret>,
    pub width: i64,
    pub height: i64,
    baseline: i64,
}
impl MathLayout {
    fn translate(&mut self, x: i64, y: i64) {
        for g in &mut self.glyphs {
            g.x += x;
            g.y += y;
        }
        for r in &mut self.rules {
            r.x += x;
            r.y += y;
        }
        for c in &mut self.carets {
            c.x += x;
            c.y += y;
        }
    }
    fn append(&mut self, mut other: Self, x: i64, y: i64) {
        other.translate(x, y);
        self.glyphs.extend(other.glyphs);
        self.rules.extend(other.rules);
        self.carets.extend(other.carets);
    }
    pub fn cursor(&self, byte: usize) -> (i64, i64, i64) {
        self.carets
            .iter()
            .find(|c| c.byte == byte)
            .map_or((self.width, 0, self.height), |c| (c.x, c.y, c.height))
    }
    pub fn hit_test(&self, x: i64, y: i64) -> usize {
        self.carets
            .iter()
            .min_by_key(|c| {
                let dx = c.x - x;
                let dy = if c.boundary {
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
        for g in &self.glyphs {
            if g.source.start < selection.end && g.source.end > selection.start {
                grob.fill_rect(
                    (x + g.x).max(1),
                    y + g.y,
                    (x + g.x + measure(&g.text, g.px)).min(318),
                    y + g.y + (g.px.ceil() as i64) + 2,
                    0xb7d4f6,
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
        boundary: false,
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
        let width = measure(&ch.to_string(), px);
        out.glyphs.push(Glyph {
            x: out.width,
            y: 0,
            text: ch.to_string(),
            px,
            source: start + byte..start + byte + ch.len_utf8(),
        });
        out.width += width;
        out.carets.push(Caret {
            byte: start + byte + ch.len_utf8(),
            x: out.width,
            y: 0,
            height,
            boundary: false,
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
fn list_delimiters(body: MathLayout, px: f32) -> MathLayout {
    if body.height <= px.ceil() as i64 + 3 {
        return sequence(vec![text("{", px), body, text("}", px)]);
    }

    let bend = (px * 0.08).round().max(1.) as i64;
    let inset = 3 * bend + 2;
    let mut out = MathLayout {
        width: body.width + 2 * inset,
        height: body.height,
        baseline: body.baseline,
        ..Default::default()
    };
    out.append(body, inset, 0);
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
            parts.push(template_layout(editor, template, px));
            pos = template.span.end;
        } else {
            let ch = editor.text()[pos..source.end].chars().next().unwrap();
            parts.push(literal(&ch.to_string(), pos, px));
            pos += ch.len_utf8();
        }
    }
    sequence(parts)
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

fn radical(degree: Option<MathLayout>, mut body: MathLayout, px: f32) -> MathLayout {
    let degree = degree.unwrap_or_default();
    let mut sign = palette::mark("√", px);
    let tall = body.height > sign.height;
    if tall {
        // Leave room under the overbar and keep the hook beside the lowest
        // line of the radicand. The degree stays on the expression baseline.
        body.translate(0, 2);
        body.height += 2;
        body.baseline += 2;
        sign.glyphs.clear();
        sign.width = sign.width.max(5);
        sign.height = body.height;
        sign.baseline = body.baseline;
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
    let x = degree.width + sign.width - i64::from(tall);
    let width = body.width;
    let baseline = body.baseline;
    let mut out = sequence(vec![degree, sign, body]);
    out.rules.push(Rule {
        x,
        y: out.baseline - baseline,
        width,
        height: 0,
    });
    out
}

fn absolute(body: MathLayout) -> MathLayout {
    let mut out = MathLayout {
        width: body.width + 6,
        height: body.height,
        baseline: body.baseline,
        ..Default::default()
    };
    out.append(body, 3, 0);
    for x in [0, out.width - 1] {
        out.rules.push(Rule {
            x,
            y: 1,
            width: 0,
            height: out.height - 2,
        });
    }
    out
}

fn template_layout(editor: &ExpressionEditor, t: &Template, px: f32) -> MathLayout {
    use TemplateKind::*;
    let mut out = match t.kind {
        Palette(form) => palette::layout(editor, t, form, px),
        Fraction if t.fields.len() == 2 => {
            let upper = range(editor, t.fields[0].clone(), px * 0.86);
            let lower = range(editor, t.fields[1].clone(), px * 0.86);
            let width = upper.width.max(lower.width) + 6;
            let upper_h = upper.height;
            let lower_h = lower.height;
            let mut out = MathLayout {
                width,
                height: upper_h + lower_h + 5,
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
        Power if t.fields.len() == 2 => {
            let base = range(editor, t.fields[0].clone(), px);
            let exponent = range(editor, t.fields[1].clone(), px * 0.72);
            power(base, exponent)
        }
        NthRoot if t.fields.len() == 2 => {
            // G1 displays the degree on the same baseline as the radicand;
            // Left/Right traverse the two fields, Up/Down leave them alone.
            let degree = range(editor, t.fields[0].clone(), px);
            let body = range(editor, t.fields[1].clone(), px);
            radical(Some(degree), body, px)
        }
        Root => {
            let body = range(editor, t.fields[0].clone(), px);
            radical(None, body, px)
        }
        Function("ABS") if t.fields.len() == 1 => absolute(range(editor, t.fields[0].clone(), px)),
        _ => {
            // Function names, brackets, lists and nth-root arguments retain
            // exact syntax; their fields still share this geometry/hit model.
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
    out.carets.push(Caret {
        byte: t.span.start,
        x: 0,
        y: 0,
        height: out.height,
        boundary: true,
    });
    out.carets.push(Caret {
        byte: t.span.end,
        x: out.width,
        y: 0,
        height: out.height,
        boundary: true,
    });
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
        if result.height <= max_height || px <= 4.0 {
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
