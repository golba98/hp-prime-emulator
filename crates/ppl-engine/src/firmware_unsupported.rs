//! Native execution remains available where Linux process supervision is absent.
pub use crate::firmware_options::Options;
use crate::{Cancellation, Engine, Operation, Report, Status, source::Source};
use std::path::{Path, PathBuf};
pub fn discover_runner(_explicit: Option<&Path>) -> Result<PathBuf, String> {
    Err("the original-firmware backend currently requires Linux".into())
}
pub fn execute(
    source: Source,
    operation: Operation,
    options: &Options,
    cancel: &Cancellation,
) -> Report {
    let mut report = Report::new(Engine::Firmware, operation);
    report.program = source.stem.clone();
    report.source = Some(source.identity);
    report.requested_entry = options.entry.clone();
    if cancel.cancelled() {
        report.fail(Status::Cancelled, "request", "cancelled before execution");
    } else {
        report.fail(
            Status::Unsupported,
            "platform",
            "the original-firmware backend currently requires Linux; native mode remains available",
        );
    }
    report
}
