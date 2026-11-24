//! Catalog writes use the calculator worker and consume their own completion.
//! They must not inherit a previous operation's handled flag or create Home
//! history/program-output transitions when editing a standalone object.
use super::*;
use crate::ide::native_tools::{Page, Request};

pub(super) struct CatalogProgramReturn {
    page: Page,
    view: NativeView,
    run_id: u64,
}

pub(super) struct CatalogTask {
    name: String,
    page: Option<Page>,
    view: NativeView,
}

pub(super) enum CatalogChange {
    Saved { name: String, page: Page },
    Renamed { new: String, page: Page },
    Deleted { name: String, page: Option<Page> },
}
impl CatalogChange {
    pub(super) fn from_request(request: &Request) -> Option<Self> {
        match request {
            Request::SaveNote(name, _) => Some(Self::Saved {
                name: name.clone(),
                page: Page::Notes,
            }),
            Request::SaveProgram(name, _) => Some(Self::Saved {
                name: name.clone(),
                page: Page::Programs,
            }),
            Request::RenameObject { new, program, .. } => Some(Self::Renamed {
                new: new.clone(),
                page: if *program {
                    Page::Programs
                } else {
                    Page::Notes
                },
            }),
            Request::DeleteNote(name) => Some(Self::Deleted {
                name: name.clone(),
                page: Some(Page::Notes),
            }),
            Request::DeleteProgram(name) => Some(Self::Deleted {
                name: name.clone(),
                page: Some(Page::Programs),
            }),
            Request::DeleteVariable(name) => Some(Self::Deleted {
                name: name.clone(),
                page: None,
            }),
            _ => None,
        }
    }
}

impl IdeDesktopApp {
    pub(super) fn apply_native_catalog_change(&mut self, change: Option<CatalogChange>) {
        match change {
            Some(CatalogChange::Saved { name, page }) if self.native_tools.page == Some(page) => {
                self.native_tools.memory.name = name.clone();
                self.native_tools.memory.original_name = name;
            }
            Some(CatalogChange::Renamed { new, page }) if self.native_tools.page == Some(page) => {
                self.native_tools.memory.name = new.clone();
                self.native_tools.memory.original_name = new;
            }
            Some(CatalogChange::Deleted { name, page })
                if self.native_tools.memory.name == name
                    && (page.is_some_and(|page| self.native_tools.page == Some(page))
                        || (page.is_none()
                            && matches!(
                                self.native_tools.page,
                                Some(Page::Variables | Page::Lists | Page::Matrices)
                            ))) =>
            {
                self.native_tools.memory.name.clear();
                self.native_tools.memory.original_name.clear();
                self.native_tools.memory.content.clear();
            }
            _ => {}
        }
    }

    pub(super) fn start_native_catalog_variable(
        &mut self,
        name: String,
        expression: String,
    ) -> Result<(), String> {
        if self.catalog_task.is_some()
            || self.user_key_task.is_some()
            || self.native_compile.is_some()
            || self.native_stopping
            || self.controller.running()
        {
            return Err("Calculator is busy".into());
        }
        if matches!(
            self.controller.store_variable(name.clone(), expression),
            crate::ide::execution::RunStart::AlreadyRunning
        ) {
            return Err("Calculator is busy".into());
        }
        self.catalog_task = Some(CatalogTask {
            name,
            page: self.native_tools.page,
            view: self.state.native.view,
        });
        self.native_tools.close();
        self.state.calc_input_focus = true;
        self.running = None;
        self.state.native.operation = None;
        self.state.native.outcome_handled = false;
        self.state.native.error = None;
        self.state.native.run_id = self.state.native.run_id.wrapping_add(1);
        self.state.native.set_view(NativeView::ProgramOutput);
        self.state.vm_state = crate::ide::VmExecutionState::Running;
        self.state.status_message = "Calculator object operation".into();
        self.release_native_input();
        Ok(())
    }

    pub(super) fn run_native_catalog_program(
        &mut self,
        name: String,
        source: String,
    ) -> Result<(), String> {
        self.run_native_catalog_program_configured(name, source, None, String::new())
    }

    pub(super) fn run_native_catalog_program_configured(
        &mut self,
        name: String,
        source: String,
        entry: Option<String>,
        args: String,
    ) -> Result<(), String> {
        let args = (!args.trim().is_empty()).then_some(args);
        self.run_native_catalog_program_with(name, source, entry, args)
    }

    fn run_native_catalog_program_with(
        &mut self,
        name: String,
        source: String,
        entry: Option<String>,
        args: Option<String>,
    ) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("A program needs a name".into());
        }
        if self.catalog_task.is_some()
            || self.user_key_task.is_some()
            || self.native_compile.is_some()
            || self.native_stopping
            || self.controller.running()
        {
            return Err("Calculator is busy".into());
        }
        let mut sources = self
            .controller
            .session_snapshot()
            .programs
            .into_iter()
            .map(|(name, source)| {
                (
                    name.clone(),
                    ppl_vm::ModuleSource {
                        identity: ppl_vm::SourceIdentity::new(&name, &source),
                        source,
                    },
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        sources.insert(
            name.clone(),
            ppl_vm::ModuleSource {
                identity: ppl_vm::SourceIdentity::new(&name, &source),
                source: source.clone(),
            },
        );
        // Installed objects keep their identity when an identical IDE source is
        // also open. Other immutable IDE sources remain available dependencies.
        for tab in self
            .editor
            .tabs
            .iter()
            .filter(|tab| tab.debug_source.is_none())
        {
            if !sources.values().any(|module| module.source == tab.content) {
                let namespace = Self::native_namespace(tab);
                sources.insert(
                    namespace.clone(),
                    ppl_vm::ModuleSource {
                        identity: ppl_vm::SourceIdentity::new(namespace, &tab.content),
                        source: tab.content.clone(),
                    },
                );
            }
        }
        self.controller.start_catalog_program(
            &name,
            &source,
            entry.as_deref(),
            args.as_deref(),
            std::sync::Arc::new(sources),
        )?;
        let view = self.state.native.view;
        self.native_tools.close();
        self.state.calc_input_focus = true;
        self.running = None;
        self.state.native.begin(
            NativeOperation::Program {
                label: name.clone(),
                home_expression: None,
            },
            crate::ide::native::NativeCaller::Catalog,
        );
        self.catalog_program_return = Some(CatalogProgramReturn {
            page: Page::Programs,
            view,
            run_id: self.state.native.run_id,
        });
        self.state.status_message = format!("Running {name}");
        self.state.native.record(serde_json::json!({"origin":"catalog","action":"program_start","name":name,"source_sha256":ppl_engine::source::hash(source.as_bytes()),"entry":entry,"arguments":args}));
        self.state.vm_state = crate::ide::VmExecutionState::Running;
        self.state.step_count = 0;
        self.state.elapsed_time = Duration::ZERO;
        self.release_native_input();
        Ok(())
    }

    pub(super) fn restore_native_catalog_program_after_stop(&mut self) {
        let Some(previous) = self.catalog_program_return.take() else {
            return;
        };
        if previous.run_id == self.state.native.run_id {
            self.native_tools.open(previous.page);
            self.state.native.set_view(previous.view);
        }
    }

    pub(super) fn dismiss_native_catalog_program(&mut self, ctx: &egui::Context) -> bool {
        if self
            .catalog_program_return
            .as_ref()
            .is_some_and(|p| p.run_id != self.state.native.run_id)
        {
            self.catalog_program_return = None;
        }
        if self.controller.running()
            || !matches!(
                self.state.native.view,
                NativeView::ProgramOutput | NativeView::Error
            )
        {
            return false;
        }
        let Some(previous) = self.catalog_program_return.take() else {
            return false;
        };
        self.state.native.outcome_handled = true;
        self.native_tools.open(previous.page);
        self.set_native_view(previous.view, ctx);
        true
    }

    /// Return before ordinary worker/stop completion handling when this owns it.
    pub(super) fn poll_native_catalog_task(&mut self) -> bool {
        if self.catalog_task.is_none() {
            // Escape from the pre-run arguments form cancels without entering
            // the program or leaving a spurious runtime-error screen.
            let owns_program = !self.state.native.outcome_handled
                && self
                    .catalog_program_return
                    .as_ref()
                    .is_some_and(|previous| previous.run_id == self.state.native.run_id);
            if !owns_program {
                return false;
            }
            for line in self.controller.drain_terminal() {
                self.console.print_output(line);
            }
            // Keep ownership for this entire UI poll while the thread is alive.
            // Otherwise it can finish between outcome() and ordinary completion,
            // which would turn a cancelled argument form into a runtime error.
            if self.controller.worker_alive() {
                return true;
            }
            let cancelled = matches!(self.controller.outcome(),Some(WorkerOutcome::Error(ref error)) if error.kind==ppl_vm::ErrorKind::Interrupted && error.detail=="Program arguments cancelled");
            if cancelled {
                let _ = self.controller.join_current();
                let operation = self.state.native.operation.take();
                self.state.native.outcome_handled = true;
                self.state.native.error = None;
                self.native_stopping = false;
                self.native_restart = None;
                self.state.native.last_outcome = Some(
                    serde_json::json!({"status":"cancelled","reason":"Program arguments cancelled","operation":format!("{operation:?}")}),
                );
                self.restore_native_catalog_program_after_stop();
                self.state.vm_state = crate::ide::VmExecutionState::Idle;
                self.native_vm_state = crate::ide::VmExecutionState::Idle;
                self.state.status_message = "Program arguments cancelled".into();
                self.release_native_input();
                return true;
            }
            return false;
        }
        for line in self.controller.drain_terminal() {
            self.console.print_output(line);
        }
        if self.controller.worker_alive() {
            return true;
        }
        let task = self.catalog_task.take().unwrap();
        let outcome = self.controller.join_current();
        let mut error = None;
        let mut interrupted = false;
        match outcome {
            Some(WorkerOutcome::Success { value_str, .. }) => {
                let snapshot = self.controller.session_snapshot();
                self.native_tools.memory.content = snapshot
                    .globals
                    .get(&task.name)
                    .map(|value| value.display(&snapshot.settings))
                    .unwrap_or(value_str);
                self.native_tools.memory.name = task.name.clone();
            }
            Some(WorkerOutcome::Error(runtime))
                if runtime.kind == ppl_vm::ErrorKind::Interrupted =>
            {
                interrupted = true
            }
            Some(WorkerOutcome::Error(runtime)) => error = Some(runtime.to_string()),
            None => error = Some("Calculator object worker ended without a result".into()),
        }
        self.state.native.set_view(task.view);
        if let Some(page) = task.page {
            self.native_tools.open(page);
        }
        self.native_tools.error = error.clone();
        self.native_tools.dirty();
        self.state.native.error = None;
        self.state.native.outcome_handled = true;
        self.native_stopping = false;
        self.native_restart = None;
        self.state.vm_state = crate::ide::VmExecutionState::Idle;
        self.native_vm_state = crate::ide::VmExecutionState::Idle;
        self.state.paused_line = 0;
        self.release_native_input();
        self.state.status_message = if let Some(message) = &error {
            self.console.print_error(message);
            message.clone()
        } else if interrupted {
            "Calculator object operation interrupted".into()
        } else {
            "Calculator object ready".into()
        };
        self.state.native.record(serde_json::json!({"origin":"catalog","action":"complete","name":task.name,
            "status":if error.is_some(){"error"}else if interrupted{"interrupted"}else{"success"},"error":error}));
        true
    }
}
