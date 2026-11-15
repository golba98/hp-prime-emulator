use ppl_engine::{Engine, Operation, Report, Status, arguments, source::Source};
use ppl_vm::Settings;
#[test]
fn lossless_arguments_and_legacy_values() {
    let s = Settings::default();
    let a = arguments::json_args(
        r##"[7,1e400,{"type":"real","value":"0.1"},"é",[1,2],{"type":"ppl","value":"#FFh"}]"##,
        &s,
    )
    .unwrap();
    assert_eq!(a.len(), 6);
    assert_eq!(arguments::encode(&a[1])["type"], "real");
    assert!(
        arguments::encode(&a[1])["value"]
            .as_str()
            .unwrap()
            .contains("400")
    );
    assert_eq!(arguments::encode(&a[4])["type"], "list");
    assert_eq!(arguments::encode(&a[5])["type"], "integer");
    let old = arguments::ppl_args(Some("[1,2],{3,4},\"x\""), &s).unwrap();
    assert_eq!(old.len(), 3);
    assert_eq!(arguments::encode(&old[0])["type"], "matrix");
    assert!(arguments::json_args("[true]", &s).is_err());
}
#[test]
fn source_identity_encodings_and_container_validation() {
    let text = "EXPORT HELLO()\r\nBEGIN\r\nEND;\r\n";
    let source = Source::text(text, "inline").unwrap();
    assert!(!source.text.contains('\r'));
    assert_ne!(source.identity.input_sha256, source.identity.source_sha256);
    for be in [false, true] {
        let bytes: Vec<_> = if be { [254, 255] } else { [255, 254] }
            .into_iter()
            .chain(
                text.encode_utf16()
                    .flat_map(|u| if be { u.to_be_bytes() } else { u.to_le_bytes() }),
            )
            .collect();
        assert_eq!(
            Source::decode(bytes, None, "é".into()).unwrap().text,
            source.text
        );
    }
    let container = ppl_syntax::hpprgm::build_program(&source.text)
        .unwrap()
        .to_bytes();
    assert_eq!(
        Source::decode(container.clone(), None, "binary".into())
            .unwrap()
            .text,
        source.text
    );
    assert!(Source::decode(container[..20].to_vec(), None, "broken".into()).is_err());
    assert!(Source::decode(vec![0xff], None, "bad".into()).is_err());
    assert!(source.guest_name(Some("é")).is_err());
    assert!(source.guest_name(None).unwrap().starts_with("PPL_"));
}
#[test]
fn report_serialization_and_native_compatibility() {
    let mut r = Report::new(Engine::Native, Operation::Run);
    r.fail(Status::InvalidEntry, "entry", "missing");
    assert_eq!(r.exit_code, 2);
    let j = serde_json::to_string(&r).unwrap();
    let again: Report = serde_json::from_str(&j).unwrap();
    assert!(again.return_value.is_none());
    assert!(again.output.is_empty());
    for s in [
        Status::TimedOut,
        Status::GuestFault,
        Status::MissingFirmware,
        Status::MissingRunner,
        Status::Cancelled,
    ] {
        let mut r = Report::new(Engine::Firmware, Operation::Run);
        r.fail(s, "test", "x");
        assert_eq!(r.exit_code, s.firmware_exit());
    }
}
#[test]
fn native_tagged_values_round_trip_without_losing_width_or_precision() {
    let settings = Settings::default();
    let values=arguments::json_args(r##"[{"type":"ppl","value":"#FF:8h"},{"type":"complex","real":"1e400","imaginary":"2"},{"type":"matrix","rows":1,"columns":2,"vector":true,"value":[1,2]},{"type":"list","value":[1e400]},{"type":"grob","index":2},{"type":"real","value":"1.375","presentation":"sexagesimal"},{"type":"complex","real":"0","imaginary":"2","presentation":"polar"},{"type":"complex","real":"1","imaginary":"2","presentation":"sexagesimal"}]"##,&settings).unwrap();
    for value in values {
        let encoded = arguments::encode(&value);
        let again =
            arguments::json_args(&serde_json::json!([encoded]).to_string(), &settings).unwrap();
        assert_eq!(arguments::encode(&again[0]), arguments::encode(&value));
    }
    assert!(
        arguments::json_args(
            r#"[{"type":"real","value":"1","presentation":"future"}]"#,
            &settings
        )
        .is_err()
    );
    assert!(
        arguments::json_args(
            r#"[{"type":"complex","real":"1","imaginary":"2","presentation":"future"}]"#,
            &settings
        )
        .is_err()
    );
    let report = ppl_engine::native::execute(
        Source::text("EXPORT METAARGS(angle,point) BEGIN HAngle:=1; RETURN {STRING(angle+2),STRING(point*2)}; END;", "METAARGS").unwrap(),
        Operation::Run,
        ppl_engine::native::Options {
            modules: Default::default(), entry: None,
            args: r#"[{"type":"real","value":"1.375","presentation":"sexagesimal"},{"type":"complex","real":"0","imaginary":"2","presentation":"polar"}]"#.into(),
            json_arguments: true, host: Default::default(), settings, limits: Default::default(),
            seed: 0, png: None, deny_warnings: false, dump_bytecode: false,
        }, &ppl_engine::Cancellation::default());
    assert_eq!(report.exit_code, 0, "{report:?}");
    assert_eq!(
        report.return_value.unwrap(),
        serde_json::json!({"type":"list","value":[
        {"type":"string","value":"3°22′30″"},{"type":"string","value":"4∡90"}]})
    );
}

fn interaction_report(source: &str, inputs: &[&str]) -> Report {
    ppl_engine::native::execute(
        Source::text(source, "TRACE").unwrap(),
        Operation::Run,
        ppl_engine::native::Options {
            modules: Default::default(),
            entry: None,
            args: "[]".into(),
            json_arguments: true,
            host: ppl_vm::HeadlessHost {
                inputs: inputs
                    .iter()
                    .map(|s| ppl_vm::ScriptedInput::parse(s))
                    .collect(),
                ..Default::default()
            },
            settings: Settings::default(),
            limits: ppl_vm::Limits::default(),
            seed: 0,
            png: None,
            deny_warnings: false,
            dump_bytecode: false,
        },
        &ppl_engine::Cancellation::default(),
    )
}

#[test]
fn native_report_retains_validation_recovery_and_cancellation_dialogs() {
    let source = r#"EXPORT TRACE() BEGIN
      LOCAL n:=0;
      REPEAT
        IF INPUT(n,"Base")==0 THEN RETURN; END;
        IF n<2 THEN MSGBOX("Base too small"); END;
      UNTIL n>=2;
      RETURN n;
    END;"#;
    let recovered = interaction_report(source, &["1", "16"]);
    assert_eq!(recovered.status, Status::Success);
    assert_eq!(recovered.return_value.unwrap()["value"], "16");
    assert_eq!(
        recovered.backend["dialogs"],
        serde_json::json!([
            "INPUT: Base -> [\"1\"]",
            "MSGBOX: Base too small",
            "INPUT: Base -> [\"16\"]"
        ])
    );
    let cancelled = interaction_report(source, &["<ESC>"]);
    assert_eq!(cancelled.status, Status::Success);
    assert_eq!(
        cancelled.backend["dialogs"],
        serde_json::json!(["INPUT: Base -> cancelled"])
    );
}

#[test]
fn native_report_preserves_prior_dialogs_when_script_exhausts() {
    let report = interaction_report(
        "EXPORT TRACE() BEGIN LOCAL n; MSGBOX(\"Start\"); INPUT(n,\"Number\"); END;",
        &[],
    );
    assert_eq!(report.status, Status::TimedOut);
    assert_eq!(
        report.backend["dialogs"],
        serde_json::json!(["MSGBOX: Start"])
    );
}

#[test]
fn binary_calculator_strings_round_trip_json_arguments_without_replacement_characters() {
    use ppl_engine::arguments::{encode, json_args};
    use ppl_vm::{Settings, Value};
    let value = Value::from_utf16(vec![55357, 0, 56832]);
    let encoded = encode(&value);
    assert_eq!(
        encoded,
        serde_json::json!({"type":"string","utf16":[55357,0,56832]})
    );
    let restored = json_args(&format!("[{encoded}]"), &Settings::default()).unwrap();
    assert!(value.equals(&restored[0]));
    for bad in [
        r#"[{"type":"string","utf16":[65536]}]"#,
        r#"[{"type":"string","utf16":[-1]}]"#,
        r#"[{"type":"string","value":"x","utf16":[55357]}]"#,
    ] {
        assert!(json_args(bad, &Settings::default()).is_err());
    }
}

#[test]
fn firmware_raw_utf16_results_can_be_passed_as_native_typed_arguments_losslessly() {
    let settings = ppl_vm::Settings::default();
    for units in [
        vec![55296],
        vec![56320],
        vec![65, 0, 66],
        vec![55357, 56832],
        vec![],
    ] {
        let wire = serde_json::json!([{"type":"utf16","value":units}]).to_string();
        let values = ppl_engine::arguments::json_args(&wire, &settings).unwrap();
        assert_eq!(values[0].string_units().unwrap().as_ref(), units);
        let encoded = ppl_engine::arguments::encode(&values[0]);
        let roundtrip =
            ppl_engine::arguments::json_args(&serde_json::json!([encoded]).to_string(), &settings)
                .unwrap();
        assert_eq!(roundtrip[0].string_units(), values[0].string_units());
    }
    for value in [
        serde_json::json!([-1]),
        serde_json::json!([65536]),
        serde_json::json!("bad"),
        serde_json::Value::Null,
    ] {
        assert!(
            ppl_engine::arguments::json_args(
                &serde_json::json!([{"type":"utf16","value":value}]).to_string(),
                &settings
            )
            .is_err()
        );
    }
}

#[test]
fn signed_word_transport_roundtrips_numeric_sign_width_and_base_affinity() {
    use ppl_math::{HpInt, IntBase};
    use ppl_vm::Value;
    for (number, width, system) in [
        (255, -8, false),
        (-5, -8, true),
        (-256, -8, true),
        (-1, -63, true),
        (u64::MAX as i128, 64, false),
    ] {
        let word = HpInt::new_with_width(number, width, IntBase::Hex)
            .unwrap()
            .with_system_base(system);
        let encoded = arguments::encode(&Value::Int(word));
        let decoded = arguments::json_args(&format!("[{encoded}]"), &Settings::default()).unwrap();
        let Value::Int(actual) = decoded[0] else {
            panic!("lost word type")
        };
        assert_eq!(actual, word);
    }
    for bad in [
        r##"[{"type":"integer","literal":"#1h","bits":64,"signed_word":true}]"##,
        r##"[{"type":"integer","literal":"#1h","signed_word":1}]"##,
    ] {
        assert!(arguments::json_args(bad, &Settings::default()).is_err());
    }
    let controls: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/firmware-word-return-controls.json"
    ))
    .unwrap();
    for case in controls["cases"].as_array().unwrap() {
        let words =
            arguments::json_args(&format!("[{}]", case["expected"]), &Settings::default()).unwrap();
        let Value::Int(word) = words[0] else {
            panic!("lost firmware word")
        };
        let expected = &case["expected"];
        assert_eq!(word.bits(), expected["bits"].as_u64().unwrap() as u8);
        assert_eq!(
            word.is_signed(),
            expected["signed_word"].as_bool().unwrap_or(false)
        );
        assert_eq!(
            word.is_system_base(),
            expected["system_base"].as_bool().unwrap_or(false)
        );
        assert_eq!(
            word.numeric_value().to_string(),
            expected["numeric"]
                .as_str()
                .or(expected["unsigned"].as_str())
                .unwrap()
        );
    }
}
