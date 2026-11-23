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
fn worksheet_cell(expression: &ppl_syntax::ast::Expr) -> Option<&str> {
    let ppl_syntax::ast::ExprKind::Ident(name) = &expression.kind else {
        return None;
    };
    let cell = name.strip_prefix("Spreadsheet.")?;
    crate::ide::native_app_models::spreadsheet::cell_coordinates(cell)?;
    Some(cell)
}
pub(super) fn cas_worksheet_value_read(expression: &ppl_syntax::ast::Expr) -> bool {
    use ppl_syntax::ast::ExprKind;
    if let ExprKind::Call(target, indices) = &expression.kind
        && let [index] = indices.as_slice()
        && matches!(&index.kind, ExprKind::Number(source)
            if ppl_math::Real::parse(source).is_some_and(ppl_math::Real::is_zero))
    {
        return worksheet_cell(target).is_some();
    }
    worksheet_cell(expression).is_some()
}
pub(super) fn cas_worksheet_assignment(
    expression: &ppl_syntax::ast::Expr,
) -> Option<(&str, &ppl_syntax::ast::Expr)> {
    use ppl_syntax::ast::ExprKind;
    let ExprKind::Assign(target, value) = &expression.kind else {
        return None;
    };
    let cell = worksheet_cell(target)?;
    (!cas_assignment(value)).then_some((cell, value))
}
pub(super) fn evaluate_qualified_cas(
    source: &str,
    snapshot: &ppl_vm::SessionSnapshot,
    shared: Arc<crate::host::GuiSharedState>,
    decimal_worksheet_read: bool,
) -> Result<(ppl_cas::CasSession, ppl_cas::CasEvaluation), ppl_cas::CasError> {
    let failure = |e: RuntimeError| ppl_cas::CasError::Protocol(e.to_string());
    let mut context = ppl_vm::SessionContext::new(snapshot.settings.clone(), 0x434153415050);
    context.active_app = snapshot.active_app.clone();
    context.app_settings = snapshot.app_settings.clone();
    context.app_numeric_settings = snapshot.app_numeric_settings.clone();
    context.app_plot_settings = snapshot.app_plot_settings.clone();
    context.geometry_plot_settings = snapshot.geometry_plot_settings.clone();
    context.app_files = snapshot.app_files.clone();
    context.cas = snapshot.cas.clone();
    for (name, value) in &snapshot.globals {
        if (name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit())
            || (context.active_app == "Geometry" && ppl_vm::GeometryPlotSettings::recognizes(name))
        {
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
        Box::new(
            crate::host::GuiHost::new(shared).with_decimal_worksheet_reads(decimal_worksheet_read),
        ),
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
    pub(super) fn apply_spreadsheet_write(
        &mut self,
        sheet: crate::ide::native_app_models::spreadsheet::Spreadsheet,
    ) {
        let models = &mut self.native_tools.memory.models;
        models.spreadsheet = sheet;
        let cell = models.selected_cell.clone();
        models.select_cell(&cell);
        self.native_tools.spreadsheet_changed(false);
    }

    fn app_access_source(&self, app: &str) -> serde_json::Value {
        if app == crate::ide::native_graph3d::NAME {
            return serde_json::json!(self.native_tools.memory.graph3d.definitions);
        }
        if app == "Statistics 2Var" {
            return serde_json::json!(
                self.native_tools
                    .memory
                    .models
                    .statistics2_book
                    .as_ref()
                    .map(|book| (
                        &book.analyses,
                        &book.fit_equations,
                        &book.fit_columns_revisions
                    ))
            );
        }
        serde_json::json!({"sheet": &self.native_tools.memory.models.spreadsheet, "function": &self.native_tools.memory.graphs["Function"].curves, "statistics1": &self.native_tools.memory.models.statistics1_book, "geometry": self.native_tools.memory.models.geometry_book.as_ref().map(|book| book.access_fingerprint())})
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
                let current = self.app_access_source(&task.message.request.app);
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
                    result.and_then(|result| {
                        if let Some(book) = result.geometry {
                            self.native_tools.memory.models.geometry_book = Some(book);
                            self.native_tools.geometry_changed();
                        }
                        if let Some(book) = result.statistics1 {
                            self.native_tools.memory.models.statistics1_book = Some(book);
                            self.native_tools
                                .memory
                                .models
                                .results
                                .retain(|name, _| !name.starts_with("Statistics 1Var"));
                            self.native_tools.dirty();
                        }
                        if let Some(update) = result.statistics2 {
                            use crate::ide::native_application_access::statistics::TwoUpdate;
                            let models = &mut self.native_tools.memory.models;
                            let book = models.statistics2_book.get_or_insert_with(Default::default);
                            match update {
                                TwoUpdate::Checkbox(analysis, enabled) => {
                                    book.analyses[analysis].enabled = enabled;
                                    models
                                        .results
                                        .retain(|name, _| !name.starts_with("Statistics 2Var"));
                                }
                                TwoUpdate::Definition {
                                    index,
                                    analysis,
                                    equation,
                                    columns_revision,
                                    error,
                                } => {
                                    book.analyses[index] = *analysis;
                                    book.fit_equations[index] = equation;
                                    book.fit_columns_revisions[index] = columns_revision;
                                    self.native_tools.statistics_invalidate(true);
                                    if let Some(error) = error {
                                        return Err(error);
                                    }
                                }
                                TwoUpdate::Fits {
                                    results,
                                    equations,
                                    columns_revisions,
                                    disabled_target,
                                } => {
                                    book.fit_equations = equations;
                                    book.fit_columns_revisions = columns_revisions;
                                    models.results.insert("Statistics 2Var".into(), results);
                                    if disabled_target {
                                        self.native_tools.dirty();
                                        return Err(RuntimeError::new(
                                            ErrorKind::InvalidInput,
                                            "Insufficient statistics data",
                                        ));
                                    }
                                }
                            }
                            self.native_tools.dirty();
                        }
                        if let Some(sheet) = result.sheet {
                            self.apply_spreadsheet_write(sheet);
                        }
                        if let Some(definitions) = result.graph3d {
                            self.native_tools.memory.graph3d.definitions = definitions;
                            self.native_tools
                                .graph3d_definitions_changed(&task.message.context);
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
                        Ok(result.value)
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
                let statistics1 = self.native_tools.memory.models.statistics1_book.clone();
                let graph3d = (message.request.app == crate::ide::native_graph3d::NAME)
                    .then(|| self.native_tools.memory.graph3d.definitions.clone());
                let statistics2 = (message.request.app == "Statistics 2Var").then(|| {
                    use crate::ide::native_statistics::TwoBook;
                    self.native_tools
                        .memory
                        .models
                        .statistics2_book
                        .as_ref()
                        .map_or_else(TwoBook::default, |book| TwoBook {
                            analyses: book.analyses.clone(),
                            fit_equations: book.fit_equations.clone(),
                            fit_columns_revisions: book.fit_columns_revisions,
                            ..TwoBook::default()
                        })
                });
                if message.request.app == "Geometry"
                    && let Err(error) = self.native_tools.memory.models.ensure_geometry_book()
                {
                    let _ = message
                        .reply
                        .send(Err(RuntimeError::new(ErrorKind::InvalidInput, error)));
                    continue;
                }
                let geometry = self.native_tools.memory.models.geometry_book.clone();
                let source = self.app_access_source(&message.request.app);
                let request = message.request.clone();
                let context = message.context.clone();
                let cancel = message.cancelled.clone();
                let flag = cancel.clone();
                let (tx, rx) = mpsc::channel();
                let repaint = ctx.clone();
                std::thread::spawn(move || {
                    let result = if request.app == "Statistics 1Var" {
                        crate::ide::native_application_access::statistics::access_with_geometry(
                            statistics1.unwrap_or_default(),
                            &request,
                            &context,
                            flag,
                            geometry.map(Arc::new),
                        )
                        .map(|(value, statistics1)| {
                            crate::ide::native_application_access::AccessResult {
                                value,
                                statistics1,
                                statistics2: None,
                                sheet: None,
                                function: None,
                                geometry: None,
                                graph3d: None,
                            }
                        })
                    } else if let Some(book) = statistics2 {
                        crate::ide::native_application_access::statistics::access_two(
                            book, &request, &context, flag,
                        )
                        .map(|(value, statistics2)| {
                            crate::ide::native_application_access::AccessResult {
                                value,
                                statistics1: None,
                                statistics2,
                                sheet: None,
                                function: None,
                                geometry: None,
                                graph3d: None,
                            }
                        })
                    } else if let Some(definitions) = graph3d {
                        crate::ide::native_application_access::graph3d::access(
                            definitions,
                            &request,
                            &context,
                            flag,
                        )
                    } else if request.app == "Geometry" {
                        crate::ide::native_application_access::geometry::access(
                            geometry.unwrap_or_default(),
                            &request,
                            &context,
                            flag,
                        )
                        .map(|(value, geometry)| {
                            crate::ide::native_application_access::AccessResult {
                                value,
                                geometry,
                                graph3d: None,
                                statistics1: None,
                                statistics2: None,
                                sheet: None,
                                function: None,
                            }
                        })
                    } else {
                        crate::ide::native_application_access::access(
                            sheet,
                            function,
                            geometry.map(Arc::new),
                            &request,
                            &context,
                            flag,
                        )
                    };
                    let _ = tx.send(result);
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
            "{source}: {}",
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
    fn unavailable_cas_attribute_assignment_is_a_visible_error_and_keeps_the_formula() {
        let (mut app, ctx) = fixture();
        app.state.native.cas_mode = true;
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A1", "=2+3")
            .unwrap();
        app.handle_native_tool_request(Request::Cas("Spreadsheet.A1(3):=1".into()), &ctx);
        assert!(app.state.native.history.back().unwrap().error);
        assert!(
            app.native_tools
                .error
                .as_deref()
                .unwrap()
                .contains("direct Spreadsheet cell")
        );
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=2+3"
        );
        assert!(!app.controller.operation_active());
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn standalone_cas_worksheet_value_is_decimal_with_exact_formula_and_operands() {
        let (mut app, ctx) = fixture();
        app.controller
            .mutate_memory(|memory| {
                memory.settings.numeric_version = ppl_math::NumericVersion::Current15751;
                Ok(())
            })
            .unwrap();
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A1", "=1/3")
            .unwrap();
        app.state.native.cas_mode = true;
        for (source, expected) in [
            ("Spreadsheet.A1", "0.333333333333"),
            ("Spreadsheet.A1(0)", "0.333333333333"),
            ("Spreadsheet.A1(1)", "1/3"),
            ("Spreadsheet.A1+1", "4/3"),
            ("Spreadsheet.B1:=Spreadsheet.A1", "1/3"),
        ] {
            app.handle_native_tool_request(Request::Cas(source.into()), &ctx);
            wait(&mut app, &ctx, |a| {
                a.cas_task.is_none() && !a.controller.operation_active()
            });
            assert_eq!(
                app.state.native.history.back().unwrap().result,
                expected,
                "{source}"
            );
        }
        let cells = &app.native_tools.memory.models.spreadsheet.cells;
        assert_eq!(cells["A1"], "=1/3");
        assert_eq!(cells["B1"], "=1/3");
    }
    #[cfg(feature = "giac-integration")]
    #[test]
    fn cas_worksheet_assignment_preserves_exact_rhs_and_rejects_a_stale_sheet() {
        let (mut app, ctx) = fixture();
        app.state.native.cas_mode = true;
        app.handle_native_tool_request(Request::Cas("Spreadsheet.A1:=1/3".into()), &ctx);
        wait(&mut app, &ctx, |a| {
            a.cas_task.is_none() && !a.controller.operation_active()
        });
        assert_eq!(app.state.native.history.back().unwrap().result, "1/3");
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=1/3"
        );

        app.handle_native_tool_request(Request::Cas("Spreadsheet.A1:=11".into()), &ctx);
        app.native_tools
            .memory
            .models
            .spreadsheet
            .set("A2", "42")
            .unwrap();
        wait(&mut app, &ctx, |a| {
            a.cas_task.is_none() && !a.controller.operation_active()
        });
        assert!(app.state.native.history.back().unwrap().error);
        assert_eq!(
            app.native_tools.memory.models.spreadsheet.cells["A1"],
            "=1/3"
        );
        assert_eq!(app.native_tools.memory.models.spreadsheet.cells["A2"], "42");
        assert_eq!(
            app.controller
                .session_snapshot()
                .cas
                .last_value
                .unwrap()
                .to_string(),
            "1/3"
        );
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
    #[test]
    fn statistics_ppl_computes_against_live_columns_and_commits_typed_results() {
        let mut app = IdeDesktopApp::default();
        let ctx = egui::Context::default();
        app.native_tools.memory.selected_app = "Statistics 1Var".into();
        app.native_tools
            .memory
            .models
            .ensure_statistics_book(false)
            .unwrap();
        app.controller.mutate_memory(|memory| {
            memory.active_app = "Statistics 1Var".into();
            memory.settings.numeric_version = ppl_math::NumericVersion::Current15751;
            memory.store_program("STATBRIDGE", "EXPORT STATBRIDGE() BEGIN D1:={1,2,4}; SetFreq(H1,3); Do1VStats(H1); RETURN {NbItem,MeanX,MeanX*3}; END;".into()).map_err(|e|e.to_string())
        }).unwrap();
        let value = home(&mut app, &ctx, "STATBRIDGE()");
        let PortableValue::List(values) = value else {
            panic!("expected statistics results")
        };
        assert_eq!(
            values[0].to_value().expect_real("count").unwrap(),
            ppl_math::Real::from_i64(9)
        );
        assert_eq!(values[1].display(&Settings::default()), "2.33333333333");
        assert_eq!(values[2].display(&Settings::default()), "6.99999999999");
        let book = app
            .native_tools
            .memory
            .models
            .statistics1_book
            .as_ref()
            .unwrap();
        assert_eq!(book.analyses[0].frequency_expression(), "3");
        assert_eq!(book.result_values["H1"].len(), 13);
        let restored: crate::ide::native_statistics::OneBook =
            serde_json::from_value(serde_json::to_value(book).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.result_values, book.result_values);
        assert_eq!(
            app.controller.session_snapshot().globals["D1"]
                .to_value()
                .string_form(Default::default()),
            "{1,2,4}"
        );
        let cached = restored.result_values.clone();
        home(&mut app, &ctx, "SetFreq(H1,-1)");
        start(&mut app, &ctx, "Do1VStats(H1)");
        wait(&mut app, &ctx, |a| {
            !a.controller.worker_alive()
                && a.state
                    .native
                    .history
                    .back()
                    .is_some_and(|h| h.expression == "Do1VStats(H1)")
        });
        assert!(app.state.native.history.back().unwrap().error);
        assert_eq!(
            app.native_tools
                .memory
                .models
                .statistics1_book
                .as_ref()
                .unwrap()
                .result_values,
            cached
        );
    }
    #[test]
    fn first_statistics_request_keeps_its_source_during_column_import() {
        let (mut app, ctx) = fixture();
        let settings = Settings::default();
        home(&mut app, &ctx, "D1:={2,4}");
        app.handle_native_tool_request(Request::SelectApp("Statistics 1Var".into()), &ctx);
        assert!(app.native_tools.memory.models.statistics1_book.is_none());

        let count = app.state.native.history.len();
        start(&mut app, &ctx, "SetSample(H1,D1+1)");
        // Put the first PPL app access and initial view sync in the same frame.
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
        frame(&mut app, &ctx, vec![]);
        assert!(app.app_variable_task.is_some());
        wait(&mut app, &ctx, |app| {
            app.state.native.history.len() > count && !app.controller.worker_alive()
        });
        let entry = app.state.native.history.back().unwrap();
        assert!(!entry.error, "{}", entry.result);
        assert_eq!(entry.result, "1");
        let book = app
            .native_tools
            .memory
            .models
            .statistics1_book
            .as_ref()
            .unwrap();
        assert_eq!(book.columns.0[0], [2., 4.]);
        assert_eq!(book.analyses[0].data_source.as_deref(), Some("D1+1"));
        home(&mut app, &ctx, "Do1VStats(H1)");
        assert_eq!(home(&mut app, &ctx, "MeanX").display(&settings), "4");
    }

    #[test]
    fn statistics_geometry_definitions_drive_ppl_summary_and_plot_after_redraw() {
        use crate::ide::{
            native_app_ui::numeric, native_model_plots::PlotData, native_tools::Page,
        };
        let (mut app, ctx) = fixture();
        for (name, program) in [
            (
                "GEOSTATSSEED",
                r#"EXPORT GEOSTATSSEED() BEGIN
                STARTAPP("Geometry");Geometry.DelInstruction();
                Geometry.Instruction(0):="point(0,0)";
                Geometry.Instruction(0):="point(3,4)";
                Geometry.Instruction(0):="distance(GA,GB)";
                STARTVIEW(0,1);RETURN Geometry.GC;
            END;"#,
            ),
            (
                "GEOSTATSSAMPLE",
                r#"EXPORT GEOSTATSSAMPLE() BEGIN
                D1:={1,2,4};SetSample(H1,D1*Geometry.GC);SetFreq(H1,1);
                Do1VStats(H1);RETURN MeanX;
            END;"#,
            ),
            (
                "GEOSTATSFREQ",
                r#"EXPORT GEOSTATSFREQ() BEGIN
                SetSample(H1,D1);SetFreq(H1,Geometry.GC*{1,1,2});H1(3):=4;
                Do1VStats(H1);RETURN {NbItem,MeanX};
            END;"#,
            ),
        ] {
            app.handle_native_tool_request(Request::SaveProgram(name.into(), program.into()), &ctx);
        }
        let settings = Settings::default();
        assert_eq!(
            home(&mut app, &ctx, "GEOSTATSSEED()").display(&settings),
            "5"
        );
        let statistics = "Statistics 1Var";
        app.handle_native_tool_request(Request::SelectApp(statistics.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "GEOSTATSSAMPLE()").display(&settings),
            "11.6666666667"
        );
        assert_eq!(
            home(&mut app, &ctx, "GEOSTATSFREQ()").display(&settings),
            "{20,2.75}"
        );

        home(&mut app, &ctx, r#"Geometry.Instruction(2):="point(6,8)""#);
        assert_eq!(
            home(&mut app, &ctx, "GEOSTATSFREQ()").display(&settings),
            "{20,2.75}"
        );
        assert_eq!(
            home(&mut app, &ctx, r#"STARTAPP("Geometry")"#).display(&settings),
            "1"
        );
        app.handle_native_tool_request(Request::SelectApp(statistics.into()), &ctx);

        app.native_tools.open(Page::Numeric);
        let model = &app.native_tools.memory.models;
        let calculation = numeric(statistics, model).unwrap();
        app.handle_native_tool_request(Request::Calculate(calculation), &ctx);
        wait(&mut app, &ctx, |app| app.model_task.is_none());
        let report = &app.native_tools.memory.models.results[statistics]["H1"];
        assert_eq!(report["count"], 40.);
        assert_eq!(report["mean"], 2.75);

        app.native_tools.open(Page::Plot);
        app.native_tools
            .autoscale(app.controller.session_snapshot());
        wait(&mut app, &ctx, |app| !app.native_tools.calculating());
        let plot: PlotData = serde_json::from_value(
            app.native_tools.memory.models.results["Statistics 1Var:plot"].clone(),
        )
        .unwrap();
        let crate::ide::native_model_plots::PlotScaleBounds::Finite([xmin, xmax, ymin, ymax]) =
            plot.scale.unwrap().bounds.unwrap()
        else {
            panic!("Expected finite Autoscale bounds");
        };
        assert!(xmin < 1. && xmax >= 3. && ymin < 0. && ymax > 20.);
        assert_eq!(plot.curves, vec![vec![[1., 10.], [2., 10.], [4., 20.]]]);
        let trace = &plot.statistics_traces[0].samples;
        assert_eq!(trace[2].frequency, Some(20));
        assert_eq!(trace[2].row, Some(3));
        assert_eq!(trace[2].y, Some(20.));

        // The original virtual G1 retains fractional frequency from a distance
        // of sqrt(73), including its decimal arithmetic on the resulting list.
        home(&mut app, &ctx, r#"Geometry.Instruction(2):="point(8,3)""#);
        home(&mut app, &ctx, r#"STARTAPP("Geometry")"#);
        app.handle_native_tool_request(Request::SelectApp(statistics.into()), &ctx);
        assert_eq!(
            home(&mut app, &ctx, "GEOSTATSFREQ()").display(&settings),
            "{34.1760149812,2.75}"
        );

        // A failed definition must neither modify Geometry nor replace prior results.
        let before = app
            .native_tools
            .memory
            .models
            .statistics1_book
            .as_ref()
            .unwrap()
            .result_values
            .clone();
        let geometry = app
            .native_tools
            .memory
            .models
            .geometry_book
            .as_ref()
            .unwrap()
            .access_fingerprint();
        home(&mut app, &ctx, "SetSample(H1,Geometry.DelInstruction())");
        start(&mut app, &ctx, "Do1VStats(H1)");
        wait(&mut app, &ctx, |a| {
            !a.controller.worker_alive()
                && a.state
                    .native
                    .history
                    .back()
                    .is_some_and(|h| h.expression == "Do1VStats(H1)")
        });
        let result = app.state.native.history.back().unwrap();
        assert!(
            result.error
                && result
                    .result
                    .contains("Read-only expressions cannot change Geometry"),
            "{}",
            result.result
        );
        assert_eq!(
            app.native_tools
                .memory
                .models
                .statistics1_book
                .as_ref()
                .unwrap()
                .result_values,
            before
        );
        assert_eq!(
            app.native_tools
                .memory
                .models
                .geometry_book
                .as_ref()
                .unwrap()
                .access_fingerprint(),
            geometry
        );
    }
    #[test]
    fn geometry_ppl_symbolic_redraw_completes_before_measurements_and_restores_definitions() {
        let (mut app, ctx) = fixture();
        let source = r#"EXPORT GEOBRIDGE() BEGIN
            STARTAPP("Geometry"); Geometry.DelInstruction();
            Geometry.Instruction(0):="point(0,0)";
            Geometry.Instruction(0):="point(4,0)";
            Geometry.Instruction(0):="point(0,3)";
            Geometry.Instruction(0):="triangle(GA,GB,GC)";
            Geometry.Instruction(0):="distance(GB,GC)";
            STARTVIEW(0,1);
            RETURN {Geometry.GE,area(Geometry.GD),perimeter(Geometry.GD)};
        END;"#;
        app.handle_native_tool_request(
            Request::SaveProgram("GEOBRIDGE".into(), source.into()),
            &ctx,
        );
        assert_eq!(
            home(&mut app, &ctx, "GEOBRIDGE()").display(&Settings::default()),
            "{5,6,12}"
        );
        let original = app
            .native_tools
            .memory
            .models
            .geometry_book
            .as_ref()
            .unwrap();
        assert_eq!(original.objects.len(), 5);
        assert!(!original.current_values.is_empty());
        let saved = serde_json::to_value(&app.native_tools.memory).unwrap();
        let restored: crate::ide::native_tools::ToolMemory = serde_json::from_value(saved).unwrap();
        restored.validate().unwrap();
        let book = restored.models.geometry_book.as_ref().unwrap();
        assert_eq!(book.objects.len(), 5);
        assert!(
            book.current_values.is_empty(),
            "A fresh process must restore definitions without a computed-value cache"
        );
        app.native_tools.memory = restored;
        let source = r#"EXPORT GEORESTORE() BEGIN
            STARTAPP("Geometry"); STARTVIEW(1,1);
            RETURN {Geometry.GE,area(Geometry.GD),perimeter(Geometry.GD)};
        END;"#;
        app.handle_native_tool_request(
            Request::SaveProgram("GEORESTORE".into(), source.into()),
            &ctx,
        );
        assert_eq!(
            home(&mut app, &ctx, "GEORESTORE()").display(&Settings::default()),
            "{5,6,12}"
        );
        wait(&mut app, &ctx, |app| {
            !app.native_tools.calculating()
                && app
                    .native_tools
                    .memory
                    .models
                    .results
                    .contains_key("Geometry:plot")
        });
        let source = r#"EXPORT GEODRAWZERO() BEGIN
            Geometry.Instruction(2):="point(8,0)";
            LOCAL before:=Geometry.GE;
            STARTVIEW(0,0);
            RETURN {before,Geometry.GE};
        END;"#;
        app.handle_native_tool_request(
            Request::SaveProgram("GEODRAWZERO".into(), source.into()),
            &ctx,
        );
        assert_eq!(
            home(&mut app, &ctx, "GEODRAWZERO()").display(&Settings::default()),
            "{5,8.54400374532}"
        );
    }
    #[test]
    fn worksheet_and_function_read_the_current_geometry_value_via_ppl() {
        let (mut app, ctx) = fixture();
        let source = r#"EXPORT GEOFORMULAS() BEGIN
            STARTAPP("Geometry");Geometry.DelInstruction();
            Geometry.Instruction(0):="point(0,0)";
            Geometry.Instruction(0):="point(3,4)";
            Geometry.Instruction(0):="distance(GA,GB)";STARTVIEW(0,1);
            STARTAPP("Spreadsheet");Spreadsheet.A1(1):='Geometry.GC+2';
            STARTAPP("Function");Function.F1:="Geometry.GC+X";
            RETURN {TYPE(Spreadsheet.A1(1)),Spreadsheet.A1,F1(2)};
        END;"#;
        app.handle_native_tool_request(
            Request::SaveProgram("GEOFORMULAS".into(), source.into()),
            &ctx,
        );
        assert_eq!(
            home(&mut app, &ctx, "GEOFORMULAS()").display(&Settings::default()),
            "{8,7,7}"
        );
        assert!(!app.controller.session_snapshot().globals.contains_key("GC"));
        let worksheet = &app.native_tools.memory.models.spreadsheet;
        assert!(worksheet.cells["A1"].starts_with('='));
        assert!(worksheet.cells["A1"].contains("Geometry.GC"));
        assert_eq!(
            app.native_tools.memory.graphs["Function"].curves[0].expression,
            "Geometry.GC+X"
        );

        let source = r#"EXPORT GEORECALC() BEGIN
            Geometry.Instruction(2):="point(6,8)";
            LOCAL prior:=Spreadsheet.A1;
            STARTAPP("Geometry");STARTVIEW(0,1);
            RETURN {prior,Spreadsheet.A1,Function.F1(2)};
        END;"#;
        app.handle_native_tool_request(
            Request::SaveProgram("GEORECALC".into(), source.into()),
            &ctx,
        );
        assert_eq!(
            home(&mut app, &ctx, "GEORECALC()").display(&Settings::default()),
            "{7,12,12}"
        );

        use crate::ide::native_tools::Page;
        app.handle_native_tool_request(Request::SelectApp("Function".into()), &ctx);
        app.native_tools
            .memory
            .graphs
            .get_mut("Function")
            .unwrap()
            .curves[0]
            .enabled = true;
        app.native_tools.open(Page::Plot);
        app.native_tools.plot(app.controller.session_snapshot());
        wait(&mut app, &ctx, |app| !app.native_tools.calculating());
        let samples = &app.native_tools.plot_samples()[0];
        assert!(samples.len() > 100);
        for [x, y] in samples
            .iter()
            .map(|point| point.expect("finite linear graph"))
        {
            assert!((y - x - 10.).abs() < 1e-8);
        }
        app.native_tools.open(Page::Numeric);
        app.native_tools.table(&app.controller.session_snapshot());
        wait(&mut app, &ctx, |app| !app.native_tools.calculating());
        let table = app.native_tools.numeric_typed_table();
        assert!(table.len() > 30);
        assert_eq!(
            table[0][0].as_ref().unwrap().display(&Settings::default()),
            "0"
        );
        assert_eq!(
            table[0][1].as_ref().unwrap().display(&Settings::default()),
            "10"
        );

        // Restoring tool memory keeps definitions but discards the rendered
        // Geometry cache. STARTAPP must rebuild it before executing a read.
        let restored = serde_json::to_value(&app.native_tools.memory).unwrap();
        let (mut reopened, ctx) = fixture();
        reopened.native_tools.memory = serde_json::from_value(restored).unwrap();
        let book = reopened
            .native_tools
            .memory
            .models
            .geometry_book
            .as_ref()
            .unwrap();
        assert!(book.current_values.is_empty());
        reopened.handle_native_tool_request(
            Request::SaveProgram(
                "GEOREAD".into(),
                r#"EXPORT GEOREAD() BEGIN
                    STARTAPP("Geometry");
                    RETURN {TYPE(Spreadsheet.A1(1)),Spreadsheet.A1,Function.F1(2)};
                END;"#
                    .into(),
            ),
            &ctx,
        );
        assert_eq!(
            reopened.controller.session_snapshot().active_app,
            "Function"
        );
        assert_ne!(reopened.controller.session_snapshot().generation, 0);
        assert_eq!(
            home(&mut reopened, &ctx, "GEOREAD()").display(&Settings::default()),
            "{8,12,12}"
        );
    }
}
