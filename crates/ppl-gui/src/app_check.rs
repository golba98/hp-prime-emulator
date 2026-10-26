//! Explicit checks use the shared diagnostic backend, off the GUI thread.
use super::*;
use ppl_engine::{checking, source::Source};
pub(super) struct CheckJob {
    pub rx: std::sync::mpsc::Receiver<(TabId, u64, String, Result<checking::CheckReport, String>)>,
    pub cancel: ppl_engine::Cancellation,
}
impl Drop for CheckJob {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl IdeDesktopApp {
    pub(super) fn request_checked_source(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.editor.active_tab_mut() else {
            return;
        };
        let (id, revision, title, text) = (
            tab.id,
            tab.revision(),
            tab.title.clone(),
            tab.content.clone(),
        );
        let mut options = checking::configured_firmware();
        let settings = &self.state.firmware;
        if self.state.firmware_backend || !settings.firmware.is_empty() {
            let mut config = options.unwrap_or_default();
            if !settings.firmware.is_empty() {
                config.firmware = Some(settings.firmware.clone().into());
            }
            if !settings.factory_a.is_empty() {
                config.factory_a = Some(settings.factory_a.clone().into());
            }
            if !settings.runner.is_empty() {
                config.runner = Some(settings.runner.clone().into());
            }
            options = Some(config);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = ppl_engine::Cancellation::default();
        let signal = cancel.clone();
        let ctx = ctx.clone();
        self.explicit_check = Some(CheckJob { rx, cancel });
        std::thread::spawn(move || {
            let result = Source::text(&text, &title)
                .map(|source| checking::check(source, options.as_ref(), &signal));
            let _ = tx.send((id, revision, title, result));
            ctx.request_repaint();
        });
    }
    pub(super) fn poll_checked_source(&mut self) {
        let result = self
            .explicit_check
            .as_ref()
            .and_then(|job| job.rx.try_recv().ok());
        let Some((id, revision, title, result)) = result else {
            return;
        };
        self.explicit_check = None;
        let Some(index) = self.editor.index_of(id) else {
            return;
        };
        if self.editor.tabs[index].revision() != revision {
            return;
        }
        self.firmware_problems
            .retain(|(item, _)| item.tab != Some(id));
        match result {
            Ok(report) => {
                self.console.print_diagnostic(format!(
                    "Native check: {:?}; local firmware: {}; physical Check: not performed",
                    report.native.status, report.firmware_status
                ));
                if let Some(firmware) = report.firmware {
                    for diagnostic in &firmware.diagnostics {
                        let mut item = DiagnosticItem::from_engine(diagnostic, &title);
                        item.tab = Some(id);
                        self.firmware_problems.push((item, revision));
                    }
                    if let Some(error) = firmware.error {
                        self.console
                            .print_error(format!("Local firmware checker: {}", error.message));
                    }
                }
            }
            Err(error) => self
                .console
                .print_error(format!("Source not checked: {error}")),
        }
        self.rebuild_problems();
    }
}
