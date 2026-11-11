//! Shared read-only Home/CAS value resolution before native graph compilation.
//! Only referenced CAS custom variables are evaluated, in one supervised batch.
//! Existing Home values, calculator defaults and independent parameters retain
//! their namespaces. No session answer or memory is committed.
use ppl_cas::{CasValue, Options};
use ppl_syntax::lexer::{Tok, lex};
use ppl_vm::{PortableValue, SessionSnapshot};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};

pub fn home_values(snapshot: &SessionSnapshot) -> BTreeMap<String, CasValue> {
    snapshot
        .globals
        .iter()
        .filter_map(|(name, value)| {
            ppl_vm::cas::to_cas(&value.to_value()).map(|value| (name.clone(), value))
        })
        .collect()
}

pub fn resolve(
    snapshot: &SessionSnapshot,
    expressions: &[&str],
    independent: &[&str],
    cancelled: &AtomicBool,
) -> Result<SessionSnapshot, String> {
    check(cancelled)?;
    let mut memory = snapshot.clone();
    if snapshot.cas.bindings.is_empty() {
        return Ok(memory);
    }
    let references = reachable_references(snapshot, expressions, independent, cancelled)?;
    for name in references.writes.difference(&references.reads) {
        if snapshot.cas.bindings.contains_key(name)
            && !snapshot.globals.contains_key(name)
            && !ppl_compiler::is_system_variable(name)
        {
            memory
                .globals
                .insert(name.clone(), PortableValue::Real(ppl_math::Real::ZERO));
        }
    }
    let names = references
        .reads
        .into_iter()
        .filter(|name| {
            snapshot.cas.bindings.contains_key(name)
                && !snapshot.globals.contains_key(name)
                && !ppl_compiler::is_system_variable(name)
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Ok(memory);
    }
    let mut cas = snapshot.cas.clone();
    // Resolving a stored CAS object uses that environment's angle/precision,
    // just as reading it from Home does. The graph's own operations use the
    // caller's effective app settings in memory.settings, independently.
    let values = cas
        .evaluate_with_home(
            &format!("[\"pplGraphValues1\",{}]", names.join(",")),
            &home_values(snapshot),
            &Options::default(),
            cancelled,
        )
        .map_err(|error| error.to_string())?
        .value;
    let CasValue::List(values) = values else {
        return Err("CAS returned an invalid graph variable list".into());
    };
    // A leading string keeps the outer reply a list when every requested
    // value is itself an equal-length list (otherwise Giac represents a matrix).
    if values.len() != names.len() + 1
        || values.first() != Some(&CasValue::String("pplGraphValues1".into()))
    {
        return Err("CAS returned an incomplete graph variable list".into());
    }
    for (name, value) in names.into_iter().zip(values.into_iter().skip(1)) {
        check(cancelled)?;
        memory.globals.insert(name, graph_value(value));
    }
    check(cancelled)?;
    Ok(memory)
}
// Sampling is numerical Home work: convert collection cells once too, rather
// than invoking external CAS arithmetic for each list/matrix element per pixel.
fn graph_value(value: CasValue) -> PortableValue {
    match value {
        CasValue::List(values) => {
            PortableValue::List(values.into_iter().map(graph_value).collect())
        }
        CasValue::Matrix { rows, cols, values } => PortableValue::Matrix {
            rows,
            cols,
            vector: false,
            data: values.into_iter().map(graph_value).collect(),
        },
        value => PortableValue::from(&ppl_vm::cas::from_cas_home(value)),
    }
}
/// Names declared for graph compilation. An unreachable CAS binding needs a
/// slot declaration, but must never be evaluated merely because another
/// function in the same installed module mentions it. Program-owned exported
/// variables retain their actual module owner instead of a duplicate CAS slot.
pub fn namespace(snapshot: &SessionSnapshot) -> BTreeSet<String> {
    let mut names = snapshot.globals.keys().cloned().collect::<BTreeSet<_>>();
    if snapshot.cas.bindings.is_empty() {
        return names;
    }
    let mut program_globals = BTreeSet::new();
    for source in snapshot.programs.values() {
        let (program, diagnostics) = ppl_syntax::parse(source);
        if diagnostics.has_errors() {
            continue;
        }
        for item in program.items {
            if let ppl_syntax::ast::Item::Globals {
                exported: true,
                vars,
                ..
            } = item
            {
                program_globals.extend(vars.into_iter().map(|var| var.name.name));
            }
        }
    }
    names.extend(
        snapshot
            .cas
            .bindings
            .keys()
            .filter(|name| !program_globals.contains(*name))
            .cloned(),
    );
    names
}
fn reachable_references(
    snapshot: &SessionSnapshot,
    expressions: &[&str],
    independent: &[&str],
    cancelled: &AtomicBool,
) -> Result<ppl_vm::linker::CalculatorDependencies, String> {
    check(cancelled)?;
    let mut prefix = "__GRAPH_CONTEXT".to_owned();
    while expressions.iter().any(|source| source.contains(&prefix))
        || snapshot
            .programs
            .values()
            .any(|source| source.contains(&prefix))
        || snapshot
            .globals
            .keys()
            .chain(snapshot.cas.bindings.keys())
            .any(|name| name.contains(&prefix))
    {
        prefix.push('_');
    }
    let entry = format!("{prefix}_READ");
    let mut catalog = snapshot
        .programs
        .iter()
        .map(|(name, source)| {
            (
                name.clone(),
                ppl_vm::ModuleSource {
                    identity: ppl_vm::SourceIdentity::new(name, source),
                    source: source.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut expressions = expressions
        .iter()
        .map(|source| (*source).to_owned())
        .collect::<Vec<_>>();
    // The sequence cache is a runtime-local U1..U10 function. Use collision-free
    // analysis-only signatures, never execute a fake sequence or evaluate code.
    if independent == ["N"] {
        let mut stubs = String::new();
        for number in 1..=10 {
            let name = format!("U{number}");
            let replacement = format!("{prefix}_SEQ{number}");
            for expression in &mut expressions {
                *expression = replace_call(expression, &name, &replacement)?;
            }
            stubs.push_str(&format!("EXPORT {replacement}(N) BEGIN RETURN 0; END;\n"));
        }
        let name = format!("{prefix}_SEQUENCE");
        catalog.insert(
            name.clone(),
            ppl_vm::ModuleSource {
                identity: ppl_vm::SourceIdentity::new(&name, &stubs),
                source: stubs,
            },
        );
    }
    let body = if expressions.is_empty() {
        "0".into()
    } else {
        format!(
            "{{{}}}",
            expressions
                .iter()
                .map(|expression| format!("({expression})"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let source = format!(
        "EXPORT {entry}({}) BEGIN RETURN {body}; END;",
        independent.join(",")
    );
    let root = format!("{prefix}_ROOT");
    catalog.insert(
        root.clone(),
        ppl_vm::ModuleSource {
            identity: ppl_vm::SourceIdentity::new(&root, &source),
            source,
        },
    );
    let names = snapshot
        .globals
        .keys()
        .chain(snapshot.cas.bindings.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let image =
        ppl_vm::linker::link_home_sources(&catalog, &root, snapshot.settings.int_base, &names)
            .map_err(|error| error.to_string())?;
    check(cancelled)?;
    image
        .calculator_dependencies(&entry)
        .map_err(|error| error.to_string())
}
fn replace_call(source: &str, name: &str, replacement: &str) -> Result<String, String> {
    let mut diagnostics = ppl_syntax::Diagnostics::default();
    let tokens = lex(source, &mut diagnostics);
    if diagnostics.has_errors() {
        return Err("Invalid sequence expression".into());
    }
    let mut result = String::new();
    let mut end = 0;
    for (index, token) in tokens.iter().enumerate() {
        let start = token.span.start as usize;
        let next = token.span.end as usize;
        result.push_str(&source[end..start]);
        if matches!(&token.tok,Tok::Ident(identifier)if identifier==name)
            && tokens
                .get(index + 1)
                .is_some_and(|token| token.tok == Tok::LParen)
        {
            result.push_str(replacement);
        } else {
            result.push_str(&source[start..next]);
        }
        end = next;
    }
    result.push_str(&source[end..]);
    Ok(result)
}
fn check(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("Graph variable resolution interrupted".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "giac-integration")]
    use std::sync::Arc;
    #[test]
    fn unrelated_bindings_strings_and_independent_names_do_not_require_a_helper() {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot
            .cas
            .bindings
            .insert("x".into(), "while(1){1;}".into());
        snapshot
            .cas
            .bindings
            .insert("q".into(), "while(1){1;}".into());
        let memory = resolve(
            &snapshot,
            &["x+1", "\"q\""],
            &["x"],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!memory.globals.contains_key("x"));
        assert!(!memory.globals.contains_key("q"));
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn batch_resolves_actual_cas_values_and_home_dependencies_without_mutating_snapshot() {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot
            .globals
            .insert("T".into(), PortableValue::Real(ppl_math::Real::from_i64(7)));
        snapshot.cas.bindings.insert("q".into(), "4".into());
        snapshot.cas.bindings.insert("r".into(), "T+1".into());
        snapshot
            .cas
            .bindings
            .insert("items".into(), "[1,2,3]".into());
        snapshot.cas.last_value = Some(CasValue::Integer("1234".into()));
        let before = serde_json::to_value(&snapshot.cas).unwrap();
        let memory = resolve(
            &snapshot,
            &["X+q+r+T+items(1)"],
            &["X"],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            memory.globals["q"],
            PortableValue::Real(ppl_math::Real::from_i64(4))
        );
        assert_eq!(
            memory.globals["r"],
            PortableValue::Real(ppl_math::Real::from_i64(8))
        );
        assert_eq!(
            memory.globals["items"],
            PortableValue::List(
                (1..=3)
                    .map(|n| PortableValue::Real(ppl_math::Real::from_i64(n)))
                    .collect()
            )
        );
        assert!(!snapshot.globals.contains_key("q"));
        assert_eq!(serde_json::to_value(&snapshot.cas).unwrap(), before);
        assert_eq!(serde_json::to_value(&memory.cas).unwrap(), before);
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn home_custom_values_system_defaults_and_axis_unknowns_keep_priority() {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot
            .globals
            .insert("q".into(), PortableValue::Real(ppl_math::Real::from_i64(5)));
        for name in ["q", "A", "X", "x", "y"] {
            snapshot.cas.bindings.insert(name.into(), "99".into());
        }
        let memory = resolve(
            &snapshot,
            &["X+Y+x+y+A+q"],
            &["X", "Y", "x", "y"],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            memory.globals["q"],
            PortableValue::Real(ppl_math::Real::from_i64(5))
        );
        for name in ["A", "X", "x", "y"] {
            assert!(!memory.globals.contains_key(name));
        }
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn cancellation_stops_a_real_running_giac_binding_without_changing_context() {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot
            .cas
            .bindings
            .insert("q".into(), "while(1){1;}".into());
        let before = serde_json::to_value(&snapshot.cas).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            flag.store(true, Ordering::Release);
        });
        let error = resolve(&snapshot, &["X+q"], &["X"], &cancel).unwrap_err();
        thread.join().unwrap();
        assert!(error.to_ascii_lowercase().contains("interrupt"), "{error}");
        assert_eq!(serde_json::to_value(&snapshot.cas).unwrap(), before);
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn stored_cas_values_use_cas_settings_independently_of_graph_home_angle() {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot.cas.settings.angle = ppl_cas::AngleMode::Degrees;
        snapshot.cas.bindings.insert("q".into(), "SIN(30)".into());
        let memory = resolve(&snapshot, &["X+q"], &["X"], &AtomicBool::new(false)).unwrap();
        assert_eq!(
            memory.globals["q"],
            PortableValue::Real(ppl_math::Real::parse("0.5").unwrap())
        );
        assert_eq!(memory.settings.angle, ppl_vm::AngleMode::Radians);
        assert_eq!(memory.cas.settings.angle, ppl_cas::AngleMode::Degrees);
    }
}

#[cfg(all(test, feature = "giac-integration"))]
mod reachable_tests {
    use super::*;
    use crate::ide::native_apps::{GraphApp, GraphKind};
    use std::sync::Arc;
    fn memory() -> SessionSnapshot {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        snapshot
            .globals
            .insert("Q".into(), PortableValue::Real(ppl_math::Real::from_i64(2)));
        snapshot.cas.bindings.insert("q".into(), "4".into());
        snapshot.cas.bindings.insert("r".into(), "1".into());
        snapshot
            .cas
            .bindings
            .insert("stalled".into(), "while(1){1;}".into());
        snapshot
    }
    fn table(snapshot: &SessionSnapshot, expression: &str) -> f64 {
        let mut app = GraphApp::new(GraphKind::Function);
        app.numeric_start = 2.;
        app.curves[0].expression = expression.into();
        app.table(snapshot, 1).unwrap()[0][1].unwrap()
    }
    #[test]
    fn reachable_calls_include_private_and_transitive_installed_functions_only() {
        let mut snapshot = memory();
        snapshot.programs.insert("LIB".into(),"HELPER(X) BEGIN RETURN DEEP(X)+r; END; EXPORT LIB(X) BEGIN RETURN HELPER(X)+Q; END; EXPORT UNUSED() BEGIN RETURN stalled; END;".into());
        snapshot.programs.insert(
            "NEXT".into(),
            "EXPORT DEEP(X) BEGIN RETURN BOTTOM(X); END;".into(),
        );
        snapshot.programs.insert(
            "BOTTOM".into(),
            "EXPORT BOTTOM(X) BEGIN RETURN X*2+q; END;".into(),
        );
        snapshot.programs.insert(
            "UNRELATED".into(),
            "EXPORT DONT_CALL() BEGIN RETURN stalled; END;".into(),
        );
        assert_eq!(table(&snapshot, "LIB(X)"), 11.);
        assert!(!snapshot.globals.contains_key("q"));
        let mut app = GraphApp::new(GraphKind::Function);
        app.window.xmin = 1.;
        app.window.xmax = 3.;
        app.curves[0].expression = "LIB(X)".into();
        assert_eq!(
            app.sample(&snapshot, 3, Arc::new(AtomicBool::new(false)))
                .unwrap()[0],
            vec![Some([1., 9.]), Some([2., 11.]), Some([3., 13.])]
        );
    }
    #[test]
    fn parameters_and_function_locals_shadow_stalled_cas_values() {
        let mut snapshot = memory();
        for name in ["q", "r"] {
            snapshot
                .cas
                .bindings
                .insert(name.into(), "while(1){1;}".into());
        }
        snapshot.programs.insert("LIB".into(),"EXPORT LIB(q) BEGIN LOCAL r:=3; RETURN q+r+Q; END; EXPORT UNUSED() BEGIN RETURN stalled; END;".into());
        assert_eq!(table(&snapshot, "LIB(X)"), 7.);
    }
    #[test]
    fn file_private_variables_and_initializer_dependencies_use_correct_owners() {
        let mut snapshot = memory();
        snapshot
            .cas
            .bindings
            .insert("q".into(), "while(1){1;}".into());
        snapshot.programs.insert(
            "LIB".into(),
            "LOCAL q:=r+Q; EXPORT LIB(X) BEGIN RETURN X+q; END;".into(),
        );
        assert_eq!(table(&snapshot, "LIB(X)"), 5.);
        snapshot.programs.insert(
            "LIB".into(),
            "EXPORT q:=r+Q; EXPORT LIB(X) BEGIN RETURN X+q; END;".into(),
        );
        assert_eq!(table(&snapshot, "LIB(X)"), 5.);
        snapshot.programs.insert(
            "LIB".into(),
            "EXPORT offset:=r+Q; EXPORT LIB(X) BEGIN RETURN X+offset; END;".into(),
        );
        assert_eq!(table(&snapshot, "LIB(X)+offset"), 8.);
    }
    #[test]
    fn writes_do_not_read_prior_cas_values_and_dead_return_tail_is_not_resolved() {
        let mut snapshot = memory();
        snapshot
            .cas
            .bindings
            .insert("q".into(), "while(1){1;}".into());
        snapshot.programs.insert(
            "LIB".into(),
            "EXPORT LIB(X) BEGIN q:=3; RETURN X+Q; RETURN stalled; END;".into(),
        );
        assert_eq!(table(&snapshot, "LIB(X)"), 4.);
    }
    #[test]
    fn production_shared_graph_acceptance_reads_home_t_and_cas_q_without_an_ide_tab() {
        let mut snapshot = memory();
        snapshot
            .globals
            .insert("T".into(), PortableValue::Real(ppl_math::Real::from_i64(7)));
        snapshot.programs.insert(
            "SHARED_GRAPH".into(),
            "EXPORT SHARED_GRAPH(X) BEGIN RETURN X+T+q; END;".into(),
        );
        assert_eq!(table(&snapshot, "SHARED_GRAPH(X)"), 13.);
        let mut app = GraphApp::new(GraphKind::Function);
        app.window.xmin = 0.;
        app.window.xmax = 2.;
        app.curves[0].expression = "SHARED_GRAPH(X)".into();
        assert_eq!(
            app.sample(&snapshot, 3, Arc::new(AtomicBool::new(false)))
                .unwrap()[0],
            vec![Some([0., 11.]), Some([1., 12.]), Some([2., 13.])]
        );
    }
    #[test]
    fn resolver_matches_existing_native_selected_module_initializer_lifecycle() {
        let mut snapshot = memory();
        snapshot.programs.insert(
            "LIB".into(),
            "EXPORT LIB(X) BEGIN RETURN X+Q; END; EXPORT UNUSED(X) BEGIN RETURN EXTRA(X); END;"
                .into(),
        );
        snapshot.programs.insert(
            "EXTRA".into(),
            "LOCAL initialized:=Q:=q; EXPORT EXTRA(X) BEGIN RETURN X+initialized; END;".into(),
        );
        // The Native linker selects EXTRA and initialize_dependencies executes
        // its file initializer. q is really read there, while stalled remains
        // unrelated and must never be evaluated.
        assert_eq!(table(&snapshot, "LIB(X)"), 6.);
        assert_eq!(
            snapshot.globals["Q"],
            PortableValue::Real(ppl_math::Real::from_i64(2))
        );
    }
}
