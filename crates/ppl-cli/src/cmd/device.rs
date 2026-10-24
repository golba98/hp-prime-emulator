//! `ppl device …`: talk to a connected HP Prime.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use ppl_device::{
    ControlObject, DEFAULT_CONTROL, DeviceError, DeviceSession, FileType, Prime, Probe, ReadBack,
    describe_codes, result_note_name, validate_test_name,
};

use super::common::exit;

/// Directory holding device-control session files.
pub const SESSIONS_DIR: &str = "artifacts/device-sessions";

#[derive(Args, Debug)]
pub struct DeviceOpts {
    #[command(subcommand)]
    pub action: DeviceAction,
    /// Write every raw HID report exchanged to this file.
    #[arg(long, global = true)]
    pub log: Option<PathBuf>,
    /// Write structured protocol/USB trace to this file.
    #[arg(long, global = true)]
    pub trace: Option<PathBuf>,
    /// Emit machine-readable results for status, listener, and remote execution.
    #[arg(long, global = true)]
    pub json: bool,
    /// Select a device by its cryptographic serial-id (from device status).
    #[arg(long, global = true)]
    pub device: Option<String>,
    /// Directory containing remote execution journals and evidence.
    #[arg(long, global = true, default_value = "artifacts/remote-execution")]
    pub remote_dir: PathBuf,
    /// Seconds to wait after a request upload (timeout never cancels or retries).
    #[arg(long, global = true, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub remote_timeout_secs: u64,
}

#[derive(Subcommand, Debug)]
pub enum DeviceAction {
    /// Read device identity, execution capabilities, or a pending request result.
    Status {
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Evaluate one expression on verified physical hardware.
    Exec { expression: String },
    /// Upload a fresh ZZT_ program and invoke its exported entrypoint.
    ExecFile {
        file: PathBuf,
        #[arg(long)]
        args: Option<String>,
    },
    /// Install and test the experimental physical G1 note listener.
    Listener {
        #[command(subcommand)]
        action: super::remote::ListenerAction,
    },
    /// List HP Prime calculators on USB (read-only).
    Detect,
    /// Ping the calculator with the check-ready command (read-only).
    Ping,
    /// Show model, firmware and serial (read-only).
    Info {
        /// Print the serial number and raw reply.
        #[arg(long)]
        show_serial: bool,
    },
    /// Save the calculator screen as PNG (read-only).
    Screenshot {
        /// Output PNG path.
        out: PathBuf,
        /// Screenshot format byte (8 = 320×240, 9/10/11 = reduced variants).
        #[arg(long, default_value_t = 8)]
        format: u8,
    },
    /// Fetch one object from the calculator (read-only).
    Get {
        /// Object type: program, note, list, matrix, real, complex, app, settings.
        ty: String,
        /// Object name, e.g. ZZT_R.
        name: String,
        /// Save the raw object data here (default: print text).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Send a NEW program whose name starts with ZZT_ (never overwrites).
    ///
    /// You must first confirm (by typing the name) that it is not in the
    /// calculator's Program Catalog; then a preflight proves, twice, that a
    /// known-present control object answers while the program and its
    /// result note (NAME + "R") do not. Any doubt aborts without writing.
    Send {
        /// Program name on the calculator: ZZT_ + [A-Za-z0-9_], at most 16 characters.
        name: String,
        /// PPL source file (or, with --note, the note text file) to send unchanged.
        file: PathBuf,
        /// Optional existing object used for preflight diagnostics, as `type:name`.
        /// If omitted or absent, device-level transport verification is used.
        #[arg(long)]
        control: Option<String>,
        /// Send a NOTE instead of a program (same ZZT_ rules and preflight).
        #[arg(long)]
        note: bool,
        /// Runs directory for the upload ledger (ownership proof for `delete`).
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
    },
    /// Press one named calculator key (EC). Requires an active, authorized
    /// device-control session (`session start`); never automatic.
    Key {
        /// Key name, e.g. home, enter, esc, up, down, left, right.
        #[arg(long)]
        name: Option<String>,
        /// Space-separated key sequence, e.g. "esc esc enter".
        #[arg(long, conflicts_with = "name")]
        sequence: Option<String>,
        /// List all known key names and exit.
        #[arg(long)]
        list: bool,
        /// Session id that authorizes the press(es).
        #[arg(long)]
        session: Option<String>,
        /// Save a screenshot after the press(es) to this PNG path.
        #[arg(long)]
        screenshot_after: Option<PathBuf>,
    },
    /// Run an uploaded program on the calculator and capture its result
    /// note automatically. Read-only on the device: EC key injection is
    /// ineffective on G1 firmware 15157, so you press `Run` once and this
    /// command handles preflight, completion detection, retrieval,
    /// identity validation and the audit log.
    Run {
        /// Invoke through a hardware-verified listener instead of waiting for manual Run.
        #[arg(long)]
        remote: bool,
        /// PPL argument list for --remote, e.g. "10,20".
        #[arg(long, requires = "remote")]
        args: Option<String>,
        /// Program name on the calculator (e.g. ZZT_004752).
        name: String,
        /// Result note to wait for (default: NAME + "R").
        #[arg(long)]
        note: Option<String>,
        /// Total seconds to wait for the result note (default 600).
        #[arg(long)]
        timeout_secs: Option<u64>,
        /// Seconds per silent poll (default 2).
        #[arg(long)]
        poll_secs: Option<u64>,
        /// Optional existing object used for preflight diagnostics, as `type:name`.
        /// If omitted or absent, device-level transport verification is used.
        #[arg(long)]
        control: Option<String>,
        /// Save the captured note text to this file.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Skip the `@@ZZT|NAME|` header check (for non-campaign programs).
        #[arg(long)]
        no_header_check: bool,
        /// Do not append to the run audit log.
        #[arg(long)]
        no_audit: bool,
        /// Audit log path (default artifacts/device-runs.jsonl).
        #[arg(long, default_value = "artifacts/device-runs.jsonl")]
        audit_path: PathBuf,
    },
    /// Manage explicit device-control sessions (authorization for EC keys).
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Delete tool-owned ZZT_ programs or notes (backed up and verified).
    Delete {
        #[command(subcommand)]
        action: super::device_delete::DeleteAction,
    },
    /// List the host-known ZZT_ objects and probe their presence (read-only).
    List {
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        /// Probe at most this many names (default: all).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Ownership-verified cleanup of this project's test programs/notes.
    Cleanup {
        #[command(subcommand)]
        action: CleanupAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum CleanupAction {
    /// Show what could be cleaned (no USB writes, no deletions).
    DryRun {
        /// Restrict the plan to one campaign's objects (its programs and
        /// result notes, by id, e.g. `stage-a`).
        #[arg(long)]
        campaign: Option<String>,
        /// Runs directory (ledger + manifests).
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        /// Campaign ledgers directory (active-test protection).
        #[arg(long, default_value = ppl_compat::DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
    },
    /// Back up, verify ownership and upload the cleanup helper
    /// (asks for typed confirmation of the exact target list).
    Execute {
        /// Proof of concept: clean only the disposable ZZT_POC objects.
        #[arg(long)]
        poc: bool,
        /// Restrict the deletion to one campaign's objects (by id).
        #[arg(long)]
        campaign: Option<String>,
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        #[arg(long, default_value = ppl_compat::DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
        #[arg(long, default_value = ppl_compat::DEFAULT_CLEANUP_DIR)]
        cleanup_dir: PathBuf,
        /// Positive control object for the preflight.
        #[arg(long, default_value = DEFAULT_CONTROL)]
        control: String,
    },
    /// After the helper ran on the calculator: verify deletion happened.
    Verify {
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        #[arg(long, default_value = ppl_compat::DEFAULT_CLEANUP_DIR)]
        cleanup_dir: PathBuf,
    },
    /// Show the cleanup batch history and target states.
    Status {
        #[arg(long, default_value = ppl_compat::DEFAULT_CLEANUP_DIR)]
        cleanup_dir: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum SessionAction {
    /// Start an authorized session (budgeted, expiring).
    Start {
        /// Session id (letters, digits, dash).
        id: String,
        /// Keys that may be pressed (repeatable); default: navigation keys only.
        #[arg(long = "allow")]
        allow: Vec<String>,
        /// Hard limit on key presses in this session.
        #[arg(long, default_value_t = 50)]
        max_presses: usize,
        /// Session lifetime in minutes.
        #[arg(long, default_value_t = 30)]
        ttl_mins: u64,
    },
    /// Show a session's budget and press log.
    Status { id: String },
    /// Stop a session (no further presses allowed).
    Stop { id: String },
}

fn session_path(dir: &str, id: &str) -> PathBuf {
    Path::new(dir).join(format!("{id}.json"))
}

/// `ppl device session start|status|stop`: no USB traffic, file only.
fn run_session(action: &SessionAction) -> u8 {
    let dir = SESSIONS_DIR;
    match action {
        SessionAction::Start {
            id,
            allow,
            max_presses,
            ttl_mins,
        } => {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                eprintln!("error: session id must be non-empty letters/digits/dashes");
                return exit::USAGE;
            }
            let path = session_path(dir, id);
            if path.exists() {
                eprintln!("error: session {id} already exists ({})", path.display());
                return exit::USAGE;
            }
            let s = match DeviceSession::start(
                id,
                (!allow.is_empty()).then(|| allow.clone()),
                *max_presses,
                ttl_mins * 60,
            ) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            if let Err(e) = s.save(&path) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!("session {id} started");
            println!("  file:         {}", path.display());
            println!("  operation:    {} (EC key injection)", ppl_device::OP_KEYS);
            println!("  allowlist:    {}", s.allowed_keys.join(", "));
            println!("  press budget: {}", s.max_presses);
            println!("  expires in:   {ttl_mins} min");
            println!("next: ppl device key --session {id} --name <key>");
        }
        SessionAction::Status { id } => match DeviceSession::load(&session_path(dir, id)) {
            Ok(s) => {
                println!("{}", s.status_line());
                for p in &s.presses {
                    println!(
                        "  {:>10}s  {} (code {})",
                        p.at_unix - s.created_unix,
                        p.key,
                        p.code
                    );
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                return exit::IO;
            }
        },
        SessionAction::Stop { id } => {
            let path = session_path(dir, id);
            let mut s = match DeviceSession::load(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            s.stop();
            if let Err(e) = s.save(&path) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!("session {id} stopped ({} presses used)", s.presses.len());
        }
    }
    exit::OK
}

pub fn run(o: DeviceOpts) -> u8 {
    if super::remote::handles(&o.action) {
        return super::remote::run(&o);
    }
    let json_capable = matches!(
        o.action,
        DeviceAction::Delete { .. } | DeviceAction::List { .. }
    );
    if o.json && !json_capable {
        println!(
            "{}",
            serde_json::json!({"schema_version":1,"execution_confirmed":false,"error":{"kind":"invalid_input","message":"--json is supported by status, listener, remote execution, delete and list commands","exit_code":2}})
        );
        return exit::USAGE;
    }
    if let DeviceAction::Delete { action } = &o.action {
        return super::device_delete::run(action, o.json, o.device.as_deref(), o.log.as_deref());
    }
    if o.device.is_some() {
        eprintln!(
            "--device is currently supported by status, listener and remote execution commands"
        );
        return exit::USAGE;
    }
    let code = run_inner(&o.action, o.log.as_ref(), o.trace.as_ref(), o.json);
    if code != exit::OK {
        eprintln!(
            "hint: the calculator must be on, unlocked, and not in exam mode; close the Connectivity Kit if it is running"
        );
    }
    code
}

fn run_inner(
    action: &DeviceAction,
    log: Option<&PathBuf>,
    trace: Option<&PathBuf>,
    json: bool,
) -> u8 {
    if let DeviceAction::Detect = action {
        return match ppl_device::detect() {
            Ok(list) if list.is_empty() => {
                eprintln!(
                    "no HP Prime detected (looked for USB vendor 03F0, products 0441/1541/2441)"
                );
                exit::IO
            }
            Ok(list) => {
                for d in list {
                    println!("{} [{:04x}:{:04x}]", d.model, d.vendor_id, d.product_id);
                    println!("  path:         {}", d.path);
                    println!("  product:      {}", d.product.as_deref().unwrap_or("?"));
                    println!(
                        "  manufacturer: {}",
                        d.manufacturer.as_deref().unwrap_or("?")
                    );
                    println!(
                        "  usb release:  {:x}.{:02x}",
                        d.release >> 8,
                        d.release & 0xff
                    );
                    println!("  hid report:   {} bytes", d.report_size());
                }
                exit::OK
            }
            Err(e) => {
                eprintln!("error: {e}");
                exit::IO
            }
        };
    }
    // Sessions and the key listing need no device.
    if let DeviceAction::Session { action } = action {
        return run_session(action);
    }
    // Cleanup dry-run needs no device; execute/verify open it themselves.
    if let DeviceAction::Cleanup { action } = action {
        return run_cleanup(action);
    }
    if let DeviceAction::List { runs_dir, limit } = action {
        return super::device_delete::list(runs_dir, *limit, json);
    }
    if let DeviceAction::Delete { action } = action {
        return super::device_delete::run(action, json, None, log.map(PathBuf::as_path));
    }
    if let DeviceAction::Key { list: true, .. } = action {
        println!("code name        description");
        for k in ppl_device::KEYS {
            println!("{:4} {:12} {}", k.code, k.name, k.description);
        }
        return exit::OK;
    }
    let send = match action {
        DeviceAction::Send {
            name,
            file,
            control,
            note,
            runs_dir,
        } => match prepare_send(name, file, control.as_deref(), *note, runs_dir) {
            Ok(s) => Some(s),
            Err(code) => return code,
        },
        _ => None,
    };
    let mut p = match Prime::open_first() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    if trace.is_some() {
        p.trace_enabled = true;
    }
    let result = match &send {
        Some(s) => send_program(&mut p, s),
        None => act(&mut p, action),
    };
    if let Some(path) = log {
        if let Err(e) = std::fs::write(path, p.log_text()) {
            eprintln!("error: cannot write log {}: {e}", path.display());
        }
    }
    if let Some(path) = trace {
        if let Err(e) = std::fs::write(path, p.trace_text()) {
            eprintln!("error: cannot write trace {}: {e}", path.display());
        }
    }
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            if let DeviceError::Inconclusive(_) = e {
                eprintln!(
                    "hint: the preflight could not establish that the names are free, so nothing was written. \
                     Make sure the calculator is on, shows the Home screen, is not busy or in exam mode, and \
                     that no other program (e.g. Connectivity Kit) holds the USB connection, then retry. \
                     No control program is required; --control type:name is an optional extra probe."
                );
            }
            exit::IO
        }
    }
}

/// A validated, user-confirmed send request.
struct SendPlan {
    name: String,
    /// For a program send: its future result note. Unused for notes.
    note: String,
    text: String,
    control: Option<ControlObject>,
    /// Send a note (not a program).
    is_note: bool,
    /// Runs directory for the upload ledger.
    runs_dir: PathBuf,
}

fn prepare_send(
    name: &str,
    file: &Path,
    control: Option<&str>,
    is_note: bool,
    runs_dir: &Path,
) -> Result<SendPlan, u8> {
    if let Err(e) = validate_test_name(name) {
        eprintln!("error: {e}");
        return Err(exit::USAGE);
    }
    let note = if is_note {
        String::new()
    } else {
        result_note_name(name)
    };
    if !is_note {
        if let Err(e) = validate_test_name(&note) {
            eprintln!("error: {e}");
            return Err(exit::USAGE);
        }
    }
    let control = match control {
        Some(c) if !c.trim().is_empty() => match ControlObject::parse(c.trim()) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("error: {e}");
                return Err(exit::USAGE);
            }
        },
        _ => None,
    };
    let src = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", file.display());
            return Err(exit::IO);
        }
    };
    let (text, _) = ppl_syntax::decode_source(&src);

    if is_note {
        println!("About to create NOTE `{name}` on the calculator.");
        println!(
            "On the calculator, open the Notes catalog and check that `{name}` is NOT listed."
        );
        println!("The emulator never overwrites; this is an extra check.");
    } else {
        println!("About to create program `{name}` on the calculator (result note `{note}`).");
        println!("On the calculator, open the Program Catalog ([Shift] [1]) and check that");
        println!("`{name}` is NOT listed. The emulator never overwrites; this is an extra check.");
    }
    print!("Type the object name exactly to confirm it is not present: ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(n) if n > 0 && line.trim_end_matches(['\r', '\n']) == name => {}
        Ok(_) => {
            eprintln!("refused: confirmation did not match `{name}`; nothing was sent");
            return Err(exit::USAGE);
        }
        Err(e) => {
            eprintln!("error reading confirmation: {e}; nothing was sent");
            return Err(exit::IO);
        }
    }
    Ok(SendPlan {
        name: name.to_string(),
        note,
        text,
        control,
        is_note,
        runs_dir: runs_dir.to_path_buf(),
    })
}

/// Appends one line to `<runs_dir>/transfer-attempts.jsonl` for a transfer
/// that did not end verified. Kept apart from the ownership ledger, which
/// only ever lists read-back-verified objects, so a failed or indeterminate
/// upload is recorded without claiming ownership or presence.
fn record_attempt(s: &SendPlan, state: &str, detail: &str, bytes: usize) {
    let line = serde_json::json!({
        "utc": ppl_compat::utc_now(),
        "name": s.name,
        "kind": if s.is_note { "note" } else { "program" },
        "state": state,
        "payload_bytes": bytes,
        "detail": detail,
    });
    let path = s.runs_dir.join("transfer-attempts.jsonl");
    let res = std::fs::create_dir_all(&s.runs_dir).and_then(|_| {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(f, "{line}")
    });
    if let Err(e) = res {
        eprintln!("warning: cannot record attempt in {}: {e}", path.display());
    }
}

fn send_program(p: &mut Prime, s: &SendPlan) -> Result<u8, DeviceError> {
    let report = if s.is_note {
        if let Some(ctrl) = &s.control {
            println!(
                "preflight: control {}, target note:{} (about 20 s)...",
                ctrl, s.name
            );
        } else {
            println!(
                "preflight: device-level verification, target note:{} (about 16 s)...",
                s.name
            );
        }
        let guard = p.preflight_new_names(&[(FileType::Note, &s.name)], s.control.as_ref())?;
        for line in guard.log() {
            println!("  {line}");
        }
        println!(
            "Transfer initiated: sending note `{}` ({} UTF-16 bytes)...",
            s.name,
            s.text.encode_utf16().count() * 2
        );
        p.send_new_note(guard, &s.text)?
    } else {
        if let Some(ctrl) = &s.control {
            println!(
                "preflight: control {}, targets program:{} and note:{} (about 40 s)...",
                ctrl, s.name, s.note
            );
        } else {
            println!(
                "preflight: device-level verification, targets program:{} and note:{} (about 32 s)...",
                s.name, s.note
            );
        }
        let guard = p.preflight_new_names(
            &[(FileType::Program, &s.name), (FileType::Note, &s.note)],
            s.control.as_ref(),
        )?;
        for line in guard.log() {
            println!("  {line}");
        }
        println!(
            "Transfer initiated: sending program `{}` ({} UTF-16 bytes)...",
            s.name,
            s.text.encode_utf16().count() * 2
        );
        p.send_new_program(guard, &s.text)?
    };

    println!(
        "Payload transmitted: {} bytes submitted to USB endpoint for `{}` (no write ACK on Prime wire protocol)",
        report.bytes_sent, report.name
    );
    println!(
        "Device acknowledgement: none exists in this protocol. Awaiting read-back (F8) to verify..."
    );

    match &report.readback {
        ReadBack::Matches => {
            println!(
                "Read-back verified: exact match ({} chars, CRC valid)",
                s.text.chars().count()
            );
            let identity_log_start = p.log.len();
            let info = p.info();
            // The added ownership query must not leak raw serial bytes into --log.
            p.log.truncate(identity_log_start);
            let info = info?;
            let serial = info.serial.as_deref().ok_or_else(|| {
                DeviceError::Refused("cannot record ownership without device identity".into())
            })?;
            let serial_id = ppl_compat::serial_fingerprint(serial);
            let ty = if s.is_note {
                FileType::Note
            } else {
                FileType::Program
            };
            let file = match p.request_file(ty, &s.name)? {
                Probe::Present(f)
                    if f.name == s.name
                        && f.ty == ty
                        && f.crc_ok
                        && ppl_device::normalize_program_text(&f.text())
                            == ppl_device::normalize_program_text(&s.text) =>
                {
                    f
                }
                _ => {
                    return Err(DeviceError::Refused(
                        "creation receipt read-back failed; object retained for inspection".into(),
                    ));
                }
            };
            super::device_delete_store::record_creation(
                &s.runs_dir,
                &serial_id,
                ty,
                &s.name,
                &file.data,
            )?;
            // Record ownership so `device delete` / `device list` know
            // this object belongs to this tool.
            let names: Vec<String> = if s.is_note {
                vec![s.name.clone()]
            } else {
                vec![s.name.clone(), s.note.clone()]
            };
            if let Err(e) =
                ppl_compat::ledger_append(&s.runs_dir, &names, &s.name, &ppl_compat::utc_now())
            {
                eprintln!("warning: ledger: {e}");
            }
            if !s.is_note {
                if let Err(e) =
                    register_object_manifest(&s.runs_dir, &s.name, Some(&s.note), &s.text, None)
                {
                    eprintln!("warning: manifest: {e}");
                }
            }
            Ok(exit::OK)
        }
        ReadBack::Differs {
            sent_chars,
            got_chars,
            ..
        } => {
            record_attempt(
                s,
                "readback_differs",
                &format!("sent {sent_chars} chars, got {got_chars}"),
                report.bytes_sent,
            );
            println!(
                "Read-back differs: sent {sent_chars} chars, got {got_chars}; inspect with `ppl device get program {}`",
                report.name
            );
            Ok(exit::IO)
        }
        ReadBack::NoReply => {
            record_attempt(
                s,
                "indeterminate_no_readback",
                "payload transmitted; no F7 reply to the single read-back F8",
                report.bytes_sent,
            );
            println!(
                "Outcome INDETERMINATE: `{}` was transmitted but the calculator gave no read-back within {} s. \
                 Installation is neither confirmed nor ruled out; it was NOT retried. Do not resend under another name. \
                 Check the Program Catalog, or later run `ppl device get program {}` (read-only).",
                report.name,
                ppl_device::READBACK_TIMEOUT.as_secs(),
                report.name
            );
            Ok(exit::IO)
        }
        ReadBack::Failed { error } => {
            record_attempt(s, "indeterminate_readback_failed", error, report.bytes_sent);
            println!(
                "Outcome INDETERMINATE: read-back failed ({error}); installation is not verified and was not retried"
            );
            Ok(exit::IO)
        }
    }
}

fn act(p: &mut Prime, action: &DeviceAction) -> Result<u8, DeviceError> {
    match action {
        DeviceAction::Status { .. }
        | DeviceAction::Exec { .. }
        | DeviceAction::ExecFile { .. }
        | DeviceAction::Listener { .. }
        | DeviceAction::Detect
        | DeviceAction::Send { .. }
        | DeviceAction::Session { .. }
        | DeviceAction::Cleanup { .. }
        | DeviceAction::Delete { .. }
        | DeviceAction::List { .. } => unreachable!(),
        DeviceAction::Ping => {
            let r = p.check_ready()?;
            println!("check-ready reply: {}", hex(&r));
        }
        DeviceAction::Info { show_serial } => {
            let show_serial = *show_serial;
            let info = p.info()?;
            println!("model:          {}", p.detected.model);
            match info.build {
                Some(b) => println!("firmware build: {b}"),
                None => println!("firmware build: (not recognised; see raw reply)"),
            }
            println!("boot version:   {}", info.version.as_deref().unwrap_or("?"));
            println!(
                "serial:         {}",
                if show_serial {
                    info.serial.clone().unwrap_or_default()
                } else {
                    "(hidden; --show-serial)".into()
                }
            );
            println!(
                "serial id:      {}",
                info.serial_fingerprint().unwrap_or_default()
            );
            if show_serial {
                println!("raw ({} bytes): {}", info.raw.len(), hex(&info.raw));
            }
        }
        DeviceAction::Screenshot { out, format } => {
            let png = p.screenshot(*format)?;
            let io = |e: String| DeviceError::Hid(e);
            let raw_path = out.with_extension("raw.png");
            std::fs::write(&raw_path, &png).map_err(|e| io(e.to_string()))?;
            let img = ppl_graphics::Grob::from_prime_screenshot(&png).map_err(io)?;
            img.save_png(out).map_err(|e| io(e.to_string()))?;
            println!(
                "saved {} ({}×{}, decoded from RGB565); raw reply in {}",
                out.display(),
                img.width(),
                img.height(),
                raw_path.display()
            );
        }
        DeviceAction::Get { ty, name, out } => {
            let t = FileType::parse(ty)
                .ok_or_else(|| DeviceError::Refused(format!("unknown object type `{ty}`")))?;
            match p.request_file(t, name)? {
                Probe::NoReply { waited } => {
                    println!(
                        "{name}: no reply within {} s — this does NOT prove the object is absent",
                        waited.as_secs()
                    );
                    return Ok(exit::IO);
                }
                Probe::Present(f) => {
                    eprintln!(
                        "{} {:?} {} bytes, crc {}",
                        f.name,
                        f.ty,
                        f.data.len(),
                        if f.crc_ok { "ok" } else { "MISMATCH" }
                    );
                    match out {
                        Some(path) => std::fs::write(path, &f.data)
                            .map_err(|e| DeviceError::Hid(e.to_string()))?,
                        None if matches!(f.ty, FileType::Program | FileType::Note) => {
                            println!("{}", f.text())
                        }
                        None => println!("{}", hex(&f.data)),
                    }
                }
            }
        }
        DeviceAction::Run {
            remote: _,
            args: _,
            name,
            note,
            timeout_secs,
            poll_secs,
            control,
            out,
            no_header_check,
            no_audit,
            audit_path,
        } => {
            let s = control.as_deref().unwrap_or(DEFAULT_CONTROL);
            let ctrl = match ControlObject::parse(s) {
                Ok(c) => c,
                Err(e) => return Err(DeviceError::Refused(format!("--control: {e}"))),
            };
            let mut cfg = ppl_device::RunConfig::new(name, ctrl);
            if let Some(n) = note {
                cfg.note = n.clone();
            }
            if let Some(s) = *timeout_secs {
                cfg.timeout = std::time::Duration::from_secs(s);
            }
            if let Some(s) = *poll_secs {
                cfg.poll = std::time::Duration::from_secs(s);
            }
            if *no_header_check {
                cfg.expect_header = None;
            }
            let started = std::time::Instant::now();
            let note_name = cfg.note.clone();
            let program = name.clone();
            let outcome = ppl_device::run_program(p, &cfg, |ev| match ev {
                ppl_device::RunEvent::PreflightDone => println!(
                    "preflight ok — press Run on the calculator NOW \
                     (program `{program}`, waiting for note `{note_name}`)"
                ),
                ppl_device::RunEvent::Poll { attempt, of } => {
                    if attempt < 3 || attempt % 15 == 0 {
                        println!(
                            "  …waiting (poll {attempt}/{of}, {} s elapsed)",
                            started.elapsed().as_secs()
                        );
                    }
                }
                ppl_device::RunEvent::NoteSeen => {
                    println!("result note `{note_name}` appeared; settling")
                }
                ppl_device::RunEvent::Retrieved => println!("result note retrieved"),
            })
            .map_err(|e| DeviceError::Refused(format!("{e}")))?;
            let (code, kind, sha) = match &outcome {
                ppl_device::RunOutcome::Captured { text, polls } => {
                    let sha = ppl_compat::sha256_hex(text.as_bytes());
                    match out {
                        Some(path) => std::fs::write(path, text.as_bytes())
                            .map_err(|e| DeviceError::Hid(format!("{}: {e}", path.display())))?,
                        None => println!("{text}"),
                    }
                    println!(
                        "captured `{note_name}` ({polls} polls, {} s, sha256 {sha})",
                        started.elapsed().as_secs()
                    );
                    (exit::OK, "captured", Some(sha))
                }
                ppl_device::RunOutcome::TimedOut { note, polls } => {
                    println!(
                        "timed out after {polls} polls ({}) waiting for note `{note}`; \
                         the program may still be running — retry `ppl device run {name}` \
                         only after checking the Program Catalog",
                        started.elapsed().as_secs()
                    );
                    (exit::IO, "timeout", None)
                }
                ppl_device::RunOutcome::HeaderMismatch { note, expected } => {
                    println!(
                        "note `{note}` does not carry the expected header `{expected}`; \
                         not captured as evidence"
                    );
                    (exit::IO, "header-mismatch", None)
                }
            };
            if !*no_audit {
                let line = format!(
                    "{{\"utc\":\"{}\",\"program\":\"{}\",\"note\":\"{}\",\"outcome\":\"{kind}\",\"polls\":{}{}}}\n",
                    ppl_compat::utc_now(),
                    program,
                    note_name,
                    match &outcome {
                        ppl_device::RunOutcome::Captured { polls, .. } => *polls,
                        ppl_device::RunOutcome::TimedOut { polls, .. } => *polls,
                        ppl_device::RunOutcome::HeaderMismatch { .. } => 0,
                    },
                    sha.map(|s| format!(",\"sha256\":\"{s}\""))
                        .unwrap_or_default()
                );
                if let Some(parent) = audit_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                use std::io::Write as _;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(audit_path)
                {
                    let _ = f.write_all(line.as_bytes());
                } else {
                    eprintln!(
                        "warning: could not append run audit to {}",
                        audit_path.display()
                    );
                }
            }
            return Ok(code);
        }
        DeviceAction::Key {
            name,
            sequence,
            session,
            screenshot_after,
            ..
        } => {
            let (name, sequence, session, screenshot_after) = (
                name.clone(),
                sequence.clone(),
                session.clone(),
                screenshot_after.clone(),
            );
            // Parse and authorize first; nothing is sent on refusal.
            let mut names: Vec<String> = Vec::new();
            if let Some(n) = &name {
                names.push(n.clone());
            }
            if let Some(seq) = &sequence {
                if let Err(e) = ppl_device::parse_key_sequence(seq) {
                    eprintln!("error: {e}");
                    return Err(DeviceError::Refused(e));
                }
                names.extend(seq.split_whitespace().map(String::from));
            }
            if names.is_empty() {
                eprintln!("error: pass --name <key> or --sequence \"key key\"");
                return Err(DeviceError::Refused("no key given".into()));
            }
            let Some(session) = session else {
                eprintln!(
                    "error: --session <ID> is required (start one with `ppl device session start <ID>`)"
                );
                return Err(DeviceError::Refused("no session given".into()));
            };
            let path = session_path(SESSIONS_DIR, &session);
            let mut sess = match DeviceSession::load(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "error: {e}\nhint: start one with `ppl device session start {session}`"
                    );
                    return Err(DeviceError::Refused(e));
                }
            };
            let mut codes = Vec::new();
            for n in &names {
                match sess.authorize_press(n) {
                    Ok(c) => codes.push(c),
                    Err(e) => {
                        // refuse the whole batch: do not press part of it
                        eprintln!("refused: {e}");
                        return Err(DeviceError::Refused(e));
                    }
                }
            }
            println!(
                "session {}: pressing {} ({} of {} presses)",
                sess.id,
                describe_codes(&codes),
                sess.presses.len(),
                sess.max_presses
            );
            p.press_keys(&codes)?;
            if let Err(e) = sess.save(&path) {
                eprintln!("error: cannot record press in session: {e}");
            }
            println!("sent.");
            if let Some(out) = screenshot_after {
                let png = p.screenshot(8)?;
                let raw_path = out.with_extension("raw.png");
                std::fs::write(&raw_path, &png).map_err(|e| DeviceError::Hid(e.to_string()))?;
                let img =
                    ppl_graphics::Grob::from_prime_screenshot(&png).map_err(DeviceError::Hid)?;
                img.save_png(&out)
                    .map_err(|e| DeviceError::Hid(e.to_string()))?;
                println!(
                    "screenshot after press: {} ({}×{})",
                    out.display(),
                    img.width(),
                    img.height()
                );
            }
        }
    }
    Ok(exit::OK)
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------- cleanup

/// `ppl device cleanup …`: ownership-verified cleanup of this project's own
/// test objects (see `ppl_compat::cleanup`). Deletion is performed on the
/// calculator by a generated PPL helper using the documented
/// `Programs("name"):=""` / `Notes("name"):=""` mechanism.
fn run_cleanup(action: &CleanupAction) -> u8 {
    use ppl_compat::cleanup as cu;
    match action {
        CleanupAction::DryRun {
            campaign,
            runs_dir,
            campaigns_dir,
        } => match cu::plan_cleanup(runs_dir, campaigns_dir)
            .and_then(|mut plan| filter_campaign(&mut plan, campaign, campaigns_dir).map(|_| plan))
        {
            Ok(plan) => {
                println!("--- cleanup dry run (nothing is written) ---");
                println!(
                    "eligible targets: {} (presence on the calculator is checked at execution)",
                    plan.targets.len()
                );
                for t in &plan.targets {
                    println!(
                        "  {:<12} {:<8} owner {} — {}",
                        t.name,
                        t.ty.as_str(),
                        t.program,
                        t.reason
                    );
                }
                println!("\nexcluded:");
                for s in &plan.skipped {
                    println!("  {:<12} {}", s.name, s.reason);
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                return exit::IO;
            }
        },
        CleanupAction::Execute {
            poc,
            campaign,
            runs_dir,
            campaigns_dir,
            cleanup_dir,
            control,
        } => {
            return cleanup_execute(
                *poc,
                campaign.as_deref(),
                runs_dir,
                campaigns_dir,
                cleanup_dir,
                control,
            );
        }
        CleanupAction::Verify {
            runs_dir,
            cleanup_dir,
        } => {
            return cleanup_verify(runs_dir, cleanup_dir);
        }
        CleanupAction::Status { cleanup_dir } => match cu::load_batches(cleanup_dir) {
            Ok(batches) => {
                if batches.is_empty() {
                    println!("no cleanup batches recorded yet");
                }
                for b in &batches {
                    println!(
                        "helper {} planned {} executed {} verified {}",
                        b.helper,
                        b.planned_at_utc,
                        b.executed_at_utc.as_deref().unwrap_or("-"),
                        b.verified_at_utc.as_deref().unwrap_or("-")
                    );
                    for t in &b.targets {
                        println!("  {:<12} {:?}", t.name, t.state);
                    }
                    for s in &b.skipped {
                        println!("  (skipped) {:<12} {}", s.name, s.reason);
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                return exit::IO;
            }
        },
    }
    exit::OK
}

/// Fresh `ZZT_CLEANnn` helper name (ledger + manifest checked).
fn fresh_helper_name(runs_dir: &Path) -> Result<String, String> {
    for n in 1..=99u32 {
        let name = format!("ZZT_CLEAN{n:02}");
        if !ppl_compat::ledger_contains(runs_dir, &name)? {
            return Ok(name);
        }
    }
    Err("no unused ZZT_CLEANnn name".into())
}

/// Writes a minimal manifest so the helper/POC objects are owned by the
/// ledger from the start (cleanup planning needs manifest ownership).
fn register_object_manifest(
    runs_dir: &Path,
    program: &str,
    note: Option<&str>,
    source: &str,
    note_content: Option<&str>,
) -> Result<(), String> {
    use ppl_compat::RunManifest;
    let m = RunManifest {
        schema: 2,
        suite: "cleanup".into(),
        group: None,
        program: program.into(),
        note: note.map(String::from),
        channel: "note".into(),
        case_ids: vec![],
        short_ids: vec![],
        source: source.into(),
        sha256: ppl_compat::sha256_hex(source.as_bytes()),
        created_at_utc: ppl_compat::utc_now(),
        uploaded_at_utc: None,
        preflight_log: vec![],
        device: None,
        status: ppl_compat::RunStatus::Uploaded,
        blocked_reason: None,
        // Hash of the *normalized* content: the hardware round-trips
        // UTF-16 with possible BOM/CRLF, so identity is compared the same
        // way upload read-back does (normalize_program_text).
        note_sha256: note_content
            .map(|c| ppl_compat::sha256_hex(ppl_device::normalize_program_text(c).as_bytes())),
    };
    ppl_compat::save_manifest(runs_dir, &m).map(|_| ())
}

#[allow(clippy::too_many_lines)]
/// Restricts a cleanup plan to one campaign's objects (`--campaign ID`).
/// Targets survive only when their owner program belongs to the campaign;
/// skipped entries are filtered the same way so the display stays honest.
fn filter_campaign(
    plan: &mut ppl_compat::cleanup::CleanupPlan,
    campaign: &Option<String>,
    campaigns_dir: &Path,
) -> Result<(), String> {
    if let Some(id) = campaign {
        let objs = ppl_compat::cleanup::campaign_objects(campaigns_dir, id)?;
        plan.targets.retain(|t| objs.contains(&t.program));
        plan.skipped.retain(|s| objs.contains(&s.name));
    }
    Ok(())
}

fn cleanup_execute(
    poc: bool,
    campaign: Option<&str>,
    runs_dir: &Path,
    campaigns_dir: &Path,
    cleanup_dir: &Path,
    control: &str,
) -> u8 {
    use ppl_compat::cleanup as cu;
    use ppl_device::FileType;

    // PoC mode: create the disposable objects first.
    if poc {
        let poc_src = "EXPORT ZZT_POC01()\nBEGIN\n  RETURN 1;\nEND;\n";
        println!("--- proof of concept: disposable objects ---");
        let mut p = match Prime::open_first() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return exit::IO;
            }
        };
        let poc_note = "ZZT_POCN1".to_string();
        for (name, targets) in [
            (
                "ZZT_POC01",
                vec![(FileType::Program, "ZZT_POC01".to_string())],
            ),
            ("ZZT_POCN1", vec![(FileType::Note, poc_note.clone())]),
        ] {
            let refs: Vec<(FileType, &str)> =
                targets.iter().map(|(t, n)| (*t, n.as_str())).collect();
            print!("creating {name}: type the name to confirm: ");
            let _ = io::stdout().flush();
            let mut line = String::new();
            if io::stdin().lock().read_line(&mut line).is_err() || line.trim() != name {
                eprintln!("refused: confirmation did not match; nothing was sent");
                return exit::USAGE;
            }
            let ctrl = match ControlObject::parse(control) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            let guard = match p.preflight_new_names(&refs, Some(&ctrl)) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("error: preflight refused: {e}");
                    return exit::IO;
                }
            };
            let r = match targets[0].0 {
                FileType::Program => p.send_new_program(guard, poc_src),
                _ => p.send_new_note(guard, "poc note for deletion test"),
            };
            match r {
                Ok(report) if matches!(report.readback, ReadBack::Matches) => {
                    ppl_compat::ledger_append(
                        runs_dir,
                        &[name.to_string()],
                        "ZZT_POC01",
                        &ppl_compat::utc_now(),
                    )
                    .map_err(|e| eprintln!("warning: ledger: {e}"))
                    .ok();
                }
                Ok(_) => {
                    eprintln!("error: read-back did not confirm; aborting");
                    return exit::IO;
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            }
        }
        register_object_manifest(
            runs_dir,
            "ZZT_POC01",
            Some("ZZT_POCN1"),
            poc_src,
            Some("poc note for deletion test"),
        )
        .map_err(|e| eprintln!("warning: manifest: {e}"))
        .ok();
        println!("disposable objects created (ZZT_POC01 program, ZZT_POCN1 note)");
    }

    // Plan (PoC restricts the target set to the POC objects).
    let mut plan = match cu::plan_cleanup(runs_dir, campaigns_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    if poc {
        plan.targets
            .retain(|t| t.name == "ZZT_POC01" || t.name == "ZZT_POCN1");
        plan.skipped.clear();
    }
    if let Err(e) = filter_campaign(&mut plan, &campaign.map(String::from), campaigns_dir) {
        eprintln!("error: {e}");
        return exit::IO;
    }
    if plan.targets.is_empty() {
        println!("nothing eligible to clean");
        return exit::OK;
    }

    // Presence probe + backup + identity verification.
    let mut p = match Prime::open_first() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let backup_dir = PathBuf::from("tests/compat/backups");
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| {
            eprintln!("error: {}: {e}", backup_dir.display());
            exit::IO
        })
        .ok();
    let mut present = Vec::new();
    for t in plan.targets.iter_mut() {
        let ty = match t.ty {
            cu::ObjType::Program => FileType::Program,
            cu::ObjType::Note => FileType::Note,
        };
        let probe = p.probe_file(ty, &t.name, std::time::Duration::from_secs(3));
        match probe {
            Ok(ppl_device::Probe::Present(f)) => {
                // Identity verification before accepting the object.
                let text = f.text();
                let ok = match (&t.ownership, t.ty) {
                    (cu::Ownership::ManifestRun, cu::ObjType::Program) => {
                        let m = ppl_compat::load_manifest(runs_dir, &t.program);
                        match m {
                            Ok(m) => {
                                ppl_device::normalize_program_text(&text)
                                    == ppl_device::normalize_program_text(&m.source)
                            }
                            Err(_) => false,
                        }
                    }
                    (cu::Ownership::ManifestRun, cu::ObjType::Note) => {
                        if text.contains(&format!("@@ZZT|{}|", t.program)) {
                            true
                        } else {
                            // Plain note we created: verify the recorded
                            // content hash when one exists.
                            ppl_compat::load_manifest(runs_dir, &t.program)
                                .ok()
                                .and_then(|m| m.note_sha256)
                                .is_some_and(|want| {
                                    let got = ppl_device::normalize_program_text(&text);
                                    ppl_compat::sha256_hex(got.as_bytes()) == want
                                })
                        }
                    }
                    (cu::Ownership::KnownProbe, _) => {
                        let prefix = cu::KNOWN_PROBE_NOTES
                            .iter()
                            .find(|(n, _)| n == &t.name)
                            .map(|(_, p)| *p)
                            .unwrap_or("");
                        text.starts_with(prefix)
                    }
                };
                if !ok {
                    t.state = cu::TargetState::Blocked(
                        "content does not match our records — not deleting".into(),
                    );
                    println!("  BLOCKED {}: identity mismatch", t.name);
                    continue;
                }
                // Backup.
                let ext = if t.ty == cu::ObjType::Program {
                    "hpprgm"
                } else {
                    "note"
                };
                let path = backup_dir.join(format!("{}.{}", t.name, ext));
                if path.exists() {
                    eprintln!(
                        "error: backup {} already exists; refusing to overwrite",
                        path.display()
                    );
                    t.state = cu::TargetState::Blocked("backup exists".into());
                    continue;
                }
                std::fs::write(&path, f.data.clone())
                    .map_err(|e| {
                        eprintln!("error: {}: {e}", path.display());
                        exit::IO
                    })
                    .ok();
                t.backup = Some(path.display().to_string());
                t.backup_sha256 = Some(ppl_compat::sha256_hex(&f.data));
                t.state = cu::TargetState::BackedUp;
                present.push(t.clone());
                println!("  backed up {} ({} bytes)", t.name, f.data.len());
            }
            Ok(ppl_device::Probe::NoReply { .. }) => {
                t.state = cu::TargetState::VerifiedDeleted;
                println!("  {} already absent", t.name);
            }
            Err(e) => {
                t.state = cu::TargetState::Unknown(format!("probe failed: {e}"));
                println!("  UNKNOWN {}: {e}", t.name);
            }
        }
    }
    let program_names: Vec<String> = present
        .iter()
        .filter(|t| t.ty == cu::ObjType::Program)
        .map(|t| t.name.clone())
        .collect();
    let note_names: Vec<String> = present
        .iter()
        .filter(|t| t.ty == cu::ObjType::Note)
        .map(|t| t.name.clone())
        .collect();
    if present.is_empty() {
        println!("no present, backed-up targets — nothing to upload a helper for");
        let mut batches = cu::load_batches(cleanup_dir).unwrap_or_default();
        batches.push(cu::CleanupBatch {
            helper: "-".into(),
            planned_at_utc: plan.planned_at_utc.clone(),
            executed_at_utc: None,
            verified_at_utc: None,
            targets: plan.targets.clone(),
            skipped: plan.skipped.clone(),
        });
        let _ = cu::save_batches(cleanup_dir, &batches);
        return exit::OK;
    }

    // Generate + verify the helper.
    let helper = match fresh_helper_name(runs_dir) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let refs: Vec<&str> = program_names.iter().map(String::as_str).collect();
    let nrefs: Vec<&str> = note_names.iter().map(String::as_str).collect();
    let source = cu::generate_cleanup_helper(&helper, &refs, &nrefs);
    if let Err(errs) = cu::verify_helper(&source, &helper, &refs, &nrefs) {
        for e in errs {
            eprintln!("error: helper verification: {e}");
        }
        return exit::USAGE;
    }
    println!("\n--- cleanup helper {} ---", helper);
    println!("{}", source);
    println!("deleting exactly:");
    for n in &program_names {
        println!("  program {n}");
    }
    for n in &note_names {
        println!("  note    {n}");
    }
    println!("\nThis deletes ONLY the listed objects (verified against the ownership ledger).");
    print!("Type the helper name {} to authorize: ", helper);
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line).is_err() || line.trim() != helper {
        eprintln!("refused: authorization did not match; nothing was sent");
        return exit::USAGE;
    }

    // Guarded upload of the helper (program + its result note).
    let ctrl = match ControlObject::parse(control) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let note = ppl_device::result_note_name(&helper);
    let targets = [
        (FileType::Program, helper.clone()),
        (FileType::Note, note.clone()),
    ];
    let trefs: Vec<(FileType, &str)> = targets.iter().map(|(t, n)| (*t, n.as_str())).collect();
    let guard = match p.preflight_new_names(&trefs, Some(&ctrl)) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: preflight refused: {e}");
            return exit::IO;
        }
    };
    ppl_compat::ledger_append(
        runs_dir,
        &[helper.clone(), note.clone()],
        &helper,
        &ppl_compat::utc_now(),
    )
    .map_err(|e| {
        eprintln!("error: {e}");
        exit::IO
    })
    .ok();
    register_object_manifest(runs_dir, &helper, Some(&note), &source, None)
        .map_err(|e| eprintln!("warning: manifest: {e}"))
        .ok();
    let report = match p.send_new_program(guard, &source) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: upload failed: {e}");
            return exit::IO;
        }
    };
    if !matches!(report.readback, ReadBack::Matches) {
        eprintln!("error: read-back did not confirm; inspect with `ppl device get`");
        return exit::IO;
    }

    // Record the batch.
    for t in plan.targets.iter_mut() {
        if present.iter().any(|p| p.name == t.name) {
            t.state = cu::TargetState::Authorized;
        }
    }
    let batch = cu::CleanupBatch {
        helper: helper.clone(),
        planned_at_utc: plan.planned_at_utc.clone(),
        executed_at_utc: Some(ppl_compat::utc_now()),
        verified_at_utc: None,
        targets: plan.targets.clone(),
        skipped: plan.skipped.clone(),
    };
    let mut batches = cu::load_batches(cleanup_dir).unwrap_or_default();
    batches.retain(|b| b.helper != batch.helper);
    batches.push(batch);
    if let Err(e) = cu::save_batches(cleanup_dir, &batches) {
        eprintln!("error: {e}");
        return exit::IO;
    }
    println!(
        "\nUploaded cleanup helper `{}` (read-back verified).",
        helper
    );
    println!("=== MANUAL STEP ===");
    println!(
        "1. Run `{}` in the Program catalog on the calculator.",
        helper
    );
    println!(
        "2. Then run: ppl device cleanup verify   (I will check the result and verify absence)"
    );
    exit::OK
}

fn cleanup_verify(_runs_dir: &Path, cleanup_dir: &Path) -> u8 {
    use ppl_compat::cleanup as cu;
    use ppl_device::FileType;
    let mut batches = match cu::load_batches(cleanup_dir) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let Some(idx) = batches.iter().position(|b| b.verified_at_utc.is_none()) else {
        println!("no unverified cleanup batches");
        return exit::OK;
    };
    let helper = batches[idx].helper.clone();
    if helper == "-" {
        batches[idx].verified_at_utc = Some(ppl_compat::utc_now());
        let _ = cu::save_batches(cleanup_dir, &batches);
        println!("empty batch marked verified");
        return exit::OK;
    }
    let mut p = match Prime::open_first() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    // Control must answer, or nothing can be concluded.
    if !matches!(
        p.request_file(FileType::Program, "ZZT_CTRL"),
        Ok(ppl_device::Probe::Present(_))
    ) {
        eprintln!("error: control ZZT_CTRL does not answer; nothing can be verified");
        return exit::IO;
    }
    // Helper result note: "done" means the helper finished its deletions.
    let helper_note = ppl_device::result_note_name(&helper);
    let status = match p.probe_file(
        FileType::Note,
        &helper_note,
        std::time::Duration::from_secs(5),
    ) {
        Ok(ppl_device::Probe::Present(f)) => {
            let text = f.text();
            if text.contains("done") {
                "done"
            } else if text.contains("running") {
                "running (helper may have been interrupted)"
            } else {
                "unknown"
            }
        }
        _ => "absent (helper note missing — did the helper run?)",
    };
    println!("helper {helper}: note {helper_note} says: {status}");
    let mut verified = 0;
    let mut unknown = 0;
    for t in batches[idx].targets.iter_mut() {
        if matches!(
            t.state,
            cu::TargetState::Authorized | cu::TargetState::Unknown(_)
        ) {
            let ty = match t.ty {
                cu::ObjType::Program => FileType::Program,
                cu::ObjType::Note => FileType::Note,
            };
            match p.probe_file(ty, &t.name, std::time::Duration::from_secs(3)) {
                Ok(ppl_device::Probe::NoReply { .. }) => {
                    t.state = cu::TargetState::VerifiedDeleted;
                    println!("  {}: gone", t.name);
                    verified += 1;
                }
                Ok(ppl_device::Probe::Present(_)) => {
                    t.state = cu::TargetState::Unknown("still present after cleanup".into());
                    println!("  {}: STILL PRESENT", t.name);
                    unknown += 1;
                }
                Err(e) => {
                    t.state = cu::TargetState::Unknown(format!("probe failed: {e}"));
                    println!("  {}: probe failed: {e}", t.name);
                    unknown += 1;
                }
            }
        }
    }
    batches[idx].verified_at_utc = Some(ppl_compat::utc_now());
    let _ = cu::save_batches(cleanup_dir, &batches);
    println!(
        "\nverified deleted: {} | needs review: {} (states recorded in {}/batches.json)",
        verified,
        unknown,
        cleanup_dir.display()
    );
    println!("run `ppl device cleanup status` for the full history");
    if unknown > 0 { exit::IO } else { exit::OK }
}
