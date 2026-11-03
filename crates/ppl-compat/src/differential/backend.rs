use super::*;
use ppl_engine::{Cancellation, firmware_options::Options};

/// Minimal extension seam. Backends own their scheduler/session details;
/// the comparator never assumes shared memory, USB, or an emulator kernel.
pub trait Backend {
    fn id(&self) -> BackendId;
    fn execute(&self, scenario: &LoadedScenario, cancel: &Cancellation) -> BackendRun;
}
pub struct Native;
impl Backend for Native {
    fn id(&self) -> BackendId {
        BackendId::Native
    }
    fn execute(&self, s: &LoadedScenario, c: &Cancellation) -> BackendRun {
        run_native(s, c)
    }
}
pub struct FirmwareHle {
    pub options: Options,
}
impl Backend for FirmwareHle {
    fn id(&self) -> BackendId {
        BackendId::FirmwareHle
    }
    fn execute(&self, s: &LoadedScenario, c: &Cancellation) -> BackendRun {
        run_firmware(s, &self.options, c)
    }
}
pub struct Unavailable {
    pub backend: BackendId,
    pub reason: String,
}
impl Backend for Unavailable {
    fn id(&self) -> BackendId {
        self.backend
    }
    fn execute(&self, s: &LoadedScenario, _: &Cancellation) -> BackendRun {
        BackendRun::unavailable(self.backend, s, &self.reason)
    }
}
pub fn execute(s: &LoadedScenario, backends: &[&dyn Backend], cancel: &Cancellation) -> Comparison {
    let runs = backends
        .iter()
        .map(|backend| {
            let mut run = backend.execute(s, cancel);
            if run.backend != backend.id() {
                run.backend = backend.id();
                run.status = RunStatus::InfrastructureError;
                run.reason = Some("backend returned another backend's identity".into());
            }
            run
        })
        .collect();
    compare(s, runs)
}
