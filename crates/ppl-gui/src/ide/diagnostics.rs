//! Editor diagnostics: conversion from compiler and runtime errors, and a
//! background checker so live diagnostics never block typing.
//!
//! Classes are kept strictly apart (see [`DiagnosticClass`]): a syntax or
//! semantic error means the source is wrong; a compatibility diagnostic
//! means valid HP Prime code the emulator cannot fully run; a runtime
//! error comes from executing the program.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ppl_compiler::{CompileOptions, compile_source};
use ppl_syntax::source::{SourceFile, Span};
use ppl_syntax::{Diagnostic, DiagnosticClass, Severity};

use super::editor::TabId;

/// One problem shown in the editor and the Problems panel.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticItem {
    pub severity: Severity,
    pub class: DiagnosticClass,
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
    /// The rule is an emulator heuristic not yet checked on a calculator.
    pub unverified: bool,
    /// 1-based line and column (columns count characters).
    pub line: usize,
    pub col: usize,
    pub span: Span,
    /// File (tab title) the problem belongs to.
    pub file: String,
    /// Tab the problem belongs to, when known.
    pub tab: Option<TabId>,
}

impl DiagnosticItem {
    pub fn from_diagnostic(d: &Diagnostic, file: &SourceFile) -> Self {
        let lc = file.line_col(d.span.start);
        DiagnosticItem {
            severity: d.severity,
            class: d.class(),
            code: d.code.to_string(),
            message: d.message.clone(),
            hint: d.hint.clone(),
            unverified: d.unverified,
            line: lc.line as usize,
            col: lc.col as usize,
            span: d.span,
            file: file.name.clone(),
            tab: None,
        }
    }

    /// A runtime error, located at the innermost frame of its trace.
    pub fn runtime(err: &ppl_vm::RuntimeError, file: &SourceFile) -> Self {
        let span = err.trace.first().map(|f| f.span).unwrap_or_default();
        let lc = file.line_col(span.start);
        DiagnosticItem {
            severity: Severity::Error,
            class: DiagnosticClass::Runtime,
            code: "runtime-error".into(),
            message: err.to_string(),
            hint: None,
            unverified: false,
            line: lc.line as usize,
            col: lc.col as usize,
            span,
            file: file.name.clone(),
            tab: None,
        }
    }

    pub fn severity_label(&self) -> &'static str {
        match self.severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
            Severity::Note => "Note",
        }
    }
}

/// Parses and compiles `text`, returning every diagnostic as an item.
pub fn check_source(title: &str, text: &str) -> Vec<DiagnosticItem> {
    let file = SourceFile::new(title, text);
    let (_, diags) = compile_source(&file, &CompileOptions::default());
    diags
        .items
        .iter()
        .map(|d| DiagnosticItem::from_diagnostic(d, &file))
        .collect()
}

struct Request {
    tab: TabId,
    revision: u64,
    title: String,
    text: String,
}

/// Diagnostics computed for one tab revision.
#[derive(Debug)]
pub struct CheckResult {
    pub tab: TabId,
    pub revision: u64,
    pub items: Vec<DiagnosticItem>,
    pub elapsed: Duration,
}

/// A background thread that compiles buffers for diagnostics. Requests for
/// the same tab coalesce: only the newest revision is checked.
pub struct DiagnosticsWorker {
    tx: Option<Sender<Request>>,
    rx: Receiver<CheckResult>,
    handle: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for DiagnosticsWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiagnosticsWorker").finish_non_exhaustive()
    }
}

impl DiagnosticsWorker {
    /// Starts the worker. `on_result` runs on the worker thread after each
    /// result is sent (the IDE uses it to request a repaint).
    pub fn new(on_result: Option<Box<dyn Fn() + Send>>) -> Self {
        let (tx, req_rx) = channel::<Request>();
        let (res_tx, rx) = channel::<CheckResult>();
        let handle = std::thread::Builder::new()
            .name("ppl-diagnostics".into())
            .spawn(move || {
                while let Ok(first) = req_rx.recv() {
                    let mut pending = vec![first];
                    pending.extend(req_rx.try_iter());
                    // Keep only the newest request per tab.
                    let mut latest: Vec<Request> = Vec::new();
                    for r in pending {
                        match latest.iter_mut().find(|l| l.tab == r.tab) {
                            Some(l) if l.revision <= r.revision => *l = r,
                            Some(_) => {}
                            None => latest.push(r),
                        }
                    }
                    for r in latest {
                        let start = Instant::now();
                        let items = check_source(&r.title, &r.text);
                        let result = CheckResult {
                            tab: r.tab,
                            revision: r.revision,
                            items,
                            elapsed: start.elapsed(),
                        };
                        if res_tx.send(result).is_err() {
                            return;
                        }
                        if let Some(cb) = &on_result {
                            cb();
                        }
                    }
                }
            })
            .ok();
        DiagnosticsWorker {
            tx: Some(tx),
            rx,
            handle,
        }
    }

    /// Queues a check of `text` (revision `revision` of `tab`).
    pub fn request(&self, tab: TabId, revision: u64, title: &str, text: &str) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Request {
                tab,
                revision,
                title: title.to_string(),
                text: text.to_string(),
            });
        }
    }

    /// Results that arrived since the last call.
    pub fn poll(&self) -> Vec<CheckResult> {
        self.rx.try_iter().collect()
    }
}

impl Drop for DiagnosticsWorker {
    fn drop(&mut self) {
        // Closing the channel ends the worker loop.
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_carry_class_position_and_hint() {
        let items = check_source(
            "T.ppl",
            "EXPORT T2()\nBEGIN\n  RETURN SOLVE(X^2-1, X, 2) + y;\nEND;\n",
        );
        let compat = items
            .iter()
            .find(|i| i.class == DiagnosticClass::Compatibility)
            .unwrap();
        assert_eq!((compat.line, compat.col), (3, 10));
        assert_eq!(compat.severity, Severity::Warning);
        assert!(compat.hint.is_some());
        let undeclared = items
            .iter()
            .find(|i| i.code == "undeclared-identifier")
            .unwrap();
        assert_eq!(undeclared.class, DiagnosticClass::Semantic);
        assert_eq!(undeclared.file, "T.ppl");
    }

    #[test]
    fn worker_coalesces_and_returns_latest_revision() {
        let w = DiagnosticsWorker::new(None);
        let tab = TabId(9_999);
        for rev in 1..=5 {
            let text = format!("EXPORT P()\nBEGIN\n  RETURN {};\nEND;\n", "x".repeat(rev));
            w.request(tab, rev as u64, "P.ppl", &text);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut last = None;
        while Instant::now() < deadline {
            for r in w.poll() {
                last = Some(r);
            }
            if last.as_ref().is_some_and(|r| r.revision == 5) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let r = last.expect("a result");
        assert_eq!(r.revision, 5);
        assert!(r.items.iter().any(|i| i.message.contains("xxxxx")));
    }
}
