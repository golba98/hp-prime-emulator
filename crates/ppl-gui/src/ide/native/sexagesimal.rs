//! HP Prime degrees/minutes/seconds key entry and per-answer format toggle.
//! Numeric meaning follows User Guide p.15: 1.375 = 1°22′30″.
use super::expression::ExpressionEditor;
use ppl_math::Real;

pub fn next_unit(prefix: &str) -> &'static str {
    let before_number = prefix
        .trim_end()
        .trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == 'E' || c == 'e');
    if before_number.ends_with('′') {
        "″"
    } else if before_number.ends_with('°') {
        "′"
    } else {
        "°"
    }
}
pub fn insert_unit(editor: &mut ExpressionEditor) {
    let unit = next_unit(&editor.text()[..editor.selection().start]);
    editor.insert(unit);
}
pub fn format_decimal(text: &str) -> Result<String, String> {
    let value = Real::parse(text).ok_or_else(|| "Invalid decimal result".to_owned())?;
    let sign = if value.is_negative() { "-" } else { "" };
    let value = value.abs();
    let result = (|| -> Result<String, ppl_math::MathError> {
        let mut degrees = value.floor();
        let minutes = value.sub(degrees)?.mul(Real::from_i64(60))?;
        let mut whole_minutes = minutes.floor();
        let mut seconds = minutes.sub(whole_minutes)?.mul(Real::from_i64(60))?;
        if seconds.cmp_value(Real::from_i64(60)).is_ge() {
            seconds = Real::ZERO;
            whole_minutes = whole_minutes.add(Real::ONE)?;
        }
        if whole_minutes.cmp_value(Real::from_i64(60)).is_ge() {
            whole_minutes = Real::ZERO;
            degrees = degrees.add(Real::ONE)?;
        }
        Ok(format!("{sign}{degrees}°{whole_minutes}′{seconds}″"))
    })();
    result.map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcut_uses_caret_and_current_number_including_nested_expressions() {
        let mut editor = ExpressionEditor::from_text("36");
        insert_unit(&mut editor);
        editor.insert("40");
        insert_unit(&mut editor);
        editor.insert("20");
        insert_unit(&mut editor);
        assert_eq!(editor.text(), "36°40′20″");
        editor.insert("+SIN(2");
        insert_unit(&mut editor);
        assert_eq!(editor.text(), "36°40′20″+SIN(2°");
        editor.set_text("12+36°40′20″");
        editor.set_cursor(2, false);
        insert_unit(&mut editor);
        assert_eq!(editor.text(), "12°+36°40′20″");
        editor.apply(super::super::expression::EditCommand::Undo);
        assert_eq!(editor.text(), "12+36°40′20″");
    }
    #[test]
    fn documented_result_conversion_and_negative_values() {
        for (decimal, expected) in [
            ("1.375", "1°22′30″"),
            ("-1.375", "-1°22′30″"),
            ("0", "0°0′0″"),
            ("5.25", "5°15′0″"),
        ] {
            assert_eq!(format_decimal(decimal).unwrap(), expected);
        }
        assert!(format_decimal("not a number").is_err());
    }
}
