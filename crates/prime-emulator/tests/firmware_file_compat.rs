//! Local firmware only. Opt in with PRIME_FILE_COMPAT_OUT and PRIME_TEST_*.
//! Every session is confined to a temporary profile. Evidence is retained only
//! in the explicitly supplied artifacts directory, never in the factory tree.
#[allow(dead_code)]
#[path = "support/hpprgm_corpus.rs"]
mod corpus;
use prime_emulator::{calcverify, import, progverify, sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn configured(key: &str) -> PathBuf {
    let p = PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("{key} must be set")));
    assert!(p.is_absolute(), "{key} must be absolute");
    p
}

#[test]
#[ignore = "requires local firmware and a fresh PRIME_FILE_COMPAT_OUT artifact directory"]
fn local_firmware_corpus_import_reopen_and_reboot() {
    let out = configured("PRIME_FILE_COMPAT_OUT");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::create_dir(&out)
        .expect("evidence output must be fresh; never overwrite a previous capture");
    let firmware = configured("PRIME_TEST_FIRMWARE");
    let os_rom = configured("PRIME_TEST_OS_ROM");
    let factory = configured("PRIME_TEST_FACTORY_A");
    let init = configured("PRIME_TEST_INIT_DATA");
    let hash = |p: &Path| sha256::hex(&std::fs::read(p).unwrap());
    assert_eq!(
        hash(&firmware),
        "ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39"
    );
    let mut results = Vec::new();
    for c in corpus::cases().into_iter().filter(|c| c.firmware) {
        eprintln!("firmware case {}", c.id);
        let cfg = progverify::ProgramVerifyConfig {
            firmware_path: firmware.clone(),
            os_rom_path: Some(os_rom.clone()),
            factory_a: factory.clone(),
            out_dir: out.join(&c.id),
            programs: vec![progverify::ProgramSpec {
                path: corpus::root().join(&c.path),
                name: c.name.clone(),
                as_source: c.as_source,
            }],
            init_data: Some(init.clone()),
            setup_keys: calcverify::DEFAULT_SETUP_KEYS.into(),
            settle_ms: 300,
            boot_timeout: Duration::from_secs(30),
            key_timeout: Duration::from_secs(8),
            max_instructions: 8_000_000_000,
            max_seconds: 60.0,
            persistence: true,
            verbosity: 0,
        };
        let report = progverify::verify(&cfg);
        std::fs::write(
            out.join(&c.id).join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        let imported = import::prepare(&import::ImportRequest {
            program: corpus::root().join(&c.path),
            data_dir: out.clone(),
            name: c.name,
            replace: false,
            as_source: c.as_source,
        })
        .unwrap();
        let mut saves = Vec::new();
        for session in ["session1", "session2"] {
            let path = out.join(&c.id).join(session).join("data");
            // progverify stores the profile as out/<session>/data.
            let file = import::find_program(&import::data_dir_of(&path), &imported.name);
            let saved = file.and_then(|p| std::fs::read(p).ok());
            saves.push(saved.map(|b| {
                let source = prime_emulator::hpprgm::parse(&b).and_then(|c| c.source()).ok().map(|s|s.source);
                serde_json::json!({"sha256":sha256::hex(&b),"size":b.len(),"source_sha256":source.as_ref().map(|s|sha256::hex(s.as_bytes())),"source":source})
            }));
        }
        results.push(serde_json::json!({"id":c.id,"input_sha256":c.sha256,"imported_sha256":sha256::hex(&imported.bytes),"source_sha256":sha256::hex(imported.source.as_bytes()),"report":report,"saves":saves}));
        std::fs::write(out.join("capture.json"),serde_json::to_vec_pretty(&serde_json::json!({"provenance":"local-firmware","firmware_version":"2.4.2","firmware_sha256":hash(&firmware),"os_rom_sha256":hash(&os_rom),"cases":results})).unwrap()).unwrap();
    }
    assert!(results.len() >= 20);
    let failures: Vec<_> = results
        .iter()
        .filter(|r| r["report"]["status"] != "passed")
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(
        failures.is_empty(),
        "firmware cases failed: {failures:?}; see {}",
        out.display()
    );
}

#[test]
#[ignore = "requires local firmware and a fresh PRIME_FILE_COMPAT_OUT artifact directory"]
fn local_firmware_editor_modification_survives_two_reboots() {
    use prime_emulator::{
        hpprgm, keys::parse_keys, runner, script::ScriptConfig, services::UnsupportedPolicy,
        storage, trace::TraceConfig,
    };
    let out = configured("PRIME_FILE_COMPAT_OUT").join("editor-modification");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::create_dir(&out).expect("editor evidence output must be fresh");
    let profile = tempfile::tempdir().unwrap();
    let data = profile.path().join("data");
    calcverify::copy_tree(&configured("PRIME_TEST_INIT_DATA"), &data).unwrap();
    let source = "EXPORT EDITTEST()\nBEGIN\n RETURN 1;\nEND;\n";
    let input = profile.path().join("EDITTEST.ppl");
    std::fs::write(&input, source).unwrap();
    import::import(&import::ImportRequest {
        program: input,
        data_dir: data.clone(),
        name: None,
        replace: false,
        as_source: false,
    })
    .unwrap();
    let sentinel = data.join("C/DATA/ZZT_SENTINEL.hpprgm");
    let sentinel_bytes = hpprgm::build_program("EXPORT ZZT_SENTINEL() BEGIN RETURN 9; END;\n")
        .unwrap()
        .to_bytes();
    std::fs::write(&sentinel, &sentinel_bytes).unwrap();
    let expected = source.replace("RETURN 1", "RETURN 2");
    let mut evidence = Vec::new();
    for (index,keys) in [
        "SHIFT,1,TAP:26:229,DOWN,DOWN,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,BACK,2,ESC",
        "SHIFT,1,TAP:26:229,ESC",
        "SHIFT,1,TAP:26:229,ESC",
    ].into_iter().enumerate() {
        let before=storage::snapshot(&data);
        let cfg=runner::RunConfig {
            firmware_path:configured("PRIME_TEST_FIRMWARE"), os_rom_path:Some(configured("PRIME_TEST_OS_ROM")),
            data_dir:data.clone(), on_unsupported:UnsupportedPolicy::ZeroReturn,
            max_instructions:8_000_000_000, max_seconds:60.0,
            trace:TraceConfig {fs_trace:true,..Default::default()},
            script:Some(ScriptConfig {keys:parse_keys(keys).unwrap(), screenshot_dir:Some(out.join(format!("boot{index}"))), key_timeout:Duration::from_secs(8),..Default::default()}),
            storage_diff:true, ui_timers:true,..Default::default()
        };
        let run=runner::run(&cfg).unwrap();
        std::fs::write(out.join(format!("boot{index}.json")),serde_json::to_vec_pretty(&run.report).unwrap()).unwrap();
        assert_eq!(run.outcome,runner::Outcome::ScriptComplete,"{}",run.report.evidence);
        let bytes=std::fs::read(data.join("C/DATA/EDITTEST.hpprgm")).unwrap();
        let saved=hpprgm::parse(&bytes).unwrap().source().unwrap().source;
        evidence.push(serde_json::json!({"boot":index,"source":saved,"saved_sha256":sha256::hex(&bytes),"source_sha256":sha256::hex(saved.as_bytes()),"changes":storage::diff(&before,&storage::snapshot(&data))}));
        std::fs::write(out.join("capture.json"),serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        assert_eq!(saved,expected,"editor keys must actually modify the number; see {}",out.display());
        assert_eq!(std::fs::read(&sentinel).unwrap(),sentinel_bytes);
    }
}
