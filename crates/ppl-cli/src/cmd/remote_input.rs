//! Syntax checks for device requests. Emulator builtin coverage is not a
//! statement about firmware support.
use ppl_syntax::ast::{Expr, ExprKind, Item, Stmt, StmtKind};

pub fn program_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.chars().count() > 16
        || !name.starts_with(|c: char| c.is_alphabetic())
        || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        return Err("program names must start with a letter and contain at most 16 letters, digits, or underscores".into());
    }
    Ok(())
}

fn no_assignment(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Assign(..) | ExprKind::Store(..) | ExprKind::Error => false,
        ExprKind::Unary(_, e) | ExprKind::Postfix(_, e) | ExprKind::Quoted(e) => no_assignment(e),
        ExprKind::Binary(_, a, b) | ExprKind::ComplexPair(a, b) => {
            no_assignment(a) && no_assignment(b)
        }
        ExprKind::Call(f, args) => no_assignment(f) && args.iter().all(no_assignment),
        ExprKind::Index(a, b) => no_assignment(a) && b.iter().all(no_assignment),
        ExprKind::List(xs) | ExprKind::Bracket(xs) => xs.iter().all(no_assignment),
        _ => true,
    }
}

pub fn expression(text: &str) -> Result<(), String> {
    if text.encode_utf16().count() > ppl_device::listener::MAX_REQUEST_CHARS - 35 {
        return Err("expression exceeds mailbox payload limit".into());
    }
    if text.trim().is_empty() {
        return Err("expected a nonempty PPL expression".into());
    }
    let (expr, diags) = ppl_syntax::parse_expression(text);
    if text.trim().is_empty() || diags.has_errors() || !no_assignment(&expr) {
        return Err(
            "expected one complete PPL expression without assignments or statement sequences"
                .into(),
        );
    }
    Ok(())
}

pub fn call(name: &str, args: Option<&str>) -> Result<String, String> {
    program_name(name)?;
    let text = format!("{name}({})", args.unwrap_or(""));
    expression(&text)?;
    Ok(text)
}

/// Conservative device profile: split long declarations before upload.
/// The G1 rejected our 17-variable statement and accepted groups of six.
pub fn declarations(text: &str) -> Result<(), String> {
    fn block(body: &[Stmt]) -> bool {
        body.iter().all(|stmt| match &stmt.kind {
            StmtKind::Local(vars) => vars.len() <= 8,
            StmtKind::Block(body)
            | StmtKind::For { body, .. }
            | StmtKind::While { body, .. }
            | StmtKind::Repeat { body, .. } => block(body),
            StmtKind::If {
                then_body,
                else_body,
                ..
            } => block(then_body) && else_body.as_ref().is_none_or(|b| block(b)),
            StmtKind::IfErr {
                body,
                handler,
                else_body,
            } => block(body) && block(handler) && else_body.as_ref().is_none_or(|b| block(b)),
            StmtKind::Case { arms, default } => {
                arms.iter().all(|(_, b)| block(b)) && default.as_ref().is_none_or(|b| block(b))
            }
            _ => true,
        })
    }
    let (ast, diags) = ppl_syntax::parse(text);
    if diags.has_errors() {
        return Err("PPL source has syntax errors".into());
    }
    if ast
        .items
        .iter()
        .any(|item| matches!(item,Item::Function(f) if !block(&f.body)))
    {
        return Err("split LOCAL declarations into groups of at most eight variables for the conservative device profile".into());
    }
    Ok(())
}

pub fn has_unverified_call(text: &str) -> bool {
    fn walk(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Call(f, args) => {
                let known = matches!(&f.kind,ExprKind::Ident(name) if ppl_compiler::builtins::lookup(name).is_some());
                !known || args.iter().any(walk)
            }
            ExprKind::Unary(_, e) | ExprKind::Postfix(_, e) | ExprKind::Quoted(e) => walk(e),
            ExprKind::Binary(_, a, b)
            | ExprKind::ComplexPair(a, b)
            | ExprKind::Assign(a, b)
            | ExprKind::Store(a, b) => walk(a) || walk(b),
            ExprKind::Index(a, b) => walk(a) || b.iter().any(walk),
            ExprKind::List(xs) | ExprKind::Bracket(xs) => xs.iter().any(walk),
            _ => false,
        }
    }
    walk(&ppl_syntax::parse_expression(text).0)
}

pub fn source(text: &str) -> Result<String, String> {
    declarations(text)?;
    let (ast, diags) = ppl_syntax::parse(text);
    if diags.has_errors() {
        return Err("PPL source has syntax errors; run `ppl check` for diagnostics (device-only builtins may be unsupported locally)".into());
    }
    let exports: Vec<_> = ast
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(f) if f.exported => Some(f),
            _ => None,
        })
        .collect();
    if exports.len() != 1
        || ast
            .items
            .iter()
            .any(|item| matches!(item, Item::Globals { exported: true, .. }))
    {
        return Err("exec-file requires one exported function and no exported globals; helpers may be local".into());
    }
    let name = &exports[0].name.name;
    ppl_device::validate_test_name(name).map_err(|e| e.to_string())?;
    if exports[0].is_key {
        return Err("exec-file entrypoint cannot be a key handler".into());
    }
    Ok(name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expressions_are_complete_and_do_not_assign() {
        for s in ["2+2", "SIN(30)", "TEST({1,2},\"a,b\")"] {
            assert!(expression(s).is_ok(), "{s}");
        }
        for s in ["", "2;3", "A:=2", "TEST(A:=2)", "2 ▶ A", "2 garbage"] {
            assert!(expression(s).is_err(), "{s}");
        }
    }
    #[test]
    fn arguments_preserve_nested_commas_and_reject_injection() {
        assert_eq!(
            call("TEST", Some("5,{1,2},\"a,b\"")).unwrap(),
            "TEST(5,{1,2},\"a,b\")"
        );
        for s in ["5); DELETE(X); TEST(", "1,", "A:=2"] {
            assert!(call("TEST", Some(s)).is_err());
        }
        for s in ["", "1TEST", "A);", "A/B", "ABCDEFGHIJKLMNOPQ"] {
            assert!(call(s, None).is_err());
        }
    }
    #[test]
    fn uploaded_source_requires_a_safe_single_export() {
        assert_eq!(
            source("EXPORT ZZT_TEST(A) BEGIN RETURN A+1; END;").unwrap(),
            "ZZT_TEST"
        );
        for s in [
            "EXPORT TEST() BEGIN END;",
            "EXPORT ZZT_A;",
            "EXPORT ZZT_A() BEGIN END; EXPORT ZZT_B() BEGIN END;",
            "EXPORT ZZT_A() BEGIN",
        ] {
            assert!(source(s).is_err(), "{s}");
        }
    }
}

#[cfg(test)]
mod device_profile_tests {
    #[test]
    fn long_declarations_are_caught_even_inside_nested_blocks() {
        assert!(
            super::declarations(
                "EXPORT ZZT_A() BEGIN IF 1 THEN LOCAL a,b,c,d,e,f,g,h,j; END; END;"
            )
            .is_err()
        );
        assert!(
            super::declarations("EXPORT ZZT_A() BEGIN LOCAL a,b,c,d,e,f; LOCAL g,h,j; END;")
                .is_ok()
        );
        let listener = ppl_device::listener::source("ZZT_TEST", "ABC123").unwrap();
        super::declarations(&listener).unwrap();
    }
    #[test]
    fn calls_require_separate_program_capability() {
        assert!(!super::has_unverified_call("2+SIN(30)"));
        assert!(super::has_unverified_call("TEST(10)"));
        assert!(super::has_unverified_call("SIN(TEST(10))"));
    }
}

#[cfg(test)]
mod stop_behavior_tests {
    use ppl_graphics::Grob;
    use ppl_vm::{Dialog, DialogResult, Host, RuntimeError};
    use std::sync::{Arc, Mutex};
    struct TapHost {
        key: Option<i64>,
        tap_on_sleep: bool,
        sleeps: Arc<Mutex<Vec<f64>>>,
    }
    impl Host for TapHost {
        fn print(&mut self, _: &str) {}
        fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
            Ok(self.key.take())
        }
        fn is_key_down(&mut self, _: i64) -> Result<bool, RuntimeError> {
            Ok(false)
        }
        fn wait_key(&mut self, _: Option<f64>) -> Result<Option<i64>, RuntimeError> {
            Ok(None)
        }
        fn sleep(&mut self, s: f64) -> Result<(), RuntimeError> {
            self.sleeps.lock().unwrap().push(s);
            if self.tap_on_sleep {
                self.key = Some(4);
            }
            Ok(())
        }
        fn touch(&mut self) -> Result<Option<ppl_vm::host::Touch>, RuntimeError> {
            Ok(None)
        }
        fn dialog(&mut self, _: &Dialog, _: &Grob) -> Result<DialogResult, RuntimeError> {
            Ok(DialogResult::Cancelled)
        }
        fn now_ms(&mut self) -> f64 {
            self.sleeps.lock().unwrap().iter().sum::<f64>() * 1000.0
        }
        fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
            (2000, 1, 1, 0, 0, 0.0)
        }
    }
    fn run(tap: bool) -> (ppl_vm::Value, Vec<f64>) {
        let (name, helper) = ppl_device::listener::stop_helper("ABC123").unwrap();
        let source = ppl_syntax::SourceFile::new(
            "stop-test",
            format!("{helper}\nEXPORT ZZT_STOP() BEGIN RETURN {name}(); END;"),
        );
        let (program, diagnostics) = ppl_compiler::compile_source(&source, &Default::default());
        assert!(!diagnostics.has_errors(), "{diagnostics:?}");
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let host = TapHost {
            key: None,
            tap_on_sleep: tap,
            sleeps: sleeps.clone(),
        };
        let mut machine = ppl_vm::Machine::new(
            std::rc::Rc::new(program.unwrap()),
            Box::new(host),
            Default::default(),
            Default::default(),
        );
        machine.run_init().unwrap();
        let value = machine.call_function("ZZT_STOP", vec![]).unwrap();
        let times = sleeps.lock().unwrap().clone();
        (value, times)
    }
    #[test]
    fn released_esc_tap_is_not_lost_during_idle_wait() {
        // The key is never held according to ISKEYDOWN. Only the queue can
        // catch this tap. This is a VM regression test, not hardware proof.
        let (value, sleeps) = run(true);
        assert_eq!(value.expect_i64("stop result").unwrap(), 1);
        assert_eq!(sleeps, vec![0.05]);
    }
    #[test]
    fn idle_wait_sleeps_instead_of_busy_spinning() {
        let (value, sleeps) = run(false);
        assert_eq!(value.expect_i64("stop result").unwrap(), 0);
        assert_eq!(sleeps.len(), 20);
        assert!(sleeps.iter().all(|s| *s == 0.05));
    }
}
