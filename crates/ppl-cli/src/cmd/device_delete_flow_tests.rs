//! Hardware-independent lifecycle tests. The memory transport is not physical evidence.
use super::*;
use ppl_device::{Detected, Transport};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Memory {
    objects: HashMap<(u8, String), Vec<u8>>,
    input: Vec<u8>,
    replies: VecDeque<Vec<u8>>,
    commands: Vec<u8>,
    ignore_delete: bool,
}
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}
impl Memory {
    fn reply(&mut self, msg: &[u8]) {
        for (i, part) in msg.chunks(63).enumerate() {
            let mut frame = vec![((i + i / 255) % 256) as u8];
            frame.extend(part);
            frame.resize(64, 0);
            self.replies.push_back(frame);
        }
    }
}
struct Wire(Arc<Mutex<Memory>>);
impl Transport for Wire {
    fn write(&mut self, report: &[u8]) -> ppl_device::Result<usize> {
        let mut m = self.0.lock().unwrap();
        m.input.extend_from_slice(&report[2..]);
        let complete = if matches!(
            m.input.first(),
            Some(&ppl_device::CMD_CHECK_READY) | Some(&ppl_device::CMD_GET_INFO)
        ) {
            true
        } else {
            m.input.len() >= 6
                && m.input.len()
                    >= 6 + u32::from_be_bytes(m.input[2..6].try_into().unwrap()) as usize
        };
        if complete {
            let msg = std::mem::take(&mut m.input);
            let cmd = msg[0];
            m.commands.push(cmd);
            match cmd {
                ppl_device::CMD_CHECK_READY => m.reply(&[ppl_device::READY_REPLY]),
                ppl_device::CMD_GET_INFO => {
                    let mut raw = vec![0xFA, 0x01, 0, 0, 0, 0x6C];
                    let mut body = vec![0u8; 108];
                    body[65] = 0x30;
                    body[68..76].copy_from_slice(&[0x24, 0x20, 0x01, 0x39, 0x35, 0x3B, 0, 0]);
                    body[76..86].copy_from_slice(b"V0.050.640");
                    body[92..102].copy_from_slice(b"0000000000");
                    raw.extend(body);
                    m.reply(&raw);
                }
                ppl_device::CMD_REQ_FILE => {
                    let f = ppl_device::files::decode_file_message(&msg).unwrap();
                    if let Some(data) = m.objects.get(&(f.ty.code(), f.name.clone())).cloned() {
                        m.reply(&ppl_device::files::encode_file_message(
                            ppl_device::CMD_FILE,
                            f.ty,
                            &f.name,
                            &data,
                        ));
                    }
                }
                ppl_device::CMD_FILE => {
                    let f = ppl_device::files::decode_file_message(&msg).unwrap();
                    m.objects.insert((f.ty.code(), f.name), f.data);
                }
                ppl_device::CMD_DELETE_OBJECT => {
                    let f = ppl_device::files::decode_file_message(&msg).unwrap();
                    if !m.ignore_delete {
                        m.objects.remove(&(f.ty.code(), f.name));
                    }
                }
                _ => panic!("unexpected command {cmd:02x}"),
            }
        }
        Ok(report.len())
    }
    fn read_timeout(&mut self, buf: &mut [u8], _: i32) -> ppl_device::Result<usize> {
        if let Some(frame) = self.0.lock().unwrap().replies.pop_front() {
            buf[..frame.len()].copy_from_slice(&frame);
            Ok(frame.len())
        } else {
            Ok(0)
        }
    }
}
fn fixture() -> (
    Prime,
    Arc<Mutex<Memory>>,
    Store,
    Operation,
    ppl_device::Deletion,
) {
    let m = Arc::new(Mutex::new(Memory::default()));
    m.lock().unwrap().objects.insert(
        (6, "ZZT_CTRL".into()),
        utf16("EXPORT ZZT_CTRL() BEGIN END;"),
    );
    m.lock().unwrap().objects.insert(
        (6, "ZZT_TEST".into()),
        utf16("EXPORT ZZT_TEST() BEGIN RETURN 1; END;"),
    );
    let detected = Detected {
        vendor_id: 0x03f0,
        product_id: 0x1541,
        model: "mock G1",
        path: "mock".into(),
        manufacturer: None,
        product: None,
        serial: None,
        release: 0,
        interface: 0,
    };
    let mut p = Prime::with_transport(detected, Box::new(Wire(m.clone())));
    let control = ControlObject::parse(ppl_device::DEFAULT_CONTROL).unwrap();
    let d = ppl_device::prepare_deletion(&mut p, &[DeleteTarget::program("ZZT_TEST")], &control)
        .unwrap();
    let store = Store::new(&std::env::temp_dir().join(ppl_device::fresh_test_name()));
    let op = Operation {
        schema_version: 1,
        id: d.helper.clone(),
        device_serial_id: ppl_compat::serial_fingerprint("0000000000"),
        usb_pid: 0x1541,
        firmware_build: Some(15157),
        boot_version: Some("V0.050.640".into()),
        control: control.to_string(),
        state: "prepared".into(),
        mechanism: "ppl_helper".into(),
        created_utc: "test".into(),
        updated_utc: "test".into(),
        targets: d
            .backups
            .iter()
            .map(|b| Target {
                name: b.name.clone(),
                ty: b.ty.label().into(),
                sha256: ppl_compat::sha256_hex(&b.data),
                bytes: b.data.len(),
                status: "not_submitted".into(),
            })
            .collect(),
        helper: None,
        helper_note: None,
        helper_source: None,
        helper_uploaded: false,
        completion: None,
        artifacts: Vec::new(),
        cleanup_state: "not_required".into(),
        runs_dir: store.root.join("runs"),
        error: None,
        class: "test".into(),
        authorization: None,
        requested_utc: None,
        submission: Vec::new(),
    };
    store.create(&op, &d.backups).unwrap();
    m.lock().unwrap().commands.clear();
    (p, m, store, op, d)
}
#[test]
fn prepare_only_uploads_one_verified_helper_without_deletion_or_keys() {
    let (mut p, m, store, mut op, d) = fixture();
    manual(
        &mut p,
        &d,
        &store,
        &mut op,
        &DeleteOptions::default(),
        true,
        true,
    )
    .unwrap();
    assert_eq!(op.state, "awaiting_run");
    assert!(op.helper_uploaded);
    let m = m.lock().unwrap();
    assert!(m.objects.contains_key(&(6, "ZZT_TEST".into())));
    assert_eq!(
        m.commands
            .iter()
            .filter(|&&c| c == ppl_device::CMD_FILE)
            .count(),
        1
    );
    assert!(!m.commands.contains(&ppl_device::CMD_DELETE_OBJECT));
    assert!(!m.commands.contains(&ppl_device::CMD_KEY));
    assert!(!m.objects.contains_key(&(5, d.helper_note)));
    drop(m);
    std::fs::remove_dir_all(store.root).unwrap();
}
#[test]
fn journal_failure_prevents_helper_upload_and_preserves_backups() {
    let (mut p, m, store, mut op, d) = fixture();
    let journal = store.root.join("journal.jsonl");
    std::fs::remove_file(&journal).unwrap();
    std::fs::create_dir(&journal).unwrap();
    assert!(
        manual(
            &mut p,
            &d,
            &store,
            &mut op,
            &DeleteOptions::default(),
            true,
            true
        )
        .is_err()
    );
    assert!(m.lock().unwrap().commands.is_empty());
    assert_eq!(store.backups(&op).unwrap()[0].data, d.backups[0].data);
    std::fs::remove_dir_all(store.root).unwrap();
}
fn archived_fixture() -> (Prime, Arc<Mutex<Memory>>, Store, Operation) {
    let (p, m, store, mut op, d) = fixture();
    op.targets[0].status = "confirmed_deleted".into();
    op.helper = Some(d.helper.clone());
    op.helper_source = Some(d.source.clone());
    op.helper_note = Some(d.helper_note.clone());
    op.completion = Some("done".into());
    op.cleanup_state = "pending".into();
    op.state = "verified".into();
    let text = format!("@@PPLDELETE|{}|done", d.helper);
    for (ty, name, text) in [
        (FileType::Program, d.helper, d.source),
        (FileType::Note, d.helper_note, text),
    ] {
        let data = utf16(&text);
        m.lock()
            .unwrap()
            .objects
            .insert((ty.code(), name.clone()), data.clone());
        op.artifacts.push(records::Artifact {
            name,
            ty: ty.label().into(),
            sha256: ppl_compat::sha256_hex(&data),
            text,
            status: "present".into(),
        });
    }
    m.lock().unwrap().objects.remove(&(6, "ZZT_TEST".into()));
    m.lock()
        .unwrap()
        .objects
        .insert((5, "ZZT_SENTINEL".into()), utf16("keep me"));
    store.save(&op).unwrap();
    (p, m, store, op)
}
#[test]
fn cleanup_is_finite_archived_and_never_creates_more_helpers_or_notes() {
    let (mut p, m, store, mut op) = archived_fixture();
    cleanup(&mut p, &store, &mut op).unwrap();
    assert_eq!(op.cleanup_state, "complete");
    assert!(op.confirmed());
    let saved = store.load(&op.id).unwrap();
    assert_eq!(saved.cleanup_state, "complete");
    assert_eq!(saved.artifacts.len(), 2);
    assert!(
        saved
            .artifacts
            .iter()
            .all(|a| a.status == "confirmed_deleted")
    );
    let m = m.lock().unwrap();
    assert_eq!(
        m.commands
            .iter()
            .filter(|&&c| c == ppl_device::CMD_DELETE_OBJECT)
            .count(),
        2
    );
    assert!(!m.commands.contains(&ppl_device::CMD_FILE));
    assert!(!m.commands.contains(&ppl_device::CMD_KEY));
    assert_eq!(
        m.objects.get(&(5, "ZZT_SENTINEL".into())),
        Some(&utf16("keep me"))
    );
    drop(m);
    for a in &op.artifacts {
        assert!(
            store
                .directory(&op.id)
                .unwrap()
                .join("cleanup-backups")
                .join(format!("{}-{}.bin", a.ty, a.name))
                .exists()
        );
    }
    reconcile(&mut p, &store, &mut op).unwrap();
    assert_eq!(op.state, "completed");
    std::fs::remove_dir_all(store.root).unwrap();
}
#[test]
fn uncertain_cleanup_is_reconciled_but_never_resubmitted() {
    let (mut p, m, store, mut op) = archived_fixture();
    m.lock().unwrap().ignore_delete = true;
    cleanup(&mut p, &store, &mut op).unwrap();
    assert_eq!(op.cleanup_state, "unresolved");
    let mut recovered = store.load(&op.id).unwrap();
    assert!(cleanup(&mut p, &store, &mut recovered).is_err());
    assert_eq!(
        m.lock()
            .unwrap()
            .commands
            .iter()
            .filter(|&&c| c == ppl_device::CMD_DELETE_OBJECT)
            .count(),
        2
    );
    // A subsequent read-only reconciliation may establish delayed removal.
    m.lock()
        .unwrap()
        .objects
        .retain(|(_, name), _| name == "ZZT_CTRL" || name == "ZZT_SENTINEL");
    cleanup(&mut p, &store, &mut recovered).unwrap();
    assert_eq!(recovered.cleanup_state, "complete");
    std::fs::remove_dir_all(store.root).unwrap();
}
#[test]
fn cleanup_refuses_unresolved_operations_or_changed_artifacts_without_writes() {
    let (mut p, m, store, mut op) = archived_fixture();
    op.targets[0].status = "unknown".into();
    assert!(cleanup(&mut p, &store, &mut op).is_err());
    assert!(m.lock().unwrap().commands.is_empty());
    op.targets[0].status = "confirmed_deleted".into();
    let a = &op.artifacts[0];
    m.lock()
        .unwrap()
        .objects
        .insert((6, a.name.clone()), utf16("changed user content"));
    assert!(cleanup(&mut p, &store, &mut op).is_err());
    assert!(
        !m.lock()
            .unwrap()
            .commands
            .contains(&ppl_device::CMD_DELETE_OBJECT)
    );
    std::fs::remove_dir_all(store.root).unwrap();
}

#[test]
fn confirmed_legacy_evidence_requires_device_backup_and_terminal_journal_proof() {
    let (mut p, m, old_store, old_op, d) = fixture();
    let store = Store::new(&std::env::temp_dir().join(ppl_device::fresh_test_name()));
    std::fs::create_dir_all(store.root.join("backups")).unwrap();
    let data = &d.backups[0].data;
    std::fs::write(store.root.join("backups/ZZT_TEST.hpprgm"), data).unwrap();
    let info = ppl_device::CalcInfo {
        build: Some(15157),
        version: Some("V0.050.640".into()),
        serial: Some("dummy-test-serial".into()),
        raw: Vec::new(),
    };
    let prepared = json!({"id":d.helper,"phase":"prepared","device_serial_id":info.serial_fingerprint(),"targets":[{"name":"ZZT_TEST","type":"program","backup_sha256":ppl_compat::sha256_hex(data)}]});
    let verified = json!({"id":d.helper,"phase":"verify","device_serial_id":info.serial_fingerprint(),"targets":[{"object":"ZZT_TEST","type":"program","deletion_confirmed":true}]});
    let journal = store.root.join("journal.jsonl");
    std::fs::write(&journal, format!("{prepared}\n{verified}\n")).unwrap();
    assert!(
        crate::cmd::device_delete_legacy::import(
            &store,
            &d.helper,
            &mut p,
            "ffffffffffffffff",
            &ppl_device::CalcInfo {
                serial: Some("another-serial".into()),
                ..info.clone()
            }
        )
        .is_err()
    );
    assert!(m.lock().unwrap().commands.is_empty());
    m.lock().unwrap().objects.remove(&(6, "ZZT_TEST".into()));
    m.lock()
        .unwrap()
        .objects
        .insert((5, d.helper_note.clone()), utf16("done"));
    let mut op = crate::cmd::device_delete_legacy::import(
        &store,
        &d.helper,
        &mut p,
        &old_op.device_serial_id,
        &info,
    )
    .unwrap();
    assert!(op.confirmed());
    assert_eq!(op.artifacts.len(), 1);
    assert_eq!(store.backups(&op).unwrap()[0].data, *data);
    cleanup(&mut p, &store, &mut op).unwrap();
    assert_eq!(op.cleanup_state, "complete");
    let commands = &m.lock().unwrap().commands;
    assert_eq!(
        commands
            .iter()
            .filter(|&&c| c == ppl_device::CMD_DELETE_OBJECT)
            .count(),
        1
    );
    assert!(!commands.contains(&ppl_device::CMD_FILE));
    std::fs::remove_dir_all(store.root).unwrap();
    std::fs::remove_dir_all(old_store.root).unwrap();
}

#[test]
fn direct_delete_profile_does_not_inherit_support_on_unknown_firmware_or_g2() {
    let (mut p, _, store, _, _) = fixture();
    let mut info = ppl_device::CalcInfo {
        build: Some(15157),
        version: Some("V0.050.640".into()),
        ..Default::default()
    };
    assert!(direct_profile(&p, &info));
    info.build = Some(15158);
    assert!(!direct_profile(&p, &info));
    info.build = Some(15157);
    p.detected.product_id = 0x2441;
    assert!(!direct_profile(&p, &info));
    std::fs::remove_dir_all(store.root).unwrap();
}

// ---- ordinary-object deletion against the memory transport (not hardware) ----

const SERIAL: &str = "0123456789abcdef";

fn ordinary_fixture() -> (Prime, Arc<Mutex<Memory>>, PathBuf) {
    let m = Arc::new(Mutex::new(Memory::default()));
    {
        let mut g = m.lock().unwrap();
        g.objects.insert(
            (6, "ZZT_CTRL".into()),
            utf16("EXPORT ZZT_CTRL() BEGIN END;"),
        );
        g.objects.insert(
            (6, "FIREWORKS".into()),
            utf16("EXPORT FIREWORKS() BEGIN END;"),
        );
        g.objects
            .insert((6, "HELLO".into()), utf16("EXPORT HELLO() BEGIN END;"));
        g.objects.insert((5, "FIREWORKS".into()), utf16("a note"));
        g.objects
            .insert((0, "calc.settings".into()), utf16("settings"));
    }
    let detected = Detected {
        vendor_id: 0x03f0,
        product_id: 0x1541,
        model: "mock G1",
        path: "mock".into(),
        manufacturer: None,
        product: None,
        serial: None,
        release: 0,
        interface: 0,
    };
    let p = Prime::with_transport(detected, Box::new(Wire(m.clone())));
    (
        p,
        m,
        std::env::temp_dir().join(ppl_device::fresh_test_name()),
    )
}

fn info() -> ppl_device::CalcInfo {
    ppl_device::CalcInfo::parse(&{
        let mut raw = vec![0xFA, 0x01, 0, 0, 0, 0x6C];
        let mut body = vec![0u8; 108];
        body[65] = 0x30;
        body[68..76].copy_from_slice(&[0x24, 0x20, 0x01, 0x39, 0x35, 0x3B, 0, 0]);
        body[76..86].copy_from_slice(b"V0.050.640");
        body[92..102].copy_from_slice(b"0000000000");
        raw.extend(body);
        raw
    })
}

#[allow(clippy::too_many_arguments)]
fn run_with(
    p: &mut Prime,
    ty: FileType,
    name: &str,
    dir: &Path,
    auth: user::Authorization,
    settings_verified: bool,
    typed: &str,
    progress: &mut Vec<u8>,
) -> Result<(Value, u8), DeviceError> {
    let t = [DeleteTarget {
        ty,
        name: name.into(),
    }];
    user::perform_user(
        p,
        SERIAL,
        &info(),
        &t,
        None,
        settings_verified,
        auth,
        (&dir.join("runs"), &dir.join("store"), None),
        &mut typed.as_bytes(),
        &mut Vec::new(),
        &mut user::Progress(Some(progress)),
    )
}

fn run_user(
    p: &mut Prime,
    ty: FileType,
    name: &str,
    dir: &Path,
    interactive: bool,
    typed: &str,
) -> Result<(Value, u8), DeviceError> {
    run_with(
        p,
        ty,
        name,
        dir,
        user::Authorization::Typed { interactive },
        true,
        typed,
        &mut Vec::new(),
    )
}

fn eb_sent(m: &Arc<Mutex<Memory>>) -> usize {
    m.lock()
        .unwrap()
        .commands
        .iter()
        .filter(|c| **c == ppl_device::CMD_DELETE_OBJECT)
        .count()
}

#[test]
fn ordinary_program_is_backed_up_confirmed_deleted_and_neighbours_survive() {
    let (mut p, m, dir) = ordinary_fixture();
    let (v, code) = run_user(
        &mut p,
        FileType::Program,
        "FIREWORKS",
        &dir,
        true,
        "FIREWORKS\n",
    )
    .unwrap();
    assert_eq!(code, exit::OK, "{v}");
    assert_eq!(v["result"], "confirmed_deleted");
    assert_eq!(v["authorization"], "exact_typed_confirmation");
    assert_eq!(v["object_class"], "user");
    assert_eq!(eb_sent(&m), 1);
    let g = m.lock().unwrap();
    assert!(!g.objects.contains_key(&(6, "FIREWORKS".into())));
    // Same-named note, other programs and the control are untouched.
    for k in [(5, "FIREWORKS"), (6, "HELLO"), (6, "ZZT_CTRL")] {
        assert!(g.objects.contains_key(&(k.0, k.1.to_string())), "{k:?}");
    }
    drop(g);
    // The raw backup is on disk, bound to the exact content.
    let store = Store::new(&dir.join("store"));
    let op = store.load(v["operation_id"].as_str().unwrap()).unwrap();
    assert_eq!(op.class, "user");
    assert_eq!(
        op.authorization.as_deref(),
        Some("exact_typed_confirmation")
    );
    assert_eq!(
        store.backups(&op).unwrap()[0].data,
        utf16("EXPORT FIREWORKS() BEGIN END;")
    );
    assert!(!dir.join("runs").exists(), "no ownership receipts involved");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wrong_confirmation_or_non_interactive_session_sends_nothing_but_keeps_the_backup() {
    for (interactive, typed) in [(true, "fireworks\n"), (true, "\n"), (false, "FIREWORKS\n")] {
        let (mut p, m, dir) = ordinary_fixture();
        let (v, code) = run_user(
            &mut p,
            FileType::Program,
            "FIREWORKS",
            &dir,
            interactive,
            typed,
        )
        .unwrap();
        assert_eq!(code, exit::IO);
        assert_eq!(v["result"], "authorization_refused");
        assert_eq!(v["deletion_confirmed"], false);
        assert_eq!(eb_sent(&m), 0);
        assert!(
            m.lock()
                .unwrap()
                .objects
                .contains_key(&(6, "FIREWORKS".into()))
        );
        let store = Store::new(&dir.join("store"));
        let op = store.load(v["operation_id"].as_str().unwrap()).unwrap();
        assert_eq!(op.state, "not_submitted");
        assert_eq!(store.backups(&op).unwrap().len(), 1);
        // A refused authorization does not block a later attempt.
        let t = [DeleteTarget::program("FIREWORKS")];
        store.reject_pending(SERIAL, &t).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_program_is_never_deleted_as_a_note_and_missing_objects_send_nothing() {
    let (mut p, m, dir) = ordinary_fixture();
    m.lock().unwrap().objects.remove(&(5, "FIREWORKS".into()));
    assert!(
        run_user(
            &mut p,
            FileType::Note,
            "FIREWORKS",
            &dir,
            true,
            "FIREWORKS\n"
        )
        .is_err()
    );
    assert!(run_user(&mut p, FileType::Program, "NOPE", &dir, true, "NOPE\n").is_err());
    assert_eq!(eb_sent(&m), 0);
    assert!(
        m.lock()
            .unwrap()
            .objects
            .contains_key(&(6, "FIREWORKS".into()))
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn ignored_delete_is_reported_still_present_and_is_never_resent() {
    let (mut p, m, dir) = ordinary_fixture();
    m.lock().unwrap().ignore_delete = true;
    let (v, code) = run_user(
        &mut p,
        FileType::Program,
        "FIREWORKS",
        &dir,
        true,
        "FIREWORKS\n",
    )
    .unwrap();
    assert_eq!(code, exit::IO);
    assert_eq!(v["result"], "still_present");
    assert_eq!(eb_sent(&m), 1);
    let store = Store::new(&dir.join("store"));
    let op = store.load(v["operation_id"].as_str().unwrap()).unwrap();
    assert_eq!(op.state, "unresolved");
    // The unresolved operation blocks a second attempt on the same target.
    assert!(
        store
            .reject_pending(SERIAL, &[DeleteTarget::program("FIREWORKS")])
            .is_err()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unverified_device_profile_is_refused_before_any_backup_or_traffic() {
    let (mut p, m, dir) = ordinary_fixture();
    p.detected.product_id = 0x2441;
    assert!(
        run_user(
            &mut p,
            FileType::Program,
            "FIREWORKS",
            &dir,
            true,
            "FIREWORKS\n"
        )
        .is_err()
    );
    assert!(!dir.join("store").exists());
    assert_eq!(eb_sent(&m), 0);
}

fn without_control(m: &Arc<Mutex<Memory>>) {
    m.lock().unwrap().objects.remove(&(6, "ZZT_CTRL".into()));
}

#[test]
fn yes_flag_deletes_one_program_with_no_stdin_receipt_or_control_program() {
    let (mut p, m, dir) = ordinary_fixture();
    without_control(&m);
    let mut out = Vec::new();
    let (v, code) = run_with(
        &mut p,
        FileType::Program,
        "FIREWORKS",
        &dir,
        user::Authorization::Yes,
        true,
        "", // stdin is empty: it must never be read
        &mut out,
    )
    .unwrap();
    assert_eq!(code, exit::OK, "{v}");
    assert_eq!(v["result"], "confirmed_deleted");
    assert_eq!(v["authorization"], "explicit_yes_flag");
    assert_eq!(v["liveness_anchor"], "settings:calc.settings");
    assert_eq!(v["submission_state"], "submitted");
    assert_eq!(v["verification"], "absence_confirmed");
    assert_eq!(eb_sent(&m), 1);
    {
        let g = m.lock().unwrap();
        assert!(!g.objects.contains_key(&(6, "FIREWORKS".into())));
        for k in [(5, "FIREWORKS"), (6, "HELLO"), (0, "calc.settings")] {
            assert!(g.objects.contains_key(&(k.0, k.1.to_string())), "{k:?}");
        }
    }
    // Backup, metadata and authorization are on disk, bound to the object.
    let store = Store::new(&dir.join("store"));
    let id = v["operation_id"].as_str().unwrap();
    let opdir = store.directory(id).unwrap();
    let meta: Value =
        serde_json::from_slice(&std::fs::read(opdir.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(meta["object"]["name"], "FIREWORKS");
    assert_eq!(meta["device"]["fingerprint"], SERIAL);
    assert_eq!(meta["object"]["sha256"], v["backup_sha256"]);
    let auth: Value =
        serde_json::from_slice(&std::fs::read(opdir.join("authorization.json")).unwrap()).unwrap();
    assert_eq!(auth["authorization"], "explicit_yes_flag");
    assert!(!dir.join("runs").exists());
    // Progress lines appear in order and only for completed steps.
    let text = String::from_utf8(out).unwrap();
    let order = [
        "Device connected",
        "Liveness anchor verified",
        "Program found",
        "Program contents verified",
        "Local backup created",
        "Deletion authorized",
        "Native USB delete submitted",
        "Independent absence verified",
        "Result: FIREWORKS deleted successfully.",
    ];
    let mut at = 0;
    for needle in order {
        at += text[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing/out of order:\n{text}"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn yes_flag_also_covers_zzt_names_and_unknown_programs_report_not_found() {
    let (mut p, m, dir) = ordinary_fixture();
    m.lock().unwrap().objects.insert(
        (6, "ZZT_TEST".into()),
        utf16("EXPORT ZZT_TEST() BEGIN END;"),
    );
    let (v, code) = run_with(
        &mut p,
        FileType::Program,
        "ZZT_TEST",
        &dir,
        user::Authorization::Yes,
        true,
        "",
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        (code, v["result"].as_str()),
        (exit::OK, Some("confirmed_deleted"))
    );
    assert!(
        m.lock()
            .unwrap()
            .objects
            .contains_key(&(6, "ZZT_CTRL".into()))
    );

    let mut out = Vec::new();
    let err = run_with(
        &mut p,
        FileType::Program,
        "NOPE",
        &dir,
        user::Authorization::Yes,
        true,
        "",
        &mut out,
    )
    .unwrap_err();
    assert!(matches!(err, DeviceError::Absent(_)), "{err}");
    assert_eq!(user::failure_result(&err), "not_found");
    assert_eq!(eb_sent(&m), 1, "only the ZZT_TEST delete was sent");
    let text = String::from_utf8(out).unwrap();
    assert!(
        !text.contains("found\n") || !text.contains("Program found"),
        "{text}"
    );
    assert!(
        !text.contains("Local backup") && !text.contains("Native USB"),
        "{text}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_any_f8_anchor_nothing_is_backed_up_or_sent() {
    // Settings anchor not evidenced and no control program installed.
    let (mut p, m, dir) = ordinary_fixture();
    without_control(&m);
    let err = run_with(
        &mut p,
        FileType::Program,
        "FIREWORKS",
        &dir,
        user::Authorization::Yes,
        false,
        "",
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(user::failure_result(&err), "unsupported_device", "{err}");
    // Settings object missing as well.
    let (mut p2, m2, dir2) = ordinary_fixture();
    without_control(&m2);
    m2.lock()
        .unwrap()
        .objects
        .remove(&(0, "calc.settings".into()));
    let err = run_with(
        &mut p2,
        FileType::Program,
        "FIREWORKS",
        &dir2,
        user::Authorization::Yes,
        true,
        "",
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(user::failure_result(&err), "unsupported_device", "{err}");
    for (mm, d) in [(&m, &dir), (&m2, &dir2)] {
        assert_eq!(eb_sent(mm), 0);
        assert!(!d.join("store").exists());
        assert!(
            mm.lock()
                .unwrap()
                .objects
                .contains_key(&(6, "FIREWORKS".into()))
        );
    }
}

#[test]
fn yes_flag_classification_is_exactly_one_program() {
    let c = ControlObject::parse(ppl_device::DEFAULT_CONTROL).unwrap();
    let one = vec!["ZZT_TEST".to_string()];
    let (policy, t) = classify(FileType::Program, &one, &c, true).unwrap();
    assert_eq!((policy, t.len()), (ppl_device::NamePolicy::User, 1));
    for bad in [
        vec![],
        vec!["A".to_string(), "B".to_string()],
        vec!["../x".to_string()],
        vec!["ZZT_CTRL".to_string()],
    ] {
        assert!(
            classify(FileType::Program, &bad, &c, true).is_err(),
            "{bad:?}"
        );
    }
    // Without --yes a ZZT name keeps the receipt/test path.
    assert_eq!(
        classify(FileType::Program, &one, &c, false).unwrap().0,
        ppl_device::NamePolicy::Test
    );
}
