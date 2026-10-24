//! `ppl compare`: differential testing of the emulator against hardware evidence.
//!
//! Hardware workflow (every step is a separate, explicit invocation):
//!
//! 1. `--device prepare [--group G | --cases a,b]` (no USB traffic): generates
//!    the calculator program under a fresh `ZZT_xxxxxx` name, dry-runs it in
//!    the emulator and freezes it in a run manifest (`tests/compat/runs`).
//! 2. `--device preflight --run P` (read-only `FF`/`F8`): runs the existence
//!    checks without writing anything.
//! 3. `--device upload --run P`: refuses names in the local ledger, asks the
//!    user to type the program name after checking the Program Catalog, runs
//!    the guarded preflight, records the names in the ledger, sends exactly
//!    the manifest source and verifies the read-back.
//! 4. The user runs the program on the calculator by hand. No key presses
//!    (`EC`) are ever sent by this command.
//! 5. `--device screenshot --run P` (read-only `FC`): saves the terminal.
//! 6. `--device record --run P --transcript T --artifact PNG --user-confirmed`
//!    (no USB traffic): parses the user-confirmed transcript and saves evidence.
//!    `--device fetch --run P` does the same for note-channel runs over `F8`.
//!
//! Without `--device`, the emulator runs the suite and is compared against
//! every evidence file recorded for it.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, ValueEnum};
use ppl_compat::{
    Channel, DEFAULT_EVIDENCE_DIR, DEFAULT_RUNS_DIR, DeviceRecord, EvidenceV2, METHOD_NOTE_FETCH,
    METHOD_SCREENSHOT, Report, RunManifest, RunStatus, Status, Suite, compare_v2,
    emulate_print_program, generate_device_program, ledger_append, ledger_contains, load_evidence,
    load_manifest, load_suite, load_suite_evidence, merge_evidence, parse_result_lines,
    run_emulator, save_evidence_v2, save_manifest, select_cases, serial_fingerprint, sha256_hex,
    utc_now,
};
use ppl_device::{
    ControlObject, DEFAULT_CONTROL, DeviceError, FileType, Prime, Probe, ReadBack, fresh_test_name,
    result_note_name,
};
use ppl_vm::{DisplayMode as VmDisplayMode, HeadlessHost, Limits, Machine, Settings};

use super::common::{self, exit};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum DeviceAction {
    /// Generate and freeze a run manifest (no USB traffic).
    Prepare,
    /// Read-only existence checks for a prepared run.
    Preflight,
    /// Guarded upload of a prepared run (asks for typed confirmation).
    Upload,
    /// Read-only screenshot of the calculator screen for a run.
    Screenshot,
    /// Record a user-confirmed screenshot transcript as evidence (no USB traffic).
    Record,
    /// Fetch the result note of a note-channel run (read-only `F8`).
    Fetch,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ChannelArg {
    /// One PRINT line per case, read back from a screenshot.
    Print,
    /// Results written to a new note named after the program.
    Note,
}

#[derive(Args, Debug, Clone)]
pub struct CompareOpts {
    /// Path to a test suite (TOML) or a PPL program (.hpprgm / .ppl).
    pub target: PathBuf,

    /// Only the cases of this group.
    #[arg(long)]
    pub group: Option<String>,

    /// Only these case ids (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub cases: Vec<String>,

    /// Print a preview of the generated device program and exit.
    #[arg(long)]
    pub print_program: bool,

    /// Result channel for generated programs.
    #[arg(long, value_enum, default_value_t = ChannelArg::Print)]
    pub channel: ChannelArg,

    /// Hardware workflow step (see the module documentation).
    #[arg(long, value_enum, value_name = "ACTION")]
    pub device: Option<DeviceAction>,

    /// Program name of a prepared run (e.g. ZZT_3F9A2C).
    #[arg(long, value_name = "PROGRAM")]
    pub run: Option<String>,

    /// Object known to exist on the calculator, used as the positive control.
    #[arg(long, default_value = DEFAULT_CONTROL, value_name = "TYPE:NAME")]
    pub control: String,

    /// Transcript of the calculator screen (for `--device record`).
    #[arg(long)]
    pub transcript: Option<PathBuf>,

    /// Raw artefact the transcript was made from (for `--device record`).
    #[arg(long)]
    pub artifact: Option<PathBuf>,

    /// Confirms that the user checked the transcript against the calculator screen.
    #[arg(long)]
    pub user_confirmed: bool,

    /// Extra legacy (v1) hardware evidence JSON file.
    #[arg(long)]
    pub evidence: Option<PathBuf>,

    /// Directory where reports are saved.
    #[arg(long, default_value = "docs/reports")]
    pub out_dir: PathBuf,

    /// Directory where evidence files are stored/looked up.
    #[arg(long, default_value = DEFAULT_EVIDENCE_DIR)]
    pub evidence_dir: PathBuf,

    /// Directory for run manifests and the upload ledger.
    #[arg(long, default_value = DEFAULT_RUNS_DIR)]
    pub runs_dir: PathBuf,

    /// Do not write reports to disk.
    #[arg(long)]
    pub no_report: bool,
}

pub fn run(o: CompareOpts) -> u8 {
    let ext = o
        .target
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext == "toml" {
        run_suite(o)
    } else {
        run_program_render(o)
    }
}

struct Failure(u8, String);

fn usage(msg: impl Into<String>) -> Failure {
    Failure(exit::USAGE, msg.into())
}

fn io_err(msg: impl Into<String>) -> Failure {
    Failure(exit::IO, msg.into())
}

fn run_suite(o: CompareOpts) -> u8 {
    let suite = match load_suite(&o.target) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let result = if o.print_program {
        print_preview(&o, &suite)
    } else {
        match o.device {
            None => offline_compare(&o, &suite),
            Some(DeviceAction::Prepare) => prepare(&o, &suite),
            Some(action) => with_run(&o, &suite, action),
        }
    };
    match result {
        Ok(()) => exit::OK,
        Err(Failure(code, msg)) => {
            eprintln!("error: {msg}");
            code
        }
    }
}

fn with_run(o: &CompareOpts, suite: &Suite, action: DeviceAction) -> Result<(), Failure> {
    let program = o.run.clone().ok_or_else(|| {
        usage(format!(
            "`--device {action:?}` needs `--run <PROGRAM>` (from `--device prepare`)"
        ))
    })?;
    let m = load_manifest(&o.runs_dir, &program).map_err(usage)?;
    m.verify_source().map_err(usage)?;
    if m.suite != suite.suite {
        return Err(usage(format!(
            "run {program} belongs to suite `{}`, not `{}`",
            m.suite, suite.suite
        )));
    }
    match action {
        DeviceAction::Preflight => preflight_only(o, &m),
        DeviceAction::Upload => upload(o, m),
        DeviceAction::Screenshot => screenshot(o, &m),
        DeviceAction::Record => record(o, suite, m),
        DeviceAction::Fetch => fetch(o, suite, m),
        DeviceAction::Prepare => unreachable!("handled by the caller"),
    }
}

fn selected<'a>(o: &CompareOpts, suite: &'a Suite) -> Result<Vec<&'a ppl_compat::Case>, Failure> {
    let ids = (!o.cases.is_empty()).then_some(o.cases.as_slice());
    let cases = select_cases(suite, o.group.as_deref(), ids).map_err(usage)?;
    if cases.is_empty() {
        return Err(usage("no cases selected"));
    }
    Ok(cases)
}

fn channel_for(o: &CompareOpts, program: &str) -> Channel {
    match o.channel {
        ChannelArg::Print => Channel::Print,
        ChannelArg::Note => Channel::Note {
            note: result_note_name(program),
        },
    }
}

fn print_preview(o: &CompareOpts, suite: &Suite) -> Result<(), Failure> {
    let cases = selected(o, suite)?;
    let name = "ZZT_PREVIEW";
    let g = generate_device_program(suite, &cases, name, &channel_for(o, name))
        .map_err(|errs| usage(errs.join("\n       ")))?;
    println!("// Preview only: `--device prepare` generates the same program under a fresh name.");
    print!("{}", g.source);
    Ok(())
}

/// Picks a `ZZT_xxxxxx` name unused by any earlier run on this machine.
fn fresh_run_name(o: &CompareOpts) -> Result<String, Failure> {
    for _ in 0..32 {
        let name = fresh_test_name();
        let note = result_note_name(&name);
        let used = ledger_contains(&o.runs_dir, &name).map_err(io_err)?
            || ledger_contains(&o.runs_dir, &note).map_err(io_err)?
            || o.runs_dir.join(format!("{name}.json")).exists();
        if !used {
            return Ok(name);
        }
    }
    Err(io_err("could not find an unused program name"))
}

fn prepare(o: &CompareOpts, suite: &Suite) -> Result<(), Failure> {
    let cases = selected(o, suite)?;
    let program = fresh_run_name(o)?;
    let channel = channel_for(o, &program);
    let g = generate_device_program(suite, &cases, &program, &channel)
        .map_err(|errs| usage(errs.join("\n       ")))?;

    let dry_run = match channel {
        Channel::Print => Some(emulate_print_program(&g).map_err(|e| {
            Failure(
                exit::RUNTIME,
                format!("emulator dry run of {program} failed: {e}"),
            )
        })?),
        Channel::Note { .. } => None,
    };

    let m = RunManifest::prepared(&suite.suite, o.group.as_deref(), &g, &utc_now());
    let path = save_manifest(&o.runs_dir, &m).map_err(io_err)?;

    println!("{}", g.source);
    println!("------------------------------------------------------------");
    println!(
        "Prepared run `{program}` ({} cases, channel {}).",
        g.short_ids.len(),
        g.channel.as_str()
    );
    for (short, id) in &g.short_ids {
        println!("  {short} = {id}");
    }
    if let Some(note) = g.channel.note() {
        println!("Result note: `{note}`");
    }
    println!("Source SHA-256: {}", g.sha256);
    println!("Manifest: {}", path.display());
    if let Some(t) = dry_run {
        println!("\nEmulator dry run of the exact program (expected screen):\n{t}");
    }
    println!(
        "Nothing was sent. Review the program, then:\n  ppl compare {} --device upload --run {program}",
        o.target.display()
    );
    Ok(())
}

fn targets(m: &RunManifest) -> Vec<(FileType, String)> {
    let mut t = vec![(FileType::Program, m.program.clone())];
    if let Some(n) = &m.note {
        t.push((FileType::Note, n.clone()));
    }
    t
}

fn open_device() -> Result<Prime, Failure> {
    Prime::open_first().map_err(|e| io_err(format!("cannot open HP Prime: {e}")))
}

fn device_record(p: &mut Prime) -> Result<DeviceRecord, Failure> {
    let info = p
        .info()
        .map_err(|e| io_err(format!("device info (FA) failed: {e}")))?;
    Ok(DeviceRecord {
        model: p.detected.model.to_string(),
        usb_pid: format!("{:04x}:{:04x}", p.detected.vendor_id, p.detected.product_id),
        firmware_build: info.build,
        boot_version: info.version.clone(),
        serial_id: info.serial.as_deref().map(serial_fingerprint),
    })
}

fn control(o: &CompareOpts) -> Result<ControlObject, Failure> {
    ControlObject::parse(&o.control).map_err(|e| usage(format!("--control: {e}")))
}

fn explain(e: &DeviceError) -> String {
    match e {
        DeviceError::Inconclusive(_) => format!(
            "{e}\nhint: the positive-control object must exist on the calculator (create the empty \
             program ZZT_CTRL by hand); the calculator must be on the Home screen, unlocked, not in \
             exam mode, and the Connectivity Kit must be closed"
        ),
        _ => e.to_string(),
    }
}

fn preflight_only(o: &CompareOpts, m: &RunManifest) -> Result<(), Failure> {
    let control = control(o)?;
    let mut p = open_device()?;
    let t = targets(m);
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = p
        .preflight_new_names(&refs, Some(&control))
        .map_err(|e| io_err(format!("preflight did not pass: {}", explain(&e))))?;
    for line in guard.log() {
        println!("  {line}");
    }
    println!(
        "Preflight passed for {} (nothing was written; the guard is discarded).",
        guard.names().join(", ")
    );
    Ok(())
}

fn read_confirmation(program: &str, note: Option<&str>) -> Result<(), Failure> {
    println!("About to upload NEW program `{program}` to the calculator.");
    println!(
        "On the calculator, open the Program Catalog (Shift + 1) and check that `{program}` is NOT listed."
    );
    if let Some(n) = note {
        println!("Also open the Notes catalog (Shift + 0) and check that `{n}` is NOT listed.");
    }
    print!("Type the program name to confirm it is absent (anything else aborts): ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| io_err(format!("reading confirmation: {e}")))?;
    if line.trim() != program {
        return Err(usage("confirmation did not match; nothing was sent"));
    }
    Ok(())
}

fn upload(o: &CompareOpts, mut m: RunManifest) -> Result<(), Failure> {
    if m.status != RunStatus::Prepared {
        return Err(usage(format!(
            "run {} has status {:?}; only a Prepared run can be uploaded (prepare a new run)",
            m.program, m.status
        )));
    }
    let t = targets(&m);
    for (_, n) in &t {
        if ledger_contains(&o.runs_dir, n).map_err(io_err)? {
            return Err(usage(format!(
                "`{n}` is already in the upload ledger; it was sent before. Prepare a new run."
            )));
        }
    }
    let control = control(o)?;
    read_confirmation(&m.program, m.note.as_deref())?;

    let mut p = open_device()?;
    let device = device_record(&mut p)?;
    println!(
        "Connected: {} [{}], firmware build {}",
        device.model,
        device.usb_pid,
        device
            .firmware_build
            .map(|b| b.to_string())
            .unwrap_or_else(|| "?".into())
    );
    m.device = Some(device);

    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = match p.preflight_new_names(&refs, Some(&control)) {
        Ok(g) => g,
        Err(e) => {
            let msg = format!("preflight did not pass: {}", explain(&e));
            m.status = RunStatus::Blocked;
            m.blocked_reason = Some(msg.clone());
            save_manifest(&o.runs_dir, &m).map_err(io_err)?;
            return Err(io_err(msg));
        }
    };
    for line in guard.log() {
        println!("  {line}");
    }

    // The names are recorded before the write so they can never be reused,
    // even if the transfer fails half-way.
    let names: Vec<String> = t.iter().map(|(_, n)| n.clone()).collect();
    ledger_append(&o.runs_dir, &names, &m.program, &utc_now()).map_err(io_err)?;

    let report = match p.send_new_program(guard, &m.source) {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("upload failed (state on the calculator unknown): {e}");
            m.status = RunStatus::Blocked;
            m.blocked_reason = Some(msg.clone());
            save_manifest(&o.runs_dir, &m).map_err(io_err)?;
            return Err(io_err(msg));
        }
    };
    m.preflight_log = report.preflight_log.clone();
    m.uploaded_at_utc = Some(utc_now());
    let verified = matches!(report.readback, ReadBack::Matches);
    if verified {
        m.status = RunStatus::Uploaded;
    } else {
        m.status = RunStatus::UploadUnverified;
        m.blocked_reason = Some(format!("read-back: {:?}", report.readback));
    }
    save_manifest(&o.runs_dir, &m).map_err(io_err)?;
    if !verified {
        return Err(io_err(format!(
            "sent {} bytes but the read-back did not match: {:?}",
            report.bytes_sent, report.readback
        )));
    }

    println!(
        "Uploaded `{}` ({} bytes); read-back matches the manifest source.",
        m.program, report.bytes_sent
    );
    println!("\n=== MANUAL STEP (nothing is pressed for you) ===");
    println!(
        "1. On the calculator press Shift + 1 (Program) and select `{}`.",
        m.program
    );
    println!(
        "2. Press Run (or type {}() on the Home screen and press Enter).",
        m.program
    );
    println!(
        "3. The terminal should show `@@ZZT|{}|…`, one line per case, then `@@END`.",
        m.program
    );
    println!("4. Leave the terminal on screen, then run:");
    println!(
        "   ppl compare {} --device screenshot --run {}",
        o.target.display(),
        m.program
    );
    Ok(())
}

/// First unused path `<stem><suffix>` in `dir` (never overwrites).
fn unused_path(dir: &Path, stem: &str, suffix: &str) -> PathBuf {
    let first = dir.join(format!("{stem}{suffix}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|i| dir.join(format!("{stem}-{i}{suffix}")))
        .find(|p| !p.exists())
        .expect("unbounded search")
}

fn screenshot(o: &CompareOpts, m: &RunManifest) -> Result<(), Failure> {
    let mut p = open_device()?;
    let device = device_record(&mut p)?;
    if m.device.is_none() {
        let mut updated = m.clone();
        updated.device = Some(device);
        save_manifest(&o.runs_dir, &updated).map_err(io_err)?;
    }
    let raw = p
        .screenshot(8)
        .map_err(|e| io_err(format!("screenshot (FC 08) failed: {e}")))?;
    let img = ppl_graphics::Grob::from_prime_screenshot(&raw)
        .map_err(|e| io_err(format!("cannot decode screenshot: {e}")))?;
    let dir = o.evidence_dir.join(&m.suite);
    std::fs::create_dir_all(&dir).map_err(|e| io_err(format!("{}: {e}", dir.display())))?;
    let raw_path = unused_path(&dir, &format!("{}.screen", m.program), ".raw.png");
    let png_path = PathBuf::from(
        raw_path
            .to_string_lossy()
            .replace(".raw.png", ".decoded.png"),
    );
    std::fs::write(&raw_path, &raw).map_err(|e| io_err(format!("{}: {e}", raw_path.display())))?;
    img.save_png(&png_path)
        .map_err(|e| io_err(format!("{}: {e}", png_path.display())))?;
    println!(
        "Saved raw calculator reply: {} (sha256 {})",
        raw_path.display(),
        sha256_hex(&raw)
    );
    println!(
        "Saved decoded screen ({}×{}): {}",
        img.width(),
        img.height(),
        png_path.display()
    );
    println!(
        "Transcribe the screen into a text file, have the user confirm it, then:\n  ppl compare {} \
         --device record --run {} --transcript <file> --artifact {} --user-confirmed",
        o.target.display(),
        m.program,
        raw_path.display()
    );
    Ok(())
}

fn finish_evidence(
    o: &CompareOpts,
    suite: &Suite,
    mut m: RunManifest,
    mut ev: EvidenceV2,
) -> Result<(), Failure> {
    let emu = run_emulator(suite);
    ev.compare_with(&emu);
    let path = save_evidence_v2(&o.evidence_dir, &ev).map_err(io_err)?;
    println!("Saved hardware evidence: {}", path.display());
    for (id, st) in &ev.comparison {
        let dev = ev.results.get(id).map(|c| c.raw.as_str()).unwrap_or("");
        println!("  {:<11} {id}  [G1: {dev}]", st.as_str());
    }
    for (id, setting) in &ev.setting_changes {
        println!(
            "  WARNING: case {id} changed {setting} on the calculator (restored by the program)"
        );
    }
    m.status = RunStatus::EvidenceRecorded;
    save_manifest(&o.runs_dir, &m).map_err(io_err)?;
    offline_compare(o, suite)
}

fn record(o: &CompareOpts, suite: &Suite, m: RunManifest) -> Result<(), Failure> {
    if m.channel != Channel::Print.as_str() {
        return Err(usage(
            "`record` is for print-channel runs; use `--device fetch`",
        ));
    }
    if !o.user_confirmed {
        return Err(usage(
            "a screenshot transcript is only evidence once the user has confirmed it; pass \
             --user-confirmed after they do",
        ));
    }
    let (Some(tpath), Some(apath)) = (&o.transcript, &o.artifact) else {
        return Err(usage(
            "`record` needs --transcript <file> and --artifact <raw png>",
        ));
    };
    let device = m.device.clone().ok_or_else(|| {
        usage(format!(
            "run {} has no device record (was it uploaded?)",
            m.program
        ))
    })?;
    let text =
        std::fs::read_to_string(tpath).map_err(|e| io_err(format!("{}: {e}", tpath.display())))?;
    let artifact = std::fs::read(apath).map_err(|e| io_err(format!("{}: {e}", apath.display())))?;
    let parsed = parse_result_lines(&text, &m.short_ids).map_err(usage)?;
    let mut ev = EvidenceV2::from_run(&m, &parsed, METHOD_SCREENSHOT, device, text, &utc_now())
        .map_err(usage)?;
    ev.raw_artifact = Some(apath.display().to_string());
    ev.raw_artifact_sha256 = Some(sha256_hex(&artifact));
    finish_evidence(o, suite, m, ev)
}

fn fetch(o: &CompareOpts, suite: &Suite, m: RunManifest) -> Result<(), Failure> {
    let Some(note) = m.note.clone() else {
        return Err(usage(
            "`fetch` is for note-channel runs; print runs use screenshot + record",
        ));
    };
    let mut p = open_device()?;
    let device = device_record(&mut p)?;
    if let Some(prev) = &m.device
        && prev.serial_id != device.serial_id
    {
        return Err(io_err("this is not the calculator the run was uploaded to"));
    }
    let file = match p.probe_file(FileType::Note, &note, Duration::from_secs(8)) {
        Ok(Probe::Present(f)) => f,
        Ok(Probe::NoReply { waited }) => {
            return Err(io_err(format!(
                "note `{note}` did not answer within {:.0} s (this does not prove it is absent); \
                 did the program finish?",
                waited.as_secs_f64()
            )));
        }
        Err(e) => return Err(io_err(format!("fetching `{note}`: {e}"))),
    };
    let text = file.text();
    let dir = o.evidence_dir.join(&m.suite);
    std::fs::create_dir_all(&dir).map_err(|e| io_err(format!("{}: {e}", dir.display())))?;
    let raw_path = unused_path(&dir, &m.program, ".note.raw");
    std::fs::write(&raw_path, &file.data)
        .map_err(|e| io_err(format!("{}: {e}", raw_path.display())))?;
    let parsed = parse_result_lines(&text, &m.short_ids).map_err(usage)?;
    let mut ev = EvidenceV2::from_run(&m, &parsed, METHOD_NOTE_FETCH, device, text, &utc_now())
        .map_err(usage)?;
    ev.raw_artifact = Some(raw_path.display().to_string());
    ev.raw_artifact_sha256 = Some(sha256_hex(&file.data));
    finish_evidence(o, suite, m, ev)
}

fn offline_compare(o: &CompareOpts, suite: &Suite) -> Result<(), Failure> {
    println!(
        "Running {} emulator cases for suite `{}`...",
        suite.cases.len(),
        suite.suite
    );
    let emu = run_emulator(suite);
    let mut evs = load_suite_evidence(&o.evidence_dir, &suite.suite).map_err(io_err)?;
    if let Some(path) = &o.evidence {
        let v1 = load_evidence(path).map_err(io_err)?;
        evs.push(EvidenceV2::from_v1(&v1));
    }
    if evs.is_empty() {
        println!(
            "No hardware evidence for `{}`: every case is UNVERIFIED.",
            suite.suite
        );
    } else {
        println!("Loaded {} hardware evidence file(s).", evs.len());
    }
    let merged = merge_evidence(&evs);
    let report = compare_v2(suite, &emu, &merged);
    print_report_summary(&report);
    save_reports(o, &report);
    Ok(())
}

fn print_report_summary(report: &Report) {
    println!("\n--- Compatibility Report: {} ---", report.suite);
    let counts = report.counts();
    let parts: Vec<String> = counts.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    println!("Counts: {}", parts.join(", "));
    for c in &report.cases {
        if matches!(c.status, Status::Failed | Status::Partial | Status::Blocked) {
            println!(
                "  {}: case `{}` | emulator: {} | G1: {}",
                c.status.as_str(),
                c.id,
                c.emulator.summary(),
                c.device
                    .as_ref()
                    .map(|d| d.summary())
                    .unwrap_or_else(|| "—".into())
            );
        }
    }
}

fn save_reports(o: &CompareOpts, report: &Report) {
    if o.no_report {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&o.out_dir) {
        eprintln!(
            "warning: cannot create report dir {}: {e}",
            o.out_dir.display()
        );
        return;
    }
    let md_path = o.out_dir.join(format!("compat-{}.md", report.suite));
    let json_path = o.out_dir.join(format!("compat-{}.json", report.suite));
    match std::fs::write(&md_path, report.markdown()) {
        Ok(()) => println!("Wrote report markdown to {}", md_path.display()),
        Err(e) => eprintln!("warning: cannot write {}: {e}", md_path.display()),
    }
    match serde_json::to_string_pretty(report) {
        Ok(json) => match std::fs::write(&json_path, json) {
            Ok(()) => println!("Wrote report JSON to {}", json_path.display()),
            Err(e) => eprintln!("warning: cannot write {}: {e}", json_path.display()),
        },
        Err(e) => eprintln!("warning: failed to serialize report: {e}"),
    }
}

/// `ppl compare prog.hpprgm`: renders the program's final screen in the
/// emulator. Uploading arbitrary user programs is disabled: their `EXPORT`
/// names could clash with programs already on the calculator.
fn run_program_render(o: CompareOpts) -> u8 {
    if o.device.is_some() {
        eprintln!(
            "error: `--device` is only supported for TOML suites; uploading arbitrary programs is \
             disabled (their EXPORT names could clash with programs on the calculator)"
        );
        return exit::USAGE;
    }
    let source = match common::load_source(Some(&o.target), None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let (prog, _) = common::compile(&source, false);
    let Some(prog) = prog else {
        return exit::COMPILE;
    };
    let entry = match common::pick_entry(&prog, None, None) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let mut m = Machine::new(
        prog,
        Box::new(HeadlessHost::new()),
        Settings::default(),
        Limits::default(),
    );
    let _ = m.call_function(&entry, Vec::new());
    let grob = if m.display_mode() == VmDisplayMode::Terminal {
        ppl_graphics::text::render_terminal(&m.terminal)
    } else {
        m.screen().clone()
    };
    if let Err(e) = std::fs::create_dir_all(&o.out_dir) {
        eprintln!(
            "warning: cannot create out dir {}: {e}",
            o.out_dir.display()
        );
    }
    let stem = o
        .target
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("program");
    let path = o.out_dir.join(format!("{stem}-emulator.png"));
    if let Err(e) = grob.save_png(&path) {
        eprintln!("error saving emulator png: {e}");
        return exit::IO;
    }
    println!("Emulator screenshot saved to {}", path.display());
    exit::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(dir: &Path, suite: &Path) -> CompareOpts {
        CompareOpts {
            target: suite.to_path_buf(),
            group: None,
            cases: Vec::new(),
            print_program: false,
            channel: ChannelArg::Print,
            device: None,
            run: None,
            control: DEFAULT_CONTROL.into(),
            transcript: None,
            artifact: None,
            user_confirmed: false,
            evidence: None,
            out_dir: dir.join("reports"),
            evidence_dir: dir.join("evidence"),
            runs_dir: dir.join("runs"),
            no_report: false,
        }
    }

    fn temp_suite(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ppl_cmp_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("suite.toml");
        std::fs::write(
            &path,
            r#"
            suite = "cmp_test"
            [[case]]
            id = "add"
            expr = "1 + 2"
            [[case]]
            id = "mul"
            expr = "6 * 7"
            "#,
        )
        .unwrap();
        (dir, path)
    }

    fn only_run(dir: &Path) -> String {
        std::fs::read_dir(dir.join("runs"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "json"))
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .expect("manifest written")
    }

    #[test]
    fn print_program_preview_creates_no_run() {
        let (dir, path) = temp_suite("print");
        let mut o = opts(&dir, &path);
        o.print_program = true;
        assert_eq!(run(o), exit::OK);
        assert!(!dir.join("runs").exists(), "preview must not create a run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_compare_writes_reports() {
        let (dir, path) = temp_suite("offline");
        assert_eq!(run(opts(&dir, &path)), exit::OK);
        assert!(dir.join("reports/compat-cmp_test.md").exists());
        assert!(dir.join("reports/compat-cmp_test.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_then_record_confirmed_transcript() {
        let (dir, path) = temp_suite("record");
        let mut o = opts(&dir, &path);
        o.device = Some(DeviceAction::Prepare);
        assert_eq!(run(o.clone()), exit::OK);
        let program = only_run(&dir);
        assert!(program.starts_with("ZZT_"));

        // Simulate the upload record and use the emulator dry run as the
        // transcript (a real run transcribes the calculator screen).
        let mut m = load_manifest(&o.runs_dir, &program).unwrap();
        let suite = load_suite(&path).unwrap();
        let cases = select_cases(&suite, None, None).unwrap();
        let g = generate_device_program(&suite, &cases, &program, &Channel::Print).unwrap();
        assert_eq!(
            g.source, m.source,
            "prepare must freeze the generated source"
        );
        let transcript = emulate_print_program(&g).unwrap();
        m.device = Some(DeviceRecord {
            model: "unit test".into(),
            usb_pid: "0000:0000".into(),
            firmware_build: None,
            boot_version: None,
            serial_id: None,
        });
        m.status = RunStatus::Uploaded;
        save_manifest(&o.runs_dir, &m).unwrap();
        let tpath = dir.join("t.txt");
        std::fs::write(&tpath, transcript).unwrap();
        let apath = dir.join("a.png");
        std::fs::write(&apath, b"not really a png").unwrap();

        o.device = Some(DeviceAction::Record);
        o.run = Some(program);
        o.transcript = Some(tpath);
        o.artifact = Some(apath);
        // Without explicit user confirmation nothing is recorded.
        assert_eq!(run(o.clone()), exit::USAGE);
        o.user_confirmed = true;
        assert_eq!(run(o.clone()), exit::OK);
        let ev = load_suite_evidence(&o.evidence_dir, "cmp_test").unwrap();
        assert_eq!(ev.len(), 1);
        assert!(ev[0].comparison.values().all(|s| *s == Status::Verified));
        // Recording the same run twice never overwrites evidence.
        assert_ne!(run(o), exit::OK);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upload_refuses_ledger_names_before_touching_usb() {
        let (dir, path) = temp_suite("ledger");
        let mut o = opts(&dir, &path);
        o.device = Some(DeviceAction::Prepare);
        assert_eq!(run(o.clone()), exit::OK);
        let program = only_run(&dir);
        ledger_append(
            &o.runs_dir,
            std::slice::from_ref(&program),
            &program,
            &utc_now(),
        )
        .unwrap();
        o.device = Some(DeviceAction::Upload);
        o.run = Some(program);
        assert_eq!(run(o), exit::USAGE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_actions_require_a_run() {
        let (dir, path) = temp_suite("norun");
        let mut o = opts(&dir, &path);
        o.device = Some(DeviceAction::Upload);
        assert_eq!(run(o), exit::USAGE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn program_targets_never_upload() {
        let (dir, _) = temp_suite("prog");
        let prog = dir.join("p.ppl");
        std::fs::write(&prog, "EXPORT P()\nBEGIN\nEND;\n").unwrap();
        let mut o = opts(&dir, &prog);
        o.device = Some(DeviceAction::Upload);
        assert_eq!(run(o), exit::USAGE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
