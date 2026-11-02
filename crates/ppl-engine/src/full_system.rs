//! Isolated discovery and self-authored hardware research execution.
//! This never boots proprietary firmware or opens USB.
//! Configured resources are not proof of executed guest or calculator features.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub const QEMU_REVISION: &str = "32e891a9fc568de3110343c3e35b9ec0d23f6f44";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    HardwareDiagnostics,
    GuestGdb,
    PplRun,
    PplCheck,
    CalculatorUi,
    Cas,
    Graphing,
    NativeSourceDebug,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendDescriptor {
    pub schema_version: u32,
    pub backend: String,
    pub availability: String,
    pub capabilities: Vec<Capability>,
    pub unsupported: Vec<Capability>,
    pub resource_identity: Option<String>,
    pub reference_revision: Option<String>,
    pub execution_preflight_pending: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildIdentity {
    pub binary_sha256: String,
    pub image_id: String,
    pub image: String,
    pub reference_revision: String,
}
/// Bounded regular-file hashing. Refuses FIFO/device/symlink inputs before open.
pub fn regular_digest(path: &Path, limit: u64) -> Result<String, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() > limit {
        return Err(format!("{} is not a bounded regular file", path.display()));
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > limit {
            return Err("file exceeded hash budget".into());
        }
        digest.update(&buffer[..n]);
    }
    if total != metadata.len() {
        return Err("file changed while hashing".into());
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub fn validate_build(root: &Path) -> Result<BuildIdentity, String> {
    let manifest = root.join("build.json");
    regular_digest(&manifest, 65536)?;
    let bytes = std::fs::read(&manifest).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("build manifest exceeds limit".into());
    }
    let data: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if data["schema_version"] != 1
        || data["status"] != "built"
        || data["reference_revision"] != QEMU_REVISION
    {
        return Err("incompatible or incomplete pinned QEMU build".into());
    }
    let text = |field: &str| {
        data[field]
            .as_str()
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("missing {field}"))
    };
    let identity = BuildIdentity {
        binary_sha256: text("binary_sha256")?,
        image_id: text("image_id")?,
        image: text("image")?,
        reference_revision: text("reference_revision")?,
    };
    let actual = regular_digest(
        &root.join("build/arm-softmmu/qemu-system-arm"),
        512 * 1024 * 1024,
    )?;
    if actual != identity.binary_sha256 {
        return Err("QEMU binary differs from build manifest".into());
    }
    Ok(identity)
}
pub fn inspect(root: Option<&Path>) -> BackendDescriptor {
    let mut result = BackendDescriptor {
        schema_version: 1,
        backend: "full_system".into(),
        availability: "not_configured".into(),
        capabilities: vec![Capability::HardwareDiagnostics, Capability::GuestGdb],
        unsupported: vec![
            Capability::PplRun,
            Capability::PplCheck,
            Capability::CalculatorUi,
            Capability::Cas,
            Capability::Graphing,
            Capability::NativeSourceDebug,
        ],
        resource_identity: None,
        reference_revision: None,
        execution_preflight_pending: true,
        reason: Some(
            "Select an isolated pinned research build; FullSystem PPL is unavailable".into(),
        ),
    };
    if let Some(root) = root {
        match validate_build(root) {
            Ok(build) => {
                result.availability = "configured".into();
                result.resource_identity = Some(build.binary_sha256);
                result.reference_revision = Some(build.reference_revision);
                result.reason = Some("Binary identity validated; runner checks runtime image and executes only synthetic hardware cases".into());
            }
            Err(error) => {
                result.availability = "invalid_resources".into();
                result.reason = Some(error);
            }
        }
    }
    result
}
/// Validate semantic claims before displaying a research runner's result.
/// A successful hardware test cannot claim calculator kernel or physical parity.
pub fn validate_result(
    data: &serde_json::Value,
    build: &BuildIdentity,
    case: &str,
) -> Result<(), String> {
    if !["smoke", "timer_irq", "timer_fiq", "uart_gpio", "mmu"].contains(&case) {
        return Err("unknown research result case".into());
    }
    if data["schema_version"] != 1
        || data["machine"] != "hp-prime"
        || data["cpu"] != "arm926"
        || data["reference_revision"] != build.reference_revision
        || data["binary_sha256"] != build.binary_sha256
        || data["image_id"] != build.image_id
        || data["kernel_boot_verified"] != false
        || data
            .get("physical_hardware_verified")
            .is_some_and(|v| v != false)
    {
        return Err(
            "research result has incompatible identity or unsupported verification claims".into(),
        );
    }
    let scope = if case == "smoke" {
        "s3c2416_machine_model_smoke"
    } else {
        "full_system_synthetic_hardware"
    };
    if data["scope"] != scope
        || (case != "smoke"
            && (data["case"] != case || data["physical_hardware_verified"] != false))
    {
        return Err("research result belongs to another case or proof category".into());
    }
    if data["status"] == "passed" {
        let proof = if case == "smoke" {
            &data["smoke"]["executed"]
        } else {
            &data["guest_cpu_execution_verified"]
        };
        if proof != true || !data["observations"].is_object() {
            return Err("successful result lacks executed guest observations".into());
        }
        validate_passed_observations(data, case)?;
    } else if data["status"] != "failed" {
        return Err("unknown research result status".into());
    }
    Ok(())
}

// Mirrors the version-1 self-authored guest contract, rather than accepting a
// claimed execution boolean or an arbitrary observations object as evidence.
fn validate_passed_observations(data: &serde_json::Value, case: &str) -> Result<(), String> {
    let obs = &data["observations"];
    let guest = if case == "smoke" {
        &data["smoke"]
    } else {
        &data["guest"]
    };
    let digest = |value: &serde_json::Value| {
        value.as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
    };
    let word = |value: &serde_json::Value| value.as_u64().filter(|&n| n <= u64::from(u32::MAX));
    let halt = word(&obs["halt_address"]).filter(|&pc| (0x30000000..0x30001000).contains(&pc));
    if guest["executed"] != true
        || guest["linked"] != true
        || guest["entry"] != "0x30000000"
        || !digest(&guest["sha256"])
        || (case != "smoke" && !digest(&guest["source_sha256"]))
        || obs["initial_pc"] != 0x30000000u32
        || obs["initial_mailbox"] != 0
        || obs["stepped_pc"] != 0x30000004u32
        || halt.is_none()
        || word(&obs["stopped_pc"]) != halt
        || word(&obs["cpsr"]).is_none()
        || (0..13).any(|i| word(&obs["registers"][format!("r{i}")]).is_none())
    {
        return Err("successful research result has incomplete guest identity or entry/step/halt observations".into());
    }
    if case == "smoke" {
        if obs["mailbox"] != 0x600d0001u32
            || obs["aliases"]
                != serde_json::json!([0x12345678u32, 0x12345678u32, 0x12345678u32, 0x12345678u32])
        {
            return Err(
                "successful smoke result lacks the terminal mailbox and all RAM aliases".into(),
            );
        }
        return Ok(());
    }
    let words = obs["mailbox"]
        .as_array()
        .ok_or("hardware mailbox is not an array")?
        .iter()
        .map(|v| {
            word(v)
                .map(|x| x as u32)
                .ok_or("hardware mailbox contains a non-word")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if words.len() != 64 || words[0] != 0x600d0018 || words[4] != 0xfeed0018 {
        return Err(
            "hardware mailbox lacks complete guest success and resumed-mainline markers".into(),
        );
    }
    let uart = obs["uart_hex"]
        .as_str()
        .ok_or("missing exact guest UART observation")?;
    if matches!(case, "timer_irq" | "timer_fiq") {
        let stack = if case == "timer_irq" {
            0x3000f000
        } else {
            0x3000e000
        };
        if words[11] & 0x1f != 0x13
            || !(0x30000000..0x30001000).contains(&words[12])
            || !(stack - 256..stack).contains(&words[13])
            || words[12] % 4 != 0
            || words[13] % 4 != 0
        {
            return Err("hardware interrupt return context or banked stack differs".into());
        }
    }
    let passed = match case {
        "timer_irq" => {
            words[1..4] == [9, 0, 0]
                && words[5..7] == [5, 6]
                && words[17..25] == [10; 8]
                && words[25] == 11
                && words[9] & 0x1f == 0x12
                && uart == "49494949494949494945"
        }
        "timer_fiq" => words[1..4] == [0, 3, 0] && words[10] & 0x1f == 0x11 && uart == "46464645",
        "uart_gpio" => words[7..9] == [0x5555, 0xa55a] && uart == "4d313820554152540a",
        "mmu" => {
            words[7] == 0x12345678
                && words[3] == 3
                && words[8] & 0x1007 == 3
                && words[11] == words[12]
                && words[14] == 1
                && words[15] & 15 == 5
                && [
                    (words[34] & 15, words[35]),
                    (words[36] & 15, words[37]),
                    (words[38] & 15, words[39]),
                ] == [(5, 0x20000000), (1, 0x30002001), (13, 0x10002000)]
                && uart.is_empty()
        }
        _ => false,
    };
    if !passed {
        return Err(
            "hardware observations disagree with the requested synthetic case contract".into(),
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
mod process {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    use std::os::unix::{fs::DirBuilderExt, process::CommandExt};
    use std::{
        io,
        path::{Path, PathBuf},
        process::{Child, Command, ExitStatus, Stdio},
        time::{Duration, Instant},
    };

    /// Atomic ownership: losing callers never launch or inspect another run's CID.
    pub(super) fn reserve(output: &Path) -> Result<PathBuf, String> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(output)
            .map_err(|e| format!("research output must be newly owned: {e}"))?;
        Ok(output.join("run"))
    }
    pub(super) struct ManagedChild {
        child: Child,
        reaped: bool,
    }
    impl ManagedChild {
        pub(super) fn spawn(command: &mut Command) -> Result<Self, String> {
            let child = command
                .stdin(Stdio::null())
                .process_group(0)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(Self {
                child,
                reaped: false,
            })
        }
        pub(super) fn signal(&self, signal: Signal) {
            let _ = killpg(Pid::from_raw(self.child.id() as i32), signal);
        }
        fn poll_using(
            &mut self,
            poll: &mut impl FnMut(&mut Child) -> io::Result<Option<ExitStatus>>,
        ) -> Result<Option<ExitStatus>, String> {
            let status = poll(&mut self.child).map_err(|e| e.to_string())?;
            self.reaped |= status.is_some();
            Ok(status)
        }
        pub(super) fn poll(&mut self) -> Result<Option<ExitStatus>, String> {
            self.poll_using(&mut Child::try_wait)
        }
        pub(super) fn stop(&mut self) {
            if !self.reaped {
                self.signal(Signal::SIGKILL);
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.reaped = true;
            }
        }
        fn run_cleanup(&mut self) -> Result<(), String> {
            let at = Instant::now();
            while self.poll()?.is_none() {
                if at.elapsed() >= Duration::from_secs(10) {
                    return Err("owned container cleanup exceeded deadline".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(())
        }
    }
    impl Drop for ManagedChild {
        fn drop(&mut self) {
            self.stop();
        }
    }

    pub(super) struct ResearchRun {
        pub(super) launcher: ManagedChild,
        output: PathBuf,
        cleanup: PathBuf,
    }
    impl ResearchRun {
        pub(super) fn spawn(command: &mut Command, output: PathBuf) -> Result<Self, String> {
            Ok(Self {
                launcher: ManagedChild::spawn(command)?,
                output,
                cleanup: "podman".into(),
            })
        }
    }
    impl Drop for ResearchRun {
        fn drop(&mut self) {
            self.launcher.stop();
            // Only a bounded regular file inside this request's atomically owned
            // private output root can authorize container removal.
            let cid = self.output.join("container.cid");
            let id = (|| -> Option<String> {
                let meta = std::fs::symlink_metadata(&cid).ok()?;
                if !meta.is_file() || meta.len() > 128 {
                    return None;
                }
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(cid)
                    .ok()?
                    .take(129)
                    .read_to_string(&mut text)
                    .ok()?;
                let id = text.trim();
                (id.len() == 64
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
                .then(|| id.to_owned())
            })();
            if let Some(id) = id {
                let cleanup = (|| -> Result<(), String> {
                    let mut command = Command::new(&self.cleanup);
                    command
                        .args(["rm", "--force", &id])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    ManagedChild::spawn(&mut command)?.run_cleanup()
                })();
                if let Err(error) = cleanup {
                    eprintln!("research cleanup warning: {error}");
                }
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::{Arc, Barrier};
        fn sleeper() -> Command {
            let mut c = Command::new("sh");
            c.args(["-c", "exec sleep 60"])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            c
        }
        fn cleanup_program(root: &Path) -> PathBuf {
            let path = root.join("cleanup.sh");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nprintf '%s\n' \"$*\" >> '{}'\n",
                    root.join("removed.log").display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            path
        }
        #[test]
        fn poll_error_unwinds_kills_and_reaps_the_owned_child() {
            let mut child = ManagedChild::spawn(&mut sleeper()).unwrap();
            let pid = child.child.id();
            assert!(
                child
                    .poll_using(&mut |_| Err(io::Error::other("injected wait failure")))
                    .is_err()
            );
            drop(child);
            assert_eq!(
                nix::sys::wait::waitpid(Pid::from_raw(pid as i32), None),
                Err(nix::errno::Errno::ECHILD)
            );
        }
        #[test]
        fn same_output_race_loser_cannot_remove_the_winners_container() {
            let root = tempfile::tempdir().unwrap();
            let output = root.path().join("out");
            let barrier = Arc::new(Barrier::new(2));
            let threads = (0..2)
                .map(|_| {
                    let output = output.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        reserve(&output)
                    })
                })
                .collect::<Vec<_>>();
            let choices = threads
                .into_iter()
                .map(|t| t.join().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(choices.iter().filter(|r| r.is_ok()).count(), 1);
            assert_eq!(
                std::fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o700
            );
            let run = choices.into_iter().find_map(Result::ok).unwrap();
            std::fs::create_dir(&run).unwrap();
            std::fs::write(run.join("container.cid"), "a".repeat(64)).unwrap();
            let mut owned = ResearchRun::spawn(&mut sleeper(), run).unwrap();
            owned.cleanup = cleanup_program(root.path());
            assert!(reserve(&output).is_err());
            assert!(owned.launcher.poll().unwrap().is_none());
            assert!(!root.path().join("removed.log").exists());
            drop(owned);
            assert_eq!(
                std::fs::read_to_string(root.path().join("removed.log"))
                    .unwrap()
                    .trim(),
                format!("rm --force {}", "a".repeat(64))
            );
        }
        #[test]
        fn early_poll_error_preserves_failure_and_still_cleans_the_owned_container() {
            let root = tempfile::tempdir().unwrap();
            let output = root.path().join("out");
            let path = reserve(&output).unwrap();
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("container.cid"), "c".repeat(64)).unwrap();
            let error = (|| -> Result<(), String> {
                let mut owned = ResearchRun::spawn(&mut sleeper(), path).unwrap();
                owned.cleanup = cleanup_program(root.path());
                owned
                    .launcher
                    .poll_using(&mut |_| Err(io::Error::other("original injected failure")))?;
                Ok(())
            })()
            .unwrap_err();
            assert_eq!(error, "original injected failure");
            assert_eq!(
                std::fs::read_to_string(root.path().join("removed.log"))
                    .unwrap()
                    .trim(),
                format!("rm --force {}", "c".repeat(64))
            );
        }
        #[test]
        fn cid_symlinks_and_oversized_files_cannot_authorize_cleanup() {
            let root = tempfile::tempdir().unwrap();
            for symlink in [false, true] {
                let run = root.path().join(if symlink { "symlink" } else { "large" });
                std::fs::create_dir(&run).unwrap();
                let cid = run.join("container.cid");
                if symlink {
                    let other = root.path().join("other");
                    std::fs::write(&other, "b".repeat(64)).unwrap();
                    std::os::unix::fs::symlink(other, cid).unwrap();
                } else {
                    std::fs::write(cid, "b".repeat(1024)).unwrap();
                }
                let mut owned = ResearchRun::spawn(&mut sleeper(), run).unwrap();
                owned.cleanup = cleanup_program(root.path());
                drop(owned);
            }
            assert!(!root.path().join("removed.log").exists());
        }
    }
}

/// Reserve a fresh private output root, then run the isolated Python adapter.
/// All raw artifacts and result.json live in `output/run`; a competing caller
/// cannot inspect or remove this request's resources. Every post-spawn return
/// drops owned process/container guards, including result validation failures.
#[cfg(target_os = "linux")]
pub fn execute(
    build_root: &Path,
    case: &str,
    output: &Path,
    timeout: u64,
    cancel: &crate::Cancellation,
) -> Result<serde_json::Value, String> {
    use nix::sys::signal::Signal;
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    if !(1..=120).contains(&timeout)
        || !["smoke", "timer_irq", "timer_fiq", "uart_gpio", "mmu"].contains(&case)
    {
        return Err("unsupported research case or deadline (1..120 seconds)".into());
    }
    if case == "smoke" && timeout != 30 {
        return Err("legacy smoke uses a fixed 30-second deadline".into());
    }
    let build = validate_build(build_root)?;
    if output.exists() {
        return Err("research output directory must be new".into());
    }
    if cancel.cancelled() {
        return Err("research execution cancelled before launch".into());
    }
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/emulator");
    let runner = scripts.join(if case == "smoke" {
        "full_system_run.py"
    } else {
        "full_system_hardware.py"
    });
    if !runner.is_file() {
        return Err(
            "research runner unavailable; use the source checkout's isolated tooling".into(),
        );
    }
    let run_output = process::reserve(output)?;
    let mut command = Command::new("python3");
    command
        .arg(runner)
        .arg("--build-root")
        .arg(build_root)
        .arg("--output")
        .arg(&run_output);
    if case != "smoke" {
        command
            .arg("--case")
            .arg(case)
            .arg("--timeout")
            .arg(timeout.to_string());
    }
    // Runner retains bounded logs under output; its stdout JSON is read from result.json.
    command.stdout(Stdio::null()).stderr(Stdio::inherit());
    let mut run = process::ResearchRun::spawn(&mut command, run_output.clone())?;
    let started = Instant::now();
    let mut interrupted = None;
    let exit = loop {
        if let Some(exit) = run.launcher.poll()? {
            break exit;
        }
        if interrupted.is_none()
            && (cancel.cancelled() || started.elapsed() > Duration::from_secs(timeout + 20))
        {
            run.launcher.signal(Signal::SIGINT);
            interrupted = Some(Instant::now());
        }
        if interrupted.is_some_and(|t| t.elapsed() > Duration::from_secs(15)) {
            return Err(
                "research execution cancellation grace expired; owned process cleanup required"
                    .into(),
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if interrupted.is_some() {
        return Err(
            "research execution cancelled or exceeded its deadline; inspect retained artifacts"
                .into(),
        );
    }
    let result = run_output.join("result.json");
    regular_digest(&result, 1024 * 1024)?;
    let data = serde_json::from_reader(File::open(&result).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    validate_result(&data, &build, case)?;
    if exit.success() != (data["status"] == "passed") {
        return Err("runner exit status disagrees with retained result".into());
    }
    Ok(data)
}
#[cfg(not(target_os = "linux"))]
pub fn execute(
    _: &Path,
    _: &str,
    _: &Path,
    _: u64,
    _: &crate::Cancellation,
) -> Result<serde_json::Value, String> {
    Err("isolated FullSystem research currently requires Linux; Native remains available".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn build(root: &Path) {
        std::fs::create_dir_all(root.join("build/arm-softmmu")).unwrap();
        let binary = root.join("build/arm-softmmu/qemu-system-arm");
        std::fs::write(&binary, b"mock executable").unwrap();
        std::fs::write(root.join("build.json"), serde_json::to_vec(&serde_json::json!({"schema_version":1,"status":"built","reference_revision":QEMU_REVISION,"binary_sha256":regular_digest(&binary,1024).unwrap(),"image_id":"mock-image","image":"mock"})).unwrap()).unwrap();
    }
    fn identity(report: &serde_json::Value) -> BuildIdentity {
        BuildIdentity {
            binary_sha256: report["binary_sha256"].as_str().unwrap().into(),
            image_id: report["image_id"].as_str().unwrap().into(),
            image: "local".into(),
            reference_revision: QEMU_REVISION.into(),
        }
    }
    fn hardware_report(case: &str) -> serde_json::Value {
        let mut words = vec![0u32; 64];
        words[0] = 0x600d0018;
        words[4] = 0xfeed0018;
        let uart = match case {
            "timer_irq" => {
                words[1] = 9;
                words[5] = 5;
                words[6] = 6;
                words[9] = 0x12;
                words[11] = 0x13;
                words[12] = 0x30000080;
                words[13] = 0x3000efc0;
                words[17..25].fill(10);
                words[25] = 11;
                "49494949494949494945"
            }
            "timer_fiq" => {
                words[2] = 3;
                words[10] = 0x11;
                words[11] = 0x13;
                words[12] = 0x30000080;
                words[13] = 0x3000dfc0;
                "46464645"
            }
            "uart_gpio" => {
                words[7] = 0x5555;
                words[8] = 0xa55a;
                "4d313820554152540a"
            }
            "mmu" => {
                words[7] = 0x12345678;
                words[3] = 3;
                words[8] = 3;
                words[11] = 0x123456;
                words[12] = 0x123456;
                words[14] = 1;
                words[15] = 5;
                words[34] = 5;
                words[35] = 0x20000000;
                words[36] = 1;
                words[37] = 0x30002001;
                words[38] = 13;
                words[39] = 0x10002000;
                ""
            }
            _ => unreachable!(),
        };
        let regs = (0..13)
            .map(|i| (format!("r{i}"), serde_json::json!(0)))
            .collect::<serde_json::Map<_, _>>();
        serde_json::json!({"schema_version":1,"machine":"hp-prime","cpu":"arm926","reference_revision":QEMU_REVISION,"binary_sha256":"binary","image_id":"image","kernel_boot_verified":false,"physical_hardware_verified":false,"scope":"full_system_synthetic_hardware","case":case,"status":"passed","guest_cpu_execution_verified":true,"guest":{"executed":true,"linked":true,"entry":"0x30000000","sha256":"a".repeat(64),"source_sha256":"b".repeat(64)},"observations":{"initial_pc":0x30000000u32,"initial_mailbox":0,"stepped_pc":0x30000004u32,"stopped_pc":0x30000040u32,"halt_address":0x30000040u32,"registers":regs,"cpsr":0x13,"mailbox":words,"uart_hex":uart}})
    }
    #[test]
    fn configured_is_not_execution_or_ppl() {
        let root = tempfile::tempdir().unwrap();
        build(root.path());
        let descriptor = inspect(Some(root.path()));
        assert_eq!(descriptor.availability, "configured");
        assert!(descriptor.execution_preflight_pending);
        assert!(!descriptor.capabilities.contains(&Capability::PplRun));
        assert!(descriptor.unsupported.contains(&Capability::PplRun));
    }
    #[test]
    fn missing_tampered_and_incompatible_builds_rejected() {
        assert_eq!(inspect(None).availability, "not_configured");
        let root = tempfile::tempdir().unwrap();
        assert!(validate_build(root.path()).is_err());
        build(root.path());
        std::fs::write(
            root.path().join("build/arm-softmmu/qemu-system-arm"),
            b"changed",
        )
        .unwrap();
        assert!(validate_build(root.path()).unwrap_err().contains("differs"));
        std::fs::write(root.path().join("build.json"), "{}").unwrap();
        assert!(
            validate_build(root.path())
                .unwrap_err()
                .contains("incompatible")
        );
    }
    #[test]
    fn bounded_regular_files_only() {
        let root = tempfile::tempdir().unwrap();
        assert!(regular_digest(root.path(), 1024).is_err());
        let p = root.path().join("large");
        std::fs::write(&p, [0; 8]).unwrap();
        assert!(regular_digest(&p, 7).is_err());
        #[cfg(unix)]
        {
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            assert!(regular_digest(&link, 100).is_err());
        }
    }
    #[test]
    fn execution_evidence_cannot_forge_provenance_or_skip_observations() {
        let build = BuildIdentity {
            binary_sha256: "binary".into(),
            image_id: "image".into(),
            image: "local".into(),
            reference_revision: QEMU_REVISION.into(),
        };
        let original = hardware_report("timer_irq");
        assert!(validate_result(&original, &build, "timer_irq").is_ok());
        for (field, value) in [
            ("kernel_boot_verified", serde_json::json!(true)),
            ("physical_hardware_verified", serde_json::json!(true)),
            ("binary_sha256", serde_json::json!("other")),
            ("guest_cpu_execution_verified", serde_json::json!(false)),
            ("observations", serde_json::Value::Null),
            ("case", serde_json::json!("mmu")),
        ] {
            let mut wrong = original.clone();
            wrong[field] = value;
            assert!(
                validate_result(&wrong, &build, "timer_irq").is_err(),
                "{field}"
            );
        }
    }
    #[test]
    fn passed_reports_require_complete_guest_observations_and_exact_case_semantics() {
        for case in ["timer_irq", "timer_fiq", "uart_gpio", "mmu"] {
            let original = hardware_report(case);
            let build = identity(&original);
            assert!(validate_result(&original, &build, case).is_ok(), "{case}");
            for (pointer, value) in [
                ("/observations", serde_json::json!({})),
                ("/guest/executed", serde_json::json!(false)),
                ("/guest/sha256", serde_json::json!("missing")),
                ("/guest/source_sha256", serde_json::json!("other")),
                ("/observations/initial_pc", serde_json::json!(4)),
                ("/observations/stepped_pc", serde_json::json!(0x30000000u32)),
                ("/observations/stopped_pc", serde_json::json!(0)),
                ("/observations/halt_address", serde_json::json!(0)),
                ("/observations/cpsr", serde_json::json!(0x1_0000_0000u64)),
                ("/observations/registers/r12", serde_json::Value::Null),
                ("/observations/mailbox/0", serde_json::json!(0)),
                ("/observations/mailbox/4", serde_json::json!(0)),
                ("/observations/mailbox/60", serde_json::json!(true)),
                ("/observations/uart_hex", serde_json::json!("unexpected")),
            ] {
                let mut bad = original.clone();
                *bad.pointer_mut(pointer).unwrap() = value;
                assert!(
                    validate_result(&bad, &build, case).is_err(),
                    "accepted {case}:{pointer}"
                );
            }
            let critical = match case {
                "timer_irq" => vec![1, 5, 6, 9, 11, 12, 13, 17, 25],
                "timer_fiq" => vec![2, 10, 11, 12, 13],
                "uart_gpio" => vec![7, 8],
                "mmu" => vec![3, 7, 8, 11, 12, 14, 15, 34, 35, 36, 37, 38, 39],
                _ => unreachable!(),
            };
            for index in critical {
                let mut bad = original.clone();
                let v = bad["observations"]["mailbox"][index].as_u64().unwrap();
                bad["observations"]["mailbox"][index] = serde_json::json!(v ^ 1);
                assert!(
                    validate_result(&bad, &build, case).is_err(),
                    "accepted {case} word{index}"
                );
            }
        }
    }
    #[test]
    fn recorded_executed_smoke_requires_all_concrete_execution_observations() {
        let original: serde_json::Value =
            serde_json::from_str(include_str!("../../../evidence/m18/coordinator-smoke.json"))
                .unwrap();
        let build = identity(&original);
        assert!(validate_result(&original, &build, "smoke").is_ok());
        for (pointer, value) in [
            ("/observations", serde_json::json!({})),
            ("/smoke/executed", serde_json::json!(false)),
            ("/observations/initial_pc", serde_json::json!(0)),
            ("/observations/initial_mailbox", serde_json::json!(1)),
            ("/observations/stepped_pc", serde_json::json!(0)),
            ("/observations/stopped_pc", serde_json::json!(0)),
            ("/observations/mailbox", serde_json::json!(0)),
            ("/observations/aliases/3", serde_json::json!(0)),
        ] {
            let mut bad = original.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(
                validate_result(&bad, &build, "smoke").is_err(),
                "accepted {pointer}"
            );
        }
        assert!(validate_result(&hardware_report("timer_irq"), &build, "unknown").is_err());
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn invalid_and_cancelled_launches_leave_no_output() {
        let root = tempfile::tempdir().unwrap();
        build(root.path());
        let output = root.path().join("output");
        let cancel = crate::Cancellation::default();
        assert!(execute(root.path(), "ppl", &output, 30, &cancel).is_err());
        cancel.cancel();
        assert!(execute(root.path(), "timer_irq", &output, 30, &cancel).is_err());
        assert!(!output.exists());
    }
}
