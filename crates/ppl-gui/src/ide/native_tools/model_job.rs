//! Read-only model plots may be requested by a running PPL program. They must
//! not reserve a second execution owner or perform evaluation in the UI frame.
use crate::ide::native_app_ui::Calculation;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
pub(super) struct ModelPlotJob {
    pub name: String,
    pub source: serde_json::Value,
    pub generation: u64,
    rx: mpsc::Receiver<Result<serde_json::Value, String>>,
    cancel: Arc<AtomicBool>,
}
impl ModelPlotJob {
    pub fn start(
        name: String,
        model: crate::ide::native_app_ui::AppMemory,
        snapshot: ppl_vm::SessionSnapshot,
    ) -> Self {
        let source = model.input_fingerprint();
        let generation = snapshot.generation;
        let operation = Calculation::Plot(name.clone(), Box::new(model));
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(operation.calculate(flag, &snapshot));
        });
        Self {
            name,
            source,
            generation,
            rx,
            cancel,
        }
    }
    pub fn poll(&self) -> Option<Result<serde_json::Value, String>> {
        match self.rx.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                "Calculator model plot worker exited without a result".into(),
            )),
        }
    }
}
impl Drop for ModelPlotJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
