//! Managed M6/M7 helper. No shell commands and no native compiler fallback.
pub use crate::firmware_options::Options;
use crate::{
    Cancellation, Diagnostic, Engine, Operation, OutputEvent, Report, Status,
    source::{self, Source},
};
use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use std::{
    io::Read,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn executable(p: &Path) -> bool {
    p.is_file()
        && p.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}
pub fn discover_runner(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return if executable(p) {
            p.canonicalize().map_err(|e| e.to_string())
        } else {
            Err(format!(
                "firmware runner is not executable: {}",
                p.display()
            ))
        };
    }
    if let Ok(exe) = std::env::current_exe() {
        let p = exe.with_file_name("prime-emulator");
        if executable(&p) {
            return Ok(p);
        }
    }
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let p = dir.join("prime-emulator");
        if executable(&p) {
            return p.canonicalize().map_err(|e| e.to_string());
        }
    }
    Err("prime-emulator was not found; install the helper or set --firmware-runner / PPL_FIRMWARE_RUNNER".into())
}
struct Process {
    child: Child,
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = killpg(Pid::from_raw(self.child.id() as i32), Signal::SIGKILL);
        let _ = self.child.wait();
    }
}
struct Captured {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
    stop: Option<Status>,
}
fn drain(mut pipe: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let mut data = vec![];
    let mut truncated = false;
    let mut buf = [0; 8192];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let keep = n.min(limit.saturating_sub(data.len()));
                data.extend_from_slice(&buf[..keep]);
                truncated |= keep < n;
            }
        }
    }
    (data, truncated)
}
fn supervise(
    mut cmd: Command,
    timeout: Duration,
    cancel: &Cancellation,
) -> Result<Captured, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut p = Process {
        child: cmd.spawn().map_err(|e| e.to_string())?,
    };
    let out = p.child.stdout.take().unwrap();
    let err = p.child.stderr.take().unwrap();
    let out = std::thread::spawn(move || drain(out, 16 * 1024 * 1024));
    let err = std::thread::spawn(move || drain(err, 1024 * 1024));
    let started = Instant::now();
    let mut stop = None;
    let mut stopped = None;
    let mut term = false;
    let mut killed = false;
    let status = loop {
        if let Some(s) = p.child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if stop.is_none() && (cancel.cancelled() || started.elapsed() >= timeout) {
            stop = Some(if cancel.cancelled() {
                Status::Cancelled
            } else {
                Status::TimedOut
            });
            stopped = Some(Instant::now());
            let _ = killpg(Pid::from_raw(p.child.id() as i32), Signal::SIGINT);
        }
        if let Some(t) = stopped {
            if !term && t.elapsed() >= Duration::from_secs(1) {
                let _ = killpg(Pid::from_raw(p.child.id() as i32), Signal::SIGTERM);
                term = true;
            }
            if !killed && t.elapsed() >= Duration::from_millis(1250) {
                let _ = killpg(Pid::from_raw(p.child.id() as i32), Signal::SIGKILL);
                killed = true;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // A helper must not leave descendants holding its output pipes open.
    let _ = killpg(Pid::from_raw(p.child.id() as i32), Signal::SIGKILL);
    let (stdout, a) = out.join().map_err(|_| "stdout capture failed")?;
    let (stderr, b) = err.join().map_err(|_| "stderr capture failed")?;
    Ok(Captured {
        code: status.code(),
        stdout,
        stderr,
        truncated: a || b,
        stop,
    })
}
fn absolute(p: &Path) -> Result<PathBuf, String> {
    p.canonicalize()
        .map_err(|e| format!("{}: {e}", p.display()))
}
#[derive(Deserialize)]
struct Identity {
    schema_version: u32,
    engine: String,
    request_id: String,
    program: String,
    source_sha256: Option<String>,
}
fn diagnostic(v: &Value) -> Diagnostic {
    Diagnostic {
        source: v["source"].as_str().unwrap_or("guest-firmware").into(),
        severity: v["severity"].as_str().unwrap_or("error").into(),
        code: v["code"].as_str().map(str::to_owned).or_else(|| {
            v.get("raw_code")
                .filter(|x| !x.is_null())
                .map(Value::to_string)
        }),
        message: v["message"].as_str().map(str::to_owned),
        line: v["line"].as_u64().and_then(|x| u32::try_from(x).ok()),
        column: v["column"].as_u64().and_then(|x| u32::try_from(x).ok()),
        raw_position: v
            .get("raw_position")
            .filter(|x| !x.is_null())
            .cloned()
            .or_else(|| {
                v.get("raw_context")
                    .filter(|x| !x.is_null())
                    .map(|x| json!({"context":x}))
            }),
    }
}
fn outcome(op: Operation, s: &str) -> Option<(Status, i32)> {
    Some(match (op, s) {
        (Operation::Check, "valid") | (Operation::Run, "completed") => (Status::Success, 0),
        (Operation::Check, "invalid") | (Operation::Run, "compile_error") => {
            (Status::InvalidSource, 7)
        }
        (Operation::Check, "timeout") | (Operation::Run, "timed_out") => (Status::TimedOut, 3),
        (_, "unsupported") => (Status::Unsupported, 8),
        (_, "infrastructure_error") => (Status::InfrastructureError, 4),
        (Operation::Run, "runtime_error") => (Status::RuntimeError, 11),
        (Operation::Run, "invalid_entry") => (Status::InvalidEntry, 9),
        (Operation::Run, "invalid_arguments") => (Status::InvalidArguments, 10),
        (Operation::Run, "guest_execution_fault") => (Status::GuestFault, 1),
        (Operation::Run, "cancelled") => (Status::Cancelled, 130),
        _ => return None,
    })
}
pub fn execute(mut source: Source, op: Operation, opts: &Options, cancel: &Cancellation) -> Report {
    let start = Instant::now();
    let mut r = Report::new(Engine::Firmware, op);
    source.identity.engine_source_sha256 = source::hash(source.firmware_text.as_bytes());
    r.source = Some(source.identity.clone());
    r.requested_entry = opts.entry.clone();
    r.arguments = serde_json::from_str::<Vec<Box<RawValue>>>(&opts.args)
        .map(|v| json!({"format":"json","raw":opts.args,"count":v.len(),"resolved":null}))
        .unwrap_or(Value::Null);
    if let Err((s, e)) = perform(&source, op, opts, cancel, &mut r) {
        r.fail(s, "firmware", e)
    }
    r.timings["total_s"] = json!(start.elapsed().as_secs_f64());
    r
}
fn perform(
    source: &Source,
    op: Operation,
    o: &Options,
    cancel: &Cancellation,
    r: &mut Report,
) -> Result<(), (Status, String)> {
    let io = |e: std::io::Error| (Status::IoError, e.to_string());
    if cancel.cancelled() {
        return Err((Status::Cancelled, "cancelled before starting helper".into()));
    }
    if o.timeout.is_zero()
        || o.boot_timeout.is_zero()
        || o.compile_timeout.is_zero()
        || o.execution_timeout.is_zero()
        || o.max_instructions == 0
    {
        return Err((Status::InvalidArguments, "limits must be positive".into()));
    }
    if op == Operation::Run {
        let args: Vec<Box<RawValue>> =
            serde_json::from_str(&o.args).map_err(|e| (Status::InvalidArguments, e.to_string()))?;
        for a in args {
            let a = a.get();
            let numeric =
                a.starts_with('-') || a.as_bytes().first().is_some_and(u8::is_ascii_digit);
            if !numeric {
                let tagged: Result<std::collections::BTreeMap<String, Box<RawValue>>, _> =
                    serde_json::from_str(a);
                if let Ok(tagged) = tagged {
                    let kind = tagged
                        .get("type")
                        .and_then(|v| serde_json::from_str::<String>(v.get()).ok());
                    if kind.as_deref() == Some("real") {
                        if tagged.len() != 2
                            || tagged
                                .get("value")
                                .and_then(|v| serde_json::from_str::<String>(v.get()).ok())
                                .is_none()
                        {
                            return Err((
                                Status::InvalidArguments,
                                "real arguments require only type and a decimal value string"
                                    .into(),
                            ));
                        }
                        continue;
                    }
                }
                return Err((Status::Unsupported,"firmware arguments support real numbers only; strings, lists, matrices, booleans and symbolic values are unavailable".into()));
            }
        }
    }
    let fw = o.firmware.as_ref().ok_or((
        Status::MissingFirmware,
        "original firmware is not configured; use --firmware or PPL_FIRMWARE".into(),
    ))?;
    let fw = absolute(fw).map_err(|e| (Status::MissingFirmware, e))?;
    if !fw.is_file() {
        return Err((
            Status::MissingFirmware,
            "firmware path is not a file".into(),
        ));
    }
    let factory = o.factory_a.as_ref().ok_or((
        Status::MissingResources,
        "factory A: is not configured; use --factory-a or PPL_FACTORY_A".into(),
    ))?;
    let factory = absolute(factory).map_err(|e| (Status::MissingResources, e))?;
    if !factory.join("programs/misc/armfir.dat").is_file() {
        return Err((
            Status::MissingResources,
            "factory A: lacks programs/misc/armfir.dat".into(),
        ));
    }
    let rom = o
        .os_rom
        .as_deref()
        .map(absolute)
        .transpose()
        .map_err(|e| (Status::MissingResources, e))?;
    if rom.as_ref().is_some_and(|p| !p.is_file()) {
        return Err((Status::MissingResources, "OS ROM path is not a file".into()));
    }
    let init = o
        .init_data
        .as_deref()
        .map(absolute)
        .transpose()
        .map_err(|e| (Status::MissingResources, e))?;
    if let Some(init) = &init {
        let data = init.join("C/DATA");
        if !data.is_dir() {
            return Err((
                Status::MissingResources,
                "init-data lacks initialized C/DATA".into(),
            ));
        }
        let entries =
            std::fs::read_dir(&data).map_err(|e| (Status::MissingResources, e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| (Status::MissingResources, e.to_string()))?;
            if entry
                .path()
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("hpprgm"))
            {
                return Err((
                    Status::MissingResources,
                    "init-data must be program-free; it is never cleaned or modified".into(),
                ));
            }
        }
    }
    let runner = discover_runner(o.runner.as_deref()).map_err(|e| (Status::MissingRunner, e))?;
    let started = Instant::now();
    let mut version = Command::new(&runner);
    version.arg("--version");
    let version = supervise(version, o.timeout.min(Duration::from_secs(2)), cancel)
        .map_err(|e| (Status::InfrastructureError, e))?;
    if let Some(s) = version.stop {
        return Err((s, "runner version handshake interrupted".into()));
    }
    if version.code != Some(0) || !version.stdout.starts_with(b"prime-emulator ") {
        return Err((
            Status::Unsupported,
            "incompatible helper; install the matching prime-emulator with --version support"
                .into(),
        ));
    }
    if let Some(dir) = &o.artifacts_dir {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let work = if let Some(dir) = &o.artifacts_dir {
        tempfile::Builder::new()
            .prefix("ppl-firmware-")
            .tempdir_in(dir)
    } else {
        tempfile::Builder::new().prefix("ppl-firmware-").tempdir()
    }
    .map_err(io)?;
    let root = if o.artifacts_dir.is_some() {
        r.work_retained = true;
        work.keep()
    } else {
        work.path().to_owned()
    };
    let root = root.canonicalize().map_err(io)?;
    let nonce = root.file_name().unwrap().to_string_lossy().into_owned();
    r.request_id = Some(nonce.clone());
    let name = source
        .guest_name(o.program_name.as_deref())
        .map_err(|e| (Status::InvalidArguments, e))?;
    r.program = name.clone();
    let input = root.join(if source.container {
        "input.hpprgm"
    } else {
        "input.ppl"
    });
    std::fs::write(
        &input,
        if source.container {
            source.bytes.as_slice()
        } else {
            source.text.as_bytes()
        },
    )
    .map_err(io)?;
    let report = root.join("backend.json");
    let guest = root.join("guest");
    let mut cmd = Command::new(runner);
    cmd.current_dir(&root)
        .arg(if op == Operation::Check {
            "check-program"
        } else {
            "run-program"
        })
        .arg("--program")
        .arg(&input)
        .arg("--name")
        .arg(&name)
        .arg("--request-id")
        .arg(&nonce)
        .arg("--firmware")
        .arg(fw)
        .arg("--factory-a")
        .arg(factory)
        .arg("--out-dir")
        .arg(&guest)
        .arg("--report")
        .arg(&report)
        .arg("--boot-timeout-s")
        .arg(o.boot_timeout.as_secs_f64().to_string())
        .arg("--key-timeout-s")
        .arg(o.compile_timeout.as_secs_f64().to_string())
        .arg("--max-seconds")
        .arg(o.timeout.as_secs_f64().to_string())
        .arg("--max-instructions")
        .arg(o.max_instructions.to_string());
    if let Some(rom) = rom {
        cmd.arg("--os-rom").arg(rom);
    }
    if let Some(init) = init {
        cmd.arg("--init-data").arg(init);
    }
    if !source.container {
        cmd.arg("--as-source");
    }
    let screenshot = o.screenshot.as_ref().map(|p| {
        if p.is_absolute() {
            p.clone()
        } else {
            std::env::current_dir().unwrap().join(p)
        }
    });
    if op == Operation::Run {
        if let Some(entry) = &o.entry {
            cmd.arg("--entry").arg(entry);
        } else {
            cmd.arg("--entry-stem").arg(&source.stem);
        }
        cmd.arg("--args")
            .arg(&o.args)
            .arg("--execution-timeout-s")
            .arg(o.execution_timeout.as_secs_f64().to_string());
        if let Some(keys) = &o.runtime_keys {
            cmd.arg("--runtime-keys").arg(keys);
        }
        if let Some(p) = &screenshot {
            cmd.arg("--screenshot").arg(p);
        }
    }
    let child_started = Instant::now();
    let captured = supervise(cmd, o.timeout.saturating_sub(started.elapsed()), cancel)
        .map_err(|e| (Status::InfrastructureError, e))?;
    let child_wall = child_started.elapsed().as_secs_f64();
    std::fs::write(root.join("stderr.log"), &captured.stderr).map_err(io)?;
    std::fs::write(root.join("stdout.log"), &captured.stdout).map_err(io)?;
    if r.work_retained {
        r.artifacts
            .extend([root.join("stderr.log"), root.join("stdout.log")]);
    }
    if let Some(s) = captured.stop {
        if r.work_retained && report.is_file() {
            r.artifacts.push(report.clone());
        }
        return Err((
            s,
            "firmware request interrupted; process group terminated and reaped".into(),
        ));
    }
    if !report.is_file() {
        return Err((
            Status::InfrastructureError,
            format!(
                "helper returned {:?} without a report: {}",
                captured.code,
                String::from_utf8_lossy(&captured.stderr)
            ),
        ));
    }
    if std::fs::metadata(&report).map_err(io)?.len() > 64 * 1024 * 1024 {
        return Err((
            Status::InfrastructureError,
            "helper report exceeds 64 MiB".into(),
        ));
    }
    let bytes = std::fs::read(&report).map_err(io)?;
    let raw: Value = serde_json::from_slice(&bytes).map_err(|e| {
        (
            Status::InfrastructureError,
            format!("malformed helper JSON: {e}"),
        )
    })?;
    let id: Identity = serde_json::from_slice(&bytes).map_err(|e| {
        (
            Status::InfrastructureError,
            format!("incomplete helper identity: {e}"),
        )
    })?;
    if id.schema_version != 1 {
        return Err((
            Status::Unsupported,
            "unsupported helper report schema".into(),
        ));
    }
    if id.engine != "firmware" || id.request_id != nonce || id.program != name {
        return Err((
            Status::InfrastructureError,
            "helper report identity mismatch".into(),
        ));
    }
    let wire_status = raw[if op == Operation::Check {
        "status"
    } else {
        "execution_status"
    }]
    .as_str()
    .ok_or((Status::InfrastructureError, "missing backend status".into()))?;
    let (status, expected_exit) = outcome(op, wire_status).ok_or((
        Status::Unsupported,
        format!("unknown backend outcome {wire_status}"),
    ))?;
    if captured.code != Some(expected_exit) {
        return Err((
            Status::InfrastructureError,
            "helper exit status disagrees with report".into(),
        ));
    }
    let expected_hash = &source.identity.engine_source_sha256;
    if id
        .source_sha256
        .as_ref()
        .is_some_and(|s| s != expected_hash)
        || matches!(
            status,
            Status::Success | Status::InvalidSource | Status::RuntimeError
        ) && id.source_sha256.as_ref() != Some(expected_hash)
    {
        return Err((
            Status::InfrastructureError,
            "checked source hash mismatch or missing source identity".into(),
        ));
    }
    let compilation = if op == Operation::Check {
        &raw
    } else {
        &raw["compilation"]
    };
    if matches!(
        status,
        Status::Success | Status::InvalidSource | Status::RuntimeError
    ) && compilation["source_preserved"] != true
    {
        return Err((
            Status::InfrastructureError,
            "helper did not verify preserved source".into(),
        ));
    }
    if matches!(
        status,
        Status::Success | Status::InvalidSource | Status::RuntimeError
    ) {
        let observed = &compilation["observation"];
        let sources = observed["sources"].as_array();
        if observed["compiler_entered"] != true
            || observed["returned_ms"].is_null()
            || !sources.is_some_and(|a| {
                a.iter()
                    .any(|v| v["sha256"].as_str() == Some(expected_hash))
            })
        {
            return Err((
                Status::InfrastructureError,
                "missing genuine compiler observation in helper report".into(),
            ));
        }
    }
    r.compilation_status = if op == Operation::Run {
        raw["compile_status"].as_str().map(str::to_owned)
    } else if !compilation["observation"].is_null() {
        compilation["status"].as_str().map(str::to_owned)
    } else {
        None
    };
    r.execution_status = (op == Operation::Run).then(|| wire_status.to_owned());
    r.diagnostics = compilation["diagnostics"]
        .as_array()
        .map(|a| a.iter().map(diagnostic).collect())
        .unwrap_or_default();
    if op == Operation::Run {
        let invocations = raw["runtime"]["invocations"].as_array().ok_or((
            Status::InfrastructureError,
            "missing invocation records".into(),
        ))?;
        if invocations.len() > 1 {
            return Err((
                Status::InfrastructureError,
                "unexpected multiple helper invocations".into(),
            ));
        }
        if matches!(status, Status::InvalidSource | Status::InvalidEntry)
            && (!invocations.is_empty() || !raw["return_value"].is_null())
        {
            return Err((
                Status::InfrastructureError,
                "non-executing outcome contains stale runtime data".into(),
            ));
        }
        if let Some(c) = invocations.first() {
            if c["source_sha256"].as_str() != Some(expected_hash) {
                return Err((
                    Status::InfrastructureError,
                    "runtime source hash mismatch".into(),
                ));
            }
            r.entry = c["entry"].as_str().map(str::to_owned);
            r.arguments["resolved"] = c["calls"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|f| f["arguments"].as_array())
                .map(|a| json!(a.iter().map(|v| v["value"].clone()).collect::<Vec<_>>()))
                .unwrap_or(Value::Null);
            if o.entry
                .as_ref()
                .is_some_and(|e| Some(e) != r.entry.as_ref())
            {
                return Err((
                    Status::InfrastructureError,
                    "runtime selected a different entry".into(),
                ));
            }
            if status == Status::Success
                && (c["body_entered"] != true
                    || c["ended_ms"].is_null()
                    || c["result"]["value"].is_null()
                    || raw["runtime"]["compile_verified"] != true
                    || raw["compile_status"] != "valid")
            {
                return Err((
                    Status::InfrastructureError,
                    "incomplete runtime completion evidence".into(),
                ));
            }
            r.output = c["output"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| OutputEvent {
                            kind: e["kind"].as_str().unwrap_or("unknown").into(),
                            text: e["text"].as_str().map(str::to_owned),
                            value: e.get("value").filter(|x| !x.is_null()).cloned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if status == Status::InvalidArguments {
                if let Some(d) = c.get("diagnostic").filter(|x| !x.is_null()) {
                    r.diagnostics.push(diagnostic(d));
                }
            }
        } else if matches!(status, Status::Success | Status::RuntimeError) {
            return Err((
                Status::InfrastructureError,
                "runtime outcome has no invocation evidence".into(),
            ));
        }
        r.return_value = raw.get("return_value").filter(|v| !v.is_null()).cloned();
        r.runtime_errors = raw["runtime_errors"]
            .as_array()
            .map(|a| a.iter().map(diagnostic).collect())
            .unwrap_or_default();
    }
    r.timings = json!({"helper_wall_s":child_wall,"backend":compilation["timings"],"capture_truncated":captured.truncated});
    r.status = status;
    r.exit_code = status.firmware_exit();
    if status != Status::Success {
        r.fail(
            status,
            if status == Status::InvalidSource {
                "compile"
            } else {
                "firmware"
            },
            raw["evidence"]
                .as_str()
                .unwrap_or("firmware operation failed"),
        );
    }
    if r.work_retained {
        r.artifacts.push(report);
    }
    if let Some(p) = screenshot {
        let expected = raw["runtime"]["invocations"]
            .as_array()
            .and_then(|a| a.last())
            .and_then(|c| c["screenshot_sha256"].as_str());
        if let Some(expected) = expected {
            let image = std::fs::read(&p).map_err(io)?;
            if source::hash(&image) != expected {
                return Err((
                    Status::InfrastructureError,
                    "screenshot does not match this invocation's guest capture".into(),
                ));
            }
            r.artifacts.push(p);
        }
    }
    r.backend = json!({"exit_code":captured.code,"report":raw,"ephemeral_paths":!r.work_retained,"stderr":String::from_utf8_lossy(&captured.stderr)});
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // A sibling fork can inherit a temporary script's writable FD until exec,
    // causing Linux ETXTBSY even after its creator closes the file. Keep these
    // executable-fixture lifetimes separate; production execution is unchanged.
    static FIXTURES: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn fixture(mode: &str) -> (tempfile::TempDir, Source, Options) {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::text("EXPORT BROKEN() BEGIN PRINT(\"x\"; END;", "inline").unwrap();
        let root = dir.path();
        std::fs::write(root.join("fw"), []).unwrap();
        std::fs::create_dir_all(root.join("A/programs/misc")).unwrap();
        std::fs::write(root.join("A/programs/misc/armfir.dat"), []).unwrap();
        let helper = root.join("helper with spaces");
        let script = format!(
            r##"#!/bin/sh
if [ "$1" = "--version" ]; then echo 'prime-emulator 0.2.0'; exit 0; fi
while [ $# -gt 0 ]; do
case "$1" in
--request-id) id="$2"; shift;;
--name) name="$2"; shift;;
--report) report="$2"; shift;;
esac
shift
done
case '{mode}' in
hang) trap '' INT TERM; echo "$$" > "$(dirname "$0")/ready"; sleep 30 & wait;;
malformed) echo 'not JSON' > "$report"; exit 0;;
missing) exit 0;;
esac
case '{mode}' in
compile|staleruntime)
printf '{{"schema_version":1,"engine":"firmware","request_id":"%s","program":"%s","source_sha256":"{hash}","execution_status":"compile_error","compile_status":"invalid","compilation":{{"status":"invalid","source_preserved":true,"observation":{{"compiler_entered":true,"returned_ms":1,"sources":[{{"sha256":"{hash}"}}]}}}},"runtime":{{"invocations":[]}},"return_value":null}}' "$id" "$name" > "$report"
exit 7;;
stalenonce) id=old;;
esac
printf '{{"schema_version":1,"engine":"firmware","request_id":"%s","program":"%s","source_sha256":"{hash}","status":"valid","source_preserved":true,"diagnostics":[],"observation":{{"compiler_entered":true,"returned_ms":1,"sources":[{{"sha256":"{hash}"}}]}},"timings":{{}}}}' "$id" "$name" > "$report"
case '{mode}' in
wrongexit) exit 7;;
noisy) head -c 2097152 /dev/zero >&2;;
esac
exit 0
"##,
            hash = if mode == "wronghash" {
                "bad"
            } else {
                &source.identity.source_sha256
            }
        );
        std::fs::write(&helper, script).unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let o = Options {
            runner: Some(helper),
            firmware: Some(root.join("fw")),
            factory_a: Some(root.join("A")),
            ..Default::default()
        };
        (dir, source, o)
    }
    #[test]
    fn transport_is_independent_of_native_semantics() {
        let _fixture_lock = FIXTURES.lock().unwrap();
        let (_d, s, o) = fixture("valid");
        let r = execute(s, Operation::Check, &o, &Cancellation::default());
        assert_eq!(r.status, Status::Success, "{:?}", r.error);
        assert_eq!(r.engine, Some(Engine::Firmware));
        assert!(!r.work_retained);
    }
    #[test]
    fn rejects_missing_malformed_stale_and_disagreeing_reports() {
        let _fixture_lock = FIXTURES.lock().unwrap();
        for mode in [
            "missing",
            "malformed",
            "wronghash",
            "wrongexit",
            "stalenonce",
        ] {
            let (_d, s, o) = fixture(mode);
            let r = execute(s, Operation::Check, &o, &Cancellation::default());
            assert_eq!(
                r.status,
                Status::InfrastructureError,
                "{mode}: {:?}",
                r.error
            );
            assert_ne!(r.exit_code, 1);
        }
    }
    #[test]
    fn timeout_and_cancellation_reap_process_groups() {
        let _fixture_lock = FIXTURES.lock().unwrap();
        for cancelled in [false, true] {
            let (_d, s, mut o) = fixture("hang");
            o.timeout = Duration::from_millis(150);
            let c = Cancellation::default();
            if cancelled {
                c.cancel();
            }
            let start = Instant::now();
            let r = execute(s, Operation::Check, &o, &c);
            assert_eq!(
                r.status,
                if cancelled {
                    Status::Cancelled
                } else {
                    Status::TimedOut
                }
            );
            assert!(start.elapsed() < Duration::from_secs(4));
        }
    }
    #[test]
    fn drains_large_stderr_and_cleans_only_owned_work() {
        let _fixture_lock = FIXTURES.lock().unwrap();
        let (d, s, o) = fixture("noisy");
        std::fs::write(d.path().join("unrelated"), b"keep").unwrap();
        let r = execute(s, Operation::Check, &o, &Cancellation::default());
        assert_eq!(r.status, Status::Success);
        assert_eq!(r.timings["capture_truncated"], true);
        assert!(d.path().join("unrelated").is_file());
    }
    #[test]
    fn cancellation_during_process_and_missing_explicit_runner() {
        let _fixture_lock = FIXTURES.lock().unwrap();
        let (dir, source, options) = fixture("hang");
        let control = Cancellation::default();
        let signal = control.clone();
        let worker =
            std::thread::spawn(move || execute(source, Operation::Check, &options, &control));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.path().join("ready").is_file() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let pid: i32 = std::fs::read_to_string(dir.path().join("ready"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let child_list = format!("/proc/{pid}/task/{pid}/children");
        let descendants = loop {
            let pids = std::fs::read_to_string(&child_list).unwrap_or_default();
            if !pids.trim().is_empty() {
                break pids;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        let stopped = Instant::now();
        signal.cancel();
        let report = worker.join().unwrap();
        assert_eq!(report.status, Status::Cancelled);
        assert!(stopped.elapsed() < Duration::from_secs(4));
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        for descendant in descendants.split_whitespace() {
            let state = std::fs::read_to_string(format!("/proc/{descendant}/stat"));
            assert!(
                state.is_err() || state.unwrap().split_whitespace().nth(2) == Some("Z"),
                "running orphan descendant {descendant}"
            );
        }
        let (_dir, source, mut options) = fixture("valid");
        options.runner = Some(PathBuf::from("/missing/explicit/runner"));
        assert_eq!(
            execute(source, Operation::Check, &options, &Cancellation::default()).status,
            Status::MissingRunner
        );
    }
    #[test]
    fn existing_screenshot_is_not_attributed_to_rejected_source() {
        let _fixture_lock = FIXTURES.lock().unwrap();
        let (dir, source, mut options) = fixture("compile");
        let png = dir.path().join("old.png");
        std::fs::write(&png, b"previous invocation").unwrap();
        options.screenshot = Some(png.clone());
        let report = execute(source, Operation::Run, &options, &Cancellation::default());
        assert_eq!(report.status, Status::InvalidSource, "{:?}", report.error);
        assert!(!report.artifacts.contains(&png));
        assert_eq!(std::fs::read(png).unwrap(), b"previous invocation");
        assert!(report.return_value.is_none());
    }
}
