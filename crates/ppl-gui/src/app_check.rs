//! Explicit checks use the shared diagnostic backend, off the GUI thread.
//!
//! The original-firmware Check prefers the direct checker (one warm local
//! firmware session calling the firmware compiler directly, about 10 ms);
//! until it is ready, or whenever it reaches no verdict, the fresh checker
//! (one firmware boot per Check, about 7 s) gives the verdict instead.
use super::*;
use ppl_engine::{checking, source::Source};
#[cfg(target_os = "linux")]
use std::sync::{Arc, Mutex};
pub(super) struct CheckJob {
    pub rx: std::sync::mpsc::Receiver<(TabId, u64, String, CheckOutcome)>,
    pub cancel: ppl_engine::Cancellation,
}
pub(super) enum CheckOutcome {
    /// Native analysis plus a fresh-checker firmware Check; `direct_error`
    /// says why the direct checker was not used, when it was tried.
    Fresh {
        result: Box<Result<checking::CheckReport, String>>,
        direct_error: Option<String>,
    },
    /// A direct original-firmware compiler call.
    #[cfg(target_os = "linux")]
    Direct {
        verdict: ppl_engine::session::CheckVerdict,
        elapsed_ms: f64,
    },
}
/// The direct checker's lifecycle (shared with its starter thread).
#[cfg(target_os = "linux")]
#[derive(Default)]
pub(super) enum DirectState {
    #[default]
    Idle,
    Starting,
    #[cfg(target_os = "linux")]
    Ready(Arc<ppl_engine::direct_check::DirectChecker>),
    /// Start failed; the reason is reported once, then `Disabled`.
    Unavailable(String),
    Disabled,
}
#[cfg(target_os = "linux")]
pub(super) type DirectSlot = Arc<Mutex<DirectState>>;
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
        let direct = self.direct_checker(options.as_ref(), ctx);
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = ppl_engine::Cancellation::default();
        let signal = cancel.clone();
        let ctx = ctx.clone();
        self.explicit_check = Some(CheckJob { rx, cancel });
        std::thread::spawn(move || {
            #[cfg(target_os = "linux")]
            let direct_error = match direct {
                Some(checker) => {
                    let started = std::time::Instant::now();
                    match checker.check(&text, std::time::Duration::from_secs(30), &signal) {
                        Ok(verdict) => {
                            let outcome = CheckOutcome::Direct {
                                verdict,
                                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                            };
                            let _ = tx.send((id, revision, title, outcome));
                            ctx.request_repaint();
                            return;
                        }
                        Err(_) if signal.cancelled() => return,
                        Err(e) => Some(e),
                    }
                }
                None => None,
            };
            #[cfg(not(target_os = "linux"))]
            let direct_error: Option<String> = direct;
            let result = Source::text(&text, &title)
                .map(|source| checking::check(source, options.as_ref(), &signal));
            let outcome = CheckOutcome::Fresh {
                result: Box::new(result),
                direct_error,
            };
            let _ = tx.send((id, revision, title, outcome));
            ctx.request_repaint();
        });
    }

    /// The warm direct checker, if ready. Otherwise starts it in the
    /// background when the firmware and initialized data are configured.
    #[cfg(target_os = "linux")]
    fn direct_checker(
        &mut self,
        options: Option<&ppl_engine::firmware::Options>,
        ctx: &egui::Context,
    ) -> Option<Arc<ppl_engine::direct_check::DirectChecker>> {
        let o = options.filter(|o| o.firmware.is_some() && o.init_data.is_some())?;
        let mut state = self.direct.lock().unwrap();
        match &*state {
            DirectState::Ready(c) => return Some(c.clone()),
            DirectState::Unavailable(e) => {
                let e = e.clone();
                *state = DirectState::Disabled;
                drop(state);
                self.console.print_diagnostic(format!(
                    "Fast firmware Check unavailable ({e}); using the fresh checker"
                ));
                return None;
            }
            DirectState::Starting | DirectState::Disabled => return None,
            DirectState::Idle => {}
        }
        *state = DirectState::Starting;
        let (slot, o, ctx) = (self.direct.clone(), o.clone(), ctx.clone());
        std::thread::spawn(move || {
            let next = match ppl_engine::direct_check::DirectChecker::start(&o) {
                Ok(c) => DirectState::Ready(Arc::new(c)),
                Err(e) => DirectState::Unavailable(e),
            };
            *slot.lock().unwrap() = next;
            ctx.request_repaint();
        });
        None
    }
    #[cfg(not(target_os = "linux"))]
    fn direct_checker(
        &mut self,
        _options: Option<&ppl_engine::firmware::Options>,
        _ctx: &egui::Context,
    ) -> Option<String> {
        None
    }
    pub(super) fn poll_checked_source(&mut self) {
        let result = self
            .explicit_check
            .as_ref()
            .and_then(|job| job.rx.try_recv().ok());
        let Some((id, revision, title, outcome)) = result else {
            return;
        };
        self.explicit_check = None;
        match outcome {
            CheckOutcome::Fresh {
                result,
                direct_error,
            } => {
                if let Some(e) = direct_error {
                    self.console.print_diagnostic(format!(
                        "Direct firmware Check reached no verdict ({e}); used the fresh checker"
                    ));
                }
                self.apply_firmware_check(id, revision, &title, *result);
            }
            #[cfg(target_os = "linux")]
            CheckOutcome::Direct {
                verdict,
                elapsed_ms,
            } => self.apply_direct_check(id, revision, &title, verdict, elapsed_ms),
        }
    }

    /// Records a direct original-firmware Check of `revision`.
    #[cfg(any(target_os = "linux", test))]
    pub(super) fn apply_direct_check(
        &mut self,
        id: TabId,
        revision: u64,
        title: &str,
        verdict: ppl_engine::session::CheckVerdict,
        elapsed_ms: f64,
    ) {
        if !self.tab_at_revision(id, revision) {
            return;
        }
        self.console.print_diagnostic(format!(
            "Original firmware Check ({}, direct compiler call, {elapsed_ms:.0} ms); \
             physical Check: not performed",
            if verdict.accepted {
                "accepted"
            } else {
                "rejected"
            },
        ));
        self.record_firmware_analysis(id, revision, title, &verdict.diagnostics, None);
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
