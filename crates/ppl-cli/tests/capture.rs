//! Actual CLI offline capture cycle. Never opens a physical calculator.
use ppl_compat::{
    DeviceRecord,
    capture::*,
    differential::{Backend, LoadedScenario, Native},
    sha256_hex,
};
use ppl_engine::Cancellation;
use std::{fs, path::Path, process::Command};

fn cli(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_ppl"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn relative_output_offline_cli_cycle_preserves_mock_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/differential/arithmetic.toml")
        .canonicalize()
        .unwrap();
    cli(
        tmp.path(),
        &[
            "capture",
            "prepare",
            fixture.to_str().unwrap(),
            "--capture-id",
            "CLI_MOCK",
            "--output",
            "plan.json",
        ],
    );
    let plan: CapturePlan = read_json(&tmp.path().join("plan.json")).unwrap();
    let scenario = LoadedScenario::load(&fixture).unwrap();
    let native = Native.execute(&scenario, &Cancellation::default());
    fs::write(
        tmp.path().join("backend.json"),
        serde_json::to_vec(&native).unwrap(),
    )
    .unwrap();
    let mut recording = Recording {
        schema_version: SCHEMA_VERSION,
        capture_id: plan.capture_id.clone(),
        plan_sha256: plan.sha256().unwrap(),
        origin: CaptureOrigin::ControlledMock,
        device: DeviceRecord {
            model: "mock".into(),
            firmware_build: Some(15157),
            ..Default::default()
        },
        captured_at_utc: "2026-11-02T12:00:00Z".into(),
        initialization: Initialization {
            source_sha256: plan.source_sha256.clone(),
            statement: "controlled mock setup".into(),
        },
        identity_artifacts: vec![],
        steps: vec![],
        artifacts: vec![],
    };
    fs::create_dir(tmp.path().join("raw")).unwrap();
    for (step, observation) in plan.steps.iter().zip(native.observations) {
        let action_sha256 = sha256_hex(&serde_json::to_vec(&step.action).unwrap());
        let note = ObservationTranscript {
            schema_version: SCHEMA_VERSION,
            capture_id: plan.capture_id.clone(),
            source_sha256: plan.source_sha256.clone(),
            step: step.id.clone(),
            action_sha256: action_sha256.clone(),
            data: observation.data.clone(),
        };
        let bytes = serde_json::to_vec(&note).unwrap();
        let name = format!("{}.json", step.id);
        recording.artifacts.push(Artifact {
            name: name.clone(),
            kind: ArtifactKind::RawNote,
            size: bytes.len(),
            sha256: sha256_hex(&bytes),
        });
        fs::write(tmp.path().join("raw").join(&name), bytes).unwrap();
        recording.steps.push(CapturedStep {
            step: step.id.clone(),
            action_sha256,
            delivery: Delivery::ManualConfirmed {
                statement: "mock action".into(),
            },
            elapsed_ms: 0,
            data: observation.data,
            artifacts: vec![name],
        });
    }
    fs::write(
        tmp.path().join("recording.json"),
        serde_json::to_vec(&recording).unwrap(),
    )
    .unwrap();
    cli(
        tmp.path(),
        &[
            "capture",
            "seal",
            "--plan",
            "plan.json",
            "--recording",
            "recording.json",
            "--artifacts",
            "raw",
            "--output",
            "bundle",
        ],
    );
    let valid = cli(
        tmp.path(),
        &[
            "capture",
            "validate",
            "bundle",
            "--fixture",
            fixture.to_str().unwrap(),
        ],
    );
    assert_eq!(valid["device_access"], false);
    let report = cli(
        tmp.path(),
        &[
            "capture",
            "compare",
            fixture.to_str().unwrap(),
            "bundle",
            "--backend-run",
            "backend.json",
        ],
    );
    assert_eq!(report["category"], "untested");
    assert_eq!(report["physical_attestation"], false);
    assert_eq!(report["comparison"]["passed"], true);
}
