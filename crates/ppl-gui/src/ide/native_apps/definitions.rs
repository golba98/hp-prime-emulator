//! Bind graph-row function references before numeric sampling and Symbolic Eval.
//! Expansion follows parsed expression nodes, leaving strings and quoted data alone.
use super::{GraphApp, GraphKind, check_cancel};
use ppl_syntax::ast::{Expr, ExprKind as E};
use std::sync::atomic::AtomicBool;

const LIMIT: usize = 65_536;
struct Expansion<'a> {
    app: &'a GraphApp,
    stack: Vec<(usize, bool)>,
    calls: usize,
    depth: usize,
    nodes: usize,
    cancel: &'a AtomicBool,
}
impl GraphApp {
    pub fn expanded_definition(
        &self,
        index: usize,
        second: bool,
        cancel: &AtomicBool,
    ) -> Result<String, String> {
        Expansion {
            app: self,
            stack: vec![],
            calls: 0,
            depth: 0,
            nodes: 0,
            cancel,
        }
        .definition(index, second, None)
    }
    pub(super) fn expanded_definitions(&self, cancel: &AtomicBool) -> Result<Self, String> {
        let mut prepared = self.clone();
        // Sequence self-references have their own recurrence engine and initial terms.
        if self.kind == GraphKind::Sequence {
            return Ok(prepared);
        }
        for (index, curve) in prepared.curves.iter_mut().enumerate() {
            if !curve.enabled || curve.expression.trim().is_empty() {
                continue;
            }
            curve.expression = self.expanded_definition(index, false, cancel)?;
            if self.kind == GraphKind::Parametric && !curve.second.trim().is_empty() {
                curve.second = self.expanded_definition(index, true, cancel)?;
            }
        }
        Ok(prepared)
    }
}
impl Expansion<'_> {
    fn reference(&self, name: &str) -> Option<(usize, bool)> {
        let (prefix, second) = match self.app.kind {
            GraphKind::Function => ("F", false),
            GraphKind::Polar => ("R", false),
            GraphKind::Parametric if name.starts_with('Y') => ("Y", true),
            GraphKind::Parametric => ("X", false),
            _ => return None,
        };
        let number = name.strip_prefix(prefix)?.parse::<usize>().ok()?;
        // Accept only the actual app names, rather than aliases such as F01.
        (number <= 9 && name == format!("{prefix}{number}"))
            .then_some((if number == 0 { 9 } else { number - 1 }, second))
    }
    fn variable(&self) -> &'static str {
        match self.app.kind {
            GraphKind::Parametric => "T",
            GraphKind::Polar => "θ",
            _ => "X",
        }
    }
    fn definition(
        &mut self,
        index: usize,
        second: bool,
        argument: Option<&str>,
    ) -> Result<String, String> {
        check_cancel(self.cancel)?;
        self.calls += 1;
        if self.calls > 256 || self.stack.len() >= 32 {
            return Err("Graph definition expansion limit exceeded".into());
        }
        if self.stack.contains(&(index, second)) {
            return Err("Circular graph definition".into());
        }
        let curve = self
            .app
            .curves
            .get(index)
            .ok_or("Referenced graph definition is empty")?;
        let source = if second {
            &curve.second
        } else {
            &curve.expression
        };
        if source.trim().is_empty() {
            return if argument.is_none() {
                Ok(String::new())
            } else {
                Err("Referenced graph definition is empty".into())
            };
        }
        if source.len() > LIMIT {
            return Err("Graph definition is too large".into());
        }
        let (expr, diagnostics) = ppl_syntax::parse_expression(source);
        if diagnostics.has_errors() {
            return Err(format!(
                "Invalid graph definition: {}",
                diagnostics
                    .errors()
                    .next()
                    .map_or("syntax error", |d| d.message.as_str())
            ));
        }
        self.stack.push((index, second));
        let result = self.render(source, &expr, argument);
        self.stack.pop();
        result
    }
    fn render(
        &mut self,
        source: &str,
        expr: &Expr,
        argument: Option<&str>,
    ) -> Result<String, String> {
        if self.depth >= 128 || self.nodes >= 16_384 {
            return Err("Graph definition expansion limit exceeded".into());
        }
        self.depth += 1;
        self.nodes += 1;
        let result = self.render_inner(source, expr, argument);
        self.depth -= 1;
        result
    }
    fn render_inner(
        &mut self,
        source: &str,
        expr: &Expr,
        argument: Option<&str>,
    ) -> Result<String, String> {
        check_cancel(self.cancel)?;
        if let E::Ident(name) = &expr.kind
            && name == self.variable()
            && let Some(arg) = argument
        {
            return Ok(format!("({arg})"));
        }
        if let E::Call(callee, args) = &expr.kind
            && let E::Ident(name) = &callee.kind
            && let Some((index, second)) = self.reference(name)
        {
            if args.len() != 1 {
                return Err("A graph function needs exactly one argument".into());
            }
            let arg = self.render(source, &args[0], argument)?;
            return self
                .definition(index, second, Some(&arg))
                .map(|s| format!("({s})"));
        }
        let children: Vec<&Expr> = match &expr.kind {
            E::Unary(_, a) | E::Postfix(_, a) => vec![a],
            E::Binary(_, a, b) | E::ComplexPair(a, b) | E::Assign(a, b) | E::Store(a, b) => {
                vec![a, b]
            }
            E::List(v) | E::Bracket(v) => v.iter().collect(),
            // A function's name is not the app's independent parameter.
            E::Call(_, v) => v.iter().collect(),
            E::Index(a, v) => std::iter::once(a.as_ref()).chain(v.iter()).collect(),
            _ => vec![],
        };
        let start = expr.span.start as usize;
        let end = expr.span.end as usize;
        let mut out = String::new();
        let mut at = start;
        for child in children {
            let first = child.span.start as usize;
            let last = child.span.end as usize;
            out.push_str(source.get(at..first).ok_or("Invalid graph source span")?);
            out.push_str(&self.render(source, child, argument)?);
            if out.len() > LIMIT {
                return Err("Expanded graph definition is too large".into());
            }
            at = last;
        }
        out.push_str(source.get(at..end).ok_or("Invalid graph source span")?);
        if out.len() > LIMIT {
            return Err("Expanded graph definition is too large".into());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native_apps::Curve;
    use std::sync::{Arc, atomic::AtomicBool};
    fn curve(source: &str, second: &str, enabled: bool) -> Curve {
        Curve {
            expression: source.into(),
            second: second.into(),
            enabled,
            sequence: None,
            literal: None,
        }
    }
    fn snapshot() -> ppl_vm::SessionSnapshot {
        ppl_vm::SessionContext::default().snapshot()
    }
    #[test]
    fn nested_arguments_keep_parameter_binding_and_home_values() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves = vec![curve("X^2+3", "", false), curve("F1(F1(X))", "", true)];
        assert_eq!(
            app.table(&snapshot(), 2).unwrap(),
            vec![vec![Some(0.), Some(12.)], vec![Some(1.), Some(19.)]]
        );
        app.window.xmin = 0.;
        app.window.xmax = 1.;
        assert_eq!(
            app.sample(&snapshot(), 2, Arc::new(AtomicBool::new(false)))
                .unwrap(),
            vec![vec![Some([0., 12.]), Some([1., 19.])]]
        );
        assert_eq!(app.curves[1].expression, "F1(F1(X))");
    }
    #[test]
    fn parametric_components_and_polar_rows_use_their_own_parameter() {
        let mut app = GraphApp::new(GraphKind::Parametric);
        app.curves = vec![
            curve("T", "T^2", false),
            curve("X1(T+1)+Y1(T)", "Y1(2*T)", true),
        ];
        assert_eq!(
            app.table(&snapshot(), 3).unwrap(),
            vec![
                vec![Some(0.), Some(1.), Some(0.)],
                vec![Some(1.), Some(3.), Some(4.)],
                vec![Some(2.), Some(7.), Some(16.)]
            ]
        );
        let mut app = GraphApp::new(GraphKind::Polar);
        app.curves = vec![curve("θ+1", "", false), curve("R1(2*θ)", "", true)];
        assert_eq!(
            app.table(&snapshot(), 2).unwrap(),
            vec![vec![Some(0.), Some(1.)], vec![Some(1.), Some(3.)]]
        );
    }
    #[test]
    fn ordinary_grouping_and_unary_precedence_survive_preparation() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves[0] = curve("2*(X+1)", "", true);
        assert_eq!(
            app.table(&snapshot(), 3).unwrap(),
            vec![
                vec![Some(0.), Some(2.)],
                vec![Some(1.), Some(4.)],
                vec![Some(2.), Some(6.)]
            ]
        );
        app.curves[0].expression = "-(X+1)^2".into();
        assert_eq!(
            app.table(&snapshot(), 2).unwrap(),
            vec![vec![Some(0.), Some(-1.)], vec![Some(1.), Some(-4.)]]
        );
    }
    #[test]
    fn the_tenth_definition_is_zero_and_never_ten_or_a_leading_zero_alias() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves = (0..10).map(|_| curve("", "", false)).collect();
        app.curves[9] = curve("X+10", "", false);
        app.curves[0] = curve("2*F0(X)", "", true);
        assert_eq!(
            app.table(&snapshot(), 2).unwrap(),
            vec![vec![Some(0.), Some(20.)], vec![Some(1.), Some(22.)]]
        );
        for name in ["F10", "F00"] {
            app.curves[0].expression = format!("{name}(X)");
            assert!(
                app.table(&snapshot(), 1)
                    .unwrap_err()
                    .contains("unknown function")
            );
        }
    }
    #[test]
    fn cycles_empty_references_bad_arity_and_cancellation_are_explicit() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves = vec![curve("F2(X)", "", true), curve("F1(X)", "", true)];
        assert!(app.table(&snapshot(), 1).unwrap_err().contains("Circular"));
        app.curves[0].expression = "F3(X)".into();
        assert!(app.table(&snapshot(), 1).unwrap_err().contains("empty"));
        app.curves[0].expression = "F3(X,2)".into();
        assert!(
            app.table(&snapshot(), 1)
                .unwrap_err()
                .contains("one argument")
        );
        assert!(
            app.expanded_definition(0, false, &AtomicBool::new(true))
                .unwrap_err()
                .contains("cancel")
        );
    }
    #[test]
    fn literals_are_preserved_and_expansion_bombs_have_a_host_bound() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves = vec![
            curve("X+1", "", false),
            curve("SIZE(\"F1(X)\")+F1(X)", "", true),
        ];
        assert_eq!(
            app.table(&snapshot(), 1).unwrap(),
            vec![vec![Some(0.), Some(6.)]]
        );
        app.curves[1].expression = "F1(X)".into();
        for _ in 0..200 {
            app.curves[1].expression = format!("F1({})", app.curves[1].expression);
        }
        assert!(app.table(&snapshot(), 1).unwrap_err().contains("limit"));
    }
}
