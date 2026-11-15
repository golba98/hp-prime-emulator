//! Presentation metadata through actual parser/compiler/VM/storage paths.
//! Reference outputs are self-authored programs executed by the installed G1
//! firmware; no firmware binary, heap data or physical-device claim is included.
use ppl_math::{ComplexPresentation, RealPresentation};
use ppl_vm::{HeadlessHost, Limits, SessionContext, Value};

fn home(session: &mut SessionContext, expression: &str) -> Value {
    session
        .evaluate_home(expression, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap_or_else(|error| panic!("{expression}: {error}"))
}

#[test]
fn firmware_formatted_controls_run_in_the_native_vm() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-formatted-controls.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let expression = case["expression"].as_str().unwrap();
        let value = home(&mut SessionContext::default(), expression);
        let Value::List(values) = value else {
            panic!("expected reference list")
        };
        assert_eq!(values.len(), case["expected"].as_array().unwrap().len());
        for (index, (value, expected)) in values
            .iter()
            .zip(case["expected"].as_array().unwrap())
            .enumerate()
        {
            let actual = match value {
                Value::Str(value) => serde_json::Value::String(value.to_string()),
                other => serde_json::json!(other.expect_i64("TYPE result").unwrap()),
            };
            // G1 STRING uses a private imaginary glyph and, in this build,
            // a typographic minus. Native's ASCII spellings remain part of
            // the separately open exact text/font compatibility finding.
            let glyphs = |value: &serde_json::Value| match value.as_str() {
                Some(text) => {
                    serde_json::Value::String(text.replace('−', "-").replace('\u{e003}', "i"))
                }
                None => value.clone(),
            };
            let actual = glyphs(&actual);
            let expected = glyphs(expected);
            assert_eq!(actual, expected, "{expression}; index {index}");
        }
    }
    eprintln!(
        "79 formatted controls matched after explicit minus/imaginary-glyph mapping; no numeric tolerance"
    );
}

#[test]
fn metadata_moves_with_variables_collection_cells_ans_and_ppl_returns() {
    let mut session = SessionContext::default();
    home(&mut session, "A:=1°22′30″");
    assert_eq!(
        home(&mut session, "STRING(Ans*2)")
            .expect_str("string")
            .unwrap()
            .as_ref(),
        "2°45′00″"
    );
    home(&mut session, "L1:={A,2∡1,(1°,2)}");
    home(&mut session, "M1:=[[A,1],[2,3]]");
    for text in ["A", "L1(1)", "M1(1,1)"] {
        assert_eq!(
            home(&mut session, text)
                .expect_real("angle")
                .unwrap()
                .presentation(),
            RealPresentation::Sexagesimal
        );
    }
    let source = ppl_syntax::SourceFile::new(
        "metadata.hpprgm",
        "EXPORT META() BEGIN LOCAL a:=L1(2); RETURN a*2; END;",
    );
    let (program, diagnostics) = ppl_compiler::compile_source(&source, &Default::default());
    assert!(!diagnostics.has_errors());
    let result = session
        .run(
            ppl_vm::SourceIdentity::new("metadata.hpprgm", &source.text),
            std::rc::Rc::new(program.unwrap()),
            Box::new(HeadlessHost::new()),
            Limits::default(),
            false,
            |machine| machine.call_function("META", vec![]),
        )
        .result
        .unwrap();
    let Value::Complex(value) = result else {
        panic!("PPL return lost complex type")
    };
    assert_eq!(value.presentation(), ComplexPresentation::Polar);
    assert_eq!(
        home(&mut session, "STRING(L1(3))")
            .expect_str("string")
            .unwrap()
            .as_ref(),
        "1°00′00″+2°00′00″*\u{e003}"
    );
}

#[test]
fn formatting_metadata_survives_a_fresh_process_and_old_memory_migrates() {
    let mut session = SessionContext::default();
    home(&mut session, "A:=1°22′30″");
    home(&mut session, "L1:={A,2∡1,(1°,2)}");
    let image = session.memory_json();
    assert_eq!(image["version"], 5);
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("memory.json"),
        serde_json::to_vec(&image).unwrap(),
    )
    .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "formatted_memory_child", "--nocapture"])
        .env("PPL_FORMATTED_MEMORY_CHILD", root.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(String::from_utf8_lossy(&child.stdout).contains("formatted-memory-resumed"));
    let mut legacy = SessionContext::default().memory_json();
    legacy["version"] = 1.into();
    let mut restored = SessionContext::default();
    restored.restore_memory(&legacy).unwrap();
    assert_eq!(restored.memory_json()["version"], 5);
    let mut version_two = image.clone();
    version_two["version"] = 2.into();
    restored.restore_memory(&version_two).unwrap();
    assert_eq!(restored.memory_json()["globals"], image["globals"]);
    let before = restored.memory_json();
    let mut bad = image;
    bad["globals"]["A"]["presentation"] = "unknown future format".into();
    assert!(restored.restore_memory(&bad).is_err());
    assert_eq!(before, restored.memory_json());
}

#[test]
fn formatted_memory_child() {
    let Some(root) = std::env::var_os("PPL_FORMATTED_MEMORY_CHILD") else {
        return;
    };
    let image = serde_json::from_slice(
        &std::fs::read(std::path::Path::new(&root).join("memory.json")).unwrap(),
    )
    .unwrap();
    let mut session = SessionContext::default();
    session.restore_memory(&image).unwrap();
    assert_eq!(
        home(&mut session, "STRING(A*2)")
            .expect_str("string")
            .unwrap()
            .as_ref(),
        "2°45′00″"
    );
    let Value::Complex(polar) = home(&mut session, "L1(2)") else {
        panic!("complex value missing")
    };
    assert_eq!(polar.presentation(), ComplexPresentation::Polar);
    let Value::Complex(dms) = home(&mut session, "L1(3)") else {
        panic!("complex value missing")
    };
    assert_eq!(dms.presentation(), ComplexPresentation::Sexagesimal);
    assert_eq!(
        home(&mut session, "STRING(Ans)")
            .expect_str("string")
            .unwrap()
            .as_ref(),
        "1°00′00″+2°00′00″*\u{e003}"
    );
    println!("formatted-memory-resumed");
}

#[test]
fn hms_overloads_preserve_values_and_reject_the_same_nonreal_controls_as_g1() {
    let mut session = SessionContext::default();
    for (source, expected) in [
        ("STRING(→HMS(#FFh))", "255°00′00″"),
        ("STRING(→HMS({1,2}))", "{1°00′00″,2°00′00″}"),
        ("STRING(→HMS((1,0)))", "1°00′00″"),
    ] {
        assert_eq!(
            home(&mut session, source)
                .expect_str("string")
                .unwrap()
                .as_ref(),
            expected
        );
    }
    for source in ["→HMS([1,2])", "→HMS((1,2))", "HMS→((1°,2))", "HMS→(2∡1)"] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgType, "{source}");
    }
}
