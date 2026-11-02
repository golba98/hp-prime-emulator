//! Headless live firmware transport. Each caller owns an isolated calculator profile.
use crate::{
    Cancellation,
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
        let request = match command {
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
        };
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
        let mut sleeping = false;
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
