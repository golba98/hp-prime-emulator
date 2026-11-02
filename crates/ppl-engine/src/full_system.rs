//! Isolated research backend discovery. This never boots firmware or opens USB.
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
    } else if data["status"] != "failed" {
        return Err("unknown research result status".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn execute(
    build_root: &Path,
    case: &str,
    output: &Path,
    timeout: u64,
    cancel: &crate::Cancellation,
) -> Result<serde_json::Value, String> {
    use nix::{
        sys::signal::{Signal, killpg},
        unistd::Pid,
    };
    use std::{
        os::unix::process::CommandExt,
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
    let mut command = Command::new("python3");
    command
        .arg(runner)
        .arg("--build-root")
        .arg(build_root)
        .arg("--output")
        .arg(output);
    if case != "smoke" {
        command
            .arg("--case")
            .arg(case)
            .arg("--timeout")
            .arg(timeout.to_string());
    }
    // Runner retains bounded logs under output; its stdout JSON is read from result.json.
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut interrupted = None;
    let exit = loop {
        if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
            break exit;
        }
        if interrupted.is_none()
            && (cancel.cancelled() || started.elapsed() > Duration::from_secs(timeout + 20))
        {
            let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGINT);
            interrupted = Some(Instant::now());
        }
        if interrupted.is_some_and(|t| t.elapsed() > Duration::from_secs(15)) {
            let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
            break child.wait().map_err(|e| e.to_string())?;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A force-killed launcher may not have reached Python's finally block.
    // Only remove the exact ID recorded inside this run's newly owned directory.
    if interrupted.is_some() || !exit.success() {
        if let Ok(id) = std::fs::read_to_string(output.join("container.cid")) {
            let id = id.trim();
            if id.len() == 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                let mut cleanup = Command::new("podman")
                    .args(["rm", "--force", id])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let deadline = Instant::now();
                while cleanup.try_wait().map_err(|e| e.to_string())?.is_none() {
                    if deadline.elapsed() > Duration::from_secs(10) {
                        let _ = cleanup.kill();
                        let _ = cleanup.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
    if interrupted.is_some() {
        return Err(
            "research execution cancelled or exceeded its deadline; inspect retained artifacts"
                .into(),
        );
    }
    let result = output.join("result.json");
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
        let original = serde_json::json!({"schema_version":1,"machine":"hp-prime","cpu":"arm926","reference_revision":QEMU_REVISION,"binary_sha256":"binary","image_id":"image","kernel_boot_verified":false,"physical_hardware_verified":false,"scope":"full_system_synthetic_hardware","case":"timer_irq","status":"passed","guest_cpu_execution_verified":true,"observations":{"irq_count":5}});
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
