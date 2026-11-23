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
    result_with_matrix_font(display, typed, px, px)
}

pub(in crate::ide::native) fn result_with_matrix_font(
    display: &str,
    typed: Option<&Value>,
    px: f32,
    matrix_px: f32,
) -> MathLayout {
    result_layout(display, typed, px, matrix_px, MatrixStyle::Standard)
}

pub(in crate::ide::native) fn history_result(
    display: &str,
    typed: Option<&Value>,
    px: f32,
    compact_matrices: bool,
) -> MathLayout {
    result_layout(
        display,
        typed,
        px,
        px,
        MatrixStyle::history(compact_matrices),
    )
}

pub(in crate::ide::native) fn show_matrix_result(
    display: &str,
    typed: &Value,
    px: f32,
) -> MathLayout {
    result_layout(display, Some(typed), px, px, MatrixStyle::CompactShow)
}

pub(in crate::ide::native) fn history_fraction_grids(
    source: &str,
    display: &str,
    typed: Option<&Value>,
    px: f32,
) -> Option<(MathLayout, MathLayout)> {
    if px != 14. || source.len() > 8192 || display.len() > 8192 || !source.contains('/') {
        return None;
    }
    let typed = typed?;
    if typed["type"] != "matrix" || typed["display"].as_str().is_some() {
        return None;
    }
    let value = serde_json::from_value::<ppl_cas::CasValue>(typed.clone()).ok()?;
    let ppl_cas::CasValue::Matrix { rows, cols, .. } = &value else {
        return None;
    };
    if value.try_expression().ok()? != display {
        return None;
    }
    let (source, source_rows) = symbolic::history_fraction_grid(source, px, *rows, *cols, false)?;
    let (result, result_rows) = symbolic::history_fraction_grid(display, px, *rows, *cols, true)?;
    (source_rows == result_rows).then_some((source, result))
}

fn result_layout(
    display: &str,
    typed: Option<&Value>,
    px: f32,
    matrix_px: f32,
    matrix_style: MatrixStyle,
) -> MathLayout {
    match typed.and_then(|v| v["type"].as_str()) {
        Some("matrix" | "list") => {
            let typed = typed.unwrap();
            if needs_display(typed) {
                if let Some(restored) = legacy_display(typed, display) {
                    return value(&restored, px, matrix_px, matrix_style);
                }
            }
            value(typed, px, matrix_px, matrix_style)
        }
        Some("string") | None => text(display, px),
        Some("rational") => symbolic::rational_result(display, px, matrix_px),
        _ => symbolic::expression_with_matrix_font(display, px, matrix_px),
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
fn value(v: &Value, px: f32, matrix_px: f32, matrix_style: MatrixStyle) -> MathLayout {
    if let Some(display) = v["display"].as_str() {
        return if v["type"] == "string" {
            text(display, px)
        } else {
            symbolic::expression_with_matrix_font(display, px, matrix_px)
        };
    }
    // Giac history stores exact matrices and rational leaves as tagged CAS
    // records. Native matrix history uses separate dimensions and a flat array.
    if v["value"].is_object()
        && let Ok(value) = serde_json::from_value::<ppl_cas::CasValue>(v.clone())
        && value.validate().is_ok()
    {
        return cas_value(&value, px, matrix_px, matrix_style);
    }
    match v["type"].as_str() {
        Some("list") => list_delimiters(px, matrix_px, |px| {
            let mut parts = Vec::new();
            if let Some(values) = v["value"].as_array() {
                for (index, entry) in values.iter().enumerate() {
                    if index != 0 {
                        parts.push(text(", ", px));
                    }
                    parts.push(value(entry, px, matrix_px, MatrixStyle::Standard));
                }
            }
            parts
        }),
        Some("matrix") => {
            let rows = v["rows"].as_u64().unwrap_or(0) as usize;
            let cols = v["cols"].as_u64().unwrap_or(0) as usize;
            let data = v["value"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if rows.checked_mul(cols) != Some(data.len()) || cols == 0 || rows == 0 {
                return text("[]", px);
            }
            let cells = data
                .iter()
                .map(|v| value(v, px.min(matrix_px), matrix_px, MatrixStyle::Standard))
                .collect();
            palette::static_brackets(cells, rows, cols, true, matrix_style)
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
            symbolic::expression_with_matrix_font(s, px, matrix_px)
        }
    }
}
fn cas_value(
    value: &ppl_cas::CasValue,
    px: f32,
    matrix_px: f32,
    matrix_style: MatrixStyle,
) -> MathLayout {
    use ppl_cas::CasValue;
    match value {
        CasValue::Matrix { rows, cols, values } if *rows != 0 && *cols != 0 => {
            let cells = values
                .iter()
                .map(|value| cas_value(value, px.min(matrix_px), matrix_px, MatrixStyle::Standard))
                .collect();
            palette::static_brackets(cells, *rows, *cols, false, matrix_style)
        }
        CasValue::Rational {
            numerator,
            denominator,
        } => expression(&format!("{numerator}/{denominator}"), px),
        _ => symbolic::expression_with_matrix_font(&value.expression(), px, matrix_px),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn paired_history_fraction_grids_use_full_size_cells_with_same_columns() {
        let source = "[[8,1/2],[5,9]]";
        let display = "[[8,(1)/(2)],[5,9]]";
        let integer = |n: &str| json!({"type":"integer","value":n});
        let typed = json!({"type":"matrix","value":{"rows":2,"cols":2,"values":[
            integer("8"), json!({"type":"rational","value":{"numerator":"1","denominator":"2"}}),
            integer("5"), integer("9")
        ]}});
        let (left, right) = history_fraction_grids(source, display, Some(&typed), 14.).unwrap();
        assert_eq!((left.height, right.height), (55, 55));
        assert_eq!(left.width, expression(source, 14.).width);
        assert_eq!(right.width, result(display, Some(&typed), 14.).width);
        assert_eq!((left.glyphs[0].y, right.glyphs[0].y), (10, 10));
        assert!(history_fraction_grids("[[1/2,8],[5,9]]", display, Some(&typed), 13.).is_none());
        assert!(history_fraction_grids("[[8,5],[1/2,9]]", display, Some(&typed), 14.).is_none());
    }

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
    fn matrix_list_braces_enclose_each_level_and_center_single_rows() {
        let real = |value: &str| json!({"type":"real", "value":value, "display":value});
        let list = |values: Vec<Value>| json!({"type":"list", "value":values});
        let matrix = |rows: usize| {
            json!({
                "type":"matrix", "rows":rows, "cols":2, "vector":false,
                "value": vec![real("2"); rows * 2],
            })
        };
        for (source, value, want_height) in [
            ("{1,[[2,2]]}", list(vec![real("1"), matrix(1)]), 18),
            ("{1,[[2,2],[2,2]]}", list(vec![real("1"), matrix(2)]), 38),
            (
                "{1,{2,[[2,2],[2,2],[2,2]]}}",
                list(vec![real("1"), list(vec![real("2"), matrix(3)])]),
                60,
            ),
        ] {
            let expression = symbolic::expression_with_matrix_font(source, 17., 13.);
            let result = result_with_matrix_font(source, Some(&value), 17., 13.);
            for layout in [expression, result] {
                assert_eq!(layout.height, want_height, "{source}");
                assert_eq!(layout.glyphs[0].px, 13.);
                if want_height == 18 {
                    // A one-row matrix and its scalar peer share the same text row.
                    assert!(layout.glyphs.iter().all(|glyph| glyph.y == 1));
                }
            }
        }
    }

    #[test]
    fn nested_matrix_uses_rows_and_columns_and_strings_stay_literal() {
        let matrix = json!({"type":"matrix","rows":2,"cols":2,"vector":false,"value":[
            {"type":"real","value":"1"},{"type":"real","value":"200"},
            {"type":"real","value":"3"},{"type":"real","value":"4"}]});
        let l = result_with_matrix_font("[[1,200],[3,4]]", Some(&matrix), 17., 13.);
        assert_eq!(l.rules.len(), 6);
        assert_eq!(l.height, 36);
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
