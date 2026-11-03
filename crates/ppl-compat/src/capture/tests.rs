use super::*;
use crate::differential::{Backend, Native};
use ppl_engine::Cancellation;
use serde_json::json;
use std::path::PathBuf;

fn scenario(name: &str) -> LoadedScenario {
    LoadedScenario::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tests/differential/{name}.toml")),
    )
    .unwrap()
}
fn identity() -> DeviceRecord {
    DeviceRecord {
        model: "controlled mock G1".into(),
        usb_pid: "03f0:1541".into(),
        firmware_build: Some(15157),
        boot_version: Some("mock".into()),
        serial_id: Some(sha256_hex(b"mock identity, never physical evidence")),
    }
}
struct Operator {
    delivered: bool,
}
impl ManualOperator for Operator {
    fn initialize(&mut self, p: &CapturePlan) -> Result<Initialization, String> {
        Ok(Initialization {
            source_sha256: p.source_sha256.clone(),
            statement: "controlled mock setup".into(),
        })
    }
    fn confirm(&mut self, _: &PlannedStep) -> Result<Delivery, String> {
        if self.delivered {
            Ok(Delivery::ManualConfirmed {
                statement: "controlled mock action".into(),
            })
        } else {
            Ok(Delivery::NotDelivered {
                reason: "cancelled".into(),
            })
        }
    }
}
struct Reader {
    plan: CapturePlan,
    observations: Vec<Observation>,
    calls: usize,
    changed: bool,
    observed: usize,
}
impl CaptureReader for Reader {
    fn origin(&self) -> CaptureOrigin {
        CaptureOrigin::ControlledMock
    }
    fn identity(&mut self) -> Result<DeviceRecord, String> {
        self.calls += 1;
        let mut id = identity();
        if self.changed && self.calls > 1 {
            id.serial_id = Some(sha256_hex(b"different mock"));
        }
        Ok(id)
    }
    fn observe(&mut self, step: &PlannedStep) -> Result<ReadObservation, String> {
        self.observed += 1;
        let data = self
            .observations
            .iter()
            .find(|o| o.step == step.id)
            .unwrap()
            .data
            .clone();
        let note = ObservationTranscript {
            schema_version: SCHEMA_VERSION,
            capture_id: self.plan.capture_id.clone(),
            source_sha256: self.plan.source_sha256.clone(),
            step: step.id.clone(),
            action_sha256: json_hash(&step.action)?,
            data: data.clone(),
        };
        let mut artifacts = vec![ArtifactBytes {
            name: format!("{}.json", step.id),
            kind: ArtifactKind::RawNote,
            bytes: serde_json::to_vec(&note).unwrap(),
        }];
        if let Some(image) = data.get("image") {
            let image: differential::Image = serde_json::from_value(image.clone()).unwrap();
            let mut g = ppl_graphics::Grob::new(image.width, image.height, 0);
            for (i, p) in image.pixels.into_iter().enumerate() {
                g.set((i % image.width) as i64, (i / image.width) as i64, p);
            }
            let mut bytes = vec![];
            g.write_png(&mut bytes).unwrap();
            artifacts.push(ArtifactBytes {
                name: format!("{}.png", step.id),
                kind: ArtifactKind::ScreenshotPng,
                bytes,
            });
        }
        Ok(ReadObservation { data, artifacts })
    }
}
fn pending(name: &str) -> (LoadedScenario, CapturePlan, PendingCapture, BackendRun) {
    let s = scenario(name);
    let run = Native.execute(&s, &Cancellation::default());
    assert_eq!(run.status, RunStatus::Complete);
    let plan = CapturePlan::prepare(&s, "MOCK_M16_001").unwrap();
    let mut reader = Reader {
        plan: plan.clone(),
        observations: run.observations.clone(),
        calls: 0,
        changed: false,
        observed: 0,
    };
    let capture = collect(
        &plan,
        "2026-11-02T12:00:00Z",
        &mut Operator { delivered: true },
        &mut reader,
    )
    .unwrap();
    assert_eq!(reader.observed, plan.steps.len());
    (s, plan, capture, run)
}
fn seal_test(
    plan: &CapturePlan,
    capture: &PendingCapture,
    dir: &Path,
) -> Result<SealedCapture, String> {
    seal_bytes(plan, &capture.recording, &capture.artifacts, dir)
}
#[test]
fn mock_bundle_compares_real_native_without_physical_claim_and_never_overwrites() {
    let (s, p, c, r) = pending("arithmetic");
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("capture");
    let sealed = seal_test(&p, &c, &path).unwrap();
    let report = compare_capture(&s, &sealed, r).unwrap();
    assert!(
        report.comparison.passed,
        "{:?}",
        report.comparison.mismatches
    );
    assert_eq!(report.category, VerificationCategory::Untested);
    assert!(!report.physical_attestation);
    assert!(seal_test(&p, &c, &path).is_err());
    assert_eq!(
        SealedCapture::load(&path).unwrap().sha256(),
        sealed.sha256()
    );
}
#[test]
fn source_or_fixture_edits_cannot_reuse_capture() {
    let (s, mut p, _, _) = pending("arithmetic");
    p.source.push(' ');
    assert!(p.validate().is_err());
    p.source_sha256 = sha256_hex(p.source.as_bytes());
    assert!(p.verify_scenario(&s).is_err());
}

#[test]
fn shared_loader_source_normalization_is_preserved() {
    let mut spec = scenario("arithmetic").scenario;
    spec.program.source = Some("EXPORT MAIN()\r\nBEGIN\r\n RETURN 1;\r\nEND;".into());
    let loaded =
        LoadedScenario::from_spec(spec, Path::new("."), sha256_hex(b"CRLF fixture")).unwrap();
    let plan = CapturePlan::prepare(&loaded, "NORMALIZED_MOCK").unwrap();
    assert!(!plan.source.contains('\r'));
    plan.verify_scenario(&loaded).unwrap();
}

#[test]
fn original_single_program_plan_shape_remains_valid() {
    let s = scenario("arithmetic");
    let mut plan = CapturePlan::prepare(&s, "LEGACY_PLAN_MOCK").unwrap();
    plan.sources.clear();
    plan.source_manifest.clear();
    plan.scenario.as_object_mut().unwrap().remove("programs");
    plan.validate().unwrap();
    plan.verify_scenario(&s).unwrap();
}

#[test]
fn all_named_file_sources_are_frozen_and_secondary_changes_refused() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
        tmp.path().join("a.hpprgm"),
        "EXPORT MAIN() BEGIN RETURN 1; END;",
    )
    .unwrap();
    fs::write(
        tmp.path().join("b.hpprgm"),
        "EXPORT MAIN() BEGIN RETURN 2; END;",
    )
    .unwrap();
    let fixture = tmp.path().join("scenario.toml");
    fs::write(
        &fixture,
        r#"schema_version=2
id="frozen-two-files"
[programs.ZZT_A]
file="a.hpprgm"
[programs.ZZT_B]
file="b.hpprgm"
[[steps]]
action="import"
id="import_a"
program="ZZT_A"
[[steps]]
action="import"
id="import_b"
program="ZZT_B"
"#,
    )
    .unwrap();
    let s = LoadedScenario::load(&fixture).unwrap();
    let plan = CapturePlan::prepare(&s, "MULTI_SOURCE_MOCK").unwrap();
    assert_eq!(plan.sources.len(), 2);
    assert_eq!(plan.source_manifest, s.source_manifest());
    assert!(plan.source.is_empty());
    fs::remove_file(tmp.path().join("a.hpprgm")).unwrap();
    fs::remove_file(tmp.path().join("b.hpprgm")).unwrap();
    plan.validate().unwrap(); // No source filesystem lookup at validation time.
    plan.verify_scenario(&s).unwrap();
    let mut changed = plan.clone();
    changed.sources.get_mut("ZZT_B").unwrap().push(' ');
    assert!(changed.validate().is_err());
    let mut missing = plan.clone();
    missing.sources.remove("ZZT_B");
    assert!(missing.validate().is_err());
    let mut manifest = plan;
    manifest.source_manifest.remove("ZZT_B");
    assert!(manifest.validate().is_err());
}

#[test]
fn schema2_mock_captures_stateful_multiple_program_execution() {
    let (s, plan, capture, backend) = pending("m16/stateful");
    assert_eq!(plan.sources.len(), 2);
    let tmp = tempfile::tempdir().unwrap();
    let sealed = seal_test(&plan, &capture, &tmp.path().join("stateful")).unwrap();
    let report = compare_capture(&s, &sealed, backend).unwrap();
    assert!(
        report.comparison.passed,
        "{:?}",
        report.comparison.mismatches
    );
    assert_eq!(report.category, VerificationCategory::Untested);
}

#[test]
fn flipping_mock_origin_cannot_qualify_a_physical_capture() {
    let (_, p, mut c, _) = pending("arithmetic");
    c.recording.origin = CaptureOrigin::ManualPhysical;
    assert!(
        c.recording
            .validate(&p)
            .unwrap_err()
            .contains("retained FA identity")
    );
}

#[test]
fn raw_identity_is_bounded_and_matches_build_version_and_fingerprint() {
    // Synthetic bytes test the parser only; no physical capture is produced.
    let mut raw = vec![0xfa, 1, 0, 0, 0, 0x6c];
    raw.extend([0; 108]);
    let n = raw.len();
    raw[n - 36..n - 34].copy_from_slice(&15157u16.to_le_bytes());
    raw[n - 32..n - 22].copy_from_slice(b"V0.050.640");
    raw[n - 16..n - 6].copy_from_slice(b"MOCKSERIAL");
    let mut device = identity();
    device.boot_version = Some("V0.050.640".into());
    device.serial_id = Some(crate::serial_fingerprint("MOCKSERIAL"));
    verify_raw_identity(&raw, &device).unwrap();
    device.firmware_build = Some(1);
    assert!(verify_raw_identity(&raw, &device).is_err());
    assert!(verify_raw_identity(&raw[..12], &device).is_err());
    assert!(verify_raw_identity(&vec![0; 4097], &device).is_err());
}
#[test]
fn stale_transcript_cannot_be_rehashed_into_another_run() {
    let (_, p, mut c, _) = pending("arithmetic");
    let name = c.recording.steps[0].artifacts[0].clone();
    let mut note: ObservationTranscript = serde_json::from_slice(&c.artifacts[&name]).unwrap();
    note.capture_id = "STALE".into();
    let bytes = serde_json::to_vec(&note).unwrap();
    let a = c
        .recording
        .artifacts
        .iter_mut()
        .find(|a| a.name == name)
        .unwrap();
    a.size = bytes.len();
    a.sha256 = sha256_hex(&bytes);
    c.artifacts.insert(name, bytes);
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        seal_test(&p, &c, &tmp.path().join("bad"))
            .err()
            .unwrap()
            .contains("stale or mismatched")
    );
}
#[test]
fn mismatches_identify_backend_step_and_exact_path() {
    let (s, p, c, mut r) = pending("arithmetic");
    r.observations[1].data["return_value"]["value"] = json!("999");
    let tmp = tempfile::tempdir().unwrap();
    let sealed = seal_test(&p, &c, &tmp.path().join("bundle")).unwrap();
    let report = compare_capture(&s, &sealed, r).unwrap();
    assert!(!report.comparison.passed);
    assert!(
        report
            .comparison
            .mismatches
            .iter()
            .any(|m| m.backend == BackendId::Native && m.path == "/return_value/value")
    );
}
#[test]
fn missing_reordered_and_undelivered_steps_refused() {
    let (_, p, c, _) = pending("arithmetic");
    let mut r = c.recording.clone();
    r.steps.pop();
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.steps[0].delivery = Delivery::NotDelivered {
        reason: "cancelled".into(),
    };
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.steps[0].delivery = Delivery::ReadOnlyObservation;
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.steps[0].action_sha256 = sha256_hex(b"other action");
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.steps.swap(0, 1);
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.steps[1] = r.steps[0].clone();
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.steps.push(r.steps[0].clone());
    assert!(r.validate(&p).is_err());
}
#[test]
fn cancellation_stops_before_observing() {
    let (_, p, c, _) = pending("arithmetic");
    let mut reader = Reader {
        plan: p.clone(),
        observations: c
            .recording
            .steps
            .iter()
            .map(|s| Observation {
                step: s.step.clone(),
                data: s.data.clone(),
            })
            .collect(),
        calls: 0,
        changed: false,
        observed: 0,
    };
    assert!(
        collect(
            &p,
            "2026-11-02T12:00:00Z",
            &mut Operator { delivered: false },
            &mut reader
        )
        .is_err()
    );
    assert_eq!(reader.observed, 0);
}
#[test]
fn changed_device_identity_aborts_after_first_step() {
    let (_, p, c, _) = pending("arithmetic");
    let mut reader = Reader {
        plan: p.clone(),
        observations: c
            .recording
            .steps
            .iter()
            .map(|s| Observation {
                step: s.step.clone(),
                data: s.data.clone(),
            })
            .collect(),
        calls: 0,
        changed: true,
        observed: 0,
    };
    assert!(
        collect(
            &p,
            "2026-11-02T12:00:00Z",
            &mut Operator { delivered: true },
            &mut reader
        )
        .err()
        .unwrap()
        .contains("identity changed")
    );
    assert_eq!(reader.observed, 1);
}
#[test]
fn corrupt_artifact_manifest_and_interrupted_publish_are_invalid() {
    let (_, p, c, _) = pending("arithmetic");
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bundle");
    seal_test(&p, &c, &path).unwrap();
    fs::write(
        path.join("artifacts").join(&c.recording.artifacts[0].name),
        b"corrupt",
    )
    .unwrap();
    assert!(SealedCapture::load(&path).is_err());
    let interrupted = tmp.path().join("incomplete");
    fs::create_dir(&interrupted).unwrap();
    assert!(SealedCapture::load(&interrupted).is_err());
    assert!(seal_test(&p, &c, &interrupted).is_err());
    let path = tmp.path().join("manifest");
    seal_test(&p, &c, &path).unwrap();
    let mut e: Envelope = read_json(&path.join("capture.json")).unwrap();
    e.payload.recording.device.model.push('x');
    fs::write(path.join("capture.json"), serde_json::to_vec(&e).unwrap()).unwrap();
    assert!(SealedCapture::load(&path).is_err());
}
#[test]
fn unsafe_duplicate_or_unreferenced_artifacts_refused() {
    let (_, p, c, _) = pending("arithmetic");
    let mut r = c.recording.clone();
    r.artifacts[0].name = "../escape".into();
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.artifacts.push(r.artifacts[0].clone());
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.artifacts.push(Artifact {
        name: "unused.json".into(),
        kind: ArtifactKind::RawNote,
        size: 1,
        sha256: sha256_hex(b"x"),
    });
    assert!(r.validate(&p).is_err());
}
#[test]
fn setup_identity_firmware_and_valid_utc_are_required() {
    let (_, p, c, _) = pending("arithmetic");
    for t in ["2026-11-02", "2026-02-30T12:00:00Z", "2026-11-02T25:00:00Z"] {
        let mut r = c.recording.clone();
        r.captured_at_utc = t.into();
        assert!(r.validate(&p).is_err());
    }
    let mut r = c.recording.clone();
    r.initialization.statement.clear();
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.device.firmware_build = None;
    assert!(r.validate(&p).is_err());
    r = c.recording.clone();
    r.origin = CaptureOrigin::ManualPhysical;
    r.device.serial_id = None;
    assert!(r.validate(&p).is_err());
}
#[test]
fn png_pixels_independently_validate_transcribed_graphics() {
    let (s, p, mut c, r) = pending("graphics");
    let tmp = tempfile::tempdir().unwrap();
    let sealed = seal_test(&p, &c, &tmp.path().join("ok")).unwrap();
    assert!(compare_capture(&s, &sealed, r).unwrap().comparison.passed);
    let step = c
        .recording
        .steps
        .iter_mut()
        .find(|s| s.data.get("image").is_some())
        .unwrap();
    step.data["image"]["pixels"][32100] = json!(0);
    let name = step
        .artifacts
        .iter()
        .find(|n| n.ends_with(".json"))
        .unwrap()
        .clone();
    let mut note: ObservationTranscript = serde_json::from_slice(&c.artifacts[&name]).unwrap();
    note.data = step.data.clone();
    let bytes = serde_json::to_vec(&note).unwrap();
    let a = c
        .recording
        .artifacts
        .iter_mut()
        .find(|a| a.name == name)
        .unwrap();
    a.size = bytes.len();
    a.sha256 = sha256_hex(&bytes);
    c.artifacts.insert(name, bytes);
    assert!(
        seal_test(&p, &c, &tmp.path().join("bad"))
            .err()
            .unwrap()
            .contains("pixels differ")
    );
}
#[test]
fn png_allocation_bomb_rejected_before_decoding() {
    let b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\xff\xff\xff\xff\xff\xff\xff\xff";
    let a = Artifact {
        name: "bomb.png".into(),
        kind: ArtifactKind::ScreenshotPng,
        size: b.len(),
        sha256: sha256_hex(b),
    };
    assert!(verify_artifact(&a, b).unwrap_err().contains("320x240"));
}
#[test]
fn plan_roundtrip_and_no_clobber() {
    let (_, p, _, _) = pending("arithmetic");
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("plan.json");
    save_plan(&p, &path).unwrap();
    assert!(save_plan(&p, &path).is_err());
    let loaded: CapturePlan = read_json(&path).unwrap();
    assert_eq!(loaded.sha256().unwrap(), p.sha256().unwrap());
}
#[test]
fn verification_labels_derive_from_successful_execution() {
    let s = scenario("arithmetic");
    let report = differential::compare(&s, vec![Native.execute(&s, &Cancellation::default())]);
    assert_eq!(
        classify_execution(&report),
        VerificationCategory::NativeOnly
    );
    let mut failed = report;
    failed.passed = false;
    assert_eq!(classify_execution(&failed), VerificationCategory::Untested);
}
#[cfg(unix)]
#[test]
fn symlink_artifacts_refused() {
    let (_, p, c, _) = pending("arithmetic");
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bundle");
    seal_test(&p, &c, &path).unwrap();
    let artifact = path.join("artifacts").join(&c.recording.artifacts[0].name);
    let moved = tmp.path().join("original");
    fs::rename(&artifact, &moved).unwrap();
    std::os::unix::fs::symlink(moved, artifact).unwrap();
    assert!(SealedCapture::load(&path).is_err());
}
