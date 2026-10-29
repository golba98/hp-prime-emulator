//! Shared program management. Frontends supply explicit authorization; this
//! library never injects keys, retries mutations, or assumes inventory completeness.
pub mod delete_store;
pub mod deletion;
mod legacy_no_send;
pub mod recovery;
pub mod replacement;
pub mod service;
use ppl_device::{DeviceError, Prime, ReadBack};
pub use ppl_engine::source::Source;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub type Result<T> = std::result::Result<T, DeviceError>;
pub fn refused(message: impl Into<String>) -> DeviceError {
    DeviceError::Refused(message.into())
}
pub fn disk(error: impl std::fmt::Display) -> DeviceError {
    DeviceError::Hid(error.to_string())
}

/// Journal and backup directory shared by every frontend, so an operation
/// started in the IDE can be verified or recovered from the CLI and back.
pub const DEFAULT_STORE_DIR: &str = "artifacts/delete";

/// Plain-language advice for a device error, shared by the CLI and the IDE.
/// `None` when the error message already says everything useful.
pub fn user_hint(error: &DeviceError) -> Option<&'static str> {
    match error {
        DeviceError::NotFound => Some(
            "Connect the calculator with a USB data cable and switch it on. \
             No HP Prime was found on USB.",
        ),
        DeviceError::Inconclusive(_) | DeviceError::Timeout(_) => Some(
            "The calculator did not answer clearly, so nothing was written. Make sure it is on, \
             shows the Home screen, is not busy or in exam mode, and that no other program \
             (e.g. Connectivity Kit) holds the USB connection, then retry.",
        ),
        DeviceError::Busy(_) => Some(
            "Another program or operation is using the calculator. Close Connectivity Kit or \
             other ppl device commands, then retry.",
        ),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub serial_id: String,
    pub usb_pid: u16,
    pub firmware_build: Option<u32>,
    pub boot_version: Option<String>,
}
impl DeviceIdentity {
    pub fn read(prime: &mut Prime) -> Result<Self> {
        if prime.detected.vendor_id != 0x03f0 {
            return Err(refused("device vendor is not HP"));
        }
        let start = prime.log.len();
        let info = prime.info();
        // Preserve drained/unmatched evidence: truncating the entire identity
        // exchange would erase late object replies and defeat write safeguards.
        let mut remaining = 0usize;
        for exchange in &mut prime.log[start..] {
            if exchange.outgoing || exchange.drained {
                continue;
            }
            if exchange.bytes.first() == Some(&0)
                && exchange.bytes.get(1) == Some(&0xfa)
                && exchange.bytes.len() >= 7
            {
                remaining =
                    u32::from_be_bytes(exchange.bytes[3..7].try_into().unwrap()) as usize + 6;
            }
            if remaining > 0 {
                let consumed = exchange.bytes.len().saturating_sub(1);
                exchange.bytes[1..].fill(0); // Raw serial is private; sequence/evidence remain.
                remaining = remaining.saturating_sub(consumed);
            }
        }
        let info = info?;
        Ok(Self {
            serial_id: ppl_compat::serial_fingerprint(
                info.serial
                    .as_deref()
                    .ok_or_else(|| refused("device serial unavailable"))?,
            ),
            usb_pid: prime.detected.product_id,
            firmware_build: info.build,
            boot_version: info.version,
        })
    }
    pub fn require_reference(&self) -> Result<()> {
        if self.usb_pid != 0x1541
            || self.firmware_build != Some(15157)
            || self.boot_version.as_deref() != Some("V0.050.640")
        {
            return Err(refused(
                "program mutation requires the evidenced G1 build 15157 profile",
            ));
        }
        Ok(())
    }
    pub fn verify(&self, prime: &mut Prime) -> Result<()> {
        if *self != Self::read(prime)? {
            return Err(refused("device identity changed"));
        }
        prime.require_healthy()
    }
}

/// Common selector for every frontend. The advisory lock is held by Prime.
pub fn open_selected(selected: Option<&str>) -> Result<Prime> {
    let devices = ppl_device::detect()?;
    if selected.is_none() {
        if devices.len() > 1 {
            return Err(refused(
                "multiple calculators match; pass --device <serial-id>",
            ));
        }
        return Prime::open(devices.into_iter().next().ok_or(DeviceError::NotFound)?);
    }
    let mut matched = None;
    for device in devices {
        let mut prime = Prime::open(device)?;
        if Some(DeviceIdentity::read(&mut prime)?.serial_id.as_str()) == selected {
            if matched.is_some() {
                return Err(refused("multiple calculators share the requested identity"));
            }
            matched = Some(prime);
        }
    }
    matched.ok_or_else(|| refused("requested device serial-id was not found"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EncodedSize {
    pub file_bytes: usize,
    pub utf16_units: usize,
    pub payload_bytes: usize,
    pub object_message_bytes: usize,
    pub hid_reports: usize,
    pub host_report_bytes: usize,
    pub limit_bytes: usize,
    pub limit_kind: String,
    pub supported: bool,
}
#[derive(Clone, Debug)]
pub struct ProgramImage {
    pub name: String,
    pub source: Source,
    pub sizes: EncodedSize,
}
impl ProgramImage {
    pub fn file(name: &str, path: &Path) -> Result<Self> {
        Self::new(name, Source::file(path).map_err(refused)?)
    }
    pub fn new(name: &str, source: Source) -> Result<Self> {
        ppl_device::validate_program_name(name)?;
        let units = source.original_text.encode_utf16().count();
        let payload = units
            .checked_mul(2)
            .ok_or_else(|| refused("source size overflow"))?;
        let message = 10 + name.encode_utf16().count() * 2 + payload;
        let reports = message.div_ceil(63);
        Ok(Self {
            name: name.into(),
            sizes: EncodedSize {
                file_bytes: source.bytes.len(),
                utf16_units: units,
                payload_bytes: payload,
                object_message_bytes: message,
                hid_reports: reports,
                host_report_bytes: message + 2 * reports,
                limit_bytes: ppl_device::MAX_PROGRAM_PAYLOAD,
                limit_kind:
                    "conservative_observed_g1_15157_legacy_payload_ceiling_cause_unresolved".into(),
                supported: payload <= ppl_device::MAX_PROGRAM_PAYLOAD,
            },
            source,
        })
    }
    pub fn require_supported(&self) -> Result<()> {
        ppl_device::check_program_payload(&self.name, &self.source.original_text).map(|_| ())
    }
    pub fn payload(&self) -> Vec<u8> {
        self.source
            .original_text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadResult {
    pub operation_id: Option<String>,
    pub state: String,
    pub name: String,
    pub device: DeviceIdentity,
    pub payload_sha256: String,
    pub stored_sha256: Option<String>,
    pub comparison: Option<String>,
    pub error: Option<String>,
}
/// Creates only a proven-free name. Caller must persist submission intent first.
/// Any error after entry must be treated as potentially submitted by the journal.
pub fn upload_new(
    prime: &mut Prime,
    device: &DeviceIdentity,
    image: &ProgramImage,
) -> Result<UploadResult> {
    image.require_supported()?;
    device.require_reference()?;
    device.verify(prime)?;
    let control =
        ppl_device::ControlObject::parse(ppl_device::DEFAULT_SETTINGS_ANCHOR).map_err(refused)?;
    let note = ppl_device::result_note_name(&image.name);
    let guard = if image.name.starts_with(ppl_device::SAFE_PREFIX) {
        prime.preflight_new_names(
            &[
                (ppl_device::FileType::Program, &image.name),
                (ppl_device::FileType::Note, &note),
            ],
            Some(&control),
        )?
    } else {
        prime.preflight_new_user_program(&image.name, Some(&control))?
    };
    let report = prime.send_new_program(guard, &image.source.original_text)?;
    let mut outcome = UploadResult {
        operation_id: None,
        state: "indeterminate".into(),
        name: image.name.clone(),
        device: device.clone(),
        payload_sha256: ppl_compat::sha256_hex(&image.payload()),
        stored_sha256: None,
        comparison: None,
        error: None,
    };
    if report.readback != ReadBack::Matches {
        if matches!(report.readback, ReadBack::Differs { .. }) {
            outcome.state = "verification_failed".into();
        }
        outcome.error = Some(format!("upload readback: {:?}", report.readback));
        return Ok(outcome);
    }
    device.verify(prime)?;
    let stored = prime.read_program_exact(&image.name)?;
    outcome.stored_sha256 = Some(ppl_compat::sha256_hex(&stored.data));
    match compare_source(&stored.data, &image.source.original_text)? {
        Some(comparison) => {
            outcome.state = "upload_verified".into();
            outcome.comparison = Some(comparison.into());
        }
        None => {
            outcome.state = "verification_failed".into();
            outcome.error = Some("stored source differs; object retained".into());
        }
    }
    Ok(outcome)
}

/// Only observed G1 representation differences are accepted. Whitespace,
/// Unicode, statement boundaries, and extra NULs are never silently discarded.
pub fn compare_source(stored: &[u8], expected: &str) -> Result<Option<&'static str>> {
    let text = ppl_device::program::utf16(stored)?;
    if text == expected {
        return Ok(Some("exact_utf16le"));
    }
    let text = text.strip_suffix('\0').unwrap_or(&text);
    if text == expected {
        return Ok(Some("one_device_nul_terminator"));
    }
    if text.replace("\r\n", "\n") == expected.replace("\r\n", "\n") {
        return Ok(Some("crlf_to_lf_and_optional_device_terminator"));
    }
    Ok(None)
}

/// Durable create-only transaction used by both frontends. Existing unresolved
/// uploads and deletions block further writes to the same device/name.
pub fn create_new(
    prime: &mut Prime,
    image: &ProgramImage,
    store: &delete_store::Store,
) -> Result<UploadResult> {
    image.require_supported()?;
    let _lock = store.lock()?;
    let device = DeviceIdentity::read(prime)?;
    device.require_reference()?;
    store.reject_pending(
        &device.serial_id,
        &[ppl_device::DeleteTarget::program(&image.name)],
    )?;
    let root = store.root.join("uploads");
    std::fs::create_dir_all(&root).map_err(disk)?;
    let id = ppl_device::fresh_test_name();
    let path = root.join(format!("{id}.json"));
    use std::io::Write;
    let mut snapshot = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(format!("{id}.utf16le")))
        .map_err(disk)?;
    snapshot
        .write_all(&image.payload())
        .and_then(|_| snapshot.sync_all())
        .map_err(disk)?;
    let mut record = serde_json::json!({"schema_version":1,"operation_id":id,"operation":"create_program",
        "device":device,"name":image.name,"payload_sha256":ppl_compat::sha256_hex(&image.payload()),
        "state":"submitting","sizes":image.sizes,"requested_utc":ppl_compat::utc_now(),"error":null});
    delete_store::atomic_json(&path, &record).map_err(disk)?;
    let start = prime.log.len();
    let mut result = upload_new(prime, &device, image);
    if let Ok(outcome) = &mut result {
        outcome.operation_id = Some(id.clone());
    }
    let attempted = prime.log[start..]
        .iter()
        .any(|e| e.outgoing && e.bytes.get(1) == Some(&0) && e.bytes.get(2) == Some(&0xf7));
    match &result {
        Ok(outcome) => {
            record["state"] = serde_json::json!(outcome.state);
            record["outcome"] = serde_json::to_value(outcome).map_err(disk)?;
        }
        Err(error) => {
            record["state"] = serde_json::json!(if attempted {
                "indeterminate"
            } else {
                "not_sent"
            });
            record["error"] = serde_json::json!(error.to_string());
        }
    }
    record["updated_utc"] = serde_json::json!(ppl_compat::utc_now());
    delete_store::atomic_json(&root.join("history").join(&id).join("submission-intent.json"),&serde_json::json!({"operation_id":id,"state":"submitting","device":device,"name":image.name})).map_err(disk)?;
    delete_store::atomic_json(&path, &record).map_err(disk)?;
    result
}

/// Persist transport evidence before verification. All frontends use this
/// wrapper; a crash after submission intent remains blocked on recovery.
pub fn submit_deletion(
    prime: &mut Prime,
    deletion: &ppl_device::Deletion,
    store: &delete_store::Store,
    op: &mut delete_store::Operation,
) -> Result<ppl_device::DirectDeleteReport> {
    if op.state != "submitting" {
        return Err(refused("deletion submission intent was not persisted"));
    }
    let backups = store.backups(op)?;
    if backups.len() != deletion.backups.len()
        || backups
            .iter()
            .zip(&deletion.backups)
            .any(|(a, b)| a.name != b.name || a.ty != b.ty || a.data != b.data)
    {
        return Err(refused("deletion request differs from its durable backup"));
    }
    let guard = ppl_device::authorize_usb_deletion(prime, deletion, &op.control()?)?;
    let mut persistence_error = None;
    let mut report = ppl_device::delete_via_usb(prime, guard, |event| {
        match event {
            ppl_device::DeleteEvent::Submitted { index, state } => {
                op.submission.resize(
                    op.targets.len(),
                    ppl_device::delete::SubmissionState::NotSent,
                );
                op.submission[index] = state;
                op.state = if op
                    .submission
                    .iter()
                    .all(|s| *s == ppl_device::delete::SubmissionState::NotSent)
                {
                    "not_submitted"
                } else {
                    "sent"
                }
                .into();
            }
            ppl_device::DeleteEvent::Verifying => {
                if op.state != "not_submitted" {
                    op.state = "awaiting_verification".into();
                }
            }
            _ => return,
        }
        op.updated_utc = ppl_compat::utc_now();
        if let Err(error) = store.save(op) {
            persistence_error = Some(error.to_string());
        }
    })?;
    if let Some(error) = persistence_error {
        report.error = Some(format!(
            "{}; journal persistence failed: {error}",
            report
                .error
                .as_deref()
                .unwrap_or("transport submission observed")
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_fireworks_size_is_not_filesystem_or_hid_size() {
        // Pin the sizing vector to the test fixture, not an editable IDE program.
        let bytes = include_bytes!("../../../tests/fixtures/FIREWORKS.hpprgm").to_vec();
        let image = ProgramImage::new(
            "FIREWORKS",
            Source::decode(bytes, None, "FIREWORKS".into()).unwrap(),
        )
        .unwrap();
        assert_eq!(image.sizes.file_bytes, 10644);
        assert_eq!(image.sizes.payload_bytes, 21288);
        assert_eq!(image.sizes.object_message_bytes, 21316);
        assert_eq!(image.sizes.hid_reports, 339);
        assert_eq!(image.sizes.host_report_bytes, 21994);
        assert!(image.require_supported().is_err());
    }
    #[test]
    fn boundaries_unicode_and_line_endings_are_counted_without_rewriting() {
        for (text, units) in [("x\r\ny", 4), ("x\ny", 3), ("😀", 2)] {
            let image = ProgramImage::new("ZZT_TEST", Source::text(text, "test").unwrap()).unwrap();
            assert_eq!(image.sizes.utf16_units, units);
            assert_eq!(image.source.original_text, text);
        }
        for (units, ok) in [(7964, true), (7965, false)] {
            let image = ProgramImage::new(
                "ZZT_TEST",
                Source::text(&"x".repeat(units), "test").unwrap(),
            )
            .unwrap();
            assert_eq!(image.require_supported().is_ok(), ok);
        }
        assert!(Source::decode(vec![0xff, 0xfe, 0], None, "odd".into()).is_err());
        assert!(Source::decode(vec![0xff, 0xfe, 0, 0xd8], None, "surrogate".into()).is_err());
        assert!(Source::decode(vec![0xff], None, "utf8".into()).is_err());
    }
    #[test]
    fn readback_never_hides_whitespace_changes_or_invalid_encoding() {
        let bytes = |s: &str| {
            s.encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            compare_source(&bytes("A\r\n\0"), "A\n").unwrap(),
            Some("crlf_to_lf_and_optional_device_terminator")
        );
        assert_eq!(compare_source(&bytes("A \n\0"), "A\n").unwrap(), None);
        assert_eq!(compare_source(&bytes("A\n\0\0"), "A\n").unwrap(), None);
        assert!(compare_source(&[0], "A").is_err());
    }
}
