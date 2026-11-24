//! Asynchronous execution of calculator User keyboard bindings.
//!
//! Resolution owns immutable source text. This layer gives its VM invocation the
//! ordinary calculator worker, shared memory, host dialogs and interrupt flag;
//! a return value edits the suspended entry instead of evaluating it as Home.
use super::*;
use crate::ide::native::expression::ExpressionEditor;
use crate::ide::native_tools::Page;
use crate::ide::native_user_keys::{HandlerDescriptor, returned_text};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(super) struct UserKeyTask {
    handler: HandlerDescriptor,
    editor: ExpressionEditor,
    draft: String,
    view: NativeView,
    page: Option<Page>,
    generation: u64,
}

impl IdeDesktopApp {
    pub(super) fn dispatch_native_user_key(&mut self, code: i64, ctx: &egui::Context) -> bool {
        use crate::ide::native_user_keys::{
            HandlerCatalog, KeyStroke, SourceSnapshot, UserDispatch, UserMode,
        };
        let shifted = self.state.native.shifted;
        if self.state.native.user_keys.mode == UserMode::Off
            && !(code == crate::keymap::KEY_HELP && shifted)
        {
            return false;
        }
        if code == crate::keymap::KEY_SHIFT {
            return false;
        }
        let installed = self.controller.session_snapshot().programs;
        let open = self
            .editor
            .tabs
            .iter()
            .filter(|tab| tab.debug_source.is_none())
            .map(|tab| {
                let name = std::path::Path::new(&tab.title)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&tab.title);
                SourceSnapshot::new(name, tab.content.clone())
            })
            .collect::<Vec<_>>();
        let catalog = HandlerCatalog::discover(&installed, open.clone());
        let stroke = KeyStroke::new(
            code,
            shifted,
            self.state.native.alpha != crate::ide::native::keys::AlphaMode::Off,
        )
        .expect("central physical key");
        let dispatch = self.state.native.user_keys.dispatch(stroke, &catalog);
        self.state.native.touch_view();
        match dispatch {
            Ok(UserDispatch::Default) => false,
            Ok(UserDispatch::Consumed | UserDispatch::ModeChanged(_)) => {
                self.state.native.shifted = false;
                ctx.request_repaint();
                true
            }
            Ok(UserDispatch::Handler(handler)) => {
                self.state.native.shifted = false;
                self.state.native.alpha.consume(shifted);
                let mut sources = installed
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
                for (index, snapshot) in open.into_iter().enumerate() {
                    let mut name = snapshot.name;
                    if let Some(existing) = sources.get(&name) {
                        if existing.source == snapshot.source {
                            continue;
                        }
                        name = format!("{name}::<open:{index}>");
                    }
                    sources.insert(
                        name.clone(),
                        ppl_vm::ModuleSource {
                            identity: ppl_vm::SourceIdentity::new(&name, &snapshot.source),
                            source: snapshot.source,
                        },
                    );
                }
                if let Err(error) = self.launch_native_user_key(handler, Arc::new(sources)) {
                    self.state.native.error = Some(error.clone());
                    self.state.status_message = error;
                }
                ctx.request_repaint();
                true
            }
            Err(error) => {
                self.state.native.shifted = false;
                self.state.native.error = Some(error.to_string());
                self.state.status_message = error.to_string();
                ctx.request_repaint();
                true
            }
        }
    }
    /// The input reducer calls this only for an unambiguous KEY declaration.
    /// Dependencies are captured alongside its source, never read again on the
    /// background thread from mutable IDE tabs or the filesystem.
    pub(super) fn launch_native_user_key(
        &mut self,
        handler: HandlerDescriptor,
        catalog: Arc<BTreeMap<String, ppl_vm::ModuleSource>>,
    ) -> Result<(), String> {
        if self.user_key_task.is_some()
            || self.native_compile.is_some()
            || self.controller.running()
        {
            return Err("Calculator is busy".into());
        }
        let generation = self.controller.session_snapshot().generation;
        let mut editor = self.state.native.editor.clone();
        editor.sync_external(&self.state.native.home_input);
        self.controller
            .start_user_key(&handler.name, &handler.source, &handler.entry, catalog)?;
        self.user_key_task = Some(UserKeyTask {
            handler,
            editor,
            draft: self.state.native.home_input.clone(),
            view: self.state.native.view,
            page: self.native_tools.page,
            generation,
        });
        self.native_tools.close();
        self.running = None;
        self.native_stopping = false;
        self.native_restart = None;
        self.state.native.operation = None;
        self.state.native.error = None;
        self.state.native.outcome_handled = false;
        self.state.native.run_id = self.state.native.run_id.wrapping_add(1);
        self.state.native.set_view(NativeView::ProgramOutput);
        self.state.vm_state = crate::ide::VmExecutionState::Running;
        self.state.paused_line = 0;
        self.state.step_count = 0;
        self.state.elapsed_time = Duration::ZERO;
        self.state.status_message = "Running User key".into();
        self.release_native_input();
        self.state.native.record(serde_json::json!({
            "origin":"user-key","action":"start",
            "program":self.user_key_task.as_ref().unwrap().handler.name,
            "entry":self.user_key_task.as_ref().unwrap().handler.entry,
        }));
        Ok(())
    }

    /// Called before ordinary poll_execution handling, including its stop path.
    /// Returning true means this worker belongs to a User key, so its result must
    /// not also create a program-output screen or a Home history calculation.
    pub(super) fn poll_native_user_key(&mut self) -> bool {
        if self.user_key_task.is_none() {
            return false;
        }
        for line in self.controller.drain_terminal() {
            self.console.print_output(line);
        }
        if self.controller.worker_alive() {
            return true;
        }
        let task = self.user_key_task.take().unwrap();
        // The worker has finished. This joins only that finished thread; it
        // never waits for an executing INPUT or loop on the event-loop thread.
        let outcome = self.controller.join_current();
        let snapshot = self.controller.session_snapshot();
        let mut inserted = None;
        let mut error = None;
        let mut interrupted = false;
        match outcome {
            Some(WorkerOutcome::Success { steps, elapsed, .. }) => {
                self.state.step_count = steps;
                self.state.elapsed_time = elapsed;
                if snapshot.generation <= task.generation {
                    error =
                        Some("User key completed without publishing its calculator result".into());
                } else if self.state.native.home_input != task.draft {
                    error = Some("Calculator entry changed while the User key was running".into());
                } else if let Some(value) = &snapshot.last_value {
                    let text = returned_text(value, &snapshot.settings);
                    self.state.native.editor = task.editor;
                    self.state.native.home_input = task.draft;
                    self.state.native.edit(|editor| editor.insert(&text));
                    inserted = Some(text);
                } else {
                    error = Some("User key completed without a return value".into());
                }
            }
            Some(WorkerOutcome::Error(runtime))
                if runtime.kind == ppl_vm::ErrorKind::Interrupted =>
            {
                interrupted = true;
            }
            Some(WorkerOutcome::Error(runtime)) => error = Some(runtime.to_string()),
            None => error = Some("User key worker ended without an outcome".into()),
        }
        self.state.native.set_view(task.view);
        if let Some(page) = task.page {
            self.native_tools.open(page);
        }
        self.state.native.error = error.clone();
        self.state.native.outcome_handled = true;
        self.state.vm_state = crate::ide::VmExecutionState::Idle;
        self.state.paused_line = 0;
        self.native_stopping = false;
        self.native_restart = None;
        self.release_native_input();
        self.state.status_message = if let Some(message) = &error {
            self.console.print_error(message);
            message.clone()
        } else if interrupted {
            "User key interrupted".into()
        } else {
            "User key completed".into()
        };
        let completion = serde_json::json!({
            "origin":"user-key","action":"complete","program":task.handler.name,
            "entry":task.handler.entry,"inserted":inserted,"error":error,
            "status":if interrupted {"interrupted"} else if error.is_some() {"error"} else {"success"},
            "typed_result":if inserted.is_some() {snapshot.last_value.as_ref().map(crate::ide::native::portable_json)} else {None},
        });
        self.state.native.last_outcome = Some(completion.clone());
        self.state.native.record(completion);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialog::{DialogEvent, KeyPress};
    use crate::ide::native::expression::EditCommand;

    fn handler(source: &str) -> HandlerDescriptor {
        HandlerDescriptor {
            name: "keyboard-module".into(),
            source: source.into(),
            entry: "K_Sin".into(),
        }
    }
    fn launch(app: &mut IdeDesktopApp, source: &str) {
        app.launch_native_user_key(handler(source), Arc::new(BTreeMap::new()))
            .unwrap();
    }
    fn finish(app: &mut IdeDesktopApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.user_key_task.is_some() {
            app.poll_execution();
            assert!(Instant::now() < deadline, "User worker did not complete");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.controller.running());
        assert!(app.controller.reserve_native_operation().is_ok());
    }
    fn wait_dialog(app: &IdeDesktopApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app
            .controller
            .shared
            .active_dialog
            .lock()
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < deadline, "User INPUT did not open");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn home_seed(app: &mut IdeDesktopApp, source: &str) {
        app.controller.evaluate_home(source, Some(100_000));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.controller.running() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(
            app.controller.join_current(),
            Some(WorkerOutcome::Success { .. })
        ));
    }

    #[test]
    fn user_key_worker_shares_memory_module_locals_and_preserves_ans_history() {
        let mut app = IdeDesktopApp::default();
        home_seed(&mut app, "A:=2");
        home_seed(&mut app, "99");
        app.state.native.set_view(NativeView::Home);
        app.state.native.edit(|e| {
            e.set_text("abc");
            e.set_cursor(1, false);
            e.set_cursor(2, true);
        });
        let source = "LOCAL calls:=0; KEY K_Sin() BEGIN calls:=calls+1; A:=A+1; RETURN calls; END;";
        launch(&mut app, source);
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "a1c");
        assert_eq!(app.state.native.view, NativeView::Home);
        launch(&mut app, source);
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "a12c");
        let memory = app.controller.session_snapshot();
        assert_eq!(memory.globals["A"].display(&memory.settings), "4");
        assert_eq!(memory.globals["Ans"].display(&memory.settings), "99");
        assert!(app.state.native.history.is_empty());
        assert!(app.state.native.operation.is_none());
        assert!(app.state.native.outcome_handled);
        assert_eq!(
            app.state.native.last_outcome.as_ref().unwrap()["typed_result"]["type"],
            "real"
        );
    }

    #[test]
    fn user_key_returned_strings_are_inserted_verbatim_with_shared_undo() {
        let mut app = IdeDesktopApp::default();
        app.state.native.edit(|e| e.set_text("1+"));
        launch(&mut app, "KEY K_Sin() BEGIN RETURN \"SIN(\"; END;");
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "1+SIN(");
        app.state.native.edit(|e| e.apply(EditCommand::Undo));
        assert_eq!(app.state.native.home_input, "1+");
        launch(&mut app, "KEY K_Sin() BEGIN RETURN {1,2}; END;");
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "1+{1,2}");
    }

    #[test]
    fn user_key_executes_selected_source_snapshot_and_linked_dependencies() {
        let mut app = IdeDesktopApp::default();
        let chosen = handler("KEY K_Sin() BEGIN RETURN USERDEP(); END;");
        let mut sources = BTreeMap::new();
        for (name, source) in [
            ("keyboard-module", "KEY K_Sin() BEGIN RETURN 999; END;"),
            ("dependency", "EXPORT USERDEP() BEGIN RETURN 7; END;"),
        ] {
            sources.insert(
                name.into(),
                ppl_vm::ModuleSource {
                    identity: ppl_vm::SourceIdentity::new(name, source),
                    source: source.into(),
                },
            );
        }
        app.launch_native_user_key(chosen, Arc::new(sources))
            .unwrap();
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "7");
        assert!(
            app.state.native.error.is_none(),
            "{:?}",
            app.state.native.error
        );
    }

    #[test]
    fn user_key_input_awaits_real_modal_updates_and_continues() {
        let mut app = IdeDesktopApp::default();
        launch(
            &mut app,
            "KEY K_Sin() BEGIN LOCAL n:=3; IF INPUT(n,\"User key\",\"N\") THEN RETURN n*2; END; RETURN \"cancel\"; END;",
        );
        wait_dialog(&app);
        app.controller
            .shared
            .push_dialog_event(DialogEvent::Edit(EditCommand::SelectAll));
        app.controller
            .shared
            .push_dialog_event(DialogEvent::Text("8".into()));
        app.controller
            .shared
            .push_dialog_event(DialogEvent::Key(KeyPress::code(30)));
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "16");
        assert!(
            app.controller
                .shared
                .active_dialog
                .lock()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn user_key_on_interrupts_nested_loop_and_next_handler_runs() {
        let mut app = IdeDesktopApp::default();
        app.state.native.edit(|e| e.set_text("unchanged"));
        launch(
            &mut app,
            "KEY K_Sin() BEGIN RETURN SPIN(); END; SPIN() BEGIN WHILE 1 DO A:=A+1; END; END;",
        );
        std::thread::sleep(Duration::from_millis(30));
        app.controller.stop();
        app.native_stopping = true;
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "unchanged");
        assert!(app.state.native.error.is_none());
        assert_eq!(
            app.state.native.last_outcome.as_ref().unwrap()["status"],
            "interrupted"
        );
        assert!(!app.native_stopping);
        launch(&mut app, "KEY K_Sin() BEGIN RETURN \"!\"; END;");
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "unchanged!");
    }

    #[test]
    fn user_key_on_cancels_input_cleans_dialog_and_preserves_entry() {
        let mut app = IdeDesktopApp::default();
        app.state.native.edit(|e| e.set_text("saved"));
        launch(
            &mut app,
            "KEY K_Sin() BEGIN LOCAL n:=3; INPUT(n); RETURN n; END;",
        );
        wait_dialog(&app);
        app.controller.stop();
        app.native_stopping = true;
        finish(&mut app);
        assert!(
            app.controller
                .shared
                .active_dialog
                .lock()
                .unwrap()
                .is_none()
        );
        assert_eq!(app.state.native.home_input, "saved");
        assert_eq!(
            app.state.native.last_outcome.as_ref().unwrap()["status"],
            "interrupted"
        );
    }

    #[test]
    fn user_key_compile_error_and_busy_lease_never_insert_fabricated_success() {
        let mut app = IdeDesktopApp::default();
        app.state.native.edit(|e| e.set_text("saved"));
        let owner = app.controller.reserve_native_operation().unwrap();
        assert!(
            app.launch_native_user_key(
                handler("KEY K_Sin() BEGIN RETURN 1; END;"),
                Arc::new(BTreeMap::new())
            )
            .is_err()
        );
        assert!(app.user_key_task.is_none());
        drop(owner);
        launch(&mut app, "KEY K_Sin() BEGIN RETURN MISSINGFUNCTION(); END;");
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "saved");
        assert!(
            app.state
                .native
                .error
                .as_ref()
                .is_some_and(|e| e.contains("MISSINGFUNCTION"))
        );
        assert_eq!(
            app.state.native.last_outcome.as_ref().unwrap()["status"],
            "error"
        );
    }

    #[test]
    fn user_key_stale_editor_is_not_overwritten_by_async_result() {
        let mut app = IdeDesktopApp::default();
        app.state.native.edit(|e| e.set_text("old"));
        launch(
            &mut app,
            "KEY K_Sin() BEGIN LOCAL n:=1; INPUT(n); RETURN n; END;",
        );
        wait_dialog(&app);
        app.state.native.edit(|e| e.set_text("changed externally"));
        app.controller
            .shared
            .push_dialog_event(DialogEvent::Key(KeyPress::code(30)));
        finish(&mut app);
        assert_eq!(app.state.native.home_input, "changed externally");
        assert!(
            app.state
                .native
                .error
                .as_ref()
                .is_some_and(|e| e.contains("entry changed"))
        );
    }
}
