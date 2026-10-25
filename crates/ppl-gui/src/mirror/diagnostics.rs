//! Opt-in streaming evidence. No report/image history is retained in memory.
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{CapturedFrame, MirrorQuality, MirrorWorkerHandle, SourceFrame, UsbMirrorSource};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
fn row(file: &mut File, value: serde_json::Value) -> io::Result<()> {
    if file.metadata()?.len() >= 16 * 1024 * 1024 {
        return Err(io::Error::other("diagnostic log reached 16 MiB limit"));
    }
    serde_json::to_writer(&mut *file, &value)?;
    file.write_all(b"\n")
}

pub(super) struct CaptureDiagnostics {
    file: File,
    directory: PathBuf,
    start: Instant,
    previous: Option<Instant>,
    saved: bool,
}

impl CaptureDiagnostics {
    pub fn open(directory: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            file: File::options()
                .write(true)
                .create_new(true)
                .open(directory.join("captures.jsonl"))?,
            directory: directory.into(),
            start: Instant::now(),
            previous: None,
            saved: false,
        })
    }

    pub fn record(
        &mut self,
        seq: u64,
        frame: &SourceFrame,
        publication: Duration,
        changed: bool,
        requests: Option<u64>,
    ) -> io::Result<()> {
        let evidence = Instant::now();
        if !self.saved {
            if let Some(raw) = &frame.raw {
                std::fs::write(self.directory.join("source.raw.png"), raw)?;
            }
            frame.grob.save_png(&self.directory.join("decoded.png"))?;
            self.saved = true;
        }
        let u = &frame.usb;
        let update = frame.update.as_ref().map(|u|json!({"format":u.format,"origin":u.origin,"width":u.dimensions.0,"height":u.dimensions.1,"baseline":u.baseline,"composition_ms":ms(u.composition)}));
        let completed = frame.completed;
        let period = self
            .previous
            .map(|t| ms(completed.saturating_duration_since(t)));
        self.previous = Some(completed);
        row(
            &mut self.file,
            json!({
                "kind":"capture", "requests_attempted":requests, "incremental_update":update, "source_kind":if frame.raw.is_some() { "usb-png" } else { "synthetic" }, "seq":seq, "at_ms":ms(completed.saturating_duration_since(self.start)),
                "period_ms":period, "width":frame.grob.width(), "height":frame.grob.height(),
                "png_bytes":u.png_bytes, "reports":u.reports, "report_bytes":u.report_bytes,
                "setup_ms":ms(u.setup), "write_ms":ms(u.write), "first_response_ms":ms(u.first_response),
                "transfer_ms":ms(u.transfer), "assembly_ms":ms(u.assembly), "extraction_ms":ms(u.extraction),
                "usb_total_ms":ms(u.total), "decompression_ms":ms(frame.decode.decompression),
                "conversion_ms":ms(frame.decode.conversion), "publication_ms":ms(publication),
                "total_ms":ms(u.total + frame.decode.decompression + frame.decode.conversion + frame.update.as_ref().map_or(Duration::ZERO, |u|u.composition) + publication),
                "changed":changed, "evidence_ms":ms(evidence.elapsed())
            }),
        )
    }

    pub fn error(&mut self, stage: &str, error: &str, requests: Option<u64>) -> io::Result<()> {
        row(
            &mut self.file,
            json!({"kind":"error", "requests_attempted":requests, "stage":stage, "at_ms":ms(self.start.elapsed()), "error":error}),
        )
    }
}

pub(super) struct DisplayDiagnostics {
    start: Instant,
    file: File,
    directory: PathBuf,
    saved: bool,
}

impl DisplayDiagnostics {
    pub fn open(directory: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            start: Instant::now(),
            file: File::options()
                .write(true)
                .create_new(true)
                .open(directory.join("presentations.jsonl"))?,
            directory: directory.into(),
            saved: false,
        })
    }

    pub fn notified(&mut self, frame: &CapturedFrame) -> io::Result<()> {
        row(
            &mut self.file,
            json!({"kind":"gui_notification", "seq":frame.seq,
            "at_ms":ms(self.start.elapsed()), "publication_to_notification_ms":ms(frame.published.elapsed())}),
        )
    }

    pub fn surface(
        &mut self,
        frame: &CapturedFrame,
        pixels: &[u32],
        width: u32,
        height: u32,
    ) -> io::Result<()> {
        if !self.saved {
            frame
                .grob
                .save_png(&self.directory.join("renderer-source.png"))?;
            let mut g = ppl_graphics::Grob::new(width as usize, height as usize, 0);
            g.pixels_mut().copy_from_slice(pixels);
            g.save_png(&self.directory.join("surface.png"))?;
            self.saved = true;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn presented(
        &mut self,
        frame: &CapturedFrame,
        scale: Duration,
        present: Duration,
        age: Duration,
        width: u32,
        height: u32,
        dpi: f64,
        unique: bool,
    ) -> io::Result<()> {
        row(
            &mut self.file,
            json!({"kind":"presentation_submission", "seq":frame.seq, "at_ms": ms(self.start.elapsed()),
            "scaling_ms":ms(scale), "present_ms":ms(present), "request_age_ms":ms(age),
            "width":width, "height":height, "dpi":dpi, "unique":unique}),
        )
    }
}

/// Bounded read-only hardware benchmark using the exact worker/capture path.
/// No GUI metrics are claimed by this headless diagnostic.
pub fn capture_diagnostics(
    prime: ppl_device::Prime,
    quality: MirrorQuality,
    count: u64,
    directory: &Path,
) -> io::Result<super::MirrorMetrics> {
    if count == 0 {
        return Err(io::Error::other("capture count must be positive"));
    }
    let source = UsbMirrorSource::new(prime).with_quality(quality);
    let mut worker =
        MirrorWorkerHandle::spawn_configured(source, 10.0, None, Some(directory), Some(count))?;
    // A disconnected device must not turn an unattended diagnostic into an
    // infinite reconnect loop. Normal interactive mirroring has no such limit.
    let deadline = Instant::now() + Duration::from_secs(count.saturating_mul(7).saturating_add(15));
    while !worker.is_finished() && Instant::now() < deadline {
        if worker.state.metrics.lock().unwrap().errors > 0 {
            worker.stop();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    worker.join();
    let metrics = worker.state.metrics.lock().unwrap().clone();
    if metrics.total_captured < count {
        return Err(io::Error::other(
            "hardware capture diagnostic did not complete; see captures.jsonl",
        ));
    }
    Ok(metrics)
}
