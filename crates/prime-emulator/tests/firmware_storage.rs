//! Optional genuine firmware persistence round-trip through the storage image.
//! This boots restored storage; it does not claim restoration of live CPU state.
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::{LiveSession, copy_profile},
    session::Command,
};
use prime_emulator::storage::image::{Kind, StorageImage};
use std::{path::Path, time::Duration};

#[test]
fn genuine_saved_storage_rehydrates_programs_and_calculator_variables() {
    let (Some(firmware), Some(initial)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
    ) else {
        eprintln!(
            "skipping genuine storage test: PRIME_TEST_FIRMWARE / PRIME_TEST_INIT_DATA not set"
        );
        return;
    };
    let mut root = match std::env::var_os("PRIME_TEST_EVIDENCE_DIR") {
        Some(parent) => {
            std::fs::create_dir_all(&parent).unwrap();
            tempfile::Builder::new()
                .prefix("firmware-storage-")
                .tempdir_in(parent)
                .unwrap()
        }
        None => tempfile::tempdir().unwrap(),
    };
    if std::env::var_os("PRIME_TEST_EVIDENCE_DIR").is_some() {
        root.disable_cleanup(true);
        eprintln!("retained storage evidence: {}", root.path().display());
    }
    let original = root.path().join("original");
    for drive in ["A", "C"] {
        copy_profile(&Path::new(&initial).join(drive), &original.join(drive)).unwrap();
    }
    let options = Options {
        firmware: Some(firmware.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        runner: Some(env!("CARGO_BIN_EXE_prime-emulator").into()),
        ..Default::default()
    };
    let cancel = Cancellation::default();
    let timeout = Duration::from_secs(90);
    let source = "EXPORT STOREDTREE() BEGIN // Ω 😀\n A:=123; L1:={7,8,9}; RETURN A; END;";
    let source_hash = ppl_engine::source::hash(source.as_bytes());
    let name = format!("ZZT_ST{:X}", std::process::id());
    let mut live = LiveSession::spawn(&options, &original, &cancel).unwrap();
    let stored = live
        .request(
            1,
            &Command::StoreProgram {
                request: 1,
                name: name.clone(),
                source: source.into(),
                expected_previous_sha256: None,
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(stored.success, "{}", stored.message);
    let result = live
        .request(
            2,
            &Command::RunStored {
                request: 2,
                name: name.clone(),
                source_sha256: source_hash.clone(),
                entry: "STOREDTREE".into(),
                arguments: String::new(),
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(result.success, "{}", result.message);
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"123"}))
    );
    live.power_down(timeout, &cancel).unwrap();
    drop(live);
    let identity =
        ppl_engine::source::hash(&std::fs::read(options.firmware.as_ref().unwrap()).unwrap());
    let archive = root.path().join("storage.bin");
    StorageImage::capture(&original, &archive, &identity).unwrap();
    let mut image = StorageImage::open(&archive, &identity).unwrap();
    let file_count = image
        .manifest()
        .entries
        .iter()
        .filter(|e| matches!(e.object, Kind::File { .. }))
        .count();
    assert!(
        file_count > 10,
        "genuine storage must include firmware-produced settings and programs"
    );
    let mut restored = image.restore(root.path()).unwrap();
    if std::env::var_os("PRIME_TEST_EVIDENCE_DIR").is_some() {
        restored.disable_cleanup(true);
    }
    let raw_program = format!("C/DATA/{name}.hpprgm");
    assert_eq!(
        std::fs::read(original.join(&raw_program)).unwrap(),
        std::fs::read(restored.path().join(&raw_program)).unwrap()
    );
    for settings in ["C/DATA/calc.hpsettings", "C/DATA/cas.hpsettings"] {
        assert_eq!(
            std::fs::read(original.join(settings)).unwrap(),
            std::fs::read(restored.path().join(settings)).unwrap()
        );
    }
    let mut live = LiveSession::spawn(&options, restored.path(), &cancel).unwrap();
    let reader = "EXPORT READTREE() BEGIN RETURN {A,L1}; END;";
    let reader_name = format!("ZZT_RD{:X}", std::process::id());
    let stored = live
        .request(
            3,
            &Command::StoreProgram {
                request: 3,
                name: reader_name.clone(),
                source: reader.into(),
                expected_previous_sha256: None,
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(stored.success, "{}", stored.message);
    let result = live
        .request(
            4,
            &Command::RunStored {
                request: 4,
                name: reader_name,
                source_sha256: ppl_engine::source::hash(reader.as_bytes()),
                entry: "READTREE".into(),
                arguments: String::new(),
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(result.success, "{}", result.message);
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"list","value":[
        {"type":"real","value":"123"}, {"type":"list","value":[
            {"type":"real","value":"7"},{"type":"real","value":"8"},{"type":"real","value":"9"}]}]}))
    );
    let result = live
        .request(
            5,
            &Command::RunStored {
                request: 5,
                name,
                source_sha256: source_hash,
                entry: "STOREDTREE".into(),
                arguments: String::new(),
            },
            timeout,
            &cancel,
        )
        .unwrap();
    assert!(
        result.success,
        "restored original program: {}",
        result.message
    );
    assert_eq!(
        result.execution.unwrap().return_value,
        Some(serde_json::json!({"type":"real","value":"123"}))
    );
    live.power_down(timeout, &cancel).unwrap();
    println!(
        "genuine-storage-rehydrated: {file_count} regular files; stored programs, A=123, L1={{7,8,9}}"
    );
}
