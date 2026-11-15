//! App access from a running VM. The waiting execution retains
//! its lease; the UI only commits a source/epoch-fenced local app result.
use super::*;
use ppl_vm::{ErrorKind, RuntimeError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
pub(super) struct AppVariableTask {
    message: crate::host::AppVariableRequest,
    source: serde_json::Value,
    rx: mpsc::Receiver<Result<crate::ide::native_application_access::AccessResult, RuntimeError>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for AppVariableTask {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
fn interrupted() -> RuntimeError {
    RuntimeError::kind(ErrorKind::Interrupted)
}
pub(super) fn qualified_cas_operand(expression: &ppl_syntax::ast::Expr) -> Result<bool, String> {
    use ppl_syntax::ast::ExprKind;
    match &expression.kind {
        ExprKind::Ident(name) => {
            if let Some((app, _)) = name.split_once('.')
                && (app.eq_ignore_ascii_case("Spreadsheet") || app.eq_ignore_ascii_case("Function"))
            {
                if app != "Spreadsheet" && app != "Function" {
                    return Err(
                        "Application names are case sensitive: use Spreadsheet or Function".into(),
                    );
                }
                return Ok(true);
            }
            Ok(crate::ide::native_application_access::function_index(name).is_some())
        }
        ExprKind::Unary(_, e) | ExprKind::Postfix(_, e) | ExprKind::Quoted(e) => {
            qualified_cas_operand(e)
        }
        ExprKind::Binary(_, a, b)
        | ExprKind::ComplexPair(a, b)
        | ExprKind::Assign(a, b)
        | ExprKind::Store(a, b) => Ok(qualified_cas_operand(a)? | qualified_cas_operand(b)?),
        ExprKind::Call(f, args) | ExprKind::Index(f, args) => {
            let mut found = qualified_cas_operand(f)?;
            for e in args {
                found |= qualified_cas_operand(e)?;
            }
            Ok(found)
        }
        ExprKind::List(items) | ExprKind::Bracket(items) => {
            let mut found = false;
            for e in items {
                found |= qualified_cas_operand(e)?;
            }
            Ok(found)
        }
        _ => Ok(false),
    }
}
pub(super) fn cas_assignment(expression: &ppl_syntax::ast::Expr) -> bool {
    use ppl_syntax::ast::ExprKind::*;
    match &expression.kind {
        Assign(..) | Store(..) => true,
        Unary(_, e) | Postfix(_, e) | Quoted(e) => cas_assignment(e),
        Binary(_, a, b) | ComplexPair(a, b) => cas_assignment(a) || cas_assignment(b),
        Call(f, args) | Index(f, args) => cas_assignment(f) || args.iter().any(cas_assignment),
        List(items) | Bracket(items) => items.iter().any(cas_assignment),
        _ => false,
    }
}
pub(super) fn evaluate_qualified_cas(
    source: &str,
    snapshot: &ppl_vm::SessionSnapshot,
    shared: Arc<crate::host::GuiSharedState>,
) -> Result<(ppl_cas::CasSession, ppl_cas::CasEvaluation), ppl_cas::CasError> {
    let failure = |e: RuntimeError| ppl_cas::CasError::Protocol(e.to_string());
    let mut context = ppl_vm::SessionContext::new(snapshot.settings.clone(), 0x434153415050);
    context.active_app = snapshot.active_app.clone();
    context.app_settings = snapshot.app_settings.clone();
    context.app_numeric_settings = snapshot.app_numeric_settings.clone();
    context.app_files = snapshot.app_files.clone();
    context.cas = snapshot.cas.clone();
    for (name, value) in &snapshot.globals {
        if name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit() {
            continue;
        }
        context.set_variable(name, value.clone()).map_err(failure)?;
    }
    for (name, program) in &snapshot.programs {
        context
            .store_program(name, program.clone())
            .map_err(failure)?;
    }
    let run = context.evaluate_home(
        &format!("CAS({source})"),
        Box::new(crate::host::GuiHost::new(shared)),
        ppl_vm::Limits {
            max_steps: Some(1_000_000),
            max_time: Some(Duration::from_secs(15)),
            ..Default::default()
        },
    );
    let result = run.result.map_err(failure)?;
    let value = ppl_vm::cas::to_cas(&result).ok_or_else(|| {
        ppl_cas::CasError::Protocol("Worksheet value cannot be represented as CAS data".into())
    })?;
    let after = context.snapshot();
    let home_updates = after
        .globals
        .iter()
        .filter(|(name, v)| name.as_str() != "Ans" && snapshot.globals.get(*name) != Some(*v))
        .filter_map(|(name, v)| ppl_vm::cas::to_cas(&v.to_value()).map(|v| (name.clone(), v)))
        .collect();
    Ok((
        after.cas,
        ppl_cas::CasEvaluation {
            value,
            home_updates,
        },
    ))
}
impl IdeDesktopApp {
    fn app_access_source(&self) -> serde_json::Value {
        serde_json::json!({"sheet": &self.native_tools.memory.models.spreadsheet, "function": &self.native_tools.memory.graphs["Function"].curves})
    }
    pub(super) fn reject_qualified_cas(
        &mut self,
        source: String,
        message: String,
        catalog: bool,
        ctx: &egui::Context,
    ) {
        if !catalog {
            self.state
                .native
                .add_history(source, message.clone(), true, None);
        }
        self.native_tools.error = Some(message);
        self.native_tools.dirty();
        ctx.request_repaint();
    }
    pub(super) fn poll_app_variables(&mut self, ctx: &egui::Context) {
        let epoch = self
            .controller
            .shared
            .operation_epoch
            .load(Ordering::Acquire);
        if self.app_variable_task.as_ref().is_some_and(|t| {
            t.message.epoch != epoch || t.message.cancelled.load(Ordering::Acquire)
        }) {
            let task = self.app_variable_task.take().unwrap();
            let _ = task.message.reply.send(Err(interrupted()));
        }
        if let Some(task) = &self.app_variable_task {
            let received = match task.rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(interrupted())),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = received {
                let task = self.app_variable_task.take().unwrap();
                let current = self.app_access_source();
                let result = if task.message.epoch
                    != self
                        .controller
                        .shared
                        .operation_epoch
                        .load(Ordering::Acquire)
                    || task.message.cancelled.load(Ordering::Acquire)
                {
                    Err(interrupted())
                } else if task.source != current {
                    Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        "Application definitions changed during application access",
                    ))
                } else {
                    result.map(|result| {
                        if let Some(sheet) = result.sheet {
                            let models = &mut self.native_tools.memory.models;
                            models.spreadsheet = sheet;
                            let cell = models.selected_cell.clone();
                            models.select_cell(&cell);
                            self.native_tools.spreadsheet_changed(false);
                        }
                        if let Some(curves) = result.function {
                            // Numeric/plot projection changes need not cancel a
                            // definition access; keep the current derived fields.
                            self.native_tools
                                .memory
                                .graphs
                                .get_mut("Function")
                                .unwrap()
                                .curves = curves;
                            self.native_tools
                                .function_definitions_changed(&task.message.context);
                        }
                        result.value
                    })
                };
                let _ = task.message.reply.send(result);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(10));
            }
        }
        if self.app_variable_task.is_none() {
            loop {
                let message = self
                    .controller
                    .shared
                    .app_variables
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .pop_front();
                let Some(message) = message else { break };
                if message.epoch != epoch || message.cancelled.load(Ordering::Acquire) {
                    let _ = message.reply.send(Err(interrupted()));
                    continue;
                }
                let sheet = self.native_tools.memory.models.spreadsheet.clone();
                let function = self.native_tools.memory.graphs["Function"].clone();
                let source = self.app_access_source();
                let request = message.request.clone();
                let context = message.context.clone();
                let cancel = message.cancelled.clone();
                let flag = cancel.clone();
                let (tx, rx) = mpsc::channel();
                let repaint = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::ide::native_application_access::access(
                        sheet, function, &request, &context, flag,
                    ));
                    repaint.request_repaint();
                });
                self.app_variable_task = Some(AppVariableTask {
                    message,
                    source,
                    rx,
                    cancel,
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(10));
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::{layout::WorkspaceLayout, native_tools::Request};
    use ppl_vm::{PortableValue, Settings};
    fn fixture() -> (IdeDesktopApp, egui::Context) {
        let mut app = IdeDesktopApp::default();
        app.state.layout = WorkspaceLayout::Calculator;
        app.state.calc_input_focus = true;
        app.state.native.set_view(NativeView::Home);
        app.display.last_display_rect =
            egui::Rect::from_min_size(egui::pos2(40., 40.), egui::vec2(640., 480.));
        (app, egui::Context::default())
    }
    fn frame(app: &mut IdeDesktopApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run(
            egui::RawInput {
                focused: true,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280., 900.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.poll_execution();
                app.poll_native_tools(ctx);
                app.forward_calc_keys(ctx);
                app.render_native_tools(ctx);
            },
        );
    }
    fn wait(app: &mut IdeDesktopApp, ctx: &egui::Context, done: impl Fn(&IdeDesktopApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            frame(app, ctx, vec![]);
            if done(app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "application access stalled: {:?}",
                app.state.native.last_outcome
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn start(app: &mut IdeDesktopApp, ctx: &egui::Context, source: &str) {
        app.native_tools.close();
        app.state.native.set_view(NativeView::Home);
        app.state.native.edit(|e| e.set_text(source));
        app.submit_native_home(ctx);
    }
    fn home(app: &mut IdeDesktopApp, ctx: &egui::Context, source: &str) -> PortableValue {
        let count = app.state.native.history.len();
        start(app, ctx, source);
        wait(app, ctx, |a| {
            a.state.native.history.len() > count && !a.controller.worker_alive()
        });
        assert!(
            !app.state.native.history.back().unwrap().error,
            "{}",
            app.state.native.history.back().unwrap().result
        );
        app.controller.session_snapshot().last_value.unwrap()
    }
    fn seed_function(app: &mut IdeDesktopApp) {
        let graph = app.native_tools.memory.graphs.get_mut("Function").unwrap();
        graph.curves[0].expression = "X^2+1".into();
        graph.curves.push(crate::ide::native_apps::Curve {
            expression: "F1(X)+2".into(),
            second: String::new(),
            enabled: true,
            sequence: None,
            literal: None,
        });
        graph.curves.push(crate::ide::native_apps::Curve {
            expression: "SIN(X)".into(),
            second: String::new(),
            enabled: true,
            sequence: None,
            literal: None,
        });
    }
    #[test]
    fn function_definition_get_set_retains_types_syntax_checkboxes_and_live_names() {
        let (mut app, ctx) = fixture();
        // Older profiles had a hidden checked flag on an empty first row.
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .enabled = true;
        let source = r#"EXPORT FDEF() BEGIN STARTAPP("Function"); A:=7; X:=99; F1:=" X ^ 2 + 1 "; F2:='F1(X)+2'; RETURN {TYPE(F1),STRING(F1),F1(),F1(2),TYPE(F2),STRING(F2),F2(2),A,X}; END;"#;
        app.handle_native_tool_request(Request::SaveProgram("FDEF".into(), source.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "FDEF()").display(&Settings::default()),
            "{8,\"X^2+1\",X^2+1,5,8,\"F1(X)+2\",7,7,99}"
        );
        let graph = &app.native_tools.memory.graphs["Function"];
        assert!(!graph.curves[0].enabled && !graph.curves[1].enabled);
        assert_eq!(graph.curves[0].expression, "X^2+1");
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .enabled = true;
        assert_eq!(
            home(&mut app, &ctx, "F1:=\"2+3\"").to_value().type_code(),
            8
        );
        assert!(app.native_tools.memory.graphs["Function"].curves[0].enabled);
        assert_eq!(
            home(&mut app, &ctx, "{TYPE(F1),STRING(F1),F1(2)}").display(&Settings::default()),
            "{8,\"2+3\",5}"
        );
        assert_eq!(
            home(&mut app, &ctx, "F1:='42'").display(&Settings::default()),
            "42"
        );
        assert_eq!(
            home(&mut app, &ctx, "{TYPE(F1),STRING(F1),F1(2)}").display(&Settings::default()),
            "{0,\"42\",42}"
        );
        home(&mut app, &ctx, "F1:=\"A+1\"");
        home(&mut app, &ctx, "A:=8");
        assert_eq!(
            home(&mut app, &ctx, "{TYPE(F1),STRING(F1),F1(2),X}").display(&Settings::default()),
            "{8,\"A+1\",9,99}"
        );
        let snapshot = app.controller.session_snapshot();
        assert!(!snapshot.globals.contains_key("F1") && !snapshot.programs.contains_key("F1"));
    }
    #[test]
    fn function_definition_invalid_and_bad_types_are_atomic_and_clear_returns_text() {
        let (mut app, ctx) = fixture();
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .enabled = true;
        let source = r#"EXPORT FATOMIC() BEGIN LOCAL a,b,c,d; F1:="X^2+1"; IFERR F1:="X+("; THEN a:=STRING(F1); END; IFERR F1:=(1,2); THEN b:=F1(2); END; IFERR F1:="\"hello\""; THEN c:=STRING(F1); END; d:=(F1:=""); IFERR a:=F1; THEN a:="empty"; END; RETURN {a,b,c,d,TYPE(d)}; END;"#;
        app.handle_native_tool_request(Request::SaveProgram("FATOMIC".into(), source.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "FATOMIC()").display(&Settings::default()),
            "{\"empty\",5,\"X^2+1\",\"\",2}"
        );
        assert!(
            app.native_tools.memory.graphs["Function"].curves[0]
                .expression
                .is_empty()
        );
        assert!(!app.native_tools.memory.graphs["Function"].curves[0].enabled);
    }
    #[test]
    fn function_word_literal_metadata_survives_settings_and_profile_roundtrip() {
        let (mut app, ctx) = fixture();
        home(&mut app, &ctx, "F1:=#FF:-8h");
        home(&mut app, &ctx, "Bits:=32");
        home(&mut app, &ctx, "Base:=0");
        let before = home(&mut app, &ctx, "{F1,F1(2),F1({1,2})}");
        let PortableValue::List(items) = before else {
            panic!("typed list")
        };
        for value in &items[..2] {
            let PortableValue::Integer(word) = value else {
                panic!("typed word")
            };
            assert_eq!(word.width_code(), -8);
            assert_eq!(word.numeric_value(), 255);
            assert_eq!(word.base, ppl_math::IntBase::Hex);
        }
        let image = serde_json::to_value(&app.native_tools.memory).unwrap();
        app.native_tools.memory = serde_json::from_value(image).unwrap();
        app.native_tools.memory.validate().unwrap();
        assert_eq!(
            home(&mut app, &ctx, "{F1,F1(2),F1({1,2})}"),
            PortableValue::List(items)
        );
        // A later edit cannot continue returning the old typed word.
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .expression = "X+1".into();
        assert_eq!(
            home(&mut app, &ctx, "F1(2)").display(&Settings::default()),
            "3"
        );
    }
    #[test]
    fn function_get_set_from_foreign_app_keeps_caller_namespace() {
        let (mut app, ctx) = fixture();
        let source = r#"EXPORT FFOREIGN() BEGIN STARTAPP("Spreadsheet"); Spreadsheet.F1:=9; Function.F1:='2*X+F1'; RETURN {TYPE(Function.F1),STRING(Function.F1),Function.F1(3),Spreadsheet.F1}; END;"#;
        app.handle_native_tool_request(
            Request::SaveProgram("FFOREIGN".into(), source.into()),
            &ctx,
        );
        assert_eq!(
            home(&mut app, &ctx, "FFOREIGN()").display(&Settings::default()),
            "{8,\"2*X+F1\",15,9}"
        );
        assert_eq!(app.controller.session_snapshot().active_app, "Spreadsheet");
    }
    #[test]
    fn pending_function_set_discards_cancelled_or_changed_sources_and_keeps_derived_cache() {
        for mode in 0..3 {
            let (mut app, ctx) = fixture();
            home(&mut app, &ctx, "F1:=\"X+1\"");
            start(&mut app, &ctx, "F1:=42");
            let deadline = Instant::now() + Duration::from_secs(3);
            while app
                .controller
                .shared
                .app_variables
                .lock()
                .unwrap()
                .is_empty()
            {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            app.poll_app_variables(&ctx);
            assert!(app.app_variable_task.is_some());
            match mode {
                0 => app.controller.stop(),
                1 => {
                    app.native_tools
                        .memory
                        .graphs
                        .get_mut("Function")
                        .unwrap()
                        .curves[0]
                        .expression = "X+9".into()
                }
                _ => {
                    let graph = app.native_tools.memory.graphs.get_mut("Function").unwrap();
                    graph.numeric_start = 37.;
                    graph.numeric_step = 0.25;
                }
            }
            wait(&mut app, &ctx, |a| {
                !a.controller.worker_alive() && a.app_variable_task.is_none()
            });
            let graph = &app.native_tools.memory.graphs["Function"];
            assert_eq!(
                graph.curves[0].expression,
                match mode {
                    0 => "X+1",
                    1 => "X+9",
                    _ => "42",
                }
            );
            if mode == 2 {
                assert_eq!((graph.numeric_start, graph.numeric_step), (37., 0.25));
            }
        }
    }
    #[test]
    fn a_called_program_cannot_assign_another_function_definition() {
        let (mut app, ctx) = fixture();
        home(&mut app, &ctx, "F2:=\"X+2\"");
        let source = "EXPORT ALTER(X) BEGIN F2:=99; RETURN X; END;";
        app.handle_native_tool_request(Request::SaveProgram("ALTER".into(), source.into()), &ctx);
        home(&mut app, &ctx, "F1:=\"ALTER(X)\"");
        let count = app.state.native.history.len();
        start(&mut app, &ctx, "F1(3)");
        wait(&mut app, &ctx, |a| {
            a.state.native.history.len() > count && !a.controller.worker_alive()
        });
        assert!(app.state.native.history.back().unwrap().error);
        assert_eq!(
            app.native_tools.memory.graphs["Function"].curves[1].expression,
            "X+2"
        );
        assert_eq!(
            home(&mut app, &ctx, "F2(3)").display(&Settings::default()),
            "5"
        );
    }
    #[test]
    fn string_complex_and_list_bodies_stay_symbolic_and_evaluate_typed_values() {
        let (mut app, ctx) = fixture();
        home(&mut app, &ctx, "F1:=\"{1,2}\"");
        assert_eq!(
            home(&mut app, &ctx, "{TYPE(F1),STRING(F1),F1(2)}").display(&Settings::default()),
            "{8,\"{1,2}\",{1,2}}"
        );
        home(&mut app, &ctx, "F1:=\"(1,2)\"");
        assert_eq!(
            home(&mut app, &ctx, "TYPE(F1)").display(&Settings::default()),
            "8"
        );
        let value = home(&mut app, &ctx, "F1(2)");
        let PortableValue::Complex(value) = value else {
            panic!("typed complex")
        };
        assert_eq!(
            (value.re, value.im),
            (ppl_math::Real::ONE, ppl_math::Real::from_i64(2))
        );
    }
    #[test]
    fn current_function_scalar_source_and_home_aliases_keep_the_selected_arithmetic_model() {
        let (mut app, ctx) = fixture();
        app.controller
            .mutate_memory(|m| {
                m.settings.numeric_version = ppl_math::NumericVersion::Current15751;
                Ok(())
            })
            .unwrap();
        home(&mut app, &ctx, "F0:=\"1234567890123456789\"");
        assert_eq!(
            home(&mut app, &ctx, "F0(2)-1234567890123456788").display(&Settings::default()),
            "1"
        );
        home(&mut app, &ctx, "HAngle:=1");
        home(&mut app, &ctx, "AAngle:=1");
        home(&mut app, &ctx, "HComplex:=1");
        home(&mut app, &ctx, "AComplex:=2");
        let source =
            "EXPORT TRUEHOME(X) BEGIN RETURN {HAngle,AAngle,HComplex,AComplex,SIN(X)}; END;";
        app.handle_native_tool_request(
            Request::SaveProgram("TRUEHOME".into(), source.into()),
            &ctx,
        );
        home(&mut app, &ctx, "F1:=\"TRUEHOME(X)\"");
        assert_eq!(
            home(&mut app, &ctx, "F1(90)").display(&Settings::default()),
            "{1,1,1,2,0.893996663601}"
        );
        assert_eq!(
            app.controller.session_snapshot().settings.angle,
            ppl_vm::AngleMode::Degrees
        );
    }
    #[test]
    fn current_function_complex_modes_match_all_six_original_controls() {
        let (mut app, ctx) = fixture();
        app.controller
            .mutate_memory(|m| {
                m.settings.numeric_version = ppl_math::NumericVersion::Current15751;
                Ok(())
            })
            .unwrap();
        home(&mut app, &ctx, "F1:=\"√(X)\"");
        let source = "EXPORT ROOTMODE() BEGIN LOCAL a,ok:=1; IFERR a:=F1(-1); THEN ok:=0; END; RETURN {ok,TYPE(a),TYPE(F1((-1,0)))}; END;";
        app.handle_native_tool_request(
            Request::SaveProgram("ROOTMODE".into(), source.into()),
            &ctx,
        );
        for h in 0..=1 {
            for a in 0..=2 {
                home(&mut app, &ctx, &format!("HComplex:={h}"));
                home(&mut app, &ctx, &format!("AComplex:={a}"));
                let expected = if a == 2 || a == 0 && h == 1 {
                    "{1,3,3}"
                } else {
                    "{0,0,3}"
                };
                assert_eq!(
                    home(&mut app, &ctx, "ROOTMODE()").display(&Settings::default()),
                    expected,
                    "H{h} A{a}"
                );
            }
        }
    }
    #[test]
    fn real_function_home_ppl_expr_and_foreign_caller_names_match_genuine_controls() {
        let (mut app, ctx) = fixture();
        seed_function(&mut app);
        assert_eq!(
            home(&mut app, &ctx, "{F1(0),F1(2),F2(2)}").display(&Settings::default()),
            "{1,5,7}"
        );
        home(&mut app, &ctx, "X:=99");
        assert_eq!(
            home(&mut app, &ctx, "{F1({1,2}),F1((1,2)),F1(#FF:8h),X}")
                .display(&Settings::default()),
            "{{2,5},-2+4*i,65026,99}"
        );
        let source = "EXPORT FCALL() BEGIN STARTAPP(\"Spreadsheet\"); Spreadsheet.F1:=42; Spreadsheet.F1(2):=\"FN_CELL\"; RETURN {Function.F1(2),Function.F2(2),F1(2),Spreadsheet.F1(2),Spreadsheet.F1,X}; END;";
        app.handle_native_tool_request(Request::SaveProgram("FCALL".into(), source.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "FCALL()").display(&Settings::default()),
            "{5,\"FN_CELL2\",\"FN_CELL\",\"FN_CELL\",42,99}"
        );
        assert_eq!(
            home(&mut app, &ctx, "EXPR(\"Function.F1(3)\")").display(&Settings::default()),
            "10"
        );
        let snapshot = app.controller.session_snapshot();
        assert_eq!(snapshot.globals["X"].display(&snapshot.settings), "99");
        assert!(!snapshot.globals.contains_key("F1"));
        assert!(!snapshot.programs.contains_key("F1"));
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.names["FN_CELL"],
            crate::ide::native_app_models::spreadsheet::format::Block::cell(6, 1)
        );
    }
    #[test]
    fn function_uses_callers_app_angle_preserves_home_and_live_ppl_globals() {
        let (mut app, ctx) = fixture();
        seed_function(&mut app);
        home(&mut app, &ctx, "HAngle:=1");
        home(&mut app, &ctx, "AAngle:=1");
        assert_eq!(
            home(&mut app, &ctx, "F3(90)").display(&Settings::default()),
            "0.893996663601"
        );
        let source = "EXPORT FANGLE() BEGIN STARTAPP(\"Spreadsheet\"); AAngle:=3; Q:=7; RETURN {Function.F3(90),HAngle,AAngle,Q}; END;";
        app.handle_native_tool_request(Request::SaveProgram("FANGLE".into(), source.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "FANGLE()").display(&Settings::default()),
            "{0.987688340595,1,3,7}"
        );
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .expression = "100*HAngle+10*AAngle+SIN(X)+Q".into();
        assert_eq!(
            home(&mut app, &ctx, "Function.F1(100)").display(&Settings::default()),
            "138"
        );
        let snapshot = app.controller.session_snapshot();
        assert_eq!(snapshot.settings.angle, ppl_vm::AngleMode::Degrees);
        assert_eq!(snapshot.app_settings["Function"].angle, 1);
        assert_eq!(snapshot.app_settings["Spreadsheet"].angle, 3);
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn actual_cas_function_operands_use_vm_definitions_and_preserve_home_ans() {
        let (mut app, ctx) = fixture();
        seed_function(&mut app);
        home(&mut app, &ctx, "77");
        for (expression, expected) in [("F2(2)+1", "8"), ("Function.F1(2)+1", "6")] {
            app.handle_native_tool_request(Request::Cas(expression.into()), &ctx);
            wait(&mut app, &ctx, |a| {
                a.cas_task.is_none() && !a.controller.operation_active()
            });
            assert!(
                app.native_tools.error.is_none(),
                "{:?}",
                app.native_tools.error
            );
            let snapshot = app.controller.session_snapshot();
            assert_eq!(snapshot.cas.last_value.unwrap().to_string(), expected);
            assert_eq!(snapshot.globals["Ans"].display(&snapshot.settings), "77");
        }
    }
    #[test]
    fn function_keeps_bcd_extremes_and_rejects_writes_from_called_user_programs() {
        let (mut app, ctx) = fixture();
        seed_function(&mut app);
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .expression = "X/1E400".into();
        assert_eq!(
            home(&mut app, &ctx, "F1(1E400)").display(&Settings::default()),
            "1"
        );
        home(&mut app, &ctx, "A:=3");
        let source = "EXPORT FMUTATE() BEGIN A:=99; RETURN 7; END;";
        app.handle_native_tool_request(Request::SaveProgram("FMUTATE".into(), source.into()), &ctx);
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .expression = "FMUTATE()".into();
        let count = app.state.native.history.len();
        start(&mut app, &ctx, "F1(2)");
        wait(&mut app, &ctx, |a| {
            a.state.native.history.len() > count && !a.controller.worker_alive()
        });
        assert!(app.state.native.history.back().unwrap().error);
        assert!(
            app.state
                .native
                .history
                .back()
                .unwrap()
                .result
                .contains("cannot change calculator memory")
        );
        assert_eq!(
            app.controller.session_snapshot().globals["A"].display(&Settings::default()),
            "3"
        );
    }
    #[test]
    fn function_read_ignores_derived_numeric_view_projection_changes() {
        let (mut app, ctx) = fixture();
        seed_function(&mut app);
        start(&mut app, &ctx, "F1(2)");
        let deadline = Instant::now() + Duration::from_secs(3);
        while app
            .controller
            .shared
            .app_variables
            .lock()
            .unwrap()
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_app_variables(&ctx);
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .numeric_start = 99.;
        wait(&mut app, &ctx, |a| {
            !a.controller.worker_alive()
                && a.app_variable_task.is_none()
                && !a.state.native.history.is_empty()
        });
        let outcome = app.state.native.history.back().unwrap();
        assert!(!outcome.error, "{}", outcome.result);
        assert_eq!(outcome.result, "5");
    }
    #[test]
    fn changed_function_source_rejects_a_pending_result_and_cycles_are_bounded() {
        let (mut app, ctx) = fixture();
        seed_function(&mut app);
        start(&mut app, &ctx, "F1(2)");
        let deadline = Instant::now() + Duration::from_secs(3);
        while app
            .controller
            .shared
            .app_variables
            .lock()
            .unwrap()
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_app_variables(&ctx);
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .expression = "X+20".into();
        wait(&mut app, &ctx, |a| {
            !a.controller.worker_alive()
                && a.app_variable_task.is_none()
                && !a.state.native.history.is_empty()
        });
        assert!(app.state.native.history.back().unwrap().error);
        assert_eq!(
            home(&mut app, &ctx, "F1(2)").display(&Settings::default()),
            "22"
        );
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .expression = "F2(X)".into();
        let count = app.state.native.history.len();
        start(&mut app, &ctx, "F1(2)");
        wait(&mut app, &ctx, |a| {
            a.state.native.history.len() > count && !a.controller.worker_alive()
        });
        assert!(app.state.native.history.back().unwrap().error);
        assert!(
            app.state
                .native
                .history
                .back()
                .unwrap()
                .result
                .contains("Circular Function")
        );
    }
    #[test]
    fn real_home_ppl_expr_and_typed_word_assignment_use_authoritative_sheet_and_live_frame() {
        let (mut app, ctx) = fixture();
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A2", "=A*2")
            .unwrap();
        home(&mut app, &ctx, "A:=3");
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.A2").display(&Settings::default()),
            "6"
        );
        let word = home(&mut app, &ctx, "Spreadsheet.$A$1:=#FF:-8h");
        let sheet: serde_json::Value =
            serde_json::to_value(&app.native_tools.memory.models.spreadsheet).unwrap();
        assert_eq!(
            ppl_vm::session::decode_value(&sheet["entered_values"]["A1"]).unwrap(),
            word
        );
        assert_eq!(
            home(&mut app, &ctx, "B→R(EXPR(\"Spreadsheet.a1\"))").display(&Settings::default()),
            "255"
        );
        let source = "EXPORT QREAD() BEGIN A:=9; RETURN {Spreadsheet.A2,EXPR(\"Spreadsheet.A2\"),Spreadsheet.A1(3),TYPE(Spreadsheet.A2(1))}; END;";
        app.handle_native_tool_request(Request::SaveProgram("QREAD".into(), source.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "QREAD()").display(&Settings::default()),
            "{18,18,-1,8}"
        );
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.B2(1):=Spreadsheet.A2(1)")
                .display(&Settings::default()),
            "A*2"
        );
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["B2"],
            "=A*2"
        );
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.B2").display(&Settings::default()),
            "18"
        );
        let snapshot = app.controller.session_snapshot();
        assert_eq!(snapshot.globals["A"].display(&snapshot.settings), "9");
        assert!(!snapshot.globals.contains_key("Spreadsheet.A1"));
        assert!(app.model_task.is_none());
    }
    #[test]
    fn stopping_pending_qualified_set_discards_the_result_before_it_replaces_sheet_data() {
        let (mut app, ctx) = fixture();
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A1", "=2+3")
            .unwrap();
        start(&mut app, &ctx, "Spreadsheet.A1:=99");
        let deadline = Instant::now() + Duration::from_secs(3);
        while app
            .controller
            .shared
            .app_variables
            .lock()
            .unwrap()
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_app_variables(&ctx);
        assert!(app.app_variable_task.is_some());
        app.controller.stop();
        wait(&mut app, &ctx, |a| {
            !a.controller.worker_alive() && a.app_variable_task.is_none()
        });
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=2+3"
        );
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.A1").display(&Settings::default()),
            "5"
        );
    }
    fn key(app: &mut IdeDesktopApp, ctx: &egui::Context, key: egui::Key) {
        frame(
            app,
            ctx,
            vec![
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    #[test]
    fn actual_input_cancel_keeps_retained_formula_and_confirm_freezes_the_typed_value() {
        let (mut app, ctx) = fixture();
        home(&mut app, &ctx, "P:=10");
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A1", "=P*5")
            .unwrap();
        let source =
            "EXPORT ASKAPP() BEGIN LOCAL r:=INPUT(Spreadsheet.A1); RETURN {r,Spreadsheet.A1}; END;";
        app.handle_native_tool_request(Request::SaveProgram("ASKAPP".into(), source.into()), &ctx);
        let count = app.state.native.history.len();
        start(&mut app, &ctx, "ASKAPP()");
        wait(&mut app, &ctx, |a| {
            a.controller.shared.active_dialog.lock().unwrap().is_some()
        });
        key(&mut app, &ctx, egui::Key::Escape);
        wait(&mut app, &ctx, |a| {
            a.state.native.history.len() > count && !a.controller.worker_alive()
        });
        assert_eq!(app.state.native.history.back().unwrap().result, "{0,50}");
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=P*5"
        );
        home(&mut app, &ctx, "P:=20");
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.A1").display(&Settings::default()),
            "100"
        );
        let count = app.state.native.history.len();
        start(&mut app, &ctx, "ASKAPP()");
        wait(&mut app, &ctx, |a| {
            a.controller.shared.active_dialog.lock().unwrap().is_some()
        });
        frame(&mut app, &ctx, vec![egui::Event::Text("42".into())]);
        key(&mut app, &ctx, egui::Key::Enter);
        wait(&mut app, &ctx, |a| {
            a.state.native.history.len() > count && !a.controller.worker_alive()
        });
        assert_eq!(app.state.native.history.back().unwrap().result, "{1,42}");
        assert_eq!(app.native_tools.memory.models.spreadsheet.cells["A1"], "42");
        home(&mut app, &ctx, "P:=30");
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.A1").display(&Settings::default()),
            "42"
        );
    }
    #[test]
    fn a_changed_book_rejects_pending_assignment_without_erasing_the_newer_cells() {
        let (mut app, ctx) = fixture();
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A1", "=2+3")
            .unwrap();
        start(&mut app, &ctx, "Spreadsheet.A1:=99");
        let deadline = Instant::now() + Duration::from_secs(3);
        while app
            .controller
            .shared
            .app_variables
            .lock()
            .unwrap()
            .is_empty()
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.poll_app_variables(&ctx);
        assert!(app.app_variable_task.is_some());
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("B2", "7")
            .unwrap();
        wait(&mut app, &ctx, |a| {
            !a.controller.worker_alive()
                && a.app_variable_task.is_none()
                && !a.state.native.history.is_empty()
        });
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=2+3"
        );
        assert_eq!(app.native_tools.memory.models.spreadsheet.cells["B2"], "7");
        assert!(app.state.native.history.back().unwrap().error);
    }
    #[test]
    fn qualified_cells_use_worksheet_angle_override_after_leaving_the_app() {
        let (mut app, ctx) = fixture();
        app.controller
            .mutate_memory(|m| {
                m.settings.angle = ppl_vm::AngleMode::Degrees;
                m.active_app = "Function".into();
                m.app_settings.insert(
                    "Function".into(),
                    ppl_vm::AppSettings {
                        angle: 3,
                        ..Default::default()
                    },
                );
                m.app_settings.insert(
                    "Spreadsheet".into(),
                    ppl_vm::AppSettings {
                        angle: 1,
                        ..Default::default()
                    },
                );
                Ok(())
            })
            .unwrap();
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("H1", "=100*HAngle+10*AAngle+SIN(A)")
            .unwrap();
        home(&mut app, &ctx, "A:=90");
        assert_eq!(
            home(&mut app, &ctx, "Spreadsheet.H1").display(&Settings::default()),
            "110.893996664"
        );
        let s = app.controller.session_snapshot();
        assert_eq!(s.settings.angle, ppl_vm::AngleMode::Degrees);
        assert_eq!(s.app_settings["Function"].angle, 3);
        assert_eq!(s.app_settings["Spreadsheet"].angle, 1);
    }
    #[test]
    fn qualified_cell_live_ppl_system_slots_do_not_replace_worksheet_app_settings() {
        let (mut app, ctx) = fixture();
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("H1", "=100*HAngle+10*AAngle+SIN(A)")
            .unwrap();
        let source = "EXPORT QANGLE() BEGIN HAngle:=1; STARTAPP(\"Spreadsheet\"); AAngle:=1; A:=90; LOCAL n:=Spreadsheet.H1; STARTAPP(\"Function\"); AAngle:=3; RETURN {HAngle,AAngle,Spreadsheet.H1,n}; END;";
        app.handle_native_tool_request(Request::SaveProgram("QANGLE".into(), source.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "QANGLE()").display(&Settings::default()),
            "{1,3,110.893996664,110.893996664}"
        );
    }
    #[test]
    fn unavailable_cas_assignment_is_a_visible_error_and_keeps_the_formula() {
        let (mut app, ctx) = fixture();
        app.state.native.cas_mode = true;
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A1", "=2+3")
            .unwrap();
        app.handle_native_tool_request(Request::Cas("Spreadsheet.A1:=7".into()), &ctx);
        assert!(app.state.native.history.back().unwrap().error);
        assert!(
            app.native_tools
                .error
                .as_deref()
                .unwrap()
                .contains("not available")
        );
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=2+3"
        );
        assert!(!app.controller.operation_active());
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn actual_desktop_cas_route_resolves_typed_worksheet_operand_and_preserves_home_ans() {
        let (mut app, ctx) = fixture();
        home(&mut app, &ctx, "77");
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("D1", "=8+1")
            .unwrap();
        app.state.native.cas_mode = true;
        app.handle_native_tool_request(Request::Cas("Spreadsheet.D1+1".into()), &ctx);
        wait(&mut app, &ctx, |a| {
            a.cas_task.is_none() && !a.controller.operation_active()
        });
        assert_eq!(
            app.controller
                .session_snapshot()
                .cas
                .last_value
                .unwrap()
                .to_string(),
            "10"
        );
        assert_eq!(
            app.controller.session_snapshot().globals["Ans"].display(&Settings::default()),
            "77"
        );
        assert!(app.native_tools.error.is_none());
    }
}
