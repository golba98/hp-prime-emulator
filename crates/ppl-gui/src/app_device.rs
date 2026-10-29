//! The physical calculator workflow: Send, calculator status, the program
//! list, opening a program from the calculator, and screen comparison.
//! USB work runs on the device link's worker; this file routes requests and
//! results between it and the editor.
use super::*;
use crate::ide::device::{self, JobError, JobOutput, Purpose};
use crate::ide::send::{SendEvent, SendOutcome, SendRequest, SendStage, payload_sha256};
use ppl_syntax::DiagnosticClass;

impl IdeDesktopApp {
    /// Send to calculator: save, wait for the check, then ask or send.
    pub(super) fn begin_send(&mut self, ctx: &egui::Context) {
        if self.send.active() {
            return;
        }
        if let Some(purpose) = self.device.busy() {
            self.state.status_message = format!("Calculator busy: {}", purpose.busy_label());
            return;
        }
        let Some(tab) = self.editor.active_tab() else {
            self.state.status_message = "Open a program to send it".into();
            return;
        };
        if tab.modified || tab.path.is_none() {
            self.handle_action(IdeAction::SaveActiveFile, ctx);
        }
        let Some(tab) = self.editor.active_tab() else {
            return;
        };
        if tab.modified || tab.path.is_none() {
            self.state.status_message = "Not sent: save the program first".into();
            return;
        }
        self.send.stage = SendStage::WaitingForCheck(tab.id);
        self.continue_send(ctx);
    }

    /// Moves a send on once the background check of its tab has finished.
    fn continue_send(&mut self, ctx: &egui::Context) {
        let Some(tab_id) = self.send.waiting_for() else {
            return;
        };
        let Some(i) = self.editor.index_of(tab_id) else {
            self.send.stage = SendStage::Idle;
            return;
        };
        match self.analysis_state(i) {
            // Diagnostics are scheduled every frame; poll again next frame.
            state if !state.is_settled() => {}
            AnalysisState::Invalid { errors, .. } => {
                self.send.stage = SendStage::Idle;
                self.state.active_dock_tab = DockTab::Problems;
                self.state.show_bottom_dock = true;
                let first = self.editor.tabs[i].current_analysis().and_then(|a| {
                    a.items
                        .iter()
                        .find(|d| d.severity == ppl_syntax::Severity::Error)
                        .map(|d| (d.line, d.col))
                });
                if let Some((line, col)) = first {
                    self.handle_action(
                        IdeAction::NavigateToLocation {
                            tab: Some(tab_id),
                            line,
                            col,
                        },
                        ctx,
                    );
                }
                // After the jump, which sets its own status message.
                self.state.status_message =
                    format!("Not sent: fix {errors} error(s) first (see Problems)");
            }
            AnalysisState::AnalysisFailed(reason) => {
                self.send.stage = SendStage::Finished(SendOutcome::Failed {
                    name: self.editor.tabs[i].title.clone(),
                    message: format!("The program could not be checked: {reason}"),
                    hint: None,
                    recovery_id: None,
                });
            }
            _ => {
                let path = self.editor.tabs[i].path.clone().unwrap_or_default();
                match self.state.send_names.get(&path) {
                    Some(name) => self.prepare_send(i, name.clone(), ctx),
                    None => {
                        self.send.stage = SendStage::AskName {
                            tab: tab_id,
                            name: crate::ide::send::suggested_name(&path),
                            path,
                            error: None,
                        }
                    }
                }
            }
        }
    }

    /// Freezes the request, warns about features the emulator could not
    /// test, and starts the device job.
    fn prepare_send(&mut self, i: usize, name: String, ctx: &egui::Context) {
        let tab = &mut self.editor.tabs[i];
        let source = tab.content.clone();
        let failed = |message: String| {
            SendStage::Finished(SendOutcome::Failed {
                name: name.clone(),
                message,
                hint: None,
                recovery_id: None,
            })
        };
        let image = ppl_programs::Source::text(&source, &name)
            .map_err(ppl_programs::refused)
            .and_then(|s| ppl_programs::ProgramImage::new(&name, s));
        match image {
            Err(e) => {
                self.send.stage = failed(e.to_string());
                return;
            }
            Ok(image) if !image.sizes.supported => {
                self.send.stage = failed(format!(
                    "The program is too large to send safely ({} bytes; the verified limit is {} bytes).",
                    image.sizes.payload_bytes, image.sizes.limit_bytes
                ));
                return;
            }
            Ok(_) => {}
        }
        let untested: Vec<String> = tab
            .current_analysis()
            .map(|a| {
                a.items
                    .iter()
                    .filter(|d| d.class == DiagnosticClass::Compatibility)
                    .map(|d| format!("Line {}: {}", d.line, d.message))
                    .collect()
            })
            .unwrap_or_default();
        let request = SendRequest {
            tab: tab.id,
            path: tab.path.clone().unwrap_or_default(),
            sha256: payload_sha256(&source),
            name,
            source,
        };
        if untested.is_empty() {
            self.start_send(request, ctx);
        } else {
            self.send.stage = SendStage::ConfirmUntested { request, untested };
        }
    }

    fn start_send(&mut self, request: SendRequest, ctx: &egui::Context) {
        let (name, source, sha) = (
            request.name.clone(),
            request.source.clone(),
            request.sha256.clone(),
        );
        let store = ppl_programs::delete_store::Store::new(&self.program_store.root);
        if self.device.start(Purpose::Send, ctx, move |prime| {
            device::send(prime, &name, &source, &sha, &store)
        }) {
            self.send.stage = SendStage::Sending(request);
        } else {
            self.send.stage = SendStage::Idle;
            self.state.status_message = "Calculator busy; try again in a moment".into();
        }
    }

    pub(super) fn handle_send_event(&mut self, event: SendEvent, ctx: &egui::Context) {
        match event {
            SendEvent::NameChosen { tab, path, name } => {
                self.state.send_names.insert(path, name.clone());
                match self.editor.index_of(tab) {
                    Some(i) => self.prepare_send(i, name, ctx),
                    None => self.send.stage = SendStage::Idle,
                }
            }
            SendEvent::Start(request) => self.start_send(request, ctx),
            SendEvent::Replace {
                request,
                operation_id,
                new_sha256,
            } => {
                // The authorization covers exactly the reviewed source.
                let current = self
                    .editor
                    .index_of(request.tab)
                    .map(|i| self.editor.tabs[i].content.as_str());
                if current != Some(request.source.as_str()) {
                    if let Some(id) = self.send.invalidate_if_changed(request.tab, current) {
                        self.cancel_prepared_replacement(&id);
                    }
                    return;
                }
                let store = ppl_programs::delete_store::Store::new(&self.program_store.root);
                if self.device.start(Purpose::Send, ctx, move |prime| {
                    device::apply_replacement(prime, &operation_id, &new_sha256, &store)
                }) {
                    self.send.stage = SendStage::Replacing(request);
                }
            }
            SendEvent::OpenAdvanced(id) => {
                self.program_manager.recovery_id = id;
                self.program_manager.open = true;
            }
            SendEvent::CompareScreens => self.begin_compare(ctx),
            SendEvent::CancelReplacement(id) => self.cancel_prepared_replacement(&id),
        }
    }

    pub(super) fn cancel_send_dialog_escape(&mut self) {
        use crate::ide::send::SendStage;
        match &self.send.stage {
            SendStage::ConfirmReplace { plan, .. } => {
                let id = plan.operation_id.clone();
                self.send.stage = SendStage::Idle;
                self.cancel_prepared_replacement(&id);
            }
            SendStage::AskName { .. }
            | SendStage::ConfirmUntested { .. }
            | SendStage::Finished(_)
            | SendStage::WaitingForCheck(_) => {
                self.send.stage = SendStage::Idle;
                self.state.status_message = "Send cancelled; calculator unchanged".into();
            }
            SendStage::Sending(_) | SendStage::Replacing(_) => {
                self.state.status_message =
                    "Calculator transfer in progress; waiting for its result".into();
            }
            SendStage::Idle => {}
        }
    }

    fn cancel_prepared_replacement(&mut self, id: &str) {
        match ppl_programs::replacement::cancel_prepared(&self.program_store, id) {
            Ok(()) => {
                self.state.status_message =
                    "Replacement cancelled; calculator unchanged, backup kept".into()
            }
            Err(e) => {
                self.send.stage = SendStage::Finished(SendOutcome::Failed {
                    name: "Replacement".into(),
                    message: format!("Could not record replacement cancellation: {e}"),
                    hint: Some(
                        "Open Advanced and verify this operation before sending again.".into(),
                    ),
                    recovery_id: Some(id.into()),
                });
                self.state.status_message =
                    "Replacement cancellation could not be recorded (see Advanced)".into();
            }
        }
    }

    pub(super) fn current_compare_identity(&self) -> Option<crate::ide::compare::ScreenIdentity> {
        use crate::ide::compare::ScreenIdentity;
        let generation = if self.state.firmware_backend {
            let firmware = self.firmware.as_ref()?;
            if !firmware.alive.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            firmware
                .latest
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()?
                .generation
        } else {
            self.native_display_generation()
        };
        Some(ScreenIdentity {
            firmware: self.state.firmware_backend,
            generation,
        })
    }

    pub(crate) fn capture_compare_frame(
        &self,
    ) -> Option<(ppl_graphics::Grob, crate::ide::compare::ScreenIdentity)> {
        use crate::ide::compare::ScreenIdentity;
        if self.state.firmware_backend {
            let firmware = self.firmware.as_ref()?;
            if !firmware.alive.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            let guard = firmware.latest.lock().unwrap_or_else(|e| e.into_inner());
            let frame = guard.as_ref()?;
            if frame.pixels.len() != 320 * 240 {
                return None;
            }
            let mut grob = ppl_graphics::Grob::screen();
            for (pixel, source) in grob.pixels_mut().iter_mut().zip(frame.pixels.iter()) {
                *pixel = source & 0x00ff_ffff;
            }
            Some((
                grob,
                ScreenIdentity {
                    firmware: true,
                    generation: frame.generation,
                },
            ))
        } else {
            let (screen, generation) = self.native_display_frame();
            Some((
                screen,
                ScreenIdentity {
                    firmware: false,
                    generation,
                },
            ))
        }
    }

    /// Captures the selected emulator frame and reads the G1 on the worker.
    pub(super) fn begin_compare(&mut self, ctx: &egui::Context) {
        self.state.show_compare = true;
        if self.device.busy().is_some() {
            return;
        }
        let Some((grob, source)) = self.capture_compare_frame() else {
            self.compare.capture_unavailable("No current frame from the selected G1 firmware engine. Start it and wait for its display, then refresh.".into());
            return;
        };
        self.compare.begin(grob, source);
        if !self
            .device
            .start(Purpose::Screenshot, ctx, device::screenshot)
        {
            self.compare
                .set_error("The calculator is busy; try again in a moment.".into());
        }
    }

    pub(super) fn handle_device_action(&mut self, action: IdeAction, ctx: &egui::Context) {
        match action {
            IdeAction::SendToCalculator => self.begin_send(ctx),
            IdeAction::ShowCalculatorStatus => {
                self.calc_status = None;
                self.calc_status_open = true;
                self.device.start(Purpose::Identify, ctx, device::identify);
            }
            IdeAction::ListCalculatorPrograms => {
                self.explorer.calculator.error = None;
                self.device
                    .start(Purpose::ListPrograms, ctx, device::list_programs);
            }
            IdeAction::OpenFromCalculator(name) => {
                self.device.start(Purpose::ReadProgram, ctx, move |prime| {
                    device::read_program(prime, &name)
                });
            }
            IdeAction::CompareScreens => self.begin_compare(ctx),
            IdeAction::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.explorer.refresh_workspace(Some(&dir));
                    self.state.workspace_root = Some(dir);
                    self.state.show_explorer = true;
                }
            }
            _ => {}
        }
    }

    fn route_device_result(&mut self, purpose: Purpose, result: Result<JobOutput, JobError>) {
        match purpose {
            Purpose::Send => {
                self.send.on_result(result);
                if let SendStage::Finished(outcome) = &self.send.stage {
                    self.state.status_message = match outcome {
                        SendOutcome::Sent { name, .. } => format!("{name} sent to the calculator"),
                        SendOutcome::Failed { name, .. } => format!("{name} was not sent"),
                    };
                }
            }
            Purpose::Identify => {
                self.calc_status = Some(match result {
                    Ok(JobOutput::Identity(identity)) => Ok(identity),
                    Ok(other) => Err(JobError {
                        message: format!("Unexpected device reply: {other:?}"),
                        hint: None,
                    }),
                    Err(e) => Err(e),
                });
            }
            Purpose::ListPrograms => match result {
                Ok(JobOutput::Inventory { programs, .. }) => {
                    self.explorer.calculator.programs = Some(programs);
                }
                Ok(_) => {}
                Err(e) => self.explorer.calculator.error = Some(e.message),
            },
            Purpose::ReadProgram => match result {
                Ok(JobOutput::Program { name, source }) => {
                    self.editor.new_tab(format!("{name}.hpprgm"), source);
                    if let Some(tab) = self.editor.active_tab_mut() {
                        // Not on disk yet: closing asks, saving asks for a place.
                        tab.modified = true;
                    }
                    self.state.status_message =
                        format!("Opened a copy of {name} from the calculator (not saved yet)");
                }
                Ok(_) => {}
                Err(e) => {
                    self.state.status_message = format!("Could not read the program: {}", e.message)
                }
            },
            Purpose::Screenshot => match result {
                Ok(JobOutput::Screenshot(png)) => self.compare.set_device_png(&png),
                Ok(_) => {}
                Err(e) => self.compare.set_error(match e.hint {
                    Some(hint) => format!("{} {hint}", e.message),
                    None => e.message,
                }),
            },
        }
    }

    /// Per-frame: results, the presence light, and the device dialogs.
    pub(super) fn poll_device(&mut self, ctx: &egui::Context) {
        if let Some((purpose, result)) = self.device.poll() {
            self.route_device_result(purpose, result);
        }
        self.state.device_status = self.device.status.clone();
        self.state.device_busy = self.device.busy();
        self.continue_send(ctx);
        if let SendStage::ConfirmReplace { request, .. } = &self.send.stage {
            let tab = request.tab;
            let current = self
                .editor
                .index_of(tab)
                .map(|i| self.editor.tabs[i].content.as_str());
            if let Some(id) = self.send.invalidate_if_changed(tab, current) {
                self.cancel_prepared_replacement(&id);
            }
        }
        if let Some(event) = self.send.show(ctx) {
            self.handle_send_event(event, ctx);
        }
        self.show_calculator_status(ctx);
    }

    fn show_calculator_status(&mut self, ctx: &egui::Context) {
        if !self.calc_status_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Calculator status")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| match &self.calc_status {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading the calculator…");
                    });
                }
                Some(Ok(identity)) => {
                    egui::Grid::new("calc_identity")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            if let device::DeviceStatus::Connected { model, .. } =
                                &self.state.device_status
                            {
                                ui.label("Model");
                                ui.label(*model);
                                ui.end_row();
                            }
                            ui.label("Firmware build");
                            ui.label(
                                identity
                                    .firmware_build
                                    .map_or("unknown".into(), |b| b.to_string()),
                            );
                            ui.end_row();
                            ui.label("Device ID");
                            ui.label(egui::RichText::new(&identity.serial_id).monospace())
                                .on_hover_text(
                                    "A fingerprint of the serial number, not the serial itself",
                                );
                            ui.end_row();
                        });
                }
                Some(Err(e)) => {
                    ui.colored_label(crate::theme::ACCENT_RED, e.message.as_str());
                    if let Some(hint) = e.hint {
                        ui.label(hint);
                    }
                }
            });
        self.calc_status_open = open;
    }
}
