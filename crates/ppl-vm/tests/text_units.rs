//! Original G1 font-index behavior, without converting raw units to Unicode
//! replacement characters or substituting escape text for calculator output.
mod common;
use common::Run;
use sha2::{Digest, Sha256};

fn result(value: &ppl_vm::Value) -> serde_json::Value {
    match value {
        ppl_vm::Value::Real(value) => serde_json::json!(value.to_i64().unwrap()),
        ppl_vm::Value::List(values) => values.iter().map(result).collect(),
        ppl_vm::Value::Str(value) => serde_json::json!(value.as_ref()),
        other => panic!("Unexpected control result: {other:?}"),
    }
}

#[test]
fn original_utf16_and_typed_textout_values_match_every_return_and_pixel() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        eprintln!("Original text controls require PRIME_TEST_FONT_RESOURCE");
        return;
    };
    let fonts =
        ppl_graphics::text::prime::PrimeFonts::load_resource(std::path::Path::new(&resource))
            .unwrap();
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-text-unit-controls.json"
    ))
    .unwrap();
    let evidence = std::env::var_os("PRIME_TEST_TEXT_EVIDENCE").map(std::path::PathBuf::from);
    if let Some(root) = &evidence {
        std::fs::create_dir_all(root).unwrap();
    }
    for case in controls["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            case["source_sha256"]
        );
        let mut run = Run::new(case["name"].as_str().unwrap(), source);
        run.machine.settings.hdigits = case["settings"]["hdigits"].as_i64().unwrap();
        run.host
            .borrow_mut()
            .keys
            .push_back(ppl_vm::ScriptEvent::Key(30));
        let value = ppl_graphics::text::with_prime_fonts(fonts.clone(), || {
            run.call(case["entry"].as_str().unwrap(), &[])
        })
        .unwrap()
        .unwrap();
        assert_eq!(result(&value), case["expected"], "{}", case["name"]);
        let screen = run.machine.screen();
        if let Some(root) = &evidence {
            screen
                .save_png(&root.join(format!("{}.png", case["name"].as_str().unwrap())))
                .unwrap();
        }
        let rgb = screen
            .pixels()
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect::<Vec<_>>();
        assert_eq!(
            format!("{:x}", Sha256::digest(rgb)),
            case["rgb_sha256"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn raw_drawing_never_rewrites_the_argument_or_its_saved_units() {
    let mut run = Run::new(
        "raw glyph data",
        "EXPORT RAWGLYPH() BEGIN LOCAL s:=CHAR({55296,65,56320}),x; x:=TEXTOUT_P(s,0,0,3); DRAWMENU({s}); RETURN {ASC(s),DIM(s),x>0}; END;",
    );
    assert_eq!(
        result(&run.call("RAWGLYPH", &[]).unwrap()),
        serde_json::json!([[55296, 65, 56320], 3, 1])
    );
}
