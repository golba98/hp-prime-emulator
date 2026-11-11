//! Home evaluates one expression using a stable union of installed programs
//! and transient IDE source snapshots, through the same native VM lifecycle.
use ppl_math::Real;
use ppl_vm::{
    HeadlessHost, Limits, ModuleSource, PortableValue, SessionContext, SourceIdentity, Value,
};
use std::collections::BTreeMap;
fn catalog(name: &str, source: &str) -> BTreeMap<String, ModuleSource> {
    BTreeMap::from([(
        name.into(),
        ModuleSource {
            identity: SourceIdentity::new(name, source),
            source: source.into(),
        },
    )])
}
fn evaluate(
    session: &mut SessionContext,
    text: &str,
    sources: &BTreeMap<String, ModuleSource>,
) -> Value {
    session
        .evaluate_home_with_sources(
            text,
            sources,
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result
        .unwrap()
}
#[test]
fn nested_calls_share_one_home_expression_and_promote_ans() {
    let mut session = SessionContext::default();
    let sources = catalog("open-double", "EXPORT DOUBLE(x) BEGIN RETURN 2*x; END;");
    assert_eq!(
        evaluate(&mut session, "2+DOUBLE(DOUBLE(1))", &sources)
            .expect_i64("result")
            .unwrap(),
        6
    );
    assert_eq!(
        session.snapshot().globals["Ans"],
        PortableValue::Real(Real::from_i64(6))
    );
    assert_eq!(
        evaluate(&mut session, "DOUBLE(Ans)+1", &sources)
            .expect_i64("result")
            .unwrap(),
        13
    );
    assert!(
        session.snapshot().programs.is_empty(),
        "transient IDE sources must not silently install"
    );
}
#[test]
fn identical_installed_and_open_sources_initialize_once_across_tab_close() {
    let mut session = SessionContext::default();
    let source = "LOCAL count:=10; EXPORT COUNTER(x) BEGIN count:=count+1; RETURN count+x; END;";
    session.store_program("Stored", source.into()).unwrap();
    let sources = catalog("/project/Stored.hpprgm", source);
    assert_eq!(
        evaluate(&mut session, "COUNTER(COUNTER(0))", &sources)
            .expect_i64("result")
            .unwrap(),
        23
    );
    assert_eq!(
        evaluate(&mut session, "COUNTER(0)", &BTreeMap::new())
            .expect_i64("result")
            .unwrap(),
        13
    );
    assert_eq!(
        evaluate(&mut session, "COUNTER(0)", &sources)
            .expect_i64("result")
            .unwrap(),
        14
    );
}
#[test]
fn dependencies_read_calculator_variables_even_if_home_only_mentions_function() {
    let mut session = SessionContext::default();
    session
        .set_variable("named", PortableValue::Real(Real::from_i64(9)))
        .unwrap();
    let sources = catalog("reader", "EXPORT READNAMED(x) BEGIN RETURN named+x; END;");
    assert_eq!(
        evaluate(&mut session, "READNAMED(READNAMED(1))", &sources)
            .expect_i64("result")
            .unwrap(),
        19
    );
}
#[test]
fn source_snapshot_does_not_hide_genuinely_conflicting_exports() {
    let mut session = SessionContext::default();
    session
        .store_program("Stored", "EXPORT DOUBLE(x) BEGIN RETURN 2*x; END;".into())
        .unwrap();
    let sources = catalog("different-open", "EXPORT DOUBLE(x) BEGIN RETURN 3*x; END;");
    let result = session
        .evaluate_home_with_sources(
            "DOUBLE(1)",
            &sources,
            Box::new(HeadlessHost::new()),
            Limits::default(),
        )
        .result;
    assert!(result.unwrap_err().to_string().contains("ambiguous"));
}
#[cfg(feature = "giac-integration")]
#[test]
fn linked_dependencies_resolve_cas_bindings_without_creating_home_shadows() {
    let mut session = SessionContext::default();
    session.cas.bindings.insert("q".into(), "7".into());
    let sources = catalog("reader", "EXPORT READCAS(x) BEGIN RETURN q+x; END;");
    assert_eq!(
        evaluate(&mut session, "READCAS(READCAS(1))", &sources)
            .expect_i64("result")
            .unwrap(),
        15
    );
    assert!(!session.snapshot().globals.contains_key("q"));
    assert_eq!(session.cas.bindings["q"], "7");
    let sources = catalog("writer", "EXPORT BUMPQ() BEGIN q:=q+1; RETURN q; END;");
    assert_eq!(
        evaluate(&mut session, "BUMPQ()+1", &sources)
            .expect_i64("result")
            .unwrap(),
        9
    );
    assert_eq!(session.cas.bindings["q"], "8");
    assert!(!session.snapshot().globals.contains_key("q"));
}
