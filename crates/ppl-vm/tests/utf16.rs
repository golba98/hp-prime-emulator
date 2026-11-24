//! Lossless calculator strings, against actual local G1 ASC readbacks.
mod common;
use common::{Run, text};
use ppl_vm::{PortableValue, SessionContext, Value};

#[test]
fn actual_g1_utf16_controls_match_native_execution() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-utf16-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let mut run = Run::new(
            case["name"].as_str().unwrap(),
            case["source"].as_str().unwrap(),
        );
        let result = run.call(case["entry"].as_str().unwrap(), &[]);
        if case["success"] == true {
            let numerical = |source: &str| -> serde_json::Value {
                serde_json::from_str(&source.replace('{', "[").replace('}', "]")).unwrap()
            };
            assert_eq!(
                numerical(&text(&result.unwrap())),
                numerical(
                    case["message"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("G1 firmware returned ")
                        .unwrap()
                ),
                "{}",
                case["name"]
            );
        } else {
            assert!(
                result.is_err(),
                "{} must retain its genuine type error",
                case["name"]
            );
        }
    }
}

#[test]
fn isolated_units_survive_copy_calls_nested_collections_and_memory_round_trip() {
    let mut run = Run::new(
        "utf16-state",
        r#"
        EXPORT RAW:=0;
        COPYUNIT(x) BEGIN RETURN x; END;
        EXPORT TEST() BEGIN
          RAW:=CHAR({55357,0,56832});
          LOCAL a:={RAW}, b:=a;
          a(1):=REPLACE(a(1),1,CHAR(88));
          RETURN {ASC(COPYUNIT(RAW)),ASC(b(1)),ASC(a(1)),TYPE(RAW)};
        END;"#,
    );
    assert_eq!(
        text(&run.call("TEST", &[]).unwrap()),
        "{{55357,0,56832},{55357,0,56832},{88,0,56832},2}"
    );
    let raw = Value::from_utf16(vec![55357, 0, 56832]);
    let portable = PortableValue::from(&raw);
    assert_eq!(portable, PortableValue::Utf16(vec![55357, 0, 56832]));
    let mut memory = SessionContext::default();
    memory.set_variable("RAW", portable.clone()).unwrap();
    let encoded = memory.memory_json();
    let mut reopened = SessionContext::default();
    reopened.restore_memory(&encoded).unwrap();
    assert_eq!(reopened.snapshot().globals.get("RAW"), Some(&portable));
    assert!(portable.to_value().equals(&raw));
    assert_eq!(
        raw.display(ppl_vm::value::default_format()),
        "\\uD83D\0\\uDE00"
    );
    let mut bad = encoded.clone();
    bad["globals"]["RAW"]["value"][0] = 65536.into();
    assert!(reopened.restore_memory(&bad).is_err());
    assert_eq!(reopened.snapshot().globals.get("RAW"), Some(&portable));
}

#[test]
fn string_formatting_keeps_binary_units_and_quotes_inside_nested_values() {
    let mut run = Run::new(
        "utf16-format",
        r#"EXPORT TEST() BEGIN
       LOCAL s:=CHAR({55357,34,56832});
       RETURN {ASC(STRING(s)),ASC(STRING({s})),s(1),s(3),ASC(REVERSE(s)),ASC(HEAD(s)),ASC(TAIL(s))};
    END;"#,
    );
    assert_eq!(
        text(&run.call("TEST", &[]).unwrap()),
        "{{34,55357,92,34,56832,34},{123,34,55357,92,34,56832,34,125},55357,56832,{56832,34,55357},{55357},{34,56832}}"
    );
}
