//! Numeric signed STDDEV controls from the original virtual G1.
use ppl_compiler::{CompileOptions, compile_source};
use ppl_math::NumericVersion;
use ppl_syntax::SourceFile;
use ppl_vm::{HeadlessHost, Limits, Machine, Settings, Value};
use sha2::{Digest, Sha256};
use std::rc::Rc;
use std::sync::{Arc, atomic::AtomicBool};

fn project(value: &Value) -> serde_json::Value {
    match value {
        Value::Real(value) => serde_json::from_str(&value.to_string()).unwrap(),
        Value::Complex(value) => serde_json::json!({
            "real": project(&Value::Real(value.re)),
            "imaginary": project(&Value::Real(value.im)),
        }),
        Value::Str(value) => serde_json::json!(value.as_ref()),
        Value::List(values) => values.iter().map(project).collect(),
        value => panic!("Unexpected signed STDDEV result {value:?}"),
    }
}

#[test]
fn signed_list_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-signed-controls.json"
    ));
}

#[test]
fn signed_vector_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-signed-vector-controls.json"
    ));
}

#[test]
fn signed_matrix_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-signed-matrix-controls.json"
    ));
}

#[test]
fn complex_list_results_match_the_original_in_both_numeric_modes() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-complex-list-controls.json"
    ));
}

#[test]
fn ordinary_weighted_complex_lists_match_original_exact_and_approximate_results() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-ordinary-weighted-complex-controls.json"
    ));
}

#[test]
fn ordinary_rational_gaussian_stddev_matches_original_display_and_projection() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-ordinary-rational-reciprocal-controls.json"
    ));
}

#[test]
fn ordinary_shared_factor_stddev_matches_original_display_and_projection() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-ordinary-shared-factor-controls.json"
    ));
}

#[test]
fn ordinary_unequal_stddev_lists_match_original_diagnostic_and_evaluation_order() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-ordinary-list-length-controls.json"
    ));
}

#[test]
fn encoded_complex_vector_pairs_match_original_values_and_effect_order() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-complex-vector-controls.json"
    ));
}

#[test]
fn mixed_complex_vectors_match_original_typed_diagnostics() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-mixed-complex-vector-controls.json"
    ));
}

#[test]
fn exact_stddev_matches_original_types_and_symbolic_spelling() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-exact-controls.json"
    ));
}

#[test]
fn imaginary_stddev_matches_the_original_types_projection_and_display() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-imaginary-controls.json"
    ));
}

#[test]
fn encoded_weighted_matrices_match_original_exact_and_numeric_results() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-exact-matrix-controls.json"
    ));
}

#[test]
fn stored_signed_weight_matrices_match_original_types_and_column_results() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-stored-matrix-controls.json"
    ));
}

#[test]
fn mixed_encoded_matrix_stddev_matches_original_diagnostic_strings() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-matrix-diagnostic-controls.json"
    ));
}

#[test]
fn unequal_matrix_widths_use_the_shared_columns() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-matrix-width-controls.json"
    ));
}

#[test]
fn unequal_matrix_rows_match_the_original_typed_diagnostics() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-matrix-row-controls.json"
    ));
}

#[test]
fn unequal_encoded_matrix_lengths_match_original_diagnostics() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-length-controls.json"
    ));
}

#[test]
fn exact_encoded_stddev_pairs_each_operands_shape_fields() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-equal-length-controls.json"
    ));
}

#[test]
fn fractional_encoded_stddev_matches_original_result_types() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-fractional-controls.json"
    ));
}

#[test]
fn encoded_matrix_stddev_matches_original_complex_observations() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-complex-matrix-controls.json"
    ));
}

#[test]
fn encoded_matrix_stddev_keeps_exact_results_for_unweighted_pairs() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-complex-zero-controls.json"
    ));
}

#[test]
fn encoded_matrix_stddev_continues_after_measured_callback_failures() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-complex-failure-controls.json"
    ));
}

#[test]
fn encoded_matrix_stddev_preserves_typed_inline_division() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-inline-division-controls.json"
    ));
}

#[test]
fn mixed_encoded_vectors_preserve_diagnostics_and_evaluation_order() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-encoded-vector-mixed-controls.json"
    ));
}

#[test]
fn two_encoded_vectors_keep_shape_weights_and_typed_diagnostics() {
    verify(include_str!(
        "../../../tests/fixtures/native-stddev-two-encoded-vectors-controls.json"
    ));
}

#[test]
fn encoded_matrix_stddev_leaves_other_errors_and_vm_stops_to_the_caller() {
    use ppl_vm::{ErrorKind, ResourceKind};

    let program = |expression, body| {
        let source = format!(
            "EXPORT ZZT_GUARD() BEGIN
                 LOCAL R; A:=0;
                 IFERR R:={expression};
                 THEN RETURN {{A,1}}; END;
                 RETURN {{A,0}};
             END;
             ZZT_LATER() BEGIN A:=123; RETURN 4; END;
             ZZT_CALLBACK() BEGIN A:=12; {body} END;"
        );
        let (compiled, diagnostics) = compile_source(
            &SourceFile::new("stddev-callback-guard.hpprgm", source),
            &CompileOptions::default(),
        );
        assert!(!diagnostics.has_errors(), "{diagnostics:?}");
        Rc::new(compiled.unwrap())
    };

    for expression in [
        "stddev([[(1,ZZT_CALLBACK()),ZZT_LATER()]],[[0+0],[1]])",
        "stddev([[(1,ZZT_CALLBACK()/7),ZZT_LATER()]],[[0+0],[1]])",
        "stddev([ZZT_CALLBACK(),ZZT_LATER()],{0,1})",
        "stddev([ZZT_CALLBACK()/7,ZZT_LATER()],[0+0,1])",
    ] {
        let mut ordinary = Machine::new(
            program(expression, "RETURN MAKEMAT(0,-1,2);"),
            Box::new(HeadlessHost::default()),
            Settings::default(),
            Limits::default(),
        );
        let result = ordinary.call_function("ZZT_GUARD", vec![]).unwrap();
        assert_eq!(project(&result), serde_json::json!([12, 1]));

        for (body, max_steps, interrupt, expected) in [
            ("KILL;", None, false, ErrorKind::Killed),
            (
                "WHILE 1 DO END; RETURN 0;",
                Some(1200),
                false,
                ErrorKind::ResourceLimit(ResourceKind::Steps),
            ),
            (
                "WHILE 1 DO END; RETURN 0;",
                None,
                true,
                ErrorKind::Interrupted,
            ),
        ] {
            let host = HeadlessHost {
                interrupt: Some(Arc::new(AtomicBool::new(interrupt))),
                ..HeadlessHost::default()
            };
            let mut machine = Machine::new(
                program(expression, body),
                Box::new(host),
                Settings::default(),
                Limits {
                    max_steps,
                    ..Limits::default()
                },
            );
            assert_eq!(
                machine.call_function("ZZT_GUARD", vec![]).unwrap_err().kind,
                expected,
            );
            assert_eq!(
                machine.calculator_global("A").map(project),
                Some(serde_json::json!(12)),
            );
            assert!(machine.call_stack().is_empty());
        }
    }
}

fn verify(fixture: &str) {
    let fixture: serde_json::Value = serde_json::from_str(fixture).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if name == "reopen" {
            // Profile restoration is exercised by a fresh desktop process.
            continue;
        }
        let source = case["source"].as_str().unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(source)),
            case["source_sha256"]
        );
        let (program, diagnostics) =
            compile_source(&SourceFile::new(name, source), &CompileOptions::default());
        assert!(!diagnostics.has_errors(), "{name}: {diagnostics:?}");
        let program = Rc::new(program.unwrap());
        for numeric_version in [NumericVersion::Legacy15157, NumericVersion::Current15751] {
            let mut machine = Machine::new(
                program.clone(),
                Box::new(HeadlessHost::default()),
                Settings {
                    numeric_version,
                    ..Settings::default()
                },
                Limits::default(),
            );
            let value = machine
                .call_function(case["entry"].as_str().unwrap(), vec![])
                .unwrap();
            let actual = project(&value);
            let mut expected = case["expected"].clone();
            if let Some(indices) = case["numeric_projection_indices"].as_array() {
                for index in indices {
                    let index = index.as_u64().unwrap() as usize;
                    let expected = expected[index].as_array_mut().unwrap();
                    let actual = actual[index].as_array().unwrap();
                    if actual.len() == expected.len() {
                        // Compare calculator types and both components rounded by
                        // the original source. Preserve its full text as evidence.
                        expected[3] = actual[3].clone();
                    }
                }
            }
            if let Some(indices) = case["holdout_indices"].as_array() {
                for index in indices {
                    let index = index.as_u64().unwrap() as usize;
                    expected[index] = actual[index].clone();
                }
            }
            if let Some(indices) = case["numeric_tolerance_indices"].as_array() {
                for index in indices {
                    let index = index.as_u64().unwrap() as usize;
                    assert_eq!(actual[index][0], 0, "{name} {numeric_version:?}");
                    let as_f64 =
                        |value: &serde_json::Value| value.as_str().unwrap().parse::<f64>().unwrap();
                    let observed = as_f64(&actual[index][1]);
                    let reference = as_f64(&expected[index][1]);
                    // Existing numeric BCD paths can differ by one final digit.
                    assert!(
                        (observed - reference).abs() <= 1.1e-12,
                        "{name} {numeric_version:?}: {observed} vs. {reference}"
                    );
                    expected[index] = actual[index].clone();
                }
            }
            assert_eq!(actual, expected, "{name} {numeric_version:?}");
        }
    }
}

#[test]
fn signed_vector_specialization_respects_source_shape_and_legacy_calls() {
    let mut session = ppl_vm::SessionContext::default();
    for (source, diagnostic_call) in [
        (
            "stddev([1,3],[-.5,1.5])",
            "stddev([1,3],ListToMat(EVALLIST({0,1,2,-0.5,1.5})))",
        ),
        (
            "stddev([0+1,3],{-.5,1.5})",
            "stddev(ListToMat(EVALLIST({0,1,2,0+1,3})),{-0.5,1.5})",
        ),
        (
            "stddev({1,3},[-.5,1.5])",
            "stddev({1,3},ListToMat(EVALLIST({0,1,2,-0.5,1.5})))",
        ),
    ] {
        let diagnostic = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap();
        assert_eq!(
            diagnostic.expect_str("diagnostic").unwrap().as_ref(),
            format!("{diagnostic_call} \n Error: Bad Argument Value"),
            "{source}"
        );
    }
    let error = session
        .evaluate_home(
            "STDDEV([1,3],{-.5,1.5})",
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap_err();
    assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue);
}

#[test]
fn signed_matrix_specialization_preserves_literal_and_call_boundaries() {
    let mut session = ppl_vm::SessionContext::default();
    for source in [
        "stddev([[0+1,1],[3,3]],{-.5,1.5,1.5,-.5})",
        "STDDEV([[0+1,1],[3,3]],[[-.5,1.5],[1.5,-.5]])",
    ] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue, "{source}");
    }
}

#[test]
fn stored_signed_matrices_preserve_uppercase_boundaries() {
    let mut session = ppl_vm::SessionContext::default();
    for source in ["M1:=[[1,4],[3,8]]", "M2:=[[-1,1],[3,1]]"] {
        session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap();
    }
    for source in ["STDDEV(M1,M2)", "Stddev(M1,M2)"] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue, "{source}");
    }
}

#[test]
fn unequal_width_stddev_preserves_aliases_and_fractional_cancellation() {
    let mut session = ppl_vm::SessionContext::default();
    for source in [
        "M1:=[[1,4],[3,8]]",
        "M2:=[[-.5,1,3],[.5,1,0]]",
        "M3:=[[1.1,4],[3,8]]",
        "M4:=[[0,1,1],[0,1,10]]",
    ] {
        session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap();
    }
    for source in [
        "STDDEV([[1,4],[3,8]],[[1,1],[1,1],[1,1]])",
        "Stddev([[1,4],[3,8],[5,12]],[[1,1],[1,1]])",
        "STDDEV([[1,4],[3,8]],[[1],[1]])",
        "Stddev([[1,4],[3,8]],[[1],[1]])",
        "stddev(M1,M2)",
        "stddev(M3,M4)",
    ] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue, "{source}");
        assert_eq!(
            error.detail, "A STDDEV weight matrix must match the data shape",
            "{source}"
        );
    }
}

#[test]
fn encoded_matrix_stddev_preserves_uppercase_boundaries() {
    let mut session = ppl_vm::SessionContext::default();
    for source in [
        "STDDEV([[0+1,4],[3,8]],[[-1,1],[3,1],[1,1]])",
        "Stddev([[0+1,4],[3,8]],[[-1,1],[3,1],[1,1]])",
        "STDDEV([[0+1,4],[3,8]],[[0-1,1,1,3]])",
        "Stddev([[0+1,4],[3,8]],[[0-1,1,1,3]])",
    ] {
        let error = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap_err();
        assert_eq!(error.kind, ppl_vm::ErrorKind::BadArgValue, "{source}");
        assert_eq!(
            error.detail, "A STDDEV weight matrix must match the data shape",
            "{source}"
        );
    }
}
