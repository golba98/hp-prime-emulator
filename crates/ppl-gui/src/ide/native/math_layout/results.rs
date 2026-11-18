//! Typeset persisted values structurally; strings are never parsed as math.
use super::*;
use serde_json::Value;

pub fn text(source: &str, px: f32) -> MathLayout {
    let mut layout = literal(source, 0, px);
    if source.is_empty() {
        layout.glyphs.clear();
        layout.width = 0;
    }
    layout
}
pub fn result(display: &str, typed: Option<&Value>, px: f32) -> MathLayout {
    match typed.and_then(|v| v["type"].as_str()) {
        Some("matrix" | "list") => {
            let typed = typed.unwrap();
            if needs_display(typed) {
                if let Some(restored) = legacy_display(typed, display) {
                    return value(&restored, px);
                }
            }
            value(typed, px)
        }
        Some("string") | None => text(display, px),
        _ => expression(display, px),
    }
}
fn needs_display(v: &Value) -> bool {
    if matches!(v["type"].as_str(), Some("list" | "matrix")) {
        v["value"]
            .as_array()
            .is_some_and(|values| values.iter().any(needs_display))
    } else {
        v["display"].as_str().is_none()
    }
}
/// Old profiles kept rounded display text alongside full-precision metadata.
/// Bind the parsed display to that structure, preserving all printed scalar
/// text (including old integer values whose metadata used Debug formatting).
fn legacy_display(typed: &Value, display: &str) -> Option<Value> {
    use ppl_syntax::ast::{Expr, ExprKind};
    fn bind(v: &mut Value, e: &Expr, source: &str) -> Option<()> {
        fn sequence(e: &Expr) -> Option<&[Expr]> {
            match &e.kind {
                ExprKind::List(values) | ExprKind::Bracket(values) => Some(values),
                _ => None,
            }
        }
        match v["type"].as_str()? {
            "list" => {
                let expressions = sequence(e)?;
                let values = v["value"].as_array_mut()?;
                if values.len() != expressions.len() {
                    return None;
                }
                for (v, e) in values.iter_mut().zip(expressions) {
                    bind(v, e, source)?;
                }
            }
            "matrix" => {
                let rows = v["rows"].as_u64()? as usize;
                let cols = v["cols"].as_u64()? as usize;
                let expressions = sequence(e)?;
                let cells: Vec<_> = if expressions.len() == rows
                    && expressions
                        .iter()
                        .all(|e| sequence(e).is_some_and(|row| row.len() == cols))
                {
                    expressions
                        .iter()
                        .flat_map(|e| sequence(e).unwrap())
                        .collect()
                } else if rows == 1 && expressions.len() == cols {
                    expressions.iter().collect()
                } else {
                    return None;
                };
                let values = v["value"].as_array_mut()?;
                if values.len() != cells.len() {
                    return None;
                }
                for (v, e) in values.iter_mut().zip(cells) {
                    bind(v, e, source)?;
                }
            }
            _ => {
                if v["display"].as_str().is_none() {
                    v["display"] = source
                        .get(e.span.start as usize..e.span.end as usize)?
                        .into();
                }
            }
        }
        Some(())
    }
    let (expr, diagnostics) = ppl_syntax::parse_expression(display);
    if diagnostics.has_errors() {
        return None;
    }
    let mut value = typed.clone();
    bind(&mut value, &expr, display)?;
    Some(value)
}
fn value(v: &Value, px: f32) -> MathLayout {
    if let Some(display) = v["display"].as_str() {
        return if v["type"] == "string" {
            text(display, px)
        } else {
            expression(display, px)
        };
    }
    // Giac history stores exact matrices and rational leaves as tagged CAS
    // records. Native matrix history uses separate dimensions and a flat array.
    if v["value"].is_object()
        && let Ok(value) = serde_json::from_value::<ppl_cas::CasValue>(v.clone())
        && value.validate().is_ok()
    {
        return cas_value(&value, px);
    }
    match v["type"].as_str() {
        Some("list") => {
            let mut parts = Vec::new();
            if let Some(values) = v["value"].as_array() {
                for (index, entry) in values.iter().enumerate() {
                    if index != 0 {
                        parts.push(text(", ", px));
                    }
                    parts.push(value(entry, px));
                }
            }
            list_delimiters(sequence(parts), px)
        }
        Some("matrix") => {
            let rows = v["rows"].as_u64().unwrap_or(0) as usize;
            let cols = v["cols"].as_u64().unwrap_or(0) as usize;
            let data = v["value"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if rows.checked_mul(cols) != Some(data.len()) || cols == 0 || rows == 0 {
                return text("[]", px);
            }
            let cells: Vec<_> = data.iter().map(|v| value(v, px)).collect();
            let widths: Vec<i64> = (0..cols)
                .map(|c| {
                    cells
                        .iter()
                        .skip(c)
                        .step_by(cols)
                        .map(|l| l.width)
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let heights: Vec<i64> = cells
                .chunks(cols)
                .map(|row| row.iter().map(|l| l.height).max().unwrap_or(0))
                .collect();
            let height = heights.iter().sum::<i64>() + (rows as i64 - 1) * 6 + 6;
            let width = widths.iter().sum::<i64>() + (cols as i64 - 1) * 12 + 16;
            let mut out = MathLayout {
                width,
                height,
                baseline: height / 2,
                ..Default::default()
            };
            let mut y = 3;
            for (r, row) in cells.chunks(cols).enumerate() {
                let mut x = 8;
                for (c, cell) in row.iter().enumerate() {
                    out.append(
                        cell.clone(),
                        x + (widths[c] - cell.width) / 2,
                        y + (heights[r] - cell.height) / 2,
                    );
                    x += widths[c] + 12;
                }
                y += heights[r] + 6;
            }
            for x in [0, width - 1] {
                out.rules.push(Rule {
                    x,
                    y: 0,
                    width: 0,
                    height: height - 1,
                });
                let dx = if x == 0 { 4 } else { -4 };
                out.rules.push(Rule {
                    x,
                    y: 0,
                    width: dx,
                    height: 0,
                });
                out.rules.push(Rule {
                    x,
                    y: height - 1,
                    width: dx,
                    height: 0,
                });
            }
            out
        }
        Some("complex") => text(
            &format!(
                "{}+({})*i",
                v["real"].as_str().unwrap_or(""),
                v["imaginary"].as_str().unwrap_or("")
            ),
            px,
        ),
        Some("string") => text(&format!("\"{}\"", v["value"].as_str().unwrap_or("")), px),
        _ => {
            let s = v["value"].as_str().unwrap_or("");
            expression(s, px)
        }
    }
}
fn cas_value(value: &ppl_cas::CasValue, px: f32) -> MathLayout {
    use ppl_cas::CasValue;
    match value {
        CasValue::Matrix { rows, cols, values } if *rows != 0 && *cols != 0 => {
            let cells = values.iter().map(|value| cas_value(value, px)).collect();
            palette::brackets(cells, *rows, *cols)
        }
        CasValue::Rational {
            numerator,
            denominator,
        } => expression(&format!("{numerator}/{denominator}"), px),
        _ => expression(&value.expression(), px),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn cas_fraction_matrix_result_paints_its_exact_values() {
        let rational = |num: &str, den: &str| {
            json!({
                "type":"rational","value":{"numerator":num,"denominator":den}
            })
        };
        let typed = json!({"type":"matrix","value":{"rows":2,"cols":2,"values":[
            rational("1", "2"), rational("2", "3"), rational("3", "4"), rational("4", "5")
        ]}});
        let layout = result("[[(1)/(2),(2)/(3)],[(3)/(4),(4)/(5)]]", Some(&typed), 17.);
        assert_eq!(
            layout
                .rules
                .iter()
                .filter(|r| r.height == 0 && r.width > 5)
                .count(),
            4
        );
        assert_eq!(
            layout
                .glyphs
                .iter()
                .map(|glyph| glyph.text.as_str())
                .collect::<String>(),
            "12233445",
        );
    }

    #[test]
    fn tall_matrix_brackets_clip_continuously_to_a_scrolled_viewport() {
        let matrix = json!({"type":"matrix","rows":12,"cols":1,"vector":false,"value":
            (0..12).map(|n|json!({"type":"real","value":n.to_string()})).collect::<Vec<_>>()});
        let layout = result("", Some(&matrix), 17.);
        assert!(layout.height > 196);
        for dy in [6, -60] {
            let mut viewport = Grob::new(320, 196, 0xffffff);
            layout.paint_static(&mut viewport, 10, dy, 0x111111);
            for y in dy.max(0)..196 {
                assert_eq!(
                    viewport.get(10, y),
                    Some(0x111111),
                    "left bracket at y{y}, offset{dy}"
                );
            }
        }
    }
    #[test]
    fn old_history_uses_printed_precision_and_integer_text_without_parsing_debug() {
        let old = json!({"type":"list","value":[
            {"type":"real","value":"1.23456789"},
            {"type":"integer","value":"HpInt { value: 255, bits: 32, base: Hex }"},
            {"type":"string","value":"1/2"}]});
        let layout = result("{1.23,#FFh,\"1/2\"}", Some(&old), 17.);
        let painted = layout
            .glyphs
            .iter()
            .map(|g| g.text.as_str())
            .collect::<String>();
        assert_eq!(painted, "{1.23, #FFh, \"1/2\"}");
        assert!(layout.rules.is_empty());
        assert_eq!(old["value"][0]["value"], "1.23456789");
    }
    #[test]
    fn collection_display_precision_is_preserved_independently_of_value() {
        let value = ppl_vm::PortableValue::List(vec![ppl_vm::PortableValue::Real(
            ppl_math::Real::parse("1.23456").unwrap(),
        )]);
        let mut settings = ppl_vm::Settings::default();
        settings.set_format(1, 2);
        let typed = crate::ide::native::portable_display_json(&value, &settings);
        assert_eq!(typed["value"][0]["value"], "1.23456");
        assert_eq!(typed["value"][0]["display"], "1.23");
        let layout = result("{1.23}", Some(&typed), 17.);
        let painted = layout
            .glyphs
            .iter()
            .map(|g| g.text.as_str())
            .collect::<String>();
        assert_eq!(painted, "{1.23}");
    }
    #[test]
    fn nested_matrix_uses_rows_and_columns_and_strings_stay_literal() {
        let matrix = json!({"type":"matrix","rows":2,"cols":2,"vector":false,"value":[
            {"type":"real","value":"1"},{"type":"real","value":"200"},
            {"type":"real","value":"3"},{"type":"real","value":"4"}]});
        let l = result("[[1,200],[3,4]]", Some(&matrix), 17.);
        assert_eq!(l.rules.len(), 6);
        assert!(l.height > 40);
        assert!(
            l.glyphs.iter().find(|g| g.text == "3").unwrap().y
                > l.glyphs.iter().find(|g| g.text == "1").unwrap().y
        );
        let list = json!({"type":"list","value":[matrix,{"type":"string","value":"SQRT(9)"}]});
        let l = result("", Some(&list), 17.);
        assert_eq!(l.rules.len(), 6 + 12); // Matrix brackets and two tall braces.
        let glyphs = l
            .glyphs
            .iter()
            .map(|glyph| glyph.text.as_str())
            .collect::<String>();
        assert!(glyphs.ends_with(", \"SQRT(9)\""));
        assert!(l.width > 100);
    }
}
