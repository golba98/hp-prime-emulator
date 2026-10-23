//! Tests for the v2 pipeline (suite groups, audit, generator, transcript
//! parser, manifests, evidence).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;

fn suite_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/compat/cases/{name}.toml"))
}

fn core() -> Suite {
    load_suite(&suite_path("core")).expect("core.toml loads")
}

fn smoke() -> Suite {
    load_suite(&suite_path("g1_smoke")).expect("g1_smoke.toml loads")
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ppl-compat-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Runs a Print-channel program in the emulator, parses its transcript and
/// checks every outcome equals the per-case emulator run.
fn assert_self_consistent(suite: &Suite, g: &GeneratedProgram) {
    let transcript = emulate_print_program(g).unwrap_or_else(|e| panic!("{}: {e}", g.program));
    let run = parse_result_lines(&transcript, &g.short_ids)
        .unwrap_or_else(|e| panic!("{}: {e}\n{transcript}", g.program));
    assert_eq!(run.program, g.program);
    assert_eq!(run.sha8, g.sha8);
    assert_eq!(run.settings.len(), 3);
    assert!(
        run.setting_changes.is_empty(),
        "{}: {:?}",
        g.program,
        run.setting_changes
    );
    for (_, id) in &g.short_ids {
        let c = suite.cases.iter().find(|c| &c.id == id).unwrap();
        let direct = run_case(c);
        let via = &run.results[id].outcome;
        match (&direct, via) {
            (Outcome::Value { .. }, _) => assert_eq!(&direct, via, "case {id}\n{transcript}"),
            (Outcome::Error { .. }, Outcome::Error { .. }) => {}
            _ => panic!("case {id}: direct {direct:?}, via program {via:?}\n{transcript}"),
        }
    }
}

#[test]
fn g1_smoke_print_program() {
    let s = smoke();
    assert_eq!(s.cases.len(), 5);
    let cases = select_cases(&s, Some("smoke"), None).unwrap();
    let g = generate_device_program(&s, &cases, "ZZT_SMOKE1", &Channel::Print).unwrap();
    // Shown with `cargo test -p ppl-compat g1_smoke_print_program -- --nocapture`.
    println!("{}", g.source);
    assert_eq!(verify_program_tag(&g.source).unwrap().1, g.sha8);
    assert_eq!(g.sha256, sha256_hex(g.source.as_bytes()));
    assert_self_consistent(&s, &g);
    let transcript = emulate_print_program(&g).unwrap();
    println!("--- emulator transcript ---\n{transcript}");
}

#[test]
fn every_core_group_generates_and_is_self_consistent() {
    let s = core();
    assert!(!s.groups.is_empty());
    let mut programs = 0;
    for grp in &s.groups {
        let cases = select_cases(&s, Some(&grp.id), None).unwrap();
        assert!(cases.len() <= MAX_CASES_PER_PROGRAM, "group {}", grp.id);
        let (iso, shared): (Vec<&Case>, Vec<&Case>) = cases.into_iter().partition(|c| c.isolate);
        if !shared.is_empty() {
            let name = format!("ZZT_G{programs}");
            let g = generate_device_program(&s, &shared, &name, &Channel::Print)
                .unwrap_or_else(|e| panic!("group {}: {e:?}", grp.id));
            assert_self_consistent(&s, &g);
            programs += 1;
        }
        for c in iso {
            let name = format!("ZZT_I{programs}");
            let g = generate_device_program(&s, &[c], &name, &Channel::Print)
                .unwrap_or_else(|e| panic!("case {}: {e:?}", c.id));
            assert_self_consistent(&s, &g);
            programs += 1;
            // An isolated case never shares a program.
            let other = s.cases.iter().find(|o| o.id != c.id && !o.isolate).unwrap();
            assert!(generate_device_program(&s, &[c, other], "ZZT_X", &Channel::Print).is_err());
        }
    }
    assert!(programs > s.groups.len());
}

#[test]
fn every_core_case_is_grouped() {
    let s = core();
    for c in &s.cases {
        assert!(c.group.is_some(), "case {} has no group", c.id);
        if !c.mutates.is_empty() {
            assert!(c.isolate, "case {} mutates without isolate", c.id);
        }
    }
}

#[test]
fn hangle_cases_need_isolation_and_declaration() {
    let s = core();
    let deg = s.cases.iter().find(|c| c.id == "safe_hangle_deg").unwrap();
    let add = s
        .cases
        .iter()
        .find(|c| c.id == "num_div_one_third")
        .unwrap();
    assert_eq!(
        audit_case(deg)
            .unwrap()
            .side_effects
            .into_iter()
            .collect::<Vec<_>>(),
        ["HAngle"]
    );
    // Alone: accepted.
    let g = generate_device_program(&s, &[deg], "ZZT_HA1", &Channel::Print).unwrap();
    assert_self_consistent(&s, &g);
    // Combined with another case: refused.
    let err = generate_device_program(&s, &[deg, add], "ZZT_HA2", &Channel::Print).unwrap_err();
    assert!(err.iter().any(|e| e.contains("only case")), "{err:?}");
    // Without `mutates`: refused even alone.
    let mut bare = deg.clone();
    bare.mutates.clear();
    let err = generate_device_program(&s, &[&bare], "ZZT_HA3", &Channel::Print).unwrap_err();
    assert!(
        err.iter()
            .any(|e| e.contains("undeclared side effects: HAngle")),
        "{err:?}"
    );
    // `mutates` without `isolate`: refused.
    let mut loose = deg.clone();
    loose.isolate = false;
    let err = generate_device_program(&s, &[&loose], "ZZT_HA4", &Channel::Print).unwrap_err();
    assert!(
        err.iter().any(|e| e.contains("not `isolate = true`")),
        "{err:?}"
    );
}

fn case(id: &str, body: &str) -> Case {
    toml::from_str(&format!("id = \"{id}\"\nbody = '''\n{body}\n'''\n")).unwrap()
}

#[test]
fn generator_refusals() {
    let s = smoke();
    let one = &s.cases[0];
    for bad in [
        "ZZT_",
        "ZZT_lower",
        "ABC_X",
        "ZZT_ABCDEFGHIJKLM",
        "ZZT_A-B",
        "zzt_A",
    ] {
        assert!(
            generate_device_program(&s, &[one], bad, &Channel::Print).is_err(),
            "{bad}"
        );
    }
    assert!(generate_device_program(&s, &[one], "ZZT_ABCDEFGHIJKL", &Channel::Print).is_ok());
    let bad_note = Channel::Note {
        note: "MYNOTE".into(),
    };
    assert!(generate_device_program(&s, &[one], "ZZT_A", &bad_note).is_err());
    assert!(generate_device_program(&s, &[], "ZZT_A", &Channel::Print).is_err());
    let many: Vec<Case> = (0..13)
        .map(|i| case(&format!("k{i}"), "RETURN 1;"))
        .collect();
    let refs: Vec<&Case> = many.iter().collect();
    let note = Channel::Note {
        note: "ZZT_RN1".into(),
    };
    assert!(generate_device_program(&s, &refs, "ZZT_A", &note).is_err());
    assert!(generate_device_program(&s, &refs[..12], "ZZT_A", &note).is_ok());
    assert!(generate_device_program(&s, &refs[..9], "ZZT_A", &Channel::Print).is_err());
    assert!(generate_device_program(&s, &refs[..8], "ZZT_A", &Channel::Print).is_ok());
    assert!(generate_device_program(&s, &[one, one], "ZZT_A", &Channel::Print).is_err());

    // PRINT inside a case: refused on the Print channel even when declared
    // and isolated; allowed on the Note channel when declared and isolated.
    let mut p = case("p", "PRINT(1); RETURN 0;");
    assert!(generate_device_program(&s, &[&p], "ZZT_A", &Channel::Print).is_err());
    p.mutates = vec!["PRINT".into()];
    p.isolate = true;
    let err = generate_device_program(&s, &[&p], "ZZT_A", &Channel::Print).unwrap_err();
    assert!(err.iter().any(|e| e.contains("corrupt")), "{err:?}");
    let note = Channel::Note {
        note: "ZZT_RN1".into(),
    };
    assert!(generate_device_program(&s, &[&p], "ZZT_A", &note).is_ok());

    // Undeclared system variable write, unknown command, compile error.
    assert!(
        generate_device_program(
            &s,
            &[&case("a", "A:=1; RETURN A;")],
            "ZZT_A",
            &Channel::Print
        )
        .is_err()
    );
    assert!(
        generate_device_program(&s, &[&case("b", "RETURN (;")], "ZZT_A", &Channel::Print).is_err()
    );
}

#[test]
fn note_channel_program_shape() {
    let s = smoke();
    let cases = select_cases(&s, None, None).unwrap();
    let ch = Channel::Note {
        note: "ZZT_RN7".into(),
    };
    let g = generate_device_program(&s, &cases, "ZZT_SMOKEN", &ch).unwrap();
    let (_, d) = ppl_syntax::parse(&g.source);
    assert!(!d.has_errors(), "{:?}", d.items);
    // Hardware finding: Notes() of a missing note does not raise on the G1,
    // so the program has no in-program existence check (see generate.rs).
    assert!(!g.source.contains("ABORT"));
    assert!(!g.source.contains("IFERR t:=Notes"));
    // Crash-resilient evidence: the note is written before the first case
    // and re-written after every case (plus the final @@END write), so a
    // program that dies mid-run still leaves completed cases behind.
    let first_write = g.source.find("Notes(\"ZZT_RN7\"):=r").unwrap();
    let first_case = g.source.find("v:=ZC1()").unwrap();
    assert!(
        first_write < first_case,
        "header write precedes the first case"
    );
    assert_eq!(
        g.source.matches("Notes(\"ZZT_RN7\"):=r").count(),
        cases.len() + 2,
        "header + per-case + final writes"
    );
    assert!(!g.source.contains("PRINT(\"c1"));
    assert_eq!(verify_program_tag(&g.source).unwrap().0, "ZZT_SMOKEN");
    assert!(emulate_print_program(&g).is_err());
}

#[test]
fn program_tag_detects_tampering() {
    let s = smoke();
    let cases = select_cases(&s, None, None).unwrap();
    let g = generate_device_program(&s, &cases, "ZZT_TAG", &Channel::Print).unwrap();
    assert_eq!(
        verify_program_tag(&g.source).unwrap(),
        ("ZZT_TAG".to_string(), g.sha8.clone())
    );
    let tampered = g.source.replace("RETURN 6*7;", "RETURN 6*8;");
    assert!(verify_program_tag(&tampered).is_err());
    // Same cases, different name => different tag.
    let g2 = generate_device_program(&s, &cases, "ZZT_TAG2", &Channel::Print).unwrap();
    assert_ne!(g.sha8, g2.sha8);
}

fn ids() -> Vec<(String, String)> {
    vec![
        ("c1".into(), "a".into()),
        ("c2".into(), "b".into()),
        ("c3".into(), "c".into()),
    ]
}

#[test]
fn transcript_parser() {
    let text = "junk before\r\n@@ZZT|ZZT_P|abcd1234|1|0|12\r\nc1|2|x|y|z\nc2|E|\n\nc2|SETCHG|HAngle\nc3|0|3\n@@END\ntrailing";
    let r = parse_result_lines(text, &ids()).unwrap();
    assert_eq!(r.program, "ZZT_P");
    assert_eq!(r.sha8, "abcd1234");
    assert_eq!(r.settings["HAngle"], "1");
    assert_eq!(r.settings["HFormat"], "0");
    assert_eq!(r.settings["HDigits"], "12");
    assert_eq!(
        r.results["a"].outcome,
        Outcome::Value {
            ty: 2,
            text: "x|y|z".into()
        }
    );
    assert_eq!(r.results["a"].raw, "c1|2|x|y|z");
    assert!(matches!(r.results["b"].outcome, Outcome::Error { .. }));
    assert_eq!(r.setting_changes, [("b".to_string(), "HAngle".to_string())]);

    // Missing record in a complete transcript => Missing.
    let r = parse_result_lines("@@ZZT|P|00000000|0|0|0\nc1|0|1\n@@END", &ids()).unwrap();
    assert_eq!(r.results["c"].outcome, Outcome::Missing);
    // Truncated, unknown record, duplicate, bad type, bad header.
    assert!(parse_result_lines("@@ZZT|P|00000000|0|0|0\nc1|0|1", &ids()).is_err());
    assert!(parse_result_lines("@@ZZT|P|00000000|0|0|0\nc9|0|1\n@@END", &ids()).is_err());
    assert!(parse_result_lines("@@ZZT|P|00000000|0|0|0\nc1|0|1\nc1|0|1\n@@END", &ids()).is_err());
    assert!(parse_result_lines("@@ZZT|P|00000000|0|0|0\nc1|x|1\n@@END", &ids()).is_err());
    assert!(parse_result_lines("@@ZZT|P|0|0\nc1|0|1\n@@END", &ids()).is_err());
    assert!(parse_result_lines("c1|0|1\n@@END", &ids()).is_err());
}

#[test]
fn select_cases_rules() {
    let s = core();
    let g = select_cases(&s, Some("settings_isolated"), None).unwrap();
    assert_eq!(g.len(), 2);
    let want = vec!["safe_hangle_rad".to_string()];
    let one = select_cases(&s, Some("settings_isolated"), Some(&want)).unwrap();
    assert_eq!(one[0].id, "safe_hangle_rad");
    assert!(select_cases(&s, Some("nope"), None).is_err());
    assert!(select_cases(&s, None, Some(&["nope".to_string()])).is_err());
    assert!(select_cases(&s, Some("types"), Some(&want)).is_err());
    assert_eq!(select_cases(&s, None, None).unwrap().len(), s.cases.len());
}

#[test]
fn suite_v1_still_loads_without_groups() {
    let s: Suite = toml::from_str("suite = \"x\"\n[[case]]\nid = \"a\"\nexpr = \"1\"\n").unwrap();
    assert!(s.groups.is_empty());
    assert!(!s.cases[0].isolate && s.cases[0].mutates.is_empty());
}

#[test]
fn manifest_and_ledger() {
    let dir = temp_dir("runs");
    let s = smoke();
    let cases = select_cases(&s, None, None).unwrap();
    assert_eq!(unused_name(&dir, "smoke").unwrap(), "ZZT_SMOKE1");
    let g = generate_device_program(&s, &cases, "ZZT_SMOKE1", &Channel::Print).unwrap();
    let m = RunManifest::prepared(&s.suite, Some("smoke"), &g, "2026-10-22T00:00:00Z");
    m.verify_source().unwrap();
    let path = save_manifest(&dir, &m).unwrap();
    assert!(path.ends_with("ZZT_SMOKE1.json"));
    let back = load_manifest(&dir, "ZZT_SMOKE1").unwrap();
    assert_eq!(back, m);
    assert_eq!(back.status, RunStatus::Prepared);
    assert!(load_manifest(&dir, "../etc").is_err());

    assert!(!ledger_contains(&dir, "ZZT_SMOKE1").unwrap());
    ledger_append(
        &dir,
        &["ZZT_SMOKE1".into(), "ZZT_RN1".into()],
        "ZZT_SMOKE1",
        "t",
    )
    .unwrap();
    assert!(ledger_contains(&dir, "ZZT_SMOKE1").unwrap());
    assert!(ledger_contains(&dir, "ZZT_RN1").unwrap());
    assert_eq!(unused_name(&dir, "smoke").unwrap(), "ZZT_SMOKE2");
    assert_eq!(
        unused_name(&dir, "averyveryverylongstem").unwrap().len(),
        16
    );

    let mut bad = m.clone();
    bad.source.push(' ');
    assert!(bad.verify_source().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn serial_fingerprints() {
    let a = serial_fingerprint("CN12345678");
    assert_eq!(a.len(), 16);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(a, serial_fingerprint("CN12345678"));
    assert_ne!(a, serial_fingerprint("CN12345679"));
    assert_eq!(a, sha256_hex(b"ppl-hp-prime-serial-v1:CN12345678")[..16]);
}

fn device() -> DeviceRecord {
    DeviceRecord {
        model: "HP Prime G1".into(),
        usb_pid: "03f0:0441".into(),
        firmware_build: Some(15157),
        boot_version: None,
        serial_id: None,
    }
}

#[test]
fn evidence_v2_roundtrip_merge_and_compare() {
    let dir = temp_dir("evidence");
    let s = smoke();
    let cases = select_cases(&s, None, None).unwrap();
    let g = generate_device_program(&s, &cases, "ZZT_SMOKE1", &Channel::Print).unwrap();
    let m = RunManifest::prepared(&s.suite, None, &g, "2026-10-22T00:00:00Z");
    let emu = run_emulator(&s);

    // A synthetic transcript (unit test only, never saved as real evidence
    // outside the temp dir): exact, normalised-only, wrong, error.
    let text = format!(
        "@@ZZT|ZZT_SMOKE1|{}|0|0|12\nc1|0|3\nc2|0| 42 \nc3|0|0.333\nc4|E|\n@@END\n",
        g.sha8
    );
    let run = parse_result_lines(&text, &g.short_ids).unwrap();
    let mut ev = EvidenceV2::from_run(
        &m,
        &run,
        METHOD_SCREENSHOT,
        device(),
        text.clone(),
        "2026-10-22T01:00:00Z",
    )
    .unwrap();
    ev.compare_with(&emu);
    assert_eq!(ev.comparison["smoke_add"], Status::Verified);
    assert_eq!(ev.comparison["smoke_mul"], Status::Partial);
    assert_eq!(ev.comparison["smoke_third"], Status::Failed);
    assert_eq!(ev.comparison["smoke_round"], Status::Failed);
    assert_eq!(ev.comparison["smoke_strcat"], Status::Blocked);
    save_evidence_v2(&dir, &ev).unwrap();
    assert!(
        save_evidence_v2(&dir, &ev).is_err(),
        "evidence must not be overwritten"
    );

    // A wrong tag is rejected.
    let wrong = parse_result_lines(&text.replace(&g.sha8, "deadbeef"), &g.short_ids).unwrap();
    assert!(
        EvidenceV2::from_run(&m, &wrong, METHOD_SCREENSHOT, device(), String::new(), "t").is_err()
    );

    // Legacy v1 file alongside, older.
    let mut v1_results = BTreeMap::new();
    v1_results.insert(
        "smoke_third".to_string(),
        Outcome::Value {
            ty: 0,
            text: "0.333333333333".into(),
        },
    );
    let v1 = Evidence {
        suite: "g1_smoke".into(),
        program: "ZZT_OLD".into(),
        source_fnv: "0".into(),
        device_model: "HP Prime G1".into(),
        usb_pid: "03f0:0441".into(),
        firmware_build: Some(15157),
        boot_version: None,
        serial_id: None,
        recorded_at_utc: "2026-10-21T00:00:00Z".into(),
        method: "manual-run".into(),
        results: v1_results,
    };
    std::fs::write(
        dir.join("g1_smoke.json"),
        serde_json::to_string(&v1).unwrap(),
    )
    .unwrap();

    let all = load_suite_evidence(&dir, "g1_smoke").unwrap();
    assert_eq!(all.len(), 2);
    let merged = merge_evidence(&all);
    // Latest wins: the v2 run's (wrong) 0.333 replaces the older v1 record.
    assert_eq!(merged["smoke_third"].program, "ZZT_SMOKE1");
    // Missing never replaces an observed result; with no observation it stays.
    assert_eq!(merged["smoke_strcat"].evidence.outcome, Outcome::Missing);

    let rep = compare_v2(&s, &emu, &merged);
    let st: BTreeMap<_, _> = rep
        .cases
        .iter()
        .map(|c| (c.id.as_str(), c.status))
        .collect();
    assert_eq!(st["smoke_add"], Status::Verified);
    assert_eq!(st["smoke_mul"], Status::Partial);
    assert_eq!(st["smoke_third"], Status::Failed);
    assert_eq!(st["smoke_strcat"], Status::Blocked);
    let add = rep.cases.iter().find(|c| c.id == "smoke_add").unwrap();
    assert_eq!(add.device_raw.as_deref(), Some("c1|0|3"));
    assert_eq!(add.evidence_program.as_deref(), Some("ZZT_SMOKE1"));
    assert!(rep.markdown().contains("smoke_add"));

    // No evidence => UNVERIFIED.
    let rep = compare_v2(&s, &emu, &BTreeMap::new());
    assert!(rep.cases.iter().all(|c| c.status == Status::Unverified));
    assert!(load_suite_evidence(&dir, "absent").unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn display_normaliser_is_narrow() {
    assert_eq!(normalise_display(" 1\u{1D07}\u{2212}5 "), "1E-5");
    let emu = Outcome::Value {
        ty: 0,
        text: "1E-5".into(),
    };
    let dev = |t: &str| Outcome::Value {
        ty: 0,
        text: t.into(),
    };
    assert_eq!(status_v2(&emu, Some(&dev("1E-5"))), Status::Verified);
    assert_eq!(
        status_v2(&emu, Some(&dev("1\u{1D07}\u{2212}5"))),
        Status::Partial
    );
    assert_eq!(status_v2(&emu, Some(&dev("0.00001"))), Status::Failed);
    assert_eq!(
        status_v2(
            &emu,
            Some(&Outcome::Value {
                ty: 3,
                text: "1E-5".into()
            })
        ),
        Status::Failed
    );
    assert_eq!(status_v2(&emu, None), Status::Unverified);
}
