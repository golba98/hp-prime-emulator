//! Synthetic records only; these are harness tests, never physical evidence.
use ppl_compat::{Channel, DeviceRecord, EvidenceV2, RunManifest, Suite, assessment::qualify};

fn fixture() -> (Suite, EvidenceV2, RunManifest, Vec<u8>) {
    let suite: Suite =
        toml::from_str("suite='audit_test'\n[[case]]\nid='third'\nexpr='1/3'\n").unwrap();
    let generated = ppl_compat::generate_device_program(
        &suite,
        &[&suite.cases[0]],
        "ZZT_AUDIT",
        &Channel::Print,
    )
    .unwrap();
    let mut manifest = RunManifest::prepared(&suite.suite, None, &generated, "test");
    let text = ppl_compat::emulate_print_program(&generated).unwrap();
    let parsed = ppl_compat::parse_result_lines(&text, &generated.short_ids).unwrap();
    let device = DeviceRecord {
        model: "synthetic unit fixture, not hardware".into(),
        usb_pid: "03f0:1541".into(),
        firmware_build: Some(15157),
        boot_version: Some("V0.050.640".into()),
        serial_id: Some("synthetic".into()),
    };
    manifest.device = Some(device.clone());
    let mut ev = EvidenceV2::from_run(
        &manifest,
        &parsed,
        ppl_compat::METHOD_SCREENSHOT,
        device,
        text,
        "test",
    )
    .unwrap();
    let artifact = b"synthetic retained artifact, not a calculator capture".to_vec();
    ev.raw_artifact_sha256 = Some(ppl_compat::sha256_hex(&artifact));
    (suite, ev, manifest, artifact)
}

#[test]
fn unchanged_case_is_qualified_but_changed_source_or_artifact_is_refused() {
    let (mut suite, ev, m, raw) = fixture();
    assert!(qualify(&suite, &ev, &m, &raw).is_ok());
    assert!(qualify(&suite, &ev, &m, b"changed").is_err());
    suite.cases[0].expr = Some("2/3".into());
    assert!(
        qualify(&suite, &ev, &m, &raw)
            .unwrap_err()
            .contains("current cases")
    );
}

#[test]
fn incomplete_or_tampered_transcript_cannot_qualify() {
    let (suite, mut ev, m, raw) = fixture();
    ev.raw_text = ev.raw_text.replace("@@END", "");
    assert!(qualify(&suite, &ev, &m, &raw).is_err());
    let (suite, mut ev, m, raw) = fixture();
    ev.results.get_mut("third").unwrap().outcome = ppl_compat::Outcome::Missing;
    assert!(qualify(&suite, &ev, &m, &raw).is_err());
}

#[test]
fn settings_are_taken_from_reference_and_unknown_formats_fail_closed() {
    let (suite, mut ev, m, raw) = fixture();
    ev.settings.insert("HFormat".into(), "1".into());
    ev.settings.insert("HDigits".into(), "2".into());
    ev.raw_text = ev.raw_text.replace("|0|0|4", "|0|1|2");
    let settings = qualify(&suite, &ev, &m, &raw).unwrap();
    assert_eq!(
        ppl_compat::run_case_with_settings(&suite.cases[0], settings),
        ppl_compat::Outcome::Value {
            ty: 0,
            text: "0.33".into()
        }
    );
    ev.settings.insert("HFormat".into(), "5".into());
    ev.raw_text = ev.raw_text.replace("|0|1|2", "|0|5|2");
    assert!(qualify(&suite, &ev, &m, &raw).is_err());
}

#[test]
fn mismatched_device_or_unverified_method_cannot_qualify() {
    let (suite, mut ev, m, raw) = fixture();
    ev.device.firmware_build = Some(8151);
    assert!(qualify(&suite, &ev, &m, &raw).is_err());
    let (suite, mut ev, m, raw) = fixture();
    ev.method = "local-firmware".into();
    assert!(qualify(&suite, &ev, &m, &raw).is_err());
}

#[test]
fn conflicting_references_are_preserved_instead_of_latest_winning() {
    let (suite, mut first, manifest, raw) = fixture();
    let dir = std::env::temp_dir().join(format!("ppl-conflicting-evidence-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("synthetic.raw");
    std::fs::write(&path, &raw).unwrap();
    first.raw_artifact = Some(path.to_str().unwrap().into());
    ppl_compat::save_manifest(&dir, &manifest).unwrap();
    let mut second = first.clone();
    second.raw_text = second.raw_text.replace("0.333333333333", "0.666666666667");
    second.results = ppl_compat::parse_result_lines(&second.raw_text, &manifest.short_ids)
        .unwrap()
        .results;
    let assessed = ppl_compat::assessment::assess(&suite, &[first, second], &dir, &dir);
    assert!(assessed.audits.iter().all(|a| a.accepted));
    assert_eq!(assessed.observations["third"].len(), 2);
    assert_eq!(
        assessed.report.cases[0].status,
        ppl_compat::Status::Unstable
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn note_text_must_match_its_retained_bytes() {
    let (suite, mut ev, mut manifest, _) = fixture();
    let generated = ppl_compat::generate_device_program(
        &suite,
        &[&suite.cases[0]],
        &manifest.program,
        &Channel::Note {
            note: "ZZT_AUDITR".into(),
        },
    )
    .unwrap();
    let device = manifest.device.clone();
    manifest = RunManifest::prepared(&suite.suite, None, &generated, "test");
    manifest.device = device;
    // Note and print share the same observation grammar, but each wrapper has
    // its own source tag. Keep that tag bound to the regenerated note source.
    let tag = ppl_compat::verify_program_tag(&manifest.source).unwrap().1;
    let old_tag = ppl_compat::parse_result_lines(&ev.raw_text, &manifest.short_ids)
        .unwrap()
        .sha8;
    ev.raw_text = ev.raw_text.replace(&old_tag, &tag);
    ev.program_sha256 = manifest.sha256.clone();
    ev.method = ppl_compat::METHOD_NOTE_FETCH.into();
    let raw: Vec<_> = ev
        .raw_text
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    ev.raw_artifact_sha256 = Some(ppl_compat::sha256_hex(&raw));
    assert!(qualify(&suite, &ev, &manifest, &raw).is_ok());
    ev.raw_text = ev.raw_text.replace("0.333333333333", "0.666666666667");
    ev.results = ppl_compat::parse_result_lines(&ev.raw_text, &manifest.short_ids)
        .unwrap()
        .results;
    assert!(
        qualify(&suite, &ev, &manifest, &raw)
            .unwrap_err()
            .contains("note")
    );
}

struct Temporary(std::path::PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn registered_fixture() -> (Suite, EvidenceV2, RunManifest, Temporary) {
    let (suite, mut ev, manifest, raw) = fixture();
    let dir = std::env::temp_dir().join(format!(
        "ppl-interpretation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let artifact = dir.join("synthetic.raw");
    std::fs::write(&artifact, raw).unwrap();
    ev.raw_artifact = Some(artifact.to_str().unwrap().into());
    ppl_compat::save_manifest(&dir, &manifest).unwrap();
    (suite, ev, manifest, Temporary(dir))
}

fn dispute(ev: &EvidenceV2) -> ppl_compat::assessment::provenance::Revision {
    use ppl_compat::assessment::provenance::{Binding, Interpretation, Revision};
    Revision {
        id: "synthetic-dispute".into(),
        binding: Binding::from_evidence(ev).unwrap(),
        case_ids: vec!["third".into()],
        reason: "synthetic capture/transcription disagreement".into(),
        evidence_links: vec!["synthetic.raw; unit-test evidence only".into()],
        supersedes: vec![],
        interpretation: Interpretation::Disputed,
    }
}

#[test]
fn disputed_interpretation_is_inconclusive_without_losing_its_capture() {
    use ppl_compat::assessment::provenance::append_revision;
    let (suite, ev, _, dir) = registered_fixture();
    let before = ev.clone();
    append_revision(&dir.0.join("provenance.json"), &dispute(&ev)).unwrap();
    let result = ppl_compat::assessment::assess(&suite, std::slice::from_ref(&ev), &dir.0, &dir.0);
    assert_eq!(
        result.report.cases[0].status,
        ppl_compat::Status::Inconclusive
    );
    assert!(result.observations["third"][0].disputed);
    assert_eq!(
        result.observations["third"][0].physical,
        ev.results["third"].outcome
    );
    assert_eq!(result.interpretation_history.len(), 1);
    assert_eq!(ev, before);
}

#[test]
fn confirmed_correction_supersedes_only_the_interpretation_not_physical_stability() {
    use ppl_compat::assessment::provenance::{Interpretation, Registry, append_revision};
    let (suite, mut ev, manifest, dir) = registered_fixture();
    let correct = ev.raw_text.clone();
    ev.raw_text = correct.replace("0.333333333333", "0.666666666667");
    ev.results = ppl_compat::parse_result_lines(&ev.raw_text, &manifest.short_ids)
        .unwrap()
        .results;
    let old_ev = ev.clone();
    let original = dispute(&ev);
    let registry = dir.0.join("provenance.json");
    append_revision(&registry, &original).unwrap();
    let mut correction = original.clone();
    correction.id = "synthetic-confirmed-correction".into();
    correction.supersedes = vec![original.id.clone()];
    correction.interpretation = Interpretation::ConfirmedCorrection {
        corrected_transcript_sha256: ppl_compat::sha256_hex(correct.as_bytes()),
        corrected_transcript: correct,
        user_confirmed: true,
        confirmation_record: "synthetic unit confirmation, not a real user record".into(),
        confirmed_at_utc: "synthetic".into(),
    };
    append_revision(&registry, &correction).unwrap();
    let result = ppl_compat::assessment::assess(&suite, std::slice::from_ref(&ev), &dir.0, &dir.0);
    assert_eq!(result.report.cases[0].status, ppl_compat::Status::Verified);
    let observed = &result.observations["third"][0];
    assert!(!observed.disputed);
    assert_eq!(
        observed.original_physical.as_ref(),
        Some(&old_ev.results["third"].outcome)
    );
    assert_eq!(
        observed.interpretation_id.as_deref(),
        Some(correction.id.as_str())
    );
    assert_eq!(result.interpretation_history.len(), 2);
    assert_eq!(Registry::load(&registry).unwrap().revisions[0], original);
    assert_eq!(
        std::fs::read_dir(dir.0.join("provenance-records"))
            .unwrap()
            .count(),
        2
    );
    assert_eq!(ev, old_ev);
}

#[test]
fn correction_requires_confirmation_and_preserves_the_previous_index_on_refusal() {
    use ppl_compat::assessment::provenance::{Interpretation, append_revision};
    let (_, ev, _, dir) = registered_fixture();
    let mut revision = dispute(&ev);
    let registry = dir.0.join("provenance.json");
    append_revision(&registry, &revision).unwrap();
    let before = std::fs::read(&registry).unwrap();
    revision.supersedes = vec![revision.id.clone()];
    revision.id = "unconfirmed-correction".into();
    revision.interpretation = Interpretation::ConfirmedCorrection {
        corrected_transcript: ev.raw_text.clone(),
        corrected_transcript_sha256: ppl_compat::sha256_hex(ev.raw_text.as_bytes()),
        user_confirmed: false,
        confirmation_record: "not confirmed".into(),
        confirmed_at_utc: "synthetic".into(),
    };
    assert!(append_revision(&registry, &revision).is_err());
    assert_eq!(std::fs::read(registry).unwrap(), before);
}

#[test]
fn interpretation_binding_checks_both_raw_artifact_and_original_transcript_hash() {
    use ppl_compat::assessment::provenance::append_revision;
    for wrong_artifact in [true, false] {
        let (suite, ev, _, dir) = registered_fixture();
        let mut revision = dispute(&ev);
        if wrong_artifact {
            revision.binding.raw_artifact_sha256 = "wrong artifact hash".into();
        } else {
            revision.binding.original_transcript_sha256 = "wrong transcript hash".into();
        }
        append_revision(&dir.0.join("provenance.json"), &revision).unwrap();
        let result = ppl_compat::assessment::assess(&suite, &[ev], &dir.0, &dir.0);
        assert_eq!(
            result.report.cases[0].status,
            ppl_compat::Status::Inconclusive
        );
        assert!(result.observations.is_empty());
        assert!(result.audits[0].reason.contains("binding differs"));
    }
}

#[test]
fn observed_source_must_match_its_exact_declared_prefix_and_both_hashes() {
    use ppl_compat::assessment::provenance::{Registry, SourceObservation};
    let (suite, ev, manifest, dir) = registered_fixture();
    let observed = format!("\n {}", manifest.source);
    let raw: Vec<_> = format!("{observed}\0")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(dir.0.join("source.bin"), &raw).unwrap();
    let source = SourceObservation {
        id: "synthetic-source-prefix".into(),
        program: ev.program.clone(),
        authored_source_sha256: manifest.sha256,
        observed_source_path: "source.bin".into(),
        observed_transport_sha256: ppl_compat::sha256_hex(&raw),
        observed_utf8_sha256: ppl_compat::sha256_hex(observed.as_bytes()),
        added_prefix_before_opening_comment: "\n ".into(),
        single_final_device_nul: true,
    };
    let mut registry = Registry {
        source_observations: vec![source],
        ..Default::default()
    };
    let index = dir.0.join("provenance.json");
    std::fs::write(&index, serde_json::to_vec(&registry).unwrap()).unwrap();
    let result = ppl_compat::assessment::assess(&suite, std::slice::from_ref(&ev), &dir.0, &dir.0);
    assert_eq!(result.report.cases[0].status, ppl_compat::Status::Verified);
    assert!(result.observations["third"][0].source_observation.is_some());

    // Re-hashing a body edit does not make it an allowed whitespace prefix.
    let edited = observed.replace("RETURN 1/3", "RETURN 2/3");
    let raw: Vec<_> = format!("{edited}\0")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(dir.0.join("source.bin"), &raw).unwrap();
    registry.source_observations[0].observed_transport_sha256 = ppl_compat::sha256_hex(&raw);
    registry.source_observations[0].observed_utf8_sha256 =
        ppl_compat::sha256_hex(edited.as_bytes());
    std::fs::write(&index, serde_json::to_vec(&registry).unwrap()).unwrap();
    let result = ppl_compat::assessment::assess(&suite, &[ev], &dir.0, &dir.0);
    assert!(!result.audits[0].accepted);
    assert!(result.audits[0].reason.contains("beyond the declared"));
}

#[test]
fn correction_cannot_change_an_undeclared_case_or_reference_settings() {
    use ppl_compat::assessment::provenance::{Interpretation, append_revision};
    let (_, mut ev, old_manifest, dir) = registered_fixture();
    let suite: Suite = toml::from_str(
        "suite='audit_two'\n[[case]]\nid='third'\nexpr='1/3'\n[[case]]\nid='two'\nexpr='2'\n",
    )
    .unwrap();
    let cases = suite.cases.iter().collect::<Vec<_>>();
    let generated =
        ppl_compat::generate_device_program(&suite, &cases, "ZZT_TWOAUDIT", &Channel::Print)
            .unwrap();
    let mut manifest = RunManifest::prepared(&suite.suite, None, &generated, "synthetic");
    manifest.device = old_manifest.device;
    let original = ppl_compat::emulate_print_program(&generated).unwrap();
    let parsed = ppl_compat::parse_result_lines(&original, &manifest.short_ids).unwrap();
    ev.suite = suite.suite.clone();
    ev.program = manifest.program.clone();
    ev.program_sha256 = manifest.sha256.clone();
    ev.raw_text = original.clone();
    ev.results = parsed.results;
    ppl_compat::save_manifest(&dir.0, &manifest).unwrap();
    for (label, corrected) in [
        ("undeclared case", original.replace("c2|0|2", "c2|0|3")),
        ("settings", original.replace("|0|0|4", "|0|1|2")),
    ] {
        let registry_path = dir.0.join(format!("{label}/provenance.json"));
        let mut revision = dispute(&ev);
        revision.id = format!("synthetic-invalid-{label}");
        revision.interpretation = Interpretation::ConfirmedCorrection {
            corrected_transcript_sha256: ppl_compat::sha256_hex(corrected.as_bytes()),
            corrected_transcript: corrected,
            user_confirmed: true,
            confirmation_record: "synthetic unit fixture".into(),
            confirmed_at_utc: "synthetic".into(),
        };
        append_revision(&registry_path, &revision).unwrap();
        let result = ppl_compat::assessment::assess(
            &suite,
            std::slice::from_ref(&ev),
            &dir.0,
            registry_path.parent().unwrap(),
        );
        assert!(!result.audits[0].accepted);
        assert!(
            result.audits[0].reason.contains(label),
            "{}",
            result.audits[0].reason
        );
    }
}

#[test]
fn overlapping_revision_requires_explicit_same_origin_supersession() {
    use ppl_compat::assessment::provenance::append_revision;
    let (_, ev, _, dir) = registered_fixture();
    let first = dispute(&ev);
    let path = dir.0.join("provenance.json");
    append_revision(&path, &first).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut second = first.clone();
    second.id = "synthetic-overlap".into();
    assert!(
        append_revision(&path, &second)
            .unwrap_err()
            .contains("explicit supersession")
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    second.supersedes = vec![first.id];
    second.binding.original_transcript_sha256 = "another transcript".into();
    assert!(
        append_revision(&path, &second)
            .unwrap_err()
            .contains("differs in origin")
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}
