use ppl_cas::units::translate_home;

#[test]
fn compound_measurements_and_tools_use_giac_syntax() {
    for (input, expected) in [
        ("2_m", "2_(m)"),
        ("mksa(Ans)", "mksa(Ans)"),
        ("20_cm+5_in", "20_(cm)+5_(inch)"),
        ("MKSA(8.175_cm/s)", "mksa(8.175_(cm/s))"),
        ("UFACTOR(100_C,1_A)", "ufactor(100_(C),1_(A))"),
        ("USIMPLIFY(5_kg*m^2/s^2)", "usimplify(5_(kg*m^2/s^2))"),
        ("CONVERT(5_m,1_ft)", "convert(5_(m),1_(ft))"),
        ("2_m*3_m", "2_(m)*3_(m)"),
        ("2_m*x", "2_(m)*x"),
        ("1E-3_m+2_µm", "1E-3_(m)+2_(µm)"),
        ("3_min+1_h", "3_(mn)+1_(h)"),
        ("2_Ω", "2_(Ohm)"),
        ("(2+3)_m", "(2+3)_(m)"),
    ] {
        assert_eq!(
            translate_home(input).unwrap().as_deref(),
            Some(expected),
            "{input}"
        );
    }
}
#[test]
fn ordinary_identifiers_comments_and_strings_do_not_enable_unit_evaluation() {
    for input in [
        "2+3",
        "speed_m+1",
        "A2_m",
        "\"2_m MKSA(1_cm)\"",
        "// 2_m\n2+3",
        "/* CONVERT(2_m,1_s) */ 2+3",
    ] {
        assert_eq!(translate_home(input).unwrap(), None, "{input}");
    }
    assert_eq!(
        translate_home("\"2_in\"+2_m // 5_in").unwrap().unwrap(),
        "\"2_in\"+2_(m) // 5_in"
    );
}
#[test]
fn unknown_affine_and_malformed_units_are_explicit_errors() {
    for input in [
        "2_unknown",
        "2_degC",
        "2_gal",
        "2_torr",
        "2_fm",
        "2_°C",
        "2_cd",
        "2_",
        "2_(m+2)",
        "2_(m/unknown)",
        "2_m^x",
    ] {
        assert!(translate_home(input).is_err(), "{input}");
    }
    assert!(translate_home(&format!("2_{}m{}", "(".repeat(40), ")".repeat(40))).is_err());
}

#[cfg(feature = "giac-integration")]
mod actual_giac {
    use super::*;
    use ppl_cas::{CasSession, CasValue, Options};
    use std::{collections::BTreeMap, sync::atomic::AtomicBool};
    fn evaluate(
        input: &str,
        home: &BTreeMap<String, CasValue>,
    ) -> Result<CasValue, ppl_cas::CasError> {
        let mut session = CasSession::default();
        let source = translate_home(input)?.expect("measurement required");
        let result = session.evaluate_with_home(
            &source,
            home,
            &Options::default(),
            &AtomicBool::new(false),
        )?;
        assert!(result.home_updates.is_empty());
        assert!(session.bindings.is_empty());
        Ok(result.value)
    }
    fn equivalent(input: &str, expected: &str) {
        // Subtract real dimensional objects after MKSA conversion. Giac retains
        // units on zero; inspect its zero coefficient rather than display text.
        let comparison = format!("MKSA(({input})-({expected}))");
        let value = evaluate(&comparison, &BTreeMap::new()).unwrap();
        let coefficient = match &value {
            CasValue::Symbolic(text) => text.split_once('_').expect("zero dimensional quantity").0,
            CasValue::Integer(text) | CasValue::Decimal(text) => text,
            _ => panic!("unexpected difference {value:?}"),
        };
        let coefficient = coefficient
            .parse::<f64>()
            .expect("numeric measurement coefficient");
        // Giac's unit conversion factors use floating arithmetic. This is an
        // absolute 1e-12 tolerance in SI base units, below the HP 12-digit output
        // resolution for these guide examples; no display comparison is used.
        assert!(coefficient.abs() < 1e-12, "{input}: {value:?}");
    }
    #[test]
    fn guide_length_addition_and_resulting_speed() {
        equivalent("20_cm+5_in", "32.7_cm");
        equivalent("(20_cm+5_in)/4_s", "8.175_cm/s");
        equivalent("CONVERT((20_cm+5_in)/4_s,1_km/h)", "0.2943_km/h");
    }
    #[test]
    fn guide_unit_tools_are_actual_dimensional_operations() {
        equivalent("CONVERT(5_m,1_ft)", "5_m");
        let text = evaluate("CONVERT(5_m,1_ft)", &BTreeMap::new())
            .unwrap()
            .expression();
        assert!(
            text.contains("16.4041994751") && text.contains("ft"),
            "{text}"
        );
        equivalent("MKSA(8.175_cm/s)", "0.08175_m/s");
        equivalent("UFACTOR(100_C,1_A)", "100_A*s");
        let text = evaluate("USIMPLIFY(5_kg*m^2/s^2)", &BTreeMap::new())
            .unwrap()
            .expression();
        assert!(text.contains("5.0_J"), "{text}");
    }
    #[test]
    fn multiplication_and_unit_powers_preserve_dimensions() {
        equivalent("2_m*3_m", "6_m^2");
        assert_eq!(
            evaluate("2_m*3_m", &BTreeMap::new()).unwrap(),
            CasValue::Symbolic("6_(m^2)".into())
        );
        equivalent("3_min+1_h", "3780_s");
        equivalent("2_µm", "0.000002_m");
    }
    #[test]
    fn incompatible_dimensions_report_real_errors() {
        for input in ["2_m+3_s", "CONVERT(5_m,1_s)"] {
            assert!(matches!(
                evaluate(input, &BTreeMap::new()),
                Err(ppl_cas::CasError::Evaluation(_))
            ));
        }
    }
    #[test]
    fn unit_names_cannot_be_captured_by_home_variable_aliases() {
        let home = BTreeMap::from([
            ("m".into(), CasValue::Integer("100".into())),
            ("s".into(), CasValue::Integer("7".into())),
            ("A".into(), CasValue::Integer("12".into())),
            ("K".into(), CasValue::Integer("99".into())),
        ]);
        let value = evaluate("2_m/s+3_m/s", &home).unwrap();
        assert!(value.expression().starts_with("5_"), "{value:?}");
        assert!(!value.expression().contains("pplHomeBridge"));
        let value = evaluate("2_A+3_A", &home).unwrap();
        assert!(value.expression().starts_with("5_"), "{value:?}");
    }
}
