//! Actual PPL execution compared against complete independent G1 framebuffer hashes.
mod common;
use common::Run;
use sha2::{Digest, Sha256};

#[test]
fn installed_g1_string_fitting_uses_actual_font_codes_styles_and_components() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        eprintln!("Genuine fitting controls require PRIME_TEST_FONT_RESOURCE");
        return;
    };
    let fonts =
        ppl_graphics::text::prime::PrimeFonts::load_resource(std::path::Path::new(&resource))
            .unwrap();
    fn json(value: &ppl_vm::Value) -> serde_json::Value {
        match value {
            ppl_vm::Value::List(values) => values.iter().map(json).collect(),
            ppl_vm::Value::Str(value) => serde_json::json!(value.as_ref()),
            ppl_vm::Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
            other => panic!("unexpected fitting result: {other:?}"),
        }
    }
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-string-fitting-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            case["reference_source_sha256"],
        );
        let mut run = Run::new(case["name"].as_str().unwrap(), source);
        let result = ppl_graphics::text::with_prime_fonts(fonts.clone(), || {
            run.call(case["entry"].as_str().unwrap(), &[])
        })
        .unwrap()
        .unwrap();
        assert_eq!(json(&result), case["expected"], "{}", case["name"]);
    }
}

#[test]
fn unavailable_styled_metrics_are_reported_and_descriptors_are_validated() {
    let mut run = Run::new(
        "fitting admission",
        "EXPORT FIT() BEGIN RETURN STRING(12345.6789,1,10,-1,{40,2,1}); END;",
    );
    let error = run.call("FIT", &[]).unwrap_err();
    assert_eq!(error.kind, ppl_vm::ErrorKind::Unsupported);
    assert!(error.detail.contains("installed Prime font"));
    for descriptor in ["{40,8}", "{40,-1}", "{-1,2}", "{}", "{40,2,0,0,0,0}"] {
        let mut run = Run::new(
            "invalid fitting descriptor",
            &format!("EXPORT FIT() BEGIN RETURN STRING(12,1,10,-1,{descriptor}); END;"),
        );
        assert_eq!(
            run.call("FIT", &[]).unwrap_err().kind,
            ppl_vm::ErrorKind::BadArgValue
        );
    }
}
#[test]
fn installed_g1_texout_programs_match_every_reference_pixel() {
    let Some(resource) = std::env::var_os("PRIME_TEST_FONT_RESOURCE") else {
        eprintln!("Genuine font controls require PRIME_TEST_FONT_RESOURCE");
        return;
    };
    let fonts =
        ppl_graphics::text::prime::PrimeFonts::load_resource(std::path::Path::new(&resource))
            .unwrap();
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-font-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let mut run = Run::new("G1 font control", case["source"].as_str().unwrap());
        run.host
            .borrow_mut()
            .keys
            .push_back(ppl_vm::host::ScriptEvent::Key(30));
        let value = ppl_graphics::text::with_prime_fonts(fonts.clone(), || {
            run.call(case["entry"].as_str().unwrap(), &[])
        })
        .unwrap()
        .unwrap();
        assert_eq!(value.expect_i64("control return").unwrap(), 1);
        let rgb = run
            .machine
            .screen()
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
