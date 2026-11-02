//! Headless live firmware transport. Each caller owns an isolated calculator profile.
use crate::{
    Cancellation,
    checkpoint::{CheckpointResult, RECEIPT_VERSION},
    firmware_options::Options,
    session::{self, CheckVerdict, Command, Event, ExecutionObservation, Lifecycle, Packet},
};
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    process::{Child, ChildStdin, Command as Process, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

type RunIdentity = (String, String, String);

pub struct Response {
    pub success: bool,
    pub message: String,
    pub check: Option<CheckVerdict>,
    pub execution: Option<ExecutionObservation>,
}
pub struct LiveSession {
    child: Child,
    stdin: ChildStdin,
    events: Option<mpsc::Receiver<Result<Event, String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    stderr: Option<std::thread::JoinHandle<String>>,
    sent: BTreeMap<u64, Option<RunIdentity>>,
    checkpoint: PendingCheckpoint,
    lifecycle: Lifecycle,
    executions: BTreeMap<u64, ExecutionObservation>,
    results: BTreeMap<u64, Response>,
    frame: Arc<Mutex<Option<Vec<u32>>>>,
}
impl LiveSession {
    /// `profile` must be a caller-owned disposable image, never a user calculator image.
    pub fn spawn(options: &Options, profile: &Path, cancel: &Cancellation) -> Result<Self, String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (options, profile, cancel);
            return Err("Firmware live sessions currently require Linux".into());
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            let runner = crate::firmware::discover_runner(options.runner.as_deref())?;
            let firmware = options
                .firmware
                .as_ref()
                .ok_or("Firmware ELF is not configured")?;
            let checkpoint_firmware =
                checkpoint_fingerprint(firmware, options.os_rom.as_deref(), profile)?;
            let mut command = Process::new(runner);
            command
                .arg("session")
                .arg("--firmware")
                .arg(firmware)
                .arg("--data-dir")
                .arg(profile)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0);
            if let Some(rom) = &options.os_rom {
                command.arg("--os-rom").arg(rom);
            }
            let mut child = command.spawn().map_err(|e| e.to_string())?;
            let stdin = child.stdin.take().ok_or("No session stdin")?;
            let mut stdout = child.stdout.take().ok_or("No session stdout")?;
            let mut err = child.stderr.take().ok_or("No session stderr")?;
            let stderr = std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let mut buf = [0; 8192];
                while let Ok(n) = err.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let keep = n.min((64 * 1024usize).saturating_sub(bytes.len()));
                    bytes.extend_from_slice(&buf[..keep]);
                }
                String::from_utf8_lossy(&bytes).into_owned()
            });
            let (tx, rx) = mpsc::sync_channel(64);
            let frame = Arc::new(Mutex::new(None));
            let capture = frame.clone();
            let reader = std::thread::spawn(move || {
                loop {
                    let event = match session::read_packet(&mut stdout) {
                        Ok(Packet::Frame { pixels, .. }) => {
                            *capture.lock().unwrap() = Some(pixels);
                            continue;
                        }
                        Ok(Packet::Event(event)) => Ok(event),
                        Err(e) => Err(e.to_string()),
                    };
                    let done = event.is_err();
                    if tx.send(event).is_err() || done {
                        break;
                    }
                }
            });
            let mut s = Self {
                child,
                stdin,
                events: Some(rx),
                reader: Some(reader),
                stderr: Some(stderr),
                sent: BTreeMap::new(),
                checkpoint: PendingCheckpoint {
                    firmware: checkpoint_firmware,
                    ..Default::default()
                },
                lifecycle: Lifecycle::Booting,
                executions: BTreeMap::new(),
                results: BTreeMap::new(),
                frame,
            };
            let deadline = Instant::now() + options.boot_timeout;
            let mut hello = false;
            loop {
                match s.event(deadline, cancel)? {
                    Event::Hello { version } if version == session::VERSION => hello = true,
                    Event::Hello { version } => {
                        return Err(format!(
                            "Session protocol {version} differs from {}",
                            session::VERSION
                        ));
                    }
                    Event::Capabilities {
                        managed_programs: false,
                        reason,
                    } => {
                        return Err(
                            reason.unwrap_or("Managed firmware observations unavailable".into())
                        );
                    }
                    Event::Lifecycle {
                        state: Lifecycle::Ready,
                    } if hello => return Ok(s),
                    Event::Closed { message } | Event::Error { message } => return Err(message),
                    _ => {}
                }
            }
        }
    }
    pub fn send(&mut self, command: &Command) -> Result<(), String> {
        let request = standard_request_id(command)?;
        if self.checkpoint.id.is_some() {
            return Err("A typed checkpoint request is pending".into());
        }
        if let Some(id) = request {
            if self.sent.len() + self.results.len() >= 8
                || self.sent.contains_key(&id)
                || self.results.contains_key(&id)
            {
                return Err("Duplicate request or pending live request bound exceeded".into());
            }
        }
        session::write_command(&mut self.stdin, command).map_err(|e| e.to_string())?;
        if let Some(id) = request {
            let expected = if let Command::RunStored {
                name,
                source_sha256,
                entry,
                ..
            } = command
            {
                Some((name.clone(), source_sha256.clone(), entry.clone()))
            } else {
                None
            };
            self.sent.insert(id, expected);
        }
        Ok(())
    }
    fn validate_event(&mut self, event: &Event) -> Result<(), String> {
        match event {
            Event::Checkpoint { id, result } => self.checkpoint.accept(*id, result)?,
            Event::Lifecycle { state } => self.lifecycle = *state,
            Event::Execution {
                request,
                program,
                source_sha256,
                observation,
            } => {
                verify_execution(
                    self.sent.get(request).and_then(|e| e.as_ref()),
                    program,
                    source_sha256,
                    observation,
                )?;
                if self.executions.contains_key(request) || self.results.contains_key(request) {
                    return Err("Duplicate execution result".into());
                }
            }
            Event::ExecutionStarted {
                request,
                source_sha256,
                entry,
            } => {
                let expected = self
                    .sent
                    .get(request)
                    .ok_or("Unsolicited execution-start request")?;
                if let Some((_, hash, name)) = expected
                    && (source_sha256 != hash || entry != name)
                {
                    return Err("Execution-start identity mismatch".into());
                }
            }
            Event::Result {
                request, success, ..
            } => {
                let expected = finish_request(&mut self.sent, *request)?;
                if *success && expected.is_some() && !self.executions.contains_key(request) {
                    return Err("Successful RunStored lacks structured execution evidence".into());
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn event(&mut self, deadline: Instant, cancel: &Cancellation) -> Result<Event, String> {
        loop {
            if cancel.cancelled() {
                return Err("Live firmware operation cancelled".into());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Live firmware operation timed out".into());
            }
            match self
                .events
                .as_ref()
                .ok_or("Session closed")?
                .recv_timeout(remaining.min(Duration::from_millis(50)))
            {
                Ok(e) => {
                    let event = e?;
                    self.validate_event(&event)?;
                    return Ok(event);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    /// Saves firmware-owned persistent files; never serializes a live CPU.
    /// Wake=true additionally waits for the helper's Ready lifecycle before returning.
    pub fn checkpoint(
        &mut self,
        id: u64,
        wake: bool,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Result<CheckpointResult, String> {
        if cancel.cancelled() || timeout.is_zero() {
            return Err("Checkpoint cancelled or deadline is empty; no command sent".into());
        }
        self.checkpoint.begin(
            id,
            !self.sent.is_empty() || !self.results.is_empty() || !self.executions.is_empty(),
        )?;
        let outcome = (|| {
            session::write_command(&mut self.stdin, &Command::Checkpoint { id, wake })
                .map_err(|e| e.to_string())?;
            let deadline = Instant::now() + timeout;
            let mut saved = None;
            loop {
                match self.event(deadline, cancel)? {
                    Event::Checkpoint { result, .. } => {
                        if wake
                            && matches!(result, CheckpointResult::Saved { .. })
                            && self.lifecycle != Lifecycle::Ready
                        {
                            saved = Some(result);
                        } else {
                            return Ok(result);
                        }
                    }
                    Event::Lifecycle {
                        state: Lifecycle::Ready,
                    } if saved.is_some() => return Ok(saved.take().unwrap()),
                    Event::Closed { message } | Event::Error { message } => return Err(message),
                    _ => {}
                }
            }
        })();
        self.checkpoint.id = None;
        outcome
    }
    pub fn request(
        &mut self,
        id: u64,
        command: &Command,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Result<Response, String> {
        self.send(command)?;
        self.await_result(id, timeout, cancel)
    }
    pub fn await_result(
        &mut self,
        id: u64,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Result<Response, String> {
        if let Some(result) = self.results.remove(&id) {
            return Ok(result);
        }
        let deadline = Instant::now() + timeout;
        loop {
            match self.event(deadline, cancel)? {
                Event::Execution {
                    request,
                    observation,
                    ..
                } => {
                    if self.executions.insert(request, *observation).is_some() {
                        return Err("Duplicate execution result".into());
                    }
                }
                Event::Result {
                    request,
                    success,
                    message,
                    check,
                } => {
                    let result = Response {
                        success,
                        message,
                        check,
                        execution: self.executions.remove(&request),
                    };
                    if request == id {
                        return Ok(result);
                    }
                    self.results.insert(request, result);
                }
                Event::Closed { message } | Event::Error { message } => return Err(message),
                _ => {}
            }
        }
    }
    pub fn await_started(
        &mut self,
        id: u64,
        source_hash: &str,
        entry: &str,
        timeout: Duration,
        cancel: &Cancellation,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.event(deadline, cancel)? {
                Event::ExecutionStarted {
                    request,
                    source_sha256,
                    entry: actual,
                } if request == id => {
                    if source_sha256 != source_hash || actual != entry {
                        return Err("Started execution identity mismatch".into());
                    }
                    return Ok(());
                }
                Event::Execution {
                    request,
                    observation,
                    ..
                } => {
                    let entered = request == id && observation.body_entered;
                    if observation.runtime["invocations"][0]["source_sha256"] != source_hash
                        || observation.entry != entry
                    {
                        return Err("Execution identity mismatch".into());
                    }
                    self.executions.insert(request, *observation);
                    if entered {
                        return Ok(());
                    }
                }
                Event::Result {
                    request,
                    success,
                    message,
                    check,
                } if request == id => {
                    let execution = self.executions.remove(&id);
                    let entered = execution.as_ref().is_some_and(|o| o.body_entered);
                    self.results.insert(
                        id,
                        Response {
                            success,
                            message: message.clone(),
                            check,
                            execution,
                        },
                    );
                    if entered {
                        return Ok(());
                    }
                    return Err(format!("Program did not enter its body: {message}"));
                }
                Event::Closed { message } | Event::Error { message } => return Err(message),
                _ => {}
            }
        }
    }
    pub fn frame(&self) -> Option<Vec<u32>> {
        self.frame.lock().unwrap().clone()
    }
    pub fn power_down(&mut self, timeout: Duration, cancel: &Cancellation) -> Result<(), String> {
        self.send(&Command::PowerDown)?;
        let deadline = Instant::now() + timeout;
        let mut sleeping = self.lifecycle == Lifecycle::Sleeping;
        loop {
            match self.event(deadline, cancel)? {
                Event::Lifecycle {
                    state: Lifecycle::Sleeping,
                } => sleeping = true,
                Event::Closed { .. } if sleeping => return Ok(()),
                Event::Closed { message } | Event::Error { message } => return Err(message),
                _ => {}
            }
        }
    }
}
/// Must match helper bootstrap: the receipt binds the ELF, optional ROM and
/// profile resource payload, not a filename or ELF-only version label.
fn checkpoint_fingerprint(
    firmware: &Path,
    rom: Option<&Path>,
    profile: &Path,
) -> Result<String, String> {
    let mut identity = std::fs::read(firmware).map_err(|e| e.to_string())?;
    if let Some(rom) = rom {
        identity.extend(std::fs::read(rom).map_err(|e| e.to_string())?);
    }
    let resource = profile.join("A/programs/misc/armfir.dat");
    if resource.exists() {
        identity.extend(
            crate::source::hash(&std::fs::read(resource).map_err(|e| e.to_string())?).as_bytes(),
        );
    } else {
        identity.extend(b"no-armfir.dat");
    }
    Ok(crate::source::hash(&identity))
}
fn standard_request_id(command: &Command) -> Result<Option<u64>, String> {
    Ok(match command {
        Command::Checkpoint { .. } => {
            return Err("Use LiveSession::checkpoint for typed checkpoint results".into());
        }
        Command::LoadSource { request, .. }
        | Command::Compile { request, .. }
        | Command::DirectCheck { request, .. }
        | Command::Run { request, .. }
        | Command::Import { request, .. }
        | Command::StoreProgram { request, .. }
        | Command::RunStored { request, .. }
        | Command::ScenarioInput { request, .. }
        | Command::ReleaseInput { request } => Some(*request),
        _ => None,
    })
}
#[derive(Default)]
struct PendingCheckpoint {
    id: Option<u64>,
    firmware: String,
}
impl PendingCheckpoint {
    fn begin(&mut self, id: u64, other_pending: bool) -> Result<(), String> {
        if self.id.is_some() || other_pending {
            return Err("Await pending calculator requests before checkpointing".into());
        }
        self.id = Some(id);
        Ok(())
    }
    fn accept(&mut self, id: u64, result: &CheckpointResult) -> Result<(), String> {
        if self.id != Some(id) {
            return Err("Unsolicited or duplicate checkpoint response id".into());
        }
        if let CheckpointResult::Saved { receipt } = result {
            let hash = |text: &str| text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit());
            if receipt.version != RECEIPT_VERSION
                || receipt.restores_cpu
                || receipt.restores_framebuffer
                || receipt.generation.is_empty()
                || receipt.generation.len() > 256
                || !hash(&receipt.firmware)
                || receipt.firmware != self.firmware
                || !hash(&receipt.manifest_sha256)
                || receipt.bytes > 256 * 1024 * 1024
                || receipt.files > 100_000
            {
                return Err("Checkpoint receipt is incompatible, malformed or claims unsupported live state".into());
            }
        }
        self.id = None;
        Ok(())
    }
}
fn finish_request(
    sent: &mut BTreeMap<u64, Option<RunIdentity>>,
    request: u64,
) -> Result<Option<RunIdentity>, String> {
    sent.remove(&request)
        .ok_or_else(|| "Unsolicited or duplicate terminal request".into())
}
fn verify_execution(
    expected: Option<&RunIdentity>,
    program: &str,
    hash: &str,
    observation: &ExecutionObservation,
) -> Result<(), String> {
    let (wanted_program, wanted_hash, wanted_entry) =
        expected.ok_or("Unsolicited execution request")?;
    if program != wanted_program
        || hash != wanted_hash
        || observation.entry != *wanted_entry
        || observation.runtime["invocations"][0]["source_sha256"] != *wanted_hash
    {
        return Err(
            "Execution program/source/entry envelope differs from the exact request".into(),
        );
    }
    Ok(())
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.events.take();
        #[cfg(target_os = "linux")]
        crate::firmware::kill_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}
/// Copies initialized resources, rejecting links and leaving the original image untouched.
pub fn copy_profile(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "Profile contains symbolic link {}",
                entry.path().display()
            ));
        }
        if kind.is_dir() {
            copy_profile(&entry.path(), &to.join(entry.file_name()))?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn receipt_identity_binds_actual_firmware_rom_and_resource_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let fw = dir.path().join("elf");
        let rom = dir.path().join("rom");
        let resource = dir.path().join("A/programs/misc/armfir.dat");
        std::fs::create_dir_all(resource.parent().unwrap()).unwrap();
        std::fs::write(&fw, b"firmware").unwrap();
        std::fs::write(&rom, b"rom").unwrap();
        std::fs::write(&resource, b"resources").unwrap();
        let original = checkpoint_fingerprint(&fw, Some(&rom), dir.path()).unwrap();
        let mut bytes = b"firmwarerom".to_vec();
        bytes.extend(crate::source::hash(b"resources").as_bytes());
        assert_eq!(original, crate::source::hash(&bytes));
        std::fs::write(&fw, b"different").unwrap();
        assert_ne!(
            original,
            checkpoint_fingerprint(&fw, Some(&rom), dir.path()).unwrap()
        );
        std::fs::write(&fw, b"firmware").unwrap();
        std::fs::write(&resource, b"different").unwrap();
        assert_ne!(
            original,
            checkpoint_fingerprint(&fw, Some(&rom), dir.path()).unwrap()
        );
    }
    fn checkpoint_receipt() -> CheckpointResult {
        use crate::checkpoint::{CheckpointKind, CheckpointReceipt, CheckpointSafePoint};
        CheckpointResult::Saved {
            receipt: CheckpointReceipt {
                version: RECEIPT_VERSION,
                kind: CheckpointKind::FirmwarePersistence,
                safe_point: CheckpointSafePoint::FirmwareOff,
                generation: "generation-001".into(),
                firmware: "a".repeat(64),
                manifest_sha256: "b".repeat(64),
                bytes: 128,
                files: 1,
                restores_cpu: false,
                restores_framebuffer: false,
            },
        }
    }
    #[test]
    fn checkpoints_use_a_dedicated_bounded_correlated_reply() {
        assert!(
            standard_request_id(&Command::Checkpoint { id: 7, wake: false })
                .unwrap_err()
                .contains("LiveSession::checkpoint")
        );
        let mut pending = PendingCheckpoint {
            firmware: "a".repeat(64),
            ..Default::default()
        };
        assert!(pending.begin(7, true).is_err());
        pending.begin(7, false).unwrap();
        assert!(pending.begin(8, false).is_err());
        assert!(pending.accept(8, &checkpoint_receipt()).is_err());
        assert_eq!(pending.id, Some(7));
        pending.accept(7, &checkpoint_receipt()).unwrap();
        assert!(pending.accept(7, &checkpoint_receipt()).is_err());
        pending.begin(8, false).unwrap();
        pending
            .accept(
                8,
                &CheckpointResult::Deferred {
                    reason: "busy".into(),
                },
            )
            .unwrap();
    }
    #[test]
    fn checkpoint_receipts_cannot_claim_live_cpu_or_accept_corruption() {
        for mutation in 0..7 {
            let mut result = checkpoint_receipt();
            let CheckpointResult::Saved { receipt } = &mut result else {
                unreachable!()
            };
            match mutation {
                0 => receipt.version += 1,
                1 => receipt.restores_cpu = true,
                2 => receipt.restores_framebuffer = true,
                3 => receipt.manifest_sha256 = "not-a-hash".into(),
                4 => receipt.generation = "x".repeat(257),
                5 => receipt.bytes = 256 * 1024 * 1024 + 1,
                _ => receipt.firmware = "c".repeat(64),
            }
            let mut pending = PendingCheckpoint {
                firmware: "a".repeat(64),
                ..Default::default()
            };
            pending.begin(1, false).unwrap();
            assert!(pending.accept(1, &result).is_err());
        }
    }
    fn observed() -> ExecutionObservation {
        ExecutionObservation {
            status: "completed".into(),
            binding: "stored_source_dispatch".into(),
            entry: "ENTRY".into(),
            body_entered: true,
            completed: true,
            return_value: Some(serde_json::json!({"type":"real","value":"1"})),
            return_representation: serde_json::Value::Null,
            output: vec![],
            diagnostics: vec![],
            runtime: serde_json::json!({"invocations":[{"source_sha256":"hash"}]}),
            image: None,
        }
    }
    #[test]
    fn duplicate_and_unsolicited_terminal_ids_are_rejected() {
        let mut sent = BTreeMap::from([(7, None)]);
        assert!(finish_request(&mut sent, 7).is_ok());
        assert!(finish_request(&mut sent, 7).is_err());
        assert!(finish_request(&mut sent, 99).is_err());
    }
    #[test]
    fn wrong_program_source_entry_and_unsolicited_request_are_rejected() {
        let expected = ("ZZT_RIGHT".into(), "hash".into(), "ENTRY".into());
        assert!(verify_execution(Some(&expected), "ZZT_RIGHT", "hash", &observed()).is_ok());
        assert!(verify_execution(Some(&expected), "ZZT_WRONG", "hash", &observed()).is_err());
        assert!(verify_execution(Some(&expected), "ZZT_RIGHT", "wrong", &observed()).is_err());
        assert!(verify_execution(None, "ZZT_RIGHT", "hash", &observed()).is_err());
        let mut wrong = observed();
        wrong.entry = "OTHER".into();
        assert!(verify_execution(Some(&expected), "ZZT_RIGHT", "hash", &wrong).is_err());
    }
}
