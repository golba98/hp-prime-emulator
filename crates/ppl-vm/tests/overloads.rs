//! Documented extended forms; local firmware controls are in the remediation evidence.
mod common;
use common::{Run, text};
use ppl_vm::Value;
use sha2::{Digest, Sha256};
fn run(body: &str) -> Result<String, ppl_vm::RuntimeError> {
    let mut run = Run::new("<overloads>", &format!("EXPORT TEST() BEGIN {body} END;"));
    run.call("TEST", &[]).map(|v| text(&v))
}
#[test]
fn append_preserves_vector_and_does_not_mutate_original() {
    assert_eq!(
        run("LOCAL a:=[1,2]; LOCAL b:=APPEND(a,3); RETURN {a,b,SIZE(b)};").unwrap(),
        "{[1,2],[1,2,3],{3}}"
    );
    assert_eq!(
        run("RETURN APPEND([1,2],\"bad\");").unwrap_err().kind,
        ppl_vm::ErrorKind::BadArgType
    );
}

#[test]
fn local_zero_index_store_preserves_assignment_semantics() {
    use ppl_math::NumericVersion;
    use ppl_vm::{ErrorKind, ResourceKind};

    let mut program = Run::new(
        "<local zero index>",
        r#"
EXPORT STORE() BEGIN
    LOCAL L:={8}, Alias:=L, Count:=0;
    LOCAL Assigned:=(L(0):=(Count:=Count+1));
    LOCAL Empty;
    Empty(0):=7;
    RETURN {Assigned,L,Alias,Count,Empty};
END;
EXPORT VECTORSTORE() BEGIN
    LOCAL M:=[1,2];
    M(0):=9;
END;
EXPORT LIMITSTORE() BEGIN
    LOCAL L:={8};
    L(0):=1;
    RETURN SIZE(L);
END;
"#,
    );
    for version in [NumericVersion::Legacy15157, NumericVersion::Current15751] {
        program.machine.settings.numeric_version = version;
        assert_eq!(
            text(&program.call("STORE", &[]).unwrap()),
            "{1,{8,1},{8},1,{7}}"
        );
        assert_eq!(
            program.call("VECTORSTORE", &[]).unwrap_err().kind,
            ErrorKind::InvalidDimension
        );
    }
    program.machine.limits.max_collection_len = 1;
    let error = program.call("LIMITSTORE", &[]).unwrap_err();
    assert_eq!(error.kind, ErrorKind::ResourceLimit(ResourceKind::Memory));
    assert_eq!(error.detail, "list too long");
}

#[test]
fn lowercase_append_matches_original_collection_shapes_and_types() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-collection-controls.json"),
        "ZZT_APPCOLL",
    );
}

#[test]
fn lowercase_append_matches_original_string_payload_types() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-string-payload-controls.json"),
        "ZZT_APPSTR",
    );
}

#[test]
fn lowercase_append_matches_original_literal_assignment_and_computed_text() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-assignment-payload-controls.json"),
        "ZZT_APPSET",
    );
}

#[test]
fn lowercase_append_matches_original_literal_quotient_assignments() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-assignment-quotient-controls.json"),
        "ZZT_APPQUOT",
    );
}

#[test]
fn lowercase_append_matches_original_direct_quotient_results() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-inline-division-controls.json"),
        "ZZT_APPDIV",
    );
}

#[test]
fn lowercase_append_matches_original_callback_division_faults() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-callback-division-controls.json"),
        "ZZT_APPCBD",
    );
}

#[test]
fn lowercase_append_matches_original_callback_type_faults_and_order() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-callback-type-controls.json"),
        "ZZT_APPTYPE",
    );
}

#[test]
fn lowercase_append_matches_original_string_quotient_rejection() {
    verify_append(
        include_str!(
            "../../../tests/fixtures/native-append-string-quotient-rejection-controls.json"
        ),
        "ZZT_APPREJ",
    );
}

#[test]
fn lowercase_append_preserves_matrix_carriers_after_callback_division_faults() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-matrix-callback-controls.json"),
        "ZZT_APPMCF",
    );
}

#[test]
fn lowercase_append_matches_original_computed_string_payloads() {
    verify_append(
        include_str!("../../../tests/fixtures/native-append-computed-string-controls.json"),
        "ZZT_APPCSTR",
    );
}

#[test]
fn append_callback_guard_preserves_non_program_stops() {
    for base in ["[1,2]", "[[1,2],[3,4]]"] {
        let mut program = Run::new(
            "<append callback stop>",
            &format!(
                "EXPORT TEST() BEGIN RETURN append({base},STOPNOW()); END; STOPNOW() BEGIN KILL; END;"
            ),
        );
        assert_eq!(
            program.call("TEST", &[]).unwrap_err().kind,
            ppl_vm::ErrorKind::Killed
        );
    }
}

#[test]
fn append_assignment_stores_once_after_the_collection_is_built() {
    use ppl_vm::{ErrorKind, HeadlessHost, Limits, ResourceKind, SessionContext};

    let mut session = SessionContext::default();
    let mut evaluate = |input: &str, limits| {
        session
            .evaluate_home(input, Box::new(HeadlessHost::new()), limits)
            .result
    };
    let limits = Limits::default();
    evaluate("A:=7", limits.clone()).unwrap();
    let append = r#"append([A,2],"A:=A+1")"#;
    let error = evaluate(
        append,
        Limits {
            max_collection_len: 2,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::ResourceLimit(ResourceKind::Memory));
    assert_eq!(text(&evaluate("A", limits.clone()).unwrap()), "7");
    assert_eq!(text(&evaluate(append, limits.clone()).unwrap()), "[7,2,8]");
    assert_eq!(text(&evaluate("A", limits).unwrap()), "8");
}

fn verify_append(fixture: &str, entry: &str) {
    fn project(value: &Value) -> serde_json::Value {
        match value {
            Value::List(items) => items.iter().map(project).collect(),
            Value::Real(number) => serde_json::from_str(&number.to_string()).unwrap(),
            Value::Str(text) => serde_json::json!(text.as_ref()),
            other => panic!("unexpected APPEND probe result {other:?}"),
        }
    }

    let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let actual = Run::new(name, source).call(entry, &[]).unwrap();
        let actual = project(&actual);
        let actual = actual.as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len(), "{name}");
        let holdouts = case["holdout_indices"].as_array().unwrap();
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            if !holdouts
                .iter()
                .any(|value| value.as_u64() == Some(index as u64))
            {
                assert_eq!(actual, expected, "{name}, control {index}");
            }
        }
    }
}
#[test]
fn population_stddev_supports_real_weights_vectors_and_matrix_columns() {
    assert_eq!(
        run("RETURN ROUND(STDDEV([1,2,3]),10);").unwrap(),
        "0.8164965809"
    );
    assert_eq!(
        run("RETURN ROUND(STDDEV({1,3},{0.5,1.5}),10);").unwrap(),
        "0.8660254038"
    );
    assert_eq!(run("RETURN STDDEV([[1,2],[3,4]]);").unwrap(), "[1,1]");
    assert_eq!(run("RETURN STDDEV({1,3},{1E100,1E100});").unwrap(), "1");
    assert_eq!(run("RETURN STDDEV({1E-400,3E-400});").unwrap(), "1E-400");
    assert_eq!(run("RETURN STDDEV({1E400,3E400});").unwrap(), "1E400");
    assert!(run("RETURN STDDEV({1,2},{1});").is_err());
    assert!(run("RETURN STDDEV([1,2],[1,-1]);").is_err());
    assert!(run("RETURN STDDEV([[1,2],[3,4]],[[1,-1],[1,1]]);").is_err());
}
#[test]
fn scalar_undefined_and_matrix_stddev_match_original_g1_controls() {
    assert_eq!(
        run("RETURN {TYPE(STDDEV(3)),STDDEV(3),STDDEV(1+i)};").unwrap(),
        "{0,0,0}"
    );
    for expression in ["STDDEV({})", "STDDEV({},{})", "STDDEV({1,3},{0,0})"] {
        assert_eq!(
            run(&format!(
                "LOCAL R:={expression}; RETURN {{TYPE(R),STRING(R)}};"
            ))
            .unwrap(),
            "{8,\"undef\"}",
        );
    }
    assert_eq!(
        run("LOCAL V:=STDDEV([[1,10],[3,18],[9,38]],[[1,1],[0,3],[0,0]]); RETURN {TYPE(V),V};")
            .unwrap(),
        "{4,[0,3.46410161514]}"
    );
    assert_eq!(
        run("LOCAL V:=STDDEV([[1,10],[3,18],[9,38]],[[1,0],[3,0],[0,0]]); RETURN {TYPE(V),V};")
            .unwrap(),
        "{6,{undef}}"
    );
    for weights in ["[1,3]", "{1,3}", "[[1,1],[0,3]]"] {
        assert!(run(&format!("RETURN STDDEV([[1,10],[3,18],[9,38]],{weights});")).is_err());
    }
}

#[test]
fn string_separator_symbols_and_dotzero_follow_firmware_controls() {
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,\"123,456\");").unwrap(),
        "1234,567.89"
    );
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,\"123,456.789\");").unwrap(),
        "1,234.567.89"
    );
    assert_eq!(
        run("RETURN STRING(-0.125,2,3,{-1,\",e~\",1});").unwrap(),
        "~0,125"
    );
    assert_eq!(
        run("RETURN STRING(0.125,2,3,{-1,\"\",0});").unwrap(),
        ".125"
    );
    assert_eq!(
        run("RETURN STRING(0.125,2,3,{-1,\"\",1});").unwrap(),
        "0.125"
    );
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,10);").unwrap(),
        "12,34,567.89"
    );
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,0);").unwrap(),
        "1﹐234﹐567.89"
    );
}
#[test]
fn string_fractions_floating_rounded_and_current_settings() {
    assert_eq!(run("RETURN STRING(1/3,8,3);").unwrap(), "1/3");
    assert_eq!(run("RETURN STRING(-7/3,15,3);").unwrap(), "−2 1/3");
    assert_eq!(run("RETURN STRING(12345.6789,5,3);").unwrap(), "1.235ᴇ4");
    assert_eq!(run("RETURN STRING(12345.6789,6,3);").unwrap(), "12345.679");
    assert_eq!(run("RETURN STRING(0.001234567,5,3);").unwrap(), "0.0012");
    assert_eq!(run("RETURN STRING(0.001234567,6,3);").unwrap(), "0.001");
    assert_eq!(
        run("HFormat:=1; HDigits:=2; RETURN STRING(1.2345,0);").unwrap(),
        "1.23"
    );
    // Independently checked original15751 firmware: a nonempty custom
    // grouping pattern must end in a digit, even for a single integral digit.
    assert!(run("RETURN STRING(1,2,3,\",\");").is_err());
    assert_eq!(run("RETURN STRING(1,2,3,\"\");").unwrap(), "1.000");
}
#[test]
fn string_formatting_does_not_rewrite_string_contents_in_collections() {
    assert_eq!(
        run("RETURN STRING({1234.5,\"1234.5\"},2,1,\"123,456\");").unwrap(),
        "{1,234.5,\"1234.5\"}"
    );
}
#[test]
fn dimgrob_dimension_pair_and_depth_string_match_local_firmware() {
    assert_eq!(run("DIMGROB_P(G1,{1,2}); RETURN {GROBW_P(G1),GROBH_P(G1),GETPIX_P(G1,0,0),GETPIX_P(G1,0,1)};").unwrap(),"{1,2,#FFFFFFh,#FFFFFFh}");
    assert_eq!(
        run("LOCAL z:=TRIANGLE_P(); RETURN {DIM(z),ASC(LEFT(z,2))};").unwrap(),
        "{38400,{65535,65535}}"
    );
    assert_eq!(
        run("DIMGROB_P(G1,3,3); RETURN DIM(TRIANGLE_P(G1));").unwrap(),
        "5"
    );
}
#[test]
fn triangle_vertex_lists_gradient_and_alpha_have_actual_pixels() {
    assert_eq!(run("RECT_P(0); TRIANGLE_P({0,0,#FF0000h},{10,0,#00FF00h},{0,10,#0000FFh}); RETURN GETPIX_P(1,1);").unwrap(),"#C81C19h");
    assert_eq!(
        run("RECT_P(0); TRIANGLE_P(0,0,10,0,0,10,#FF0000h,128); RETURN GETPIX_P(1,1);").unwrap(),
        "#7F0000h"
    );
}
#[test]
fn cartesian_graphics_use_the_current_calculator_plot_window() {
    assert_eq!(run("Xmin:=0; Xmax:=10; Ymin:=0; Ymax:=10; RECT_P(0); PIXON(5,5,#FF0000h); RETURN {GETPIX_P(160,109),Xmin,Xmax,NumStart,NumStep};").unwrap(),"{#FF0000h,0,10,0,0.1}");
    assert_eq!(
        run("Xmin:=1; Xmax:=1; PIXON(0,0);").unwrap_err().kind,
        ppl_vm::ErrorKind::BadArgValue
    );
    assert_eq!(
        run("Xmin:=1; Xmax:=1; PIXON_P(0,0); RETURN 1;").unwrap(),
        "1"
    );
}
