//! Guarded deletion of tool-owned programs and notes.
//!
//! Direct USB `EB` deletion is based on the Connectivity Kit serializer
//! (see docs/reverse-engineering/usb-protocol.md). A generated PPL helper
//! remains available for explicit manual execution. Neither transport
//! success nor uploading a helper establishes deletion or execution.
//!
//! Every mutation requires exact targets, unchanged CRC-valid backups,
//! a live control, and connection-bound authorization. Absence is checked
//! twice and bracketed by control queries. Helper evidence is retained
//! until the host archives it and confirms the requested deletion.

use std::time::Duration;

use crate::guard::{NamePolicy, PROBE_TIMEOUT, fresh_test_name, result_note_name};
use crate::{ControlObject, DeviceError, FileType, Prime, Probe, UploadReport};

/// Native EB deletion was physically verified for this exact firmware profile.
/// Historical hplp compatibility and other calculator generations do not qualify.
pub fn direct_delete_supported(
    product_id: u16,
    build: Option<u32>,
    boot_version: Option<&str>,
) -> bool {
    product_id == 0x1541 && build == Some(15157) && boot_version == Some("V0.050.640")
}

/// Identity captured before a deletion; every later `FA` must match it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Baseline {
    pub serial_fp: String,
    pub build: Option<u32>,
    pub version: Option<String>,
}

impl Baseline {
    pub fn from_info(info: &crate::CalcInfo) -> Result<Baseline, DeviceError> {
        Ok(Baseline {
            serial_fp: info
                .serial_fingerprint()
                .ok_or_else(|| DeviceError::Refused("device identity is unavailable".into()))?,
            build: info.build,
            version: info.version.clone(),
        })
    }
}

/// Transport-level heartbeat: `FA` must still identify the same calculator
/// and `FF` must report ready. This proves the USB stack is alive and that
/// the device was not swapped; it does NOT prove file requests are served
/// (see docs/device/deletion.md), which is why it only supplements an
/// `F8` anchor and never replaces one.
pub fn beat(p: &mut Prime, base: &Baseline) -> Result<(), DeviceError> {
    let silent = |what: &str, e: DeviceError| match e {
        DeviceError::Timeout(_) => {
            DeviceError::Inconclusive(format!("device did not answer {what}"))
        }
        other => other,
    };
    let info = p.info().map_err(|e| silent("the identity query", e))?;
    if Baseline::from_info(&info)? != *base {
        return Err(DeviceError::Refused(
            "device identity changed during the operation".into(),
        ));
    }
    let ready = p.check_ready().map_err(|e| silent("the ready check", e))?;
    if ready.first() != Some(&crate::READY_REPLY) {
        return Err(DeviceError::Protocol(format!(
            "unexpected ready reply {ready:02X?}"
        )));
    }
    Ok(())
}

/// Whether `calc.settings` has been shown (read-only, on hardware) to be
/// fetchable with a CRC-valid reply on the verified profile, and so may serve
/// as the control-free F8 liveness anchor. Evidence: 2026-10-24, G1 build
/// 15157 / V0.050.640, `artifacts/anchor-probe/anchor-probe-2026-10-24T204230Z.json`
/// (3/3 replies for both settings objects, CRC valid). The calculator answers
/// *any* type-0 name with an empty reply, so this proves the file-request
/// handler is alive, not that the object exists; it was not tested while a
/// foreground PPL program was running (see docs/device/deletion.md).
pub const SETTINGS_ANCHOR_VERIFIED: bool = true;

/// The control object is never deletable, whatever the caller asks for.
pub const PROTECTED_OBJECTS: &[&str] = &["ZZT_CTRL"];

/// Deletion requests beyond this many objects in one batch are refused.
pub const MAX_TARGETS: usize = 32;

/// One object to delete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTarget {
    /// Only [`FileType::Program`] and [`FileType::Note`] are deletable.
    pub ty: FileType,
    pub name: String,
}

impl DeleteTarget {
    pub fn program(name: &str) -> DeleteTarget {
        DeleteTarget {
            ty: FileType::Program,
            name: name.into(),
        }
    }

    pub fn note(name: &str) -> DeleteTarget {
        DeleteTarget {
            ty: FileType::Note,
            name: name.into(),
        }
    }
}

/// A backed-up object fetched before its deletion was authorized.
#[derive(Clone, Debug)]
pub struct Backup {
    pub ty: FileType,
    pub name: String,
    /// Raw object data as the calculator sent it (UTF-16LE text for
    /// programs and notes).
    pub data: Vec<u8>,
    pub crc_ok: bool,
}

/// A validated, backed-up deletion request plus its generated helper.
///
/// Produced by [`prepare_deletion`]; nothing has been written to the
/// calculator yet. The helper source is fully deterministic.
#[derive(Clone, Debug)]
pub struct Deletion {
    /// Fresh helper program name (`ZZT_D…`).
    pub helper: String,
    /// The helper's result note (`<helper>R`); its content becomes `done`.
    pub helper_note: String,
    /// Generated helper source (exact allowlist, see the module docs).
    pub source: String,
    /// Backup of every target, in target order.
    pub backups: Vec<Backup>,
    /// Probe log of the preparation.
    pub log: Vec<String>,
    /// Naming policy the targets were validated under. A `User` deletion has
    /// no usable helper (the helper allowlist is only meaningful for `Test`).
    pub policy: NamePolicy,
    /// Identity baseline; when set, every anchor probe is paired with a
    /// [`beat`].
    pub baseline: Option<Baseline>,
}

impl Deletion {
    /// The exact allowlist the helper will delete, in source order:
    /// the targets, then the helper itself.
    pub fn allowlist(&self) -> Vec<DeleteTarget> {
        let mut out = Vec::new();
        for b in &self.backups {
            out.push(DeleteTarget {
                ty: b.ty,
                name: b.name.clone(),
            });
        }
        out.push(DeleteTarget::program(&self.helper));
        out
    }
}

/// How the deletion of one target was established.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetStatus {
    /// Silent twice while the control answered in between.
    ConfirmedDeleted,
    /// The target still answered after the helper reported completion.
    StillPresent,
    /// Silence could not be established as meaningful (device trouble).
    Unknown,
}

#[derive(Clone, Debug)]
pub struct TargetResult {
    pub ty: FileType,
    pub name: String,
    pub status: TargetStatus,
}

/// Whether the helper ran to completion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// The helper note appeared and contains exactly `done`.
    Done,
    /// The helper note never appeared within the timeout; the helper may
    /// not have been run (or has not finished).
    TimedOut { polls: u32 },
    /// The helper note appeared with unexpected content.
    Failed { got: String },
}

/// End-to-end outcome of [`delete`].
#[derive(Clone, Debug)]
pub struct DeleteReport {
    pub helper: String,
    pub helper_note: String,
    pub completion: Completion,
    /// The helper's result-note text when it was retrieved.
    pub note_text: Option<String>,
    pub targets: Vec<TargetResult>,
    /// Whether the helper deleted itself (None: could not establish).
    pub helper_removed: Option<bool>,
}

/// The helper's result note once observed.
#[derive(Clone, Debug)]
pub struct NoteWatch {
    /// Normalized note text (`done` on success).
    pub text: String,
    /// Poll attempts until the note appeared.
    pub polls: u32,
}

/// Options for the post-upload phases of [`delete`].
#[derive(Clone, Debug)]
pub struct DeleteOptions {
    /// Total time to wait for the helper's result note.
    pub timeout: Duration,
    /// Wait per silent poll.
    pub poll: Duration,
    /// Legacy API field; completion now depends on terminal note contents,
    /// not a fixed settling delay.
    pub settle: Duration,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        DeleteOptions {
            timeout: Duration::from_secs(600),
            poll: Duration::from_secs(2),
            settle: Duration::from_secs(2),
        }
    }
}

impl DeleteOptions {
    /// Whole polls that fit into the timeout (at least one).
    pub fn max_polls(&self) -> u32 {
        let t = self.timeout.as_millis();
        let p = self.poll.as_millis().max(1);
        ((t / p).max(1)) as u32
    }
}

/// Progress events for callers that print status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteEvent {
    /// The helper was uploaded and read back; the user must press Run.
    AwaitingRun { helper: String },
    /// One silent poll for the result note completed.
    Polling { attempt: u32, of: u32 },
    /// The result note appeared.
    NoteSeen,
    /// Transport evidence after one EB attempt, before independent verification.
    Submitted {
        index: usize,
        state: SubmissionState,
    },
    /// Verification probing started.
    Verifying,
}

/// Generates the deletion helper source for an exact allowlist.
///
/// The helper writes a `working` marker to its result note, deletes every
/// listed target, overwrites the marker with `done`, and finally deletes
/// itself. The result note remains on the calculator as terminal evidence.
pub fn generate_deletion_helper(helper: &str, targets: &[DeleteTarget]) -> String {
    let mut out = String::new();
    out.push_str("// Deletion helper generated by ppl (exact allowlist; run once).\n");
    out.push_str("// It deletes only the objects named below, then itself.\n");
    out.push_str(&format!("EXPORT {helper}()\nBEGIN\n"));
    out.push_str(&format!(
        "  Notes(\"{}\"):=\"working\";\n",
        result_note_name(helper)
    ));
    for t in targets {
        let accessor = match t.ty {
            FileType::Program => "Programs",
            _ => "Notes",
        };
        out.push_str(&format!("  {accessor}(\"{}\"):=\"\";\n", t.name));
    }
    out.push_str(&format!(
        "  Notes(\"{}\"):=\"done\";\n",
        result_note_name(helper)
    ));
    out.push_str(&format!("  Programs(\"{helper}\"):=\"\";\n"));
    out.push_str("  RETURN 0;\nEND;\n");
    out
}

/// A fresh helper name: `ZZT_D` + five hex digits.
fn fresh_helper_name() -> String {
    // fresh_test_name() yields ZZT_ + 6 hex; keep the 'D' marker in the
    // first suffix position. The preflight (not the name) protects
    // existing objects, so remolding one character is safe.
    let mut n = fresh_test_name();
    // ZZT_XXXXXX -> ZZT_DXXXXX
    if n.len() >= 6 {
        n.replace_range(4..5, "D");
    }
    n
}

fn check_name(policy: NamePolicy, t: &DeleteTarget) -> Result<(), DeviceError> {
    match policy {
        NamePolicy::Test => crate::guard::validate_test_name(&t.name),
        NamePolicy::User => crate::guard::validate_object_name(t.ty, &t.name),
    }
}

fn validate_target(
    t: &DeleteTarget,
    control: &ControlObject,
    others: &[DeleteTarget],
    policy: NamePolicy,
) -> Result<(), DeviceError> {
    if !matches!(t.ty, FileType::Program | FileType::Note) {
        return Err(DeviceError::Unsupported(format!(
            "only programs and notes are deletable, not {}",
            t.ty.label()
        )));
    }
    let why = if check_name(policy, t).is_err() {
        None::<String> // check_name reports the precise reason below
    } else if PROTECTED_OBJECTS
        .iter()
        .any(|n| n.eq_ignore_ascii_case(&t.name))
    {
        Some("it is the protected preflight control".into())
    } else if t.name.eq_ignore_ascii_case(&control.name) {
        Some("it is the configured control object".into())
    } else if others.iter().any(|o| o.name.eq_ignore_ascii_case(&t.name)) {
        Some("it appears more than once".into())
    } else {
        None
    };
    if let Some(r) = why {
        return Err(DeviceError::Refused(format!(
            "`{}` cannot be deleted: {r}",
            t.name
        )));
    }
    check_name(policy, t)
}

/// Probes one object and requires it to be present with a good CRC.
/// `control` selects the error semantics: a silent control is
/// [`DeviceError::Inconclusive`] (silence proves nothing), a silent
/// ordinary target is [`DeviceError::Refused`] ("not present").
fn probe_present(
    p: &mut Prime,
    ty: FileType,
    name: &str,
    round: u32,
    log: &mut Vec<String>,
    control: bool,
) -> Result<Vec<u8>, DeviceError> {
    match p.probe_file(ty, name, PROBE_TIMEOUT) {
        Ok(Probe::Present(f)) => {
            if f.name != name {
                return Err(DeviceError::Protocol(format!(
                    "probe for `{name}` was answered with object `{}` ({})",
                    f.name,
                    f.ty.label()
                )));
            }
            if f.ty != ty {
                return Err(DeviceError::Refused(format!(
                    "`{name}` exists as a {}, not as a {}; it will not be deleted as a {}",
                    f.ty.label(),
                    ty.label(),
                    ty.label()
                )));
            }
            if !f.crc_ok {
                return Err(DeviceError::Protocol(format!(
                    "`{name}` reply has a CRC mismatch; refusing to back it up or delete it"
                )));
            }
            log.push(format!(
                "{round}: `{name}` present, {} bytes, crc ok",
                f.data.len()
            ));
            Ok(f.data)
        }
        Ok(Probe::NoReply { waited }) if control => Err(DeviceError::Inconclusive(format!(
            "control `{name}` did not answer within {:.1} s (probe {round}); without a \
             known-present object answering, silence proves nothing",
            waited.as_secs_f64()
        ))),
        Ok(Probe::NoReply { waited }) => Err(DeviceError::Absent(format!(
            "`{name}` ({}): no reply within {:.1} s while the control object was just answering; \
             the object is not present on this calculator",
            ty.label(),
            waited.as_secs_f64()
        ))),
        Err(e) => Err(e),
    }
}

/// Anchor probe (control present, CRC-valid) plus an optional heartbeat.
fn anchor(
    p: &mut Prime,
    control: &ControlObject,
    round: u32,
    log: &mut Vec<String>,
    base: Option<&Baseline>,
) -> Result<(), DeviceError> {
    probe_present(p, control.ty, &control.name, round, log, true)?;
    if let Some(b) = base {
        beat(p, b)?;
    }
    Ok(())
}

/// Validates the targets, proves each one present (two probe rounds with
/// the control answering in between), reads its data as a backup, and
/// generates the deletion helper. Nothing is written to the calculator.
///
/// An object that stays silent while the control answers is reported as
/// [`DeviceError::Refused`] ("not present"); a control that stops
/// answering is [`DeviceError::Inconclusive`] — absence of a reply never
/// proves absence of an object without the control check.
pub fn prepare_deletion(
    p: &mut Prime,
    targets: &[DeleteTarget],
    control: &ControlObject,
) -> Result<Deletion, DeviceError> {
    prepare_deletion_with(p, targets, control, NamePolicy::Test, None)
}

/// [`prepare_deletion`] under an explicit naming policy. `User` accepts
/// ordinary program/note names but exactly one target per request.
pub fn prepare_deletion_with(
    p: &mut Prime,
    targets: &[DeleteTarget],
    control: &ControlObject,
    policy: NamePolicy,
    baseline: Option<&Baseline>,
) -> Result<Deletion, DeviceError> {
    if policy == NamePolicy::User && targets.len() > 1 {
        return Err(DeviceError::Refused(
            "ordinary objects are deleted one at a time".into(),
        ));
    }
    if targets.is_empty() {
        return Err(DeviceError::Refused("no deletion targets given".into()));
    }
    if targets.len() > MAX_TARGETS {
        return Err(DeviceError::Refused(format!(
            "{0} targets exceed the per-batch limit of {MAX_TARGETS}",
            targets.len()
        )));
    }
    for (i, t) in targets.iter().enumerate() {
        validate_target(t, control, &targets[..i], policy)?;
    }
    let mut log = Vec::new();
    // Round 1: control, then every target.
    anchor(p, control, 1, &mut log, baseline)?;
    let mut backups = Vec::with_capacity(targets.len());
    for t in targets {
        let f = probe_present(p, t.ty, &t.name, 1, &mut log, false)?;
        backups.push(Backup {
            ty: t.ty,
            name: t.name.clone(),
            data: f,
            crc_ok: true,
        });
    }
    // Round 2: the control must answer again, then every target again,
    // unchanged. Late or changed replies abort the preparation.
    anchor(p, control, 2, &mut log, baseline)?;
    for b in &mut backups {
        let f = probe_present(p, b.ty, &b.name, 2, &mut log, false)?;
        if f != b.data {
            return Err(DeviceError::Inconclusive(format!(
                "`{}` changed between probe rounds ({} -> {} bytes); the calculator may be busy",
                b.name,
                b.data.len(),
                f.len()
            )));
        }
    }
    // Fresh helper name that does not collide with any target.
    let mut helper = fresh_helper_name();
    for _ in 0..16 {
        if !targets.iter().any(|t| t.name.eq_ignore_ascii_case(&helper))
            && helper != control.name
            && !PROTECTED_OBJECTS.contains(&helper.as_str())
        {
            break;
        }
        helper = fresh_helper_name();
    }
    let helper_note = result_note_name(&helper);
    let source = generate_deletion_helper(&helper, targets)
        .replace("\"working\"", &format!("\"@@PPLDELETE|{helper}|working\""))
        .replace("\"done\"", &format!("\"@@PPLDELETE|{helper}|done\""));
    Ok(Deletion {
        helper,
        helper_note,
        source,
        backups,
        log,
        policy,
        baseline: baseline.cloned(),
    })
}

/// Uploads the deletion helper through the standard guarded preflight
/// (the helper program and its result note must be silent twice) and
/// requires a matching read-back. Never retried by the caller.
pub fn upload_deletion_helper(
    p: &mut Prime,
    deletion: &Deletion,
    control: &ControlObject,
) -> Result<UploadReport, DeviceError> {
    let guard = p.preflight_new_names(
        &[
            (FileType::Program, deletion.helper.as_str()),
            (FileType::Note, deletion.helper_note.as_str()),
        ],
        Some(control),
    )?;
    let report = p.send_new_program(guard, &deletion.source)?;
    Ok(report)
}

/// Waits for the helper's result note. `Ok(None)` means the note did not
/// appear within the timeout — the helper was most likely not run yet.
pub fn await_deletion_note<F>(
    p: &mut Prime,
    deletion: &Deletion,
    opts: &DeleteOptions,
    mut on_event: F,
) -> Result<Option<NoteWatch>, DeviceError>
where
    F: FnMut(DeleteEvent),
{
    let max = opts.max_polls();
    let deadline = std::time::Instant::now() + opts.timeout;
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match p.probe_file(FileType::Note, &deletion.helper_note, opts.poll) {
            Ok(Probe::Present(f)) => {
                validate_result_note(&f, &deletion.helper_note)?;
                let text = crate::normalize_program_text(&f.text());
                let correlated = deletion.source.contains("@@PPLDELETE|");
                let working = text == format!("@@PPLDELETE|{}|working", deletion.helper)
                    || (!correlated && text == "working");
                if working {
                    if std::time::Instant::now() >= deadline {
                        return Ok(None);
                    }
                    // Wait only for another observation, never as proof of success.
                    std::thread::sleep(
                        Duration::from_millis(50)
                            .min(deadline.saturating_duration_since(std::time::Instant::now())),
                    );
                    attempt = attempt.saturating_sub(1);
                    continue;
                }
                let text = if text == format!("@@PPLDELETE|{}|done", deletion.helper) {
                    "done".into()
                } else if correlated && text == "done" {
                    "uncorrelated done marker".into()
                } else {
                    text
                };
                on_event(DeleteEvent::NoteSeen);
                return Ok(Some(NoteWatch {
                    text,
                    polls: attempt,
                }));
            }
            Ok(Probe::NoReply { .. }) => {}
            Err(e) => return Err(e),
        }
        if attempt >= max || std::time::Instant::now() >= deadline {
            return Ok(None);
        }
        on_event(DeleteEvent::Polling { attempt, of: max });
    }
}

fn validate_result_note(file: &crate::PrimeFile, name: &str) -> Result<(), DeviceError> {
    if file.name != name || file.ty != FileType::Note || !file.crc_ok {
        return Err(DeviceError::Protocol(
            "invalid deletion result identity, type or CRC".into(),
        ));
    }
    Ok(())
}

/// Verifies that every listed object is gone: each must stay silent over
/// two probe rounds while the control answers in between. Returns one
/// [`TargetStatus`] per target, in order.
pub fn verify_deleted(
    p: &mut Prime,
    targets: &[DeleteTarget],
    control: &ControlObject,
) -> Result<Vec<TargetStatus>, DeviceError> {
    verify_deleted_with(p, targets, control, None)
}

/// [`verify_deleted`] with an optional identity baseline: each anchor probe
/// is then paired with an `FA`+`FF` [`beat`], and late replies are checked
/// after every phase rather than only at the end.
pub fn verify_deleted_with(
    p: &mut Prime,
    targets: &[DeleteTarget],
    control: &ControlObject,
    base: Option<&Baseline>,
) -> Result<Vec<TargetStatus>, DeviceError> {
    let exchange_start = p.log.len();
    let mut log = Vec::new();
    let mut present: Vec<bool> = vec![false; targets.len()];
    let live = |p: &mut Prime| -> Result<(), DeviceError> {
        probe_optional(p, control.ty, &control.name, &mut Vec::new())?
            .ok_or_else(|| control_lost(control))?;
        if let Some(b) = base {
            beat(p, b)?;
        }
        Ok(())
    };
    live(p)?;
    for round in 0..2 {
        for (i, t) in targets.iter().enumerate() {
            if probe_optional(p, t.ty, &t.name, &mut log)?.is_some() {
                present[i] = true;
            }
        }
        live(p)?;
        if base.is_some() && round == 0 {
            observe_late_targets(p, exchange_start, targets, &mut present)?;
        }
    }
    // A late F7 discarded beside a live control is still evidence; never
    // convert it into absence just because the original request timed out.
    observe_late_targets(p, exchange_start, targets, &mut present)?;
    Ok(present
        .iter()
        .map(|&seen| {
            if seen {
                TargetStatus::StillPresent
            } else {
                TargetStatus::ConfirmedDeleted
            }
        })
        .collect())
}

fn control_lost(control: &ControlObject) -> DeviceError {
    DeviceError::Inconclusive(format!(
        "control {control} stopped answering; silence cannot be interpreted"
    ))
}
/// Probe that maps NoReply to Ok(None) and present objects to Some(data).
fn probe_optional(
    p: &mut Prime,
    ty: FileType,
    name: &str,
    log: &mut Vec<String>,
) -> Result<Option<Vec<u8>>, DeviceError> {
    match p.probe_file(ty, name, PROBE_TIMEOUT) {
        Ok(Probe::Present(f)) => {
            if f.name != name || f.ty != ty {
                return Err(DeviceError::Protocol(format!(
                    "probe for `{name}` was answered with object `{}` ({})",
                    f.name,
                    f.ty.label()
                )));
            }
            if !f.crc_ok {
                return Err(DeviceError::Protocol(format!(
                    "F8 `{name}` has invalid CRC"
                )));
            }
            log.push(format!("`{name}` answered ({} bytes)", f.data.len()));
            Ok(Some(f.data))
        }
        Ok(Probe::NoReply { waited }) => {
            log.push(format!(
                "`{name}`: no reply within {:.1} s",
                waited.as_secs_f64()
            ));
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Runs one prepared deletion to completion: uploads the helper, waits for
/// its result note, then verifies the absence of every target and of the
/// helper itself. The helper's result note is expected to remain present
/// as terminal evidence.
///
/// This is the composed form of [`upload_deletion_helper`] +
/// [`await_deletion_note`] + [`verify_deleted`].
pub fn run_deletion<F>(
    p: &mut Prime,
    deletion: &Deletion,
    control: &ControlObject,
    opts: &DeleteOptions,
    mut on_event: F,
) -> Result<DeleteReport, DeviceError>
where
    F: FnMut(DeleteEvent),
{
    let report = upload_deletion_helper(p, deletion, control)?;
    if !matches!(report.readback, crate::ReadBack::Matches) {
        return Err(DeviceError::Protocol(format!(
            "helper upload unverified (readback {:?}); nothing further was sent — \
             inspect the Program Catalog for `{}` before retrying",
            report.readback, deletion.helper
        )));
    }
    on_event(DeleteEvent::AwaitingRun {
        helper: deletion.helper.clone(),
    });
    let note = await_deletion_note(p, deletion, opts, &mut on_event)?;
    let completion = match &note {
        None => Completion::TimedOut {
            polls: opts.max_polls(),
        },
        Some(w) if w.text == "done" => Completion::Done,
        Some(w) => Completion::Failed {
            got: w.text.clone(),
        },
    };
    on_event(DeleteEvent::Verifying);
    // Targets plus the helper itself; the helper note is expected present.
    let mut all: Vec<DeleteTarget> = deletion
        .backups
        .iter()
        .map(|b| DeleteTarget {
            ty: b.ty,
            name: b.name.clone(),
        })
        .collect();
    all.push(DeleteTarget::program(&deletion.helper));
    let verified = verify_deleted(p, &all, control)?;
    let helper_removed = verified
        .last()
        .map(|s| *s == TargetStatus::ConfirmedDeleted);
    let targets: Vec<TargetResult> = verified[..verified.len() - 1]
        .iter()
        .zip(&deletion.backups)
        .map(|(s, b)| TargetResult {
            ty: b.ty,
            name: b.name.clone(),
            status: s.clone(),
        })
        .collect();
    Ok(DeleteReport {
        helper: deletion.helper.clone(),
        helper_note: deletion.helper_note.clone(),
        completion,
        note_text: note.map(|w| w.text),
        targets,
        helper_removed,
    })
}

/// How a deletion was performed on the calculator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    /// The `EB` USB delete command (Connectivity Kit mechanism): no
    /// calculator interaction required.
    UsbDelete,
    /// The generated PPL helper: needs one manual `Run` press.
    PplHelper,
}

/// A single-use permission for an exact backed-up deletion on one connection.
/// The CLI acquires this only after syncing its backups and submission journal.
/// It cannot be constructed or edited by callers and expires after five minutes.
#[derive(Debug)]
pub struct DeleteGuard {
    owner: std::sync::Weak<()>,
    created: std::time::Instant,
    backups: Vec<Backup>,
    control: ControlObject,
    policy: NamePolicy,
    baseline: Option<Baseline>,
}

/// Late target data disproves absence, rather than turning known presence into
/// an unknown deletion. Unrelated or incomplete traffic never proves absence.
fn observe_late_targets(
    p: &Prime,
    start: usize,
    targets: &[DeleteTarget],
    present: &mut [bool],
) -> Result<(), DeviceError> {
    let drained: Vec<_> = p.log[start..].iter().filter(|e| e.drained).collect();
    for file in crate::late::files(&drained, p.report_size)? {
        let Some(index) = targets
            .iter()
            .position(|t| t.ty == file.ty && t.name == file.name)
        else {
            return Err(DeviceError::Inconclusive(format!(
                "unmatched late {}:{} during deletion verification",
                file.ty.label(),
                file.name
            )));
        };
        present[index] = true;
    }
    Ok(())
}

/// Validate a prepared request and bind its immutable snapshot to this connection.
/// The caller must durably record backups and authorization before consuming it.
pub fn authorize_usb_deletion(
    p: &Prime,
    deletion: &Deletion,
    control: &ControlObject,
) -> Result<DeleteGuard, DeviceError> {
    let targets: Vec<_> = deletion
        .backups
        .iter()
        .map(|b| DeleteTarget {
            ty: b.ty,
            name: b.name.clone(),
        })
        .collect();
    if targets.is_empty()
        || targets.len() > MAX_TARGETS
        || (deletion.policy == NamePolicy::User && targets.len() != 1)
    {
        return Err(DeviceError::Refused("invalid deletion target count".into()));
    }
    for (i, t) in targets.iter().enumerate() {
        validate_target(t, control, &targets[..i], deletion.policy)?;
        if !deletion.backups[i].crc_ok {
            return Err(DeviceError::Refused(
                "deletion requires verified backups".into(),
            ));
        }
    }
    Ok(DeleteGuard {
        owner: std::sync::Arc::downgrade(&p.owner),
        created: std::time::Instant::now(),
        backups: deletion.backups.clone(),
        control: control.clone(),
        policy: deletion.policy,
        baseline: deletion.baseline.clone(),
    })
}

/// Per-target outcome, including partial success when a submission fails.
#[derive(Clone, Debug)]
pub struct DirectDeleteReport {
    pub targets: Vec<TargetResult>,
    pub error: Option<String>,
    /// Per-target transport evidence, independent of later verification.
    pub submission: Vec<SubmissionState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionState {
    NotSent,
    PossiblySent,
    Sent,
}

/// Submit each backed-up target once, then independently reconcile every outcome.
/// No caller may infer execution or deletion from successful packet transmission.
pub fn delete_via_usb<F>(
    p: &mut Prime,
    guard: DeleteGuard,
    mut on_event: F,
) -> Result<DirectDeleteReport, DeviceError>
where
    F: FnMut(DeleteEvent),
{
    if guard.created.elapsed() > crate::GUARD_MAX_AGE
        || !guard
            .owner
            .upgrade()
            .is_some_and(|o| std::sync::Arc::ptr_eq(&o, &p.owner))
    {
        return Err(DeviceError::Refused(
            "deletion guard expired or belongs to another connection".into(),
        ));
    }
    let targets: Vec<_> = guard
        .backups
        .iter()
        .map(|b| DeleteTarget {
            ty: b.ty,
            name: b.name.clone(),
        })
        .collect();
    for (i, t) in targets.iter().enumerate() {
        validate_target(t, &guard.control, &targets[..i], guard.policy)?;
    }
    let start = p.log.len();
    let mut log = Vec::new();
    // Revalidate the whole batch before any mutation; then each target again
    // immediately before sending. Changed content never inherits old ownership.
    anchor(p, &guard.control, 1, &mut log, guard.baseline.as_ref())?;
    for b in &guard.backups {
        if probe_present(p, b.ty, &b.name, 1, &mut log, false)? != b.data {
            return Err(DeviceError::Refused(format!(
                "`{}` changed since backup",
                b.name
            )));
        }
    }
    let mut error = None;
    let mut submission = vec![SubmissionState::NotSent; guard.backups.len()];
    for (index, b) in guard.backups.iter().enumerate() {
        let write_start = p.log.len();
        let send = (|| {
            anchor(p, &guard.control, 2, &mut log, guard.baseline.as_ref())?;
            if probe_present(p, b.ty, &b.name, 2, &mut log, false)? != b.data {
                return Err(DeviceError::Refused(format!(
                    "`{}` changed before deletion",
                    b.name
                )));
            }
            p.ensure_quiet(
                start,
                &targets.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            )?;
            p.delete_object(b.ty, &b.name)
        })();
        // write_report journals an attempt BEFORE touching the transport. A
        // failed/short write is possibly applied, never safely retryable.
        let attempted = p.log[write_start..].iter().any(|e| {
            e.outgoing
                && e.bytes.get(1) == Some(&0)
                && e.bytes.get(2) == Some(&crate::CMD_DELETE_OBJECT)
        });
        submission[index] = if attempted {
            if send.is_ok() {
                SubmissionState::Sent
            } else {
                SubmissionState::PossiblySent
            }
        } else {
            SubmissionState::NotSent
        };
        on_event(DeleteEvent::Submitted {
            index,
            state: submission[index],
        });
        if let Err(e) = send {
            error = Some(e.to_string());
            break;
        }
    }
    on_event(DeleteEvent::Verifying);
    let statuses = match verify_deleted_with(p, &targets, &guard.control, guard.baseline.as_ref()) {
        Ok(statuses) => statuses,
        Err(e) => {
            error = Some(format!(
                "{}verification: {e}",
                error.map(|s| format!("{s}; ")).unwrap_or_default()
            ));
            vec![TargetStatus::Unknown; targets.len()]
        }
    };
    Ok(DirectDeleteReport {
        submission,
        targets: targets
            .into_iter()
            .zip(statuses)
            .map(|(t, status)| TargetResult {
                ty: t.ty,
                name: t.name,
                status,
            })
            .collect(),
        error,
    })
}
