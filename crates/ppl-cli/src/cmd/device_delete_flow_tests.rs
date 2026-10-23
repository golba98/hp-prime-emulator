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
        let complete = if m.input.first() == Some(&ppl_device::CMD_CHECK_READY) {
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
        device_serial_id: "0123456789abcdef".into(),
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
