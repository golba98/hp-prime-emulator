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
    let values=arguments::json_args(r##"[{"type":"ppl","value":"#FF:8h"},{"type":"complex","real":"1e400","imaginary":"2"},{"type":"matrix","rows":1,"columns":2,"vector":true,"value":[1,2]},{"type":"list","value":[1e400]},{"type":"grob","index":2}]"##,&settings).unwrap();
    for value in values {
        let encoded = arguments::encode(&value);
        let again =
            arguments::json_args(&serde_json::json!([encoded]).to_string(), &settings).unwrap();
        assert_eq!(arguments::encode(&again[0]), arguments::encode(&value));
    }
}

fn interaction_report(source: &str, inputs: &[&str]) -> Report {
    ppl_engine::native::execute(
        Source::text(source, "TRACE").unwrap(),
        Operation::Run,
        ppl_engine::native::Options {
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
