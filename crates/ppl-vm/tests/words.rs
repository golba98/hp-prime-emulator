//! Original virtual-firmware controls for word arithmetic and live metadata.
mod common;
use common::Run;
use ppl_vm::{HeadlessHost, Limits, PortableValue, SessionContext};
#[test]
fn original_firmware_word_controls() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-word-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let value = run
            .call(case["entry"].as_str().unwrap(), &[])
            .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        assert_eq!(
            value.expect_str("control").unwrap().to_string(),
            case["expected"].as_str().unwrap(),
            "{}",
            case["name"]
        );
    }
}
#[test]
fn signed_words_and_system_base_survive_shutdown_and_home_linking() {
    let mut session = SessionContext::default();
    let eval = |session: &mut SessionContext, source: &str| {
        session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap()
    };
    eval(&mut session, "W:=R→B(-5,-8)");
    eval(&mut session, "Bits:=8");
    eval(&mut session, "Signed:=1");
    eval(&mut session, "V:=#FFh");
    let saved = session.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&saved).unwrap();
    assert_eq!(restored.memory_json(), saved);
    eval(&mut restored, "Base:=0");
    let value = eval(
        &mut restored,
        "{B→R(W),GETBITS(W),STRING(W),GETBITS(V),B→R(V),STRING(V)}",
    );
    let values = value.expect_list("words").unwrap();
    assert_eq!(values[0].expect_i64("number").unwrap(), -5);
    assert_eq!(values[1].expect_i64("number").unwrap(), -8);
    assert_eq!(values[2].expect_str("paint").unwrap().to_string(), "#−101b");
    assert_eq!(values[3].expect_i64("number").unwrap(), -8);
    assert_eq!(values[4].expect_i64("number").unwrap(), 255);
    assert_eq!(values[5].expect_str("paint").unwrap().to_string(), "#FFh");
    let saved = restored.memory_json();
    for (name, code) in [("Bits", 64), ("Bits", 0), ("Base", 4)] {
        assert!(
            restored
                .set_variable(name, PortableValue::Real(ppl_math::Real::from_i64(code)))
                .is_err()
        );
        assert_eq!(restored.memory_json(), saved);
    }
    let mut broken = saved.clone();
    broken["settings"]["bits"] = 64.into();
    assert!(restored.restore_memory(&broken).is_err());
    assert_eq!(restored.memory_json(), saved);
}

#[test]
fn fractional_bit_arguments_fail_and_word_conversion_keeps_all64_bits() {
    let mut run = Run::new(
        "admission",
        r#"EXPORT WADMIT() BEGIN Base:=3; Bits:=32; Signed:=0; LOCAL out:={},r;
    IFERR r:=STRING(SETBITS(1.9,-8)); THEN r:="error"; END; out:=CONCAT(out,{r});
    IFERR r:=STRING(SETBASE(-2.5,4)); THEN r:="error"; END; out:=CONCAT(out,{r});
    IFERR r:=STRING(BITSL(1.9,1)); THEN r:="error"; END; out:=CONCAT(out,{r});
    IFERR r:=STRING(BITOR(1.9,1)); THEN r:="error"; END; out:=CONCAT(out,{r});
    IFERR r:=STRING(BITSL(5,1.9)); THEN r:="error"; END; out:=CONCAT(out,{r});
    IFERR r:=STRING(R→B(#FFFFFFFFFFFFFFFF:64h,64)); THEN r:="error"; END; out:=CONCAT(out,{r});
    IFERR r:=STRING(R→B(R→B(-1,-8),8)); THEN r:="error"; END; out:=CONCAT(out,{r}); RETURN out; END;"#,
    );
    let value = run.call("WADMIT", &[]).unwrap();
    let actual = value
        .expect_list("results")
        .unwrap()
        .iter()
        .map(|value| value.expect_str("result").unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        [
            "error",
            "error",
            "error",
            "error",
            "error",
            "#FFFFFFFFFFFFFFFF:64h",
            "#0:8h"
        ]
    );
}
#[test]
fn home_linked_root_inherits_width_but_installed_program_literals_keep32() {
    use ppl_vm::{ModuleSource, SourceIdentity};
    let mut session = SessionContext::default();
    session.settings.set_integer_bits(8).unwrap();
    session.settings.set_integer_signed(true).unwrap();
    let source = "EXPORT WORDLITERAL() BEGIN RETURN GETBITS(#FFh); END;";
    let sources = std::collections::BTreeMap::from([(
        "words".into(),
        ModuleSource {
            identity: SourceIdentity::new("words", source),
            source: source.into(),
        },
    )]);
    let value = session
        .evaluate_home_with_sources(
            "{GETBITS(#FFh),WORDLITERAL(),EXPR(\"GETBITS(#FFh)\")}",
            &sources,
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap();
    assert_eq!(
        value
            .expect_list("widths")
            .unwrap()
            .iter()
            .map(|value| value.expect_i64("width").unwrap())
            .collect::<Vec<_>>(),
        [-8, 32, -8]
    );
}
