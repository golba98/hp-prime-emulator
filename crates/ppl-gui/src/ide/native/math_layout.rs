//! One geometry model for textbook entry painting and pointer caret placement.
use super::expression::{ExpressionEditor, Template, TemplateKind};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use std::ops::Range;

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
            draw_text(grob, &g.text, x + g.x, y + g.y, g.px, 0x111111, None, None);
        }
        for r in &self.rules {
            grob.line(x + r.x, y + r.y, x + r.x + r.width, y + r.y, 0x111111);
        }
        let (cx, cy, h) = self.cursor(cursor);
        grob.line(x + cx, y + cy, x + cx, y + cy + h, 0x111111);
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
        out.width = measure("□", px);
        out.glyphs.push(Glyph {
            x: 0,
            y: 0,
            text: "□".into(),
            px,
            source: start..start,
        });
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
fn template_layout(editor: &ExpressionEditor, t: &Template, px: f32) -> MathLayout {
    use TemplateKind::*;
    let mut out = match t.kind {
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
            });
            out
        }
        Power if t.fields.len() == 2 => {
            let base = range(editor, t.fields[0].clone(), px);
            let exp = range(editor, t.fields[1].clone(), px * 0.72);
            let mut out = MathLayout {
                width: base.width + exp.width + 2,
                height: base.height + exp.height / 2,
                baseline: base.baseline + exp.height / 2,
                ..Default::default()
            };
            let x = base.width + 2;
            let y = exp.height / 2;
            out.append(base, 0, y);
            out.append(exp, x, 0);
            out
        }
        Root => {
            let body = range(editor, t.fields[0].clone(), px);
            let sign = literal("√", t.span.start, px);
            let sx = sign.width;
            let bh = body.height;
            let bw = body.width;
            let mut out = sequence(vec![sign, body]);
            out.rules.push(Rule {
                x: sx,
                y: 0,
                width: bw,
            });
            out.height = out.height.max(bh + 1);
            out
        }
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
pub fn layout(editor: &ExpressionEditor, max_height: i64) -> MathLayout {
    let mut px = 14.0;
    loop {
        let result = range(editor, 0..editor.text().len(), px);
        if result.height <= max_height || px <= 4.0 {
            return result;
        }
        px -= 1.0;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fraction_fields_have_distinct_rows_and_pointer_positions() {
        let mut e = ExpressionEditor::default();
        e.insert_template(TemplateKind::Fraction);
        e.insert("1");
        e.apply(super::super::expression::EditCommand::Right(false));
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
    fn nested_fraction_layout_stays_inside_entry_height() {
        let mut e = ExpressionEditor::default();
        e.insert_template(TemplateKind::Fraction);
        e.insert_template(TemplateKind::Fraction);
        e.insert("12");
        let l = layout(&e, 34);
        assert!(l.height <= 34);
        assert!(l.width > 0);
        assert_eq!(l.rules.len(), 2);
    }
}
