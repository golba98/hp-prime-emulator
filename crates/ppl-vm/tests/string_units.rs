//! Independent ASC/DIM controls for formatter UTF16 symbols and exact font fit.
mod common;
use common::Run;
use ppl_vm::Value;
fn json(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(json).collect(),
        other => panic!("Unexpected unit projection {other:?}"),
    }
}
fn controls(fonts: bool) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-string-utf16-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["requires_prime_fonts"] == fonts)
    {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run.call(case["entry"].as_str().unwrap(), &[]).unwrap();
        assert_eq!(json(&value), case["expected"], "{}", case["name"]);
    }
}
#[test]
fn formatter_composes_units_symbols_nul_and_validated_grouping_patterns() {
    controls(false);
}
#[test]
fn installed_fonts_measure_each_original16_bit_formatter_index() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        eprintln!("Installed formatter font metrics require PRIME_TEST_FONT_RESOURCE");
        return;
    };
    let fonts =
        ppl_graphics::text::prime::PrimeFonts::load_resource(std::path::Path::new(&resource))
            .unwrap();
    ppl_graphics::text::with_prime_fonts(fonts, || controls(true)).unwrap();
}
