use ppl_vm::{HeadlessHost, Limits, SessionContext, Value};
fn eval(session: &mut SessionContext, text: &str) -> Value {
    session
        .evaluate_home(text, Box::new(HeadlessHost::new()), Limits::default())
        .result
        .unwrap()
}
#[test]
fn quoted_expression_is_a_symbolic_value_not_an_undefined_variable() {
    let mut context = SessionContext::default();
    let value = eval(&mut context, "'x^2+1'");
    assert!(matches!(value, Value::Symbolic(_)));
    assert!(value.display(Default::default()).contains('x'));
    let image = context.memory_json();
    let mut restored = SessionContext::default();
    restored.restore_memory(&image).unwrap();
    assert_eq!(image, restored.memory_json());
}
#[test]
fn named_catalog_values_are_readable_by_home() {
    let mut context = SessionContext::default();
    context
        .set_variable(
            "custom",
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(9)),
        )
        .unwrap();
    assert_eq!(
        eval(&mut context, "custom+1").expect_i64("test").unwrap(),
        10
    );
}
#[cfg(feature = "giac-integration")]
mod actual_giac {
    use super::*;
    #[test]
    fn home_and_cas_share_variable_identity_without_sharing_ans_or_settings() {
        let mut session = SessionContext::default();
        eval(&mut session, "Q:=100");
        eval(&mut session, "CAS(\"q:=1000\")");
        assert_eq!(eval(&mut session, "5*q").expect_i64("test").unwrap(), 5000);
        assert_eq!(
            eval(&mut session, "q:=2000").expect_i64("test").unwrap(),
            2000
        );
        assert!(!session.snapshot().globals.contains_key("q"));
        assert_eq!(
            eval(&mut session, "CAS(\"Q+q\")")
                .expect_i64("test")
                .unwrap(),
            2100
        );
        eval(&mut session, "CAS(\"Q:=7\")");
        assert_eq!(eval(&mut session, "Q").expect_i64("test").unwrap(), 7);
        eval(&mut session, "CAS(\"L1:=[1/2,2]\")");
        assert_eq!(
            eval(&mut session, "L1(1)").display(Default::default()),
            "0.5"
        );
        eval(&mut session, "CAS(\"M1:=[[1,2],[3,4]]\")");
        assert_eq!(eval(&mut session, "M1(2,1)").expect_i64("test").unwrap(), 3);
        eval(&mut session, "HAngle:=1");
        assert_eq!(eval(&mut session, "SIN(90)").expect_i64("test").unwrap(), 1);
        assert_eq!(
            eval(&mut session, "CAS(\"sin(pi/2)\")")
                .expect_i64("test")
                .unwrap(),
            1
        );
        let image = session.memory_json();
        let mut restored = SessionContext::default();
        restored.restore_memory(&image).unwrap();
        assert_eq!(eval(&mut restored, "q+Q").expect_i64("test").unwrap(), 2007);
    }
    #[test]
    fn home_function_parameters_calls_and_persistence_use_one_object() {
        let mut session = SessionContext::default();
        eval(&mut session, "HVars(\"squareplus\"):='x^2+y'");
        eval(&mut session, "HVars(\"squareplus\",2):={\"x\",\"y\"}");
        assert_eq!(
            eval(&mut session, "HVars(\"squareplus\",2)").display(Default::default()),
            "{\"x\",\"y\"}"
        );
        assert_eq!(
            eval(&mut session, "squareplus(3,4)")
                .expect_i64("test")
                .unwrap(),
            13
        );
        let image = session.memory_json();
        let mut restored = SessionContext::default();
        restored.restore_memory(&image).unwrap();
        assert_eq!(
            eval(&mut restored, "squareplus(2,5)")
                .expect_i64("test")
                .unwrap(),
            9
        );
        assert_eq!(
            eval(&mut restored, "HVars(\"squareplus\",2)").display(Default::default()),
            "{\"x\",\"y\"}"
        );
        assert!(
            restored
                .evaluate_home(
                    "HVars(\"squareplus\",2):={\"x\",\"x\"}",
                    Box::new(HeadlessHost::new()),
                    Limits::default()
                )
                .result
                .is_err()
        );
    }
    #[test]
    fn home_measurements_preserve_cas_answer_and_follow_home_answer_after_restart() {
        let mut session = SessionContext::default();
        eval(&mut session, "CAS(\"3+4\")");
        let value = eval(&mut session, "20_cm+5_in");
        assert!(value.display(Default::default()).contains("_"));
        let value = eval(&mut session, "MKSA(Ans-32.7_cm)");
        let text = value.display(Default::default());
        let coefficient = text.split('_').next().unwrap().parse::<f64>().unwrap();
        assert!(coefficient.abs() < 1e-12, "{text}");
        assert_eq!(
            session.snapshot().cas.last_value,
            Some(ppl_cas::CasValue::Integer("7".into()))
        );
        let before = session.snapshot().globals["Ans"].clone();
        assert!(
            session
                .evaluate_home("2_m+1_s", Box::new(HeadlessHost::new()), Limits::default())
                .result
                .is_err()
        );
        assert_eq!(before, session.snapshot().globals["Ans"]);
        eval(&mut session, "2_m*3_m");
        let image = session.memory_json();
        let mut restored = SessionContext::default();
        restored.restore_memory(&image).unwrap();
        let value = eval(&mut restored, "MKSA(Ans-6_m^2)");
        let text = value.display(Default::default());
        assert!(
            text.split('_')
                .next()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                .abs()
                < 1e-12,
            "{text}"
        );
    }
    #[test]
    fn cas_factor_exact_and_symbolic_arithmetic_use_giac() {
        let mut context = SessionContext::default();
        let factor = eval(&mut context, "CAS(\"factor(x^2-1)\")");
        assert!(factor.display(Default::default()).contains("x-1"));
        let rational = eval(&mut context, "EXACT(0.5)");
        assert_eq!(rational.display(Default::default()), "1/2");
        assert_eq!(
            eval(&mut context, "EXACT(0.5)+EXACT(0.25)").display(Default::default()),
            "3/4"
        );
        assert_eq!(
            eval(&mut context, "EVAL('1+2')")
                .expect_i64("test")
                .unwrap(),
            3
        );
    }
    #[test]
    fn cas_namespaced_and_lowercase_commands_use_symbolic_arguments() {
        let mut context = SessionContext::default();
        let derivative = eval(&mut context, "CAS.diff(sin(x),x)");
        assert_eq!(derivative.display(Default::default()), "cos(x)");
        let solution = eval(&mut context, "solve(x^2-4=0,x)");
        assert!(solution.display(Default::default()).contains("2"));
    }
    #[test]
    fn exact_conditions_and_independent_cas_answer_survive_restart() {
        let mut context = SessionContext::default();
        assert_eq!(
            eval(
                &mut context,
                "IF EXACT(1)=1 THEN RETURN 7; ELSE RETURN 8; END"
            )
            .expect_i64("test")
            .unwrap(),
            7
        );
        assert_eq!(
            eval(&mut context, "NOT EXACT(0)")
                .expect_i64("test")
                .unwrap(),
            1
        );
        assert_eq!(
            eval(&mut context, "CAS(\"2+2\")")
                .expect_i64("test")
                .unwrap(),
            4
        );
        let image = context.memory_json();
        let mut restored = SessionContext::default();
        restored.restore_memory(&image).unwrap();
        assert_eq!(
            eval(&mut restored, "CAS(\"Ans*3\")")
                .expect_i64("test")
                .unwrap(),
            12
        );
    }
}

#[test]
#[cfg(feature = "giac-integration")]
fn actual_g1_cas_special_value_type_and_string_controls_are_preserved() {
    use ppl_vm::{HeadlessHost, Limits, SessionContext};
    let mut session = SessionContext::default();
    for (source, expected) in [
        ("TYPE(CAS(\"1/0\"))", "0"),
        ("STRING(CAS(\"1/0\"))", "+Inf"),
        ("TYPE(CAS(\"0/0\"))", "8"),
        ("STRING(CAS(\"0/0\"))", "undef"),
    ] {
        let result = session
            .evaluate_home(source, Box::new(HeadlessHost::new()), Limits::default())
            .result
            .unwrap();
        assert_eq!(
            result.display(ppl_vm::value::default_format()),
            expected,
            "{source}"
        );
    }
}

#[test]
fn eval_keeps_text_as_data_without_starting_an_algebra_helper() {
    let mut context = SessionContext::default();
    assert_eq!(
        eval(&mut context, "EVAL(\"a:=7\")")
            .expect_str("text")
            .unwrap()
            .as_ref(),
        "a:=7"
    );
    assert!(!context.snapshot().globals.contains_key("a"));
    let units = eval(&mut context, "ASC(EVAL(CHAR({55357,0,56832})))");
    assert_eq!(units.display(Default::default()), "{55357,0,56832}");
}

#[test]
fn assignment_in_a_quoted_cas_argument_is_an_error_value_without_side_effects() {
    let mut context = SessionContext::default();
    eval(&mut context, "A:=2");
    let before = context.snapshot();
    for source in [
        "CAS(A:=7)",
        "CAS(1+(A:=7))",
        "CAS(SIN(A:=7))",
        "CAS([A:=7,4])",
    ] {
        assert_eq!(
            eval(&mut context, source)
                .expect_str("error value")
                .unwrap()
                .as_ref(),
            "Error: Bad argument type"
        );
        assert_eq!(eval(&mut context, "A").expect_i64("unchanged").unwrap(), 2);
        assert_eq!(
            serde_json::to_value(context.snapshot().cas).unwrap(),
            serde_json::to_value(&before.cas).unwrap()
        );
    }
}
