//! An unchanged original-firmware program distinguishes exact from decimal
//! inputs before square roots, fractional powers and reciprocal roots.
use sha2::{Digest, Sha256};

#[test]
fn original_rational_and_pi_roots_round_the_actual_input_once() {
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-root-precision-controls.json"
    ))
    .unwrap();
    let extra = controls["additional_cases"].as_array().unwrap();
    for controls in std::iter::once(&controls).chain(extra.iter()) {
        let source = controls["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source.as_bytes())),
            controls["source_sha256"]
        );
        let mut session = ppl_vm::SessionContext::new(
            ppl_vm::Settings {
                numeric_version: ppl_math::NumericVersion::Current15751,
                ..Default::default()
            },
            1,
        );
        session
            .store_program(controls["entry"].as_str().unwrap(), source.into())
            .unwrap();
        let result = session
            .evaluate_home(
                &format!("{}()", controls["entry"].as_str().unwrap()),
                Box::new(ppl_vm::HeadlessHost::new()),
                Default::default(),
            )
            .result
            .unwrap();
        assert_eq!(
            result.expect_str("root results").unwrap().as_ref(),
            controls["expected"].as_str().unwrap()
        );
    }
}
