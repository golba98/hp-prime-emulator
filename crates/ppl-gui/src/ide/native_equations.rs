//! Solve-app mathematics on a cloned calculator revision, never UI-local
//! variables or a fresh unrelated algebra environment. No operation commits
//! CAS answer/memory or Home state. Plotting reuses the native graph evaluator.
use super::{
    native_apps::{Curve, GraphApp, GraphKind},
    native_model_plots::{PlotData, PlotStyle},
};
use ppl_cas::{CasValue, Options};
use ppl_syntax::{
    ast::{BinOp, ExprKind},
    lexer::{Tok, Token, lex},
};
use ppl_vm::SessionSnapshot;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
static NEXT_ALIAS: AtomicU64 = AtomicU64::new(1);
const MAX_SOURCE: usize = 64 * 1024;

pub fn solve(
    equation: &str,
    variable: &str,
    snapshot: &SessionSnapshot,
    cancelled: &AtomicBool,
) -> Result<CasValue, String> {
    check(cancelled)?;
    validate_variable(variable)?;
    let sides = equation_sides(equation)?;
    let home = super::native_graph_context::home_values(snapshot);
    reject_grobs(equation, snapshot)?;
    let alias = private_name("pplSolveUnknown", equation, snapshot);
    let names = BTreeMap::from([(variable.to_owned(), alias.clone())]);
    let lhs = rename(&sides.0, &names)?;
    let rhs = rename(&sides.1, &names)?;
    let mut cas = snapshot.cas.clone();
    cas.settings.angle = cas_angle(snapshot.effective_app_settings().angle);
    cas.settings.approximate = false;
    // Solve is a Native app computation, so Ans is the same Home answer used
    // by its native plot. The independent CAS answer in snapshot stays intact.
    cas.last_value = home
        .get("Ans")
        .cloned()
        .or(Some(CasValue::Integer("0".into())));
    let value = cas
        .evaluate_with_home(
            &format!("solve(({lhs})=({rhs}),{alias})"),
            &home,
            &Options::default(),
            cancelled,
        )
        .map_err(|error| error.to_string().replace(&alias, variable))?
        .value;
    check(cancelled)?;
    restore_names(value, &BTreeMap::from([(alias, variable.to_owned())]))
}

pub fn plot(
    equation: &str,
    variable: &str,
    snapshot: &SessionSnapshot,
    cancelled: Arc<AtomicBool>,
) -> Result<PlotData, String> {
    check(&cancelled)?;
    validate_variable(variable)?;
    let sides = equation_sides(equation)?;
    reject_grobs(equation, snapshot)?;
    let mut memory = snapshot.clone();
    memory.settings = snapshot.effective_app_settings();
    // Existing graph formals X/T/N/θ all denote the sampled parameter. Preserve
    // a distinct known calculator variable before mapping only the chosen
    // unknown to X; e.g. solving in x must not turn Home X=99 into another x.
    let identifiers = identifiers(equation)?;
    let mut replacements = BTreeMap::new();
    for formal in ["X", "T", "N", "θ"] {
        if formal != variable && identifiers.contains(formal) {
            let alias = private_name(
                "pplSolveKnown",
                &format!("{equation}{replacements:?}"),
                &memory,
            );
            let value = snapshot
                .globals
                .get(formal)
                .cloned()
                .unwrap_or(ppl_vm::PortableValue::Real(ppl_math::Real::ZERO));
            memory.globals.insert(alias.clone(), value);
            replacements.insert(formal.to_owned(), alias);
        }
    }
    replacements.insert(variable.to_owned(), "X".into());
    let lhs = rename(&sides.0, &replacements)?;
    let rhs = rename(&sides.1, &replacements)?;
    let mut app = GraphApp::new(GraphKind::Function);
    for (name, target) in [
        ("Xmin", &mut app.window.xmin),
        ("Xmax", &mut app.window.xmax),
        ("Ymin", &mut app.window.ymin),
        ("Ymax", &mut app.window.ymax),
    ] {
        if let Some(value) = snapshot.globals.get(name) {
            *target = value
                .to_value()
                .expect_real(name)
                .map_err(|e| e.to_string())?
                .to_f64();
        }
    }
    app.window.validate()?;
    app.curves = [lhs, rhs]
        .into_iter()
        .map(|expression| Curve {
            expression,
            second: String::new(),
            enabled: true,
            sequence: None,
        })
        .collect();
    let samples = app
        .sample(&memory, 640, cancelled.clone())
        .map_err(|mut error| {
            for (original, alias) in &replacements {
                if alias.starts_with("pplSolveKnown") {
                    error = error.replace(alias, original);
                }
            }
            error
        })?;
    check(&cancelled)?;
    let mut output = PlotData {
        curves: Vec::new(),
        labels: Vec::new(),
        styles: Vec::new(),
        x_label: variable.into(),
        y_label: "Value".into(),
    };
    let labels = [format!("Left: {}", sides.0), format!("Right: {}", sides.1)];
    for (curve, label) in samples.into_iter().zip(labels) {
        let mut segment: Vec<[f64; 2]> = Vec::new();
        for point in curve {
            check(&cancelled)?;
            if let Some(point) = point.filter(|point| point.iter().all(|n| n.is_finite())) {
                if segment.last().is_some_and(|old| {
                    (old[1] - point[1]).abs() >= (app.window.ymax - app.window.ymin) * 0.75
                }) {
                    push_segment(&mut output, &mut segment, &label);
                }
                segment.push(point);
            } else {
                push_segment(&mut output, &mut segment, &label);
            }
        }
        push_segment(&mut output, &mut segment, &label);
    }
    if output.curves.is_empty() {
        return Err("No real finite equation samples in the current plot window".into());
    }
    Ok(output)
}
fn push_segment(output: &mut PlotData, segment: &mut Vec<[f64; 2]>, label: &str) {
    if !segment.is_empty() {
        output.curves.push(std::mem::take(segment));
        output.labels.push(label.into());
        output.styles.push(PlotStyle::Line);
    }
}
fn cas_angle(angle: ppl_vm::AngleMode) -> ppl_cas::AngleMode {
    match angle {
        ppl_vm::AngleMode::Radians => ppl_cas::AngleMode::Radians,
        ppl_vm::AngleMode::Degrees => ppl_cas::AngleMode::Degrees,
        ppl_vm::AngleMode::Gradians => ppl_cas::AngleMode::Gradians,
    }
}
fn check(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err("Equation calculation interrupted".into())
    } else {
        Ok(())
    }
}
fn validate_variable(variable: &str) -> Result<(), String> {
    let tokens = tokens(variable)?;
    if tokens.len() != 2
        || tokens[0].span.start != 0
        || tokens[0].span.end as usize != variable.len()
        || !(matches!(&tokens[0].tok,Tok::Ident(name) if name==variable)
            || tokens[0].soft.as_deref() == Some(variable))
    {
        return Err("Enter one plain variable name".into());
    }
    Ok(())
}
fn tokens(source: &str) -> Result<Vec<Token>, String> {
    tokens_at(source, 0)
}
fn tokens_at(source: &str, quoted_depth: usize) -> Result<Vec<Token>, String> {
    if quoted_depth > 64 {
        return Err("Quoted equation nesting exceeds limit".into());
    }
    if source.len() > MAX_SOURCE {
        return Err("Equation exceeds the expression size limit".into());
    }
    let mut diagnostics = ppl_syntax::Diagnostics::default();
    let tokens = lex(source, &mut diagnostics);
    if diagnostics.has_errors() {
        return Err(diagnostics
            .errors()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("; "));
    }
    // Bound parser and quoted-expression recursion before processing user text.
    if tokens.len() > 512 {
        return Err("Equation exceeds the expression complexity limit".into());
    }
    let mut depth = 0;
    for token in &tokens {
        match &token.tok {
            Tok::Quoted(inner) => {
                tokens_at(inner, quoted_depth + 1)?;
            }
            Tok::LParen | Tok::LBrace | Tok::LBracket => {
                depth += 1;
                if depth > 64 {
                    return Err("Equation nesting exceeds limit".into());
                }
            }
            Tok::RParen | Tok::RBrace | Tok::RBracket => depth -= 1,
            _ => (),
        }
    }
    Ok(tokens)
}
fn equation_sides(source: &str) -> Result<(String, String), String> {
    let tokens = tokens(source)?;
    if tokens
        .iter()
        .any(|token| matches!(token.tok, Tok::Assign | Tok::Store | Tok::Semi))
    {
        return Err("Enter an equation or expression, not an assignment or program".into());
    }
    let (expr, diagnostics) = ppl_syntax::parse_expression(source);
    if diagnostics.has_errors() {
        return Err(diagnostics
            .errors()
            .map(|d| d.message.as_str())
            .collect::<Vec<_>>()
            .join("; "));
    }
    let slice = |expression: &ppl_syntax::ast::Expr| {
        source[expression.span.start as usize..expression.span.end as usize]
            .trim()
            .to_owned()
    };
    match expr.kind {
        ExprKind::Binary(BinOp::Eq, left, right) => {
            if matches!(left.kind, ExprKind::Binary(BinOp::Eq, _, _))
                || matches!(right.kind, ExprKind::Binary(BinOp::Eq, _, _))
            {
                return Err("Enter one equation".into());
            }
            Ok((slice(&left), slice(&right)))
        }
        ExprKind::Binary(BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, _, _) => {
            Err("Solve expects equality, or an expression equal to zero".into())
        }
        _ => Ok((source.trim().to_owned(), "0".into())),
    }
}
fn rename(source: &str, names: &BTreeMap<String, String>) -> Result<String, String> {
    let tokens = tokens(source)?;
    let mut output = String::with_capacity(source.len());
    let mut end = 0;
    for token in tokens {
        let start = token.span.start as usize;
        let next = token.span.end as usize;
        output.push_str(&source[end..start]);
        if let Tok::Quoted(inner) = &token.tok {
            output.push('\'');
            output.push_str(&rename(inner, names)?);
            output.push('\'');
        } else if let Some(name) = match &token.tok {
            Tok::Ident(name) => Some(name.as_str()),
            _ => token.soft.as_deref(),
        } {
            output.push_str(
                names
                    .get(name)
                    .map(String::as_str)
                    .unwrap_or(&source[start..next]),
            );
        } else {
            output.push_str(&source[start..next]);
        }
        end = next;
    }
    output.push_str(&source[end..]);
    Ok(output)
}
fn identifiers(source: &str) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    for token in tokens(source)? {
        match token.tok {
            Tok::Ident(name) => {
                names.insert(name);
            }
            Tok::Quoted(inner) => names.extend(identifiers(&inner)?),
            _ => {
                if let Some(name) = token.soft {
                    names.insert(name);
                }
            }
        }
    }
    Ok(names)
}
fn private_name(prefix: &str, equation: &str, snapshot: &SessionSnapshot) -> String {
    let mut candidate = format!("{prefix}{}", NEXT_ALIAS.fetch_add(1, Ordering::Relaxed));
    while equation.contains(&candidate)
        || snapshot.globals.keys().any(|s| s.contains(&candidate))
        || snapshot.globals.values().any(|value| {
            value
                .to_value()
                .display(snapshot.settings.format())
                .contains(&candidate)
        })
        || snapshot
            .cas
            .bindings
            .iter()
            .any(|(name, source)| name.contains(&candidate) || source.contains(&candidate))
        || snapshot.programs.values().any(|s| s.contains(&candidate))
    {
        candidate.push('_');
    }
    candidate
}
fn reject_grobs(equation: &str, snapshot: &SessionSnapshot) -> Result<(), String> {
    if identifiers(equation)?.iter().any(|name| {
        matches!(
            snapshot.globals.get(name),
            Some(ppl_vm::PortableValue::Grob(_))
        )
    }) {
        return Err("Graphics handles are not equation values".into());
    }
    Ok(())
}
fn restore_names(value: CasValue, names: &BTreeMap<String, String>) -> Result<CasValue, String> {
    Ok(match value {
        // The collision-free private name appears nowhere in user text or
        // context. Replace only that generated name/prefix, even inside Giac
        // output syntax that is intentionally not PPL-parser input.
        CasValue::Symbolic(mut source) => {
            for (name, original) in names {
                source = source.replace(name, original);
            }
            CasValue::Symbolic(source)
        }
        CasValue::String(mut source) => {
            for (name, original) in names {
                source = source.replace(name, original);
            }
            CasValue::String(source)
        }
        CasValue::List(values) => CasValue::List(
            values
                .into_iter()
                .map(|value| restore_names(value, names))
                .collect::<Result<_, _>>()?,
        ),
        CasValue::Matrix { rows, cols, values } => CasValue::Matrix {
            rows,
            cols,
            values: values
                .into_iter()
                .map(|value| restore_names(value, names))
                .collect::<Result<_, _>>()?,
        },
        CasValue::Complex { real, imaginary } => CasValue::Complex {
            real: Box::new(restore_names(*real, names)?),
            imaginary: Box::new(restore_names(*imaginary, names)?),
        },
        value => value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> SessionSnapshot {
        let mut memory = ppl_vm::SessionContext::default();
        memory
            .set_variable(
                "A",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(4)),
            )
            .unwrap();
        memory
            .set_variable(
                "X",
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(99)),
            )
            .unwrap();
        memory.snapshot()
    }
    #[test]
    fn parsing_and_token_renaming_preserve_names_strings_and_comments() {
        assert_eq!(
            equation_sides("(X^2=A)").unwrap(),
            ("X^2".into(), "A".into())
        );
        assert_eq!(
            equation_sides("X^2-A").unwrap(),
            ("X^2-A".into(), "0".into())
        );
        let renamed = rename(
            "x+xx+X+\"x\"+'x+1' // x\n+1E-3",
            &BTreeMap::from([("x".into(), "private".into())]),
        )
        .unwrap();
        assert_eq!(renamed, "private+xx+X+\"x\"+'private+1' // x\n+1E-3");
        for invalid in ["", "X:=1", "X=1;2", "X<1", "X=Y=Z", "X+"] {
            assert!(equation_sides(invalid).is_err(), "{invalid}");
        }
        for invalid in ["", "2x", "x y", "x+1", "\"x\""] {
            assert!(validate_variable(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn plot_real_equation_sides_use_home_memory_and_ignore_unknown_prior_value() {
        let memory = snapshot();
        let data = plot("X^2=A", "X", &memory, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(data.x_label, "X");
        assert!(data.labels.iter().any(|s| s == "Left: X^2"));
        assert!(data.labels.iter().any(|s| s == "Right: A"));
        for (points, label) in data.curves.iter().zip(&data.labels) {
            for point in points {
                let expected = if label.starts_with("Left") {
                    point[0] * point[0]
                } else {
                    4.
                };
                assert!((point[1] - expected).abs() < 1e-8, "{point:?}");
            }
        }
        assert_eq!(
            memory.globals["X"],
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(99))
        );
    }
    #[test]
    fn known_graph_formals_are_not_accidentally_replaced_by_unknown_parameter() {
        let mut memory = snapshot();
        memory.globals.insert(
            "T".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(7)),
        );
        let data = plot("x+X+T=A", "x", &memory, Arc::new(AtomicBool::new(false))).unwrap();
        for (points, label) in data.curves.iter().zip(&data.labels) {
            if label.starts_with("Left") {
                for point in points {
                    assert!((point[1] - (point[0] + 106.)).abs() < 1e-8, "{point:?}");
                }
            }
        }
        assert!(data.labels.iter().all(|label| !label.contains("pplSolve")));
    }
    #[test]
    fn plot_segments_do_not_join_across_poles_or_missing_real_values() {
        let data = plot("1/X=0", "X", &snapshot(), Arc::new(AtomicBool::new(false))).unwrap();
        let left = data
            .curves
            .iter()
            .zip(&data.labels)
            .filter(|(_, label)| label.starts_with("Left"))
            .map(|(curve, _)| curve)
            .collect::<Vec<_>>();
        assert!(left.len() >= 2);
        for curve in left {
            assert!(
                !curve
                    .windows(2)
                    .any(|pair| pair[0][0] < 0. && pair[1][0] > 0.)
            );
        }
        let sqrt = plot(
            "SQRT(X)=0",
            "X",
            &snapshot(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        for (curve, label) in sqrt.curves.iter().zip(&sqrt.labels) {
            if label.starts_with("Left") {
                assert!(curve.iter().all(|point| point[0] >= 0.));
            }
        }
    }
    #[test]
    fn cancelled_operations_exit_without_touching_snapshot() {
        let memory = snapshot();
        let before = serde_json::to_value(&memory.cas).unwrap();
        assert!(
            solve("X^2=A", "X", &memory, &AtomicBool::new(true))
                .unwrap_err()
                .contains("interrupted")
        );
        assert!(
            plot("X^2=A", "X", &memory, Arc::new(AtomicBool::new(true)))
                .unwrap_err()
                .contains("interrupted")
        );
        assert_eq!(serde_json::to_value(&memory.cas).unwrap(), before);
    }
    #[test]
    fn generated_symbolic_names_are_restored_without_a_ppl_output_syntax_limit() {
        let value = CasValue::Symbolic(format!(
            "rootof([1,0,-1],pplSolveUnknown42)+{}",
            vec!["pplSolveUnknown42"; 600].join("+")
        ));
        let restored = restore_names(
            value,
            &BTreeMap::from([("pplSolveUnknown42".into(), "X".into())]),
        )
        .unwrap();
        assert!(!restored.expression().contains("pplSolveUnknown42"));
        assert!(restored.expression().contains("rootof([1,0,-1],X)"));
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn real_solve_shares_home_memory_ignores_bound_unknown_and_preserves_cas_answer() {
        let mut memory = snapshot();
        memory.cas.bindings.insert("x".into(), "99".into());
        memory.cas.last_value = Some(CasValue::Integer("1234".into()));
        let before = serde_json::to_value(&memory.cas).unwrap();
        for unknown in ["X", "x"] {
            assert_eq!(
                solve(
                    &format!("{unknown}^2=A"),
                    unknown,
                    &memory,
                    &AtomicBool::new(false)
                )
                .unwrap(),
                CasValue::List(vec![
                    CasValue::Integer("-2".into()),
                    CasValue::Integer("2".into())
                ])
            );
        }
        memory.globals.insert(
            "Ans".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(4)),
        );
        assert_eq!(
            solve("X^2=Ans", "X", &memory, &AtomicBool::new(false)).unwrap(),
            CasValue::List(vec![
                CasValue::Integer("-2".into()),
                CasValue::Integer("2".into())
            ])
        );
        assert_eq!(serde_json::to_value(&memory.cas).unwrap(), before);
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn real_solve_and_plot_use_known_cas_binding_and_effective_app_angle() {
        let mut memory = snapshot();
        memory.cas.bindings.insert("q".into(), "4".into());
        assert_eq!(
            solve("X^2=q", "X", &memory, &AtomicBool::new(false)).unwrap(),
            CasValue::List(vec![
                CasValue::Integer("-2".into()),
                CasValue::Integer("2".into())
            ])
        );
        let data = plot("X^2=q", "X", &memory, Arc::new(AtomicBool::new(false))).unwrap();
        for (curve, label) in data.curves.iter().zip(&data.labels) {
            if label.starts_with("Right") {
                assert!(curve.iter().all(|point| point[1] == 4.));
            }
        }
        memory
            .app_settings
            .entry(memory.active_app.clone())
            .or_default()
            .angle = 2;
        let roots = solve("SIN(X)=0", "X", &memory, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            roots,
            CasValue::List(vec![
                CasValue::Integer("0".into()),
                CasValue::Integer("180".into())
            ])
        );
        assert_eq!(memory.settings.angle, ppl_vm::AngleMode::Radians);
    }
    #[test]
    fn cancellation_reaches_actual_plot_vm_inside_running_program() {
        let mut memory = snapshot();
        memory.programs.insert(
            "ZZT_SOLVE_CANCEL".into(),
            "EXPORT ZZT_SOLVE_CANCEL(X) BEGIN WHILE 1 DO END; END;".into(),
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            flag.store(true, Ordering::Release);
        });
        let error = plot("ZZT_SOLVE_CANCEL(X)=0", "X", &memory, cancelled).unwrap_err();
        thread.join().unwrap();
        assert!(
            error.to_ascii_lowercase().contains("interrupt")
                || error.to_ascii_lowercase().contains("cancel"),
            "{error}"
        );
    }
}
