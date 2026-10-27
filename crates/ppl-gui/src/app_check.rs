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
        self.apply_firmware_check(id, revision, &title, result);
    }
    /// Records a fresh-checker result for `revision` on its tab; dropped
    /// when the tab is gone or its text has changed since.
    pub(super) fn apply_firmware_check(
        &mut self,
        id: TabId,
        revision: u64,
        title: &str,
        result: Result<checking::CheckReport, String>,
    ) {
        if !self.tab_at_revision(id, revision) {
            return;
        }
        match result {
            Ok(report) => {
                self.console.print_diagnostic(format!(
                    "Native check: {:?}; local firmware: {}; physical Check: not performed",
                    report.native.status, report.firmware_status
                ));
                if let Some(firmware) = report.firmware {
                    // Only success and invalid_source are firmware verdicts.
                    let failure = match firmware.status {
                        ppl_engine::Status::Success | ppl_engine::Status::InvalidSource => None,
                        status => Some(
                            firmware
                                .error
                                .as_ref()
                                .map(|e| e.message.clone())
                                .unwrap_or_else(|| format!("{status:?}")),
                        ),
                    };
                    self.record_firmware_analysis(
                        id,
                        revision,
                        title,
                        &firmware.diagnostics,
                        failure,
                    );
                } else if let Some(i) = self.editor.index_of(id) {
                    self.editor.tabs[i].firmware = None;
                }
            }
            Err(error) => self
                .console
                .print_error(format!("Source not checked: {error}")),
        }
        self.rebuild_problems();
    }

    /// A Check is still running: the fresh checker, a native compile, or a
    /// firmware session request.
    pub(crate) fn check_pending(&self) -> bool {
        self.explicit_check.is_some()
            || self.native_compile.is_some()
            || !self.firmware_pending.is_empty()
    }

    fn tab_at_revision(&mut self, id: TabId, revision: u64) -> bool {
        match self.editor.index_of(id) {
            Some(i) => self.editor.tabs[i].revision() == revision,
            None => false,
        }
    }

    /// Stores a firmware verdict for `revision` of tab `id`, from either the
    /// fresh checker or the live session. `failure` means no verdict was
    /// reached; empty diagnostics without a failure mean the Check passed.
    pub(super) fn record_firmware_analysis(
        &mut self,
        id: TabId,
        revision: u64,
        title: &str,
        diagnostics: &[ppl_engine::Diagnostic],
        failure: Option<String>,
    ) {
        if !self.tab_at_revision(id, revision) {
            return;
        }
        if let Some(error) = &failure {
            self.console
                .print_error(format!("Local firmware checker: {error}"));
        }
        let Some(index) = self.editor.index_of(id) else {
            return;
        };
        let tab = &mut self.editor.tabs[index];
        let mut items: Vec<DiagnosticItem> = diagnostics
            .iter()
            .map(|d| {
                let mut item = DiagnosticItem::from_engine(d, title, &tab.content);
                item.tab = Some(id);
                item
            })
            .collect();
        items.sort_by_key(|i| (i.line, i.col));
        tab.firmware = Some(crate::ide::editor::TabAnalysis {
            revision,
            items,
            failure,
        });
        self.rebuild_problems();
    }
}
