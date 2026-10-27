//! In-memory protocol tests. These never enumerate HID or mutate hardware.
use ppl_device::{
    Detected, FileType, Prime, Transport,
    files::{decode_file_message, encode_file_message},
};
use ppl_programs::{
    DeviceIdentity, ProgramImage, Source, create_new, delete_store::Store, deletion, replacement,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct Memory {
    objects: HashMap<(u8, String), Vec<u8>>,
    rx: Vec<u8>,
    replies: VecDeque<Vec<u8>>,
    sent: Vec<(u8, String)>,
    ignore_delete: bool,
    drop_upload: bool,
    mutate_upload: bool,
    disconnect: Option<u8>,
    serial: Option<String>,
}
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}
impl Memory {
    fn reply(&mut self, message: &[u8]) {
        for (i, part) in message.chunks(63).enumerate() {
            let mut r = vec![(i % 255) as u8];
            r.extend(part);
            r.resize(64, 0xab);
            self.replies.push_back(r);
        }
    }
}
struct Wire(Arc<Mutex<Memory>>);
impl Transport for Wire {
    fn write(&mut self, report: &[u8]) -> ppl_device::Result<usize> {
        let mut m = self.0.lock().unwrap();
        m.rx.extend(&report[2..]);
        if m.rx.len() < 6 && !matches!(m.rx[0], 0xfa | 0xff) {
            return Ok(report.len());
        }
        if !matches!(m.rx[0], 0xfa | 0xff)
            && m.rx.len() < 6 + u32::from_be_bytes(m.rx[2..6].try_into().unwrap()) as usize
        {
            return Ok(report.len());
        }
        let message = std::mem::take(&mut m.rx);
        let cmd = message[0];
        if m.disconnect == Some(cmd) {
            return Err(ppl_device::DeviceError::Hid(
                "mock disconnected during submission".into(),
            ));
        }
        match cmd {
            0xff => m.reply(&[0x59]),
            0xfa => {
                let mut r = vec![0xfa, 1, 0, 0, 0, 108];
                let mut b = vec![0; 108];
                b[72..74].copy_from_slice(&15157u16.to_le_bytes());
                b[76..86].copy_from_slice(b"V0.050.640");
                b[92..102].copy_from_slice(m.serial.as_deref().unwrap_or("1234567890").as_bytes());
                r.extend(b);
                m.reply(&r);
            }
            0xf8 => {
                let f = decode_file_message(&message).unwrap();
                m.sent.push((cmd, f.name.clone()));
                let data = if f.ty == FileType::Settings {
                    Some(vec![])
                } else {
                    m.objects.get(&(f.ty.code(), f.name.clone())).cloned()
                };
                if let Some(data) = data {
                    m.reply(&encode_file_message(0xf7, f.ty, &f.name, &data));
                }
            }
            0xf7 => {
                let f = decode_file_message(&message).unwrap();
                m.sent.push((cmd, f.name.clone()));
                if !m.drop_upload {
                    let mut data = f.data;
                    if m.mutate_upload {
                        data.extend(utf16("changed"));
                    }
                    m.objects.insert((f.ty.code(), f.name), data);
                }
            }
            0xeb => {
                let f = decode_file_message(&message).unwrap();
                m.sent.push((cmd, f.name.clone()));
                if !m.ignore_delete {
                    m.objects.remove(&(f.ty.code(), f.name));
                }
            }
            _ => panic!("unapproved opcode {cmd:x}"),
        }
        Ok(report.len())
    }
    fn read_timeout(&mut self, buf: &mut [u8], _: i32) -> ppl_device::Result<usize> {
        match self.0.lock().unwrap().replies.pop_front() {
            Some(r) => {
                buf[..r.len()].copy_from_slice(&r);
                Ok(r.len())
            }
            None => Ok(0),
        }
    }
}
fn fixture() -> (Prime, Arc<Mutex<Memory>>, tempfile::TempDir, Store) {
    let memory = Arc::new(Mutex::new(Memory::default()));
    let device = Detected {
        vendor_id: 0x03f0,
        product_id: 0x1541,
        model: "offline mock",
        path: "mock".into(),
        manufacturer: None,
        product: None,
        serial: None,
        release: 0,
        interface: 0,
    };
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    (
        Prime::with_transport(device, Box::new(Wire(memory.clone()))),
        memory,
        dir,
        store,
    )
}
fn image(text: &str) -> ProgramImage {
    ProgramImage::new("ZZT_TEST", Source::text(text, "ZZT_TEST").unwrap()).unwrap()
}
const OLD: &str = "EXPORT DEMO()\nBEGIN\nRETURN 1;\nEND;\n";
const NEW: &str = "EXPORT DEMO()\nBEGIN\nRETURN 2;\nEND;\n";
fn put(memory: &Arc<Mutex<Memory>>, name: &str, text: &str) {
    memory
        .lock()
        .unwrap()
        .objects
        .insert((6, name.into()), utf16(text));
}
#[test]
fn small_upload_verified_and_existing_destination_preserved() {
    let (mut p, m, _dir, s) = fixture();
    let r = create_new(&mut p, &image(NEW), &s).unwrap();
    assert_eq!(r.state, "upload_verified");
    assert!(create_new(&mut p, &image(OLD), &s).is_err());
    assert_eq!(
        m.lock().unwrap().objects[&(6, "ZZT_TEST".into())],
        utf16(NEW)
    );
    assert!(!m.lock().unwrap().sent.iter().any(|(c, _)| *c == 0xeb));
}
#[test]
fn indeterminate_upload_survives_process_restart_and_blocks_retries() {
    let (mut p, m, dir, s) = fixture();
    m.lock().unwrap().drop_upload = true;
    assert_eq!(
        create_new(&mut p, &image(NEW), &s).unwrap().state,
        "indeterminate"
    );
    let reopened = Store::new(dir.path());
    let before = m.lock().unwrap().sent.len();
    assert!(
        create_new(&mut p, &image(NEW), &reopened)
            .unwrap_err()
            .to_string()
            .contains("unresolved upload")
    );
    assert_eq!(before, m.lock().unwrap().sent.len());
}
#[test]
fn replacement_requires_review_and_changed_destination_never_inherits_authorization() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "ZZT_TEST", OLD);
    let plan = replacement::prepare(&mut p, &image(NEW), &s).unwrap();
    assert!(
        !m.lock()
            .unwrap()
            .sent
            .iter()
            .any(|(c, _)| *c == 0xeb || *c == 0xf7)
    );
    assert!(replacement::apply(&mut p, &s, &plan.operation_id, false).is_err());
    put(&m, "ZZT_TEST", "changed after backup");
    assert!(replacement::apply(&mut p, &s, &plan.operation_id, true).is_err());
    assert!(
        !m.lock()
            .unwrap()
            .sent
            .iter()
            .any(|(c, _)| *c == 0xeb || *c == 0xf7)
    );
}
#[test]
fn replacement_sends_one_delete_and_one_create_and_keeps_sentinel_and_backup() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "ZZT_TEST", OLD);
    put(&m, "SENTINEL", "keep");
    let plan = replacement::prepare(&mut p, &image(NEW), &s).unwrap();
    let result = replacement::apply(&mut p, &s, &plan.operation_id, true).unwrap();
    assert_eq!(result["phase"], "completed");
    let m = m.lock().unwrap();
    assert_eq!(m.sent.iter().filter(|(c, _)| *c == 0xeb).count(), 1);
    assert_eq!(m.sent.iter().filter(|(c, _)| *c == 0xf7).count(), 1);
    assert_eq!(m.objects[&(6, "SENTINEL".into())], utf16("keep"));
    drop(m);
    assert_eq!(
        s.backups(&s.load(&plan.operation_id).unwrap()).unwrap()[0].data,
        utf16(OLD)
    );
    assert!(replacement::apply(&mut p, &s, &plan.operation_id, true).is_err());
}
#[test]
fn refused_large_replacement_sends_no_requests_or_mutations() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "ZZT_TEST", OLD);
    let image = ProgramImage::new(
        "FIREWORKS",
        Source::decode(
            include_bytes!("../../../examples/FIREWORKS.hpprgm").to_vec(),
            None,
            "FIREWORKS".into(),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(replacement::prepare(&mut p, &image, &s).is_err());
    assert!(m.lock().unwrap().sent.is_empty());
}
#[test]
fn ignored_deletion_stops_replacement_before_upload() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "ZZT_TEST", OLD);
    m.lock().unwrap().ignore_delete = true;
    let plan = replacement::prepare(&mut p, &image(NEW), &s).unwrap();
    assert!(replacement::apply(&mut p, &s, &plan.operation_id, true).is_err());
    assert!(!m.lock().unwrap().sent.iter().any(|(c, _)| *c == 0xf7));
    assert_eq!(s.load(&plan.operation_id).unwrap().state, "unresolved");
}
#[test]
fn explicit_delete_is_bound_to_content_and_device() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "ZZT_TEST", OLD);
    put(&m, "SENTINEL", "keep");
    let op = deletion::prepare(&mut p, "ZZT_TEST", &s).unwrap();
    assert!(deletion::apply(&mut p, &s, &op.id, "wrong hash").is_err());
    m.lock().unwrap().serial = Some("0987654321".into());
    assert!(deletion::apply(&mut p, &s, &op.id, &op.targets[0].sha256).is_err());
    assert!(!m.lock().unwrap().sent.iter().any(|(c, _)| *c == 0xeb));
}
#[test]
fn incorrect_readback_is_not_verified_and_disconnect_is_not_retried() {
    let (mut p, m, _dir, s) = fixture();
    m.lock().unwrap().mutate_upload = true;
    assert_ne!(
        create_new(&mut p, &image(NEW), &s).unwrap().state,
        "upload_verified"
    );
    assert_eq!(
        m.lock()
            .unwrap()
            .sent
            .iter()
            .filter(|(c, _)| *c == 0xf7)
            .count(),
        1
    );
    let (mut p, m, _dir, s) = fixture();
    m.lock().unwrap().disconnect = Some(0xf7);
    assert!(create_new(&mut p, &image(NEW), &s).is_err());
    assert!(
        std::fs::read_dir(s.root.join("uploads")).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .and_then(|s| s.to_str())
            == Some("json"))
    );
}
#[test]
fn identity_queries_keep_queued_file_evidence() {
    let (mut p, m, _dir, _s) = fixture();
    m.lock().unwrap().reply(&encode_file_message(
        0xf7,
        FileType::Program,
        "FIREWORKS",
        &utf16(OLD),
    ));
    DeviceIdentity::read(&mut p).unwrap();
    assert!(
        p.log
            .iter()
            .any(|e| e.drained && e.bytes.get(1) == Some(&0xf7))
    );
}

#[test]
fn read_only_upload_recovery_proves_absence_and_retains_history() {
    let (mut p, m, _dir, s) = fixture();
    m.lock().unwrap().drop_upload = true;
    let result = create_new(&mut p, &image(NEW), &s).unwrap();
    let id = result.operation_id.unwrap();
    let before = m
        .lock()
        .unwrap()
        .sent
        .iter()
        .filter(|(c, _)| *c == 0xf7 || *c == 0xeb)
        .count();
    let result = ppl_programs::recovery::upload(&mut p, &s, &id).unwrap();
    assert_eq!(result["state"], "verified_absent");
    assert_eq!(
        m.lock()
            .unwrap()
            .sent
            .iter()
            .filter(|(c, _)| *c == 0xf7 || *c == 0xeb)
            .count(),
        before
    );
    assert!(s.root.join("uploads/history").join(id).is_dir());
    m.lock().unwrap().drop_upload = false;
    assert_eq!(
        create_new(&mut p, &image(NEW), &s).unwrap().state,
        "upload_verified"
    );
}
#[test]
fn indeterminate_upload_blocks_deletion_as_well_as_upload() {
    let (mut p, m, _dir, s) = fixture();
    m.lock().unwrap().drop_upload = true;
    create_new(&mut p, &image(NEW), &s).unwrap();
    put(&m, "ZZT_TEST", OLD);
    assert!(
        deletion::prepare(&mut p, "ZZT_TEST", &s)
            .unwrap_err()
            .to_string()
            .contains("unresolved upload")
    );
    assert!(!m.lock().unwrap().sent.iter().any(|(c, _)| *c == 0xeb));
}

#[test]
fn legacy_d64210_is_an_operation_id_and_fireworks_is_the_exact_target() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "FIREWORKS", OLD);
    put(&m, "ZZT_D64210", "unrelated personal content");
    let mut operation = deletion::prepare(&mut p, "FIREWORKS", &s).unwrap();
    let backups = s.backups(&operation).unwrap();
    operation.id = "ZZT_D64210".into();
    operation.state = "unresolved".into();
    s.create(&operation, &backups).unwrap();
    let path = s.directory(&operation.id).unwrap().join("operation.json");
    let mut legacy = serde_json::to_value(&operation).unwrap();
    legacy.as_object_mut().unwrap().remove("submission");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let recovered = deletion::verify(&mut p, &s, &operation.id).unwrap();
    assert_eq!(recovered.targets[0].name, "FIREWORKS");
    assert_eq!(recovered.targets[0].status, "still_present");
    assert_eq!(recovered.state, "unresolved");
    assert!(recovered.submission.is_empty());
    assert!(
        s.directory(&operation.id)
            .unwrap()
            .join("record-v1-original.json")
            .is_file()
    );
    assert!(
        !m.lock()
            .unwrap()
            .sent
            .iter()
            .any(|(cmd, _)| matches!(*cmd, 0xeb | 0xf7))
    );
    assert_eq!(
        m.lock().unwrap().objects[&(6, "ZZT_D64210".into())],
        utf16("unrelated personal content")
    );
}

#[test]
fn unicode_and_crlf_transfer_preserves_original_encoded_source() {
    for source in [
        "EXPORT DEMO()\nBEGIN\n RETURN \"é😀\";\nEND;\n",
        "EXPORT DEMO()\r\nBEGIN\r\n RETURN 1;\r\nEND;\r\n",
    ] {
        let (mut p, m, _dir, s) = fixture();
        assert_eq!(
            create_new(&mut p, &image(source), &s).unwrap().state,
            "upload_verified"
        );
        assert_eq!(
            m.lock().unwrap().objects[&(6, "ZZT_TEST".into())],
            utf16(source)
        );
    }
}

fn legacy_attempt(
    store: &Store,
    mut op: ppl_programs::delete_store::Operation,
    first_error: &str,
) -> ppl_programs::delete_store::Operation {
    op.state = "unresolved".into();
    op.authorization = Some("explicit_yes_flag".into());
    op.error = Some(first_error.into());
    let mut legacy = serde_json::to_value(&op).unwrap();
    legacy.as_object_mut().unwrap().remove("submission");
    std::fs::write(
        store.directory(&op.id).unwrap().join("operation.json"),
        serde_json::to_vec_pretty(&legacy).unwrap(),
    )
    .unwrap();
    let mut journal = String::new();
    for phase in ["prepared", "submitting", "unresolved"] {
        journal.push_str(&serde_json::json!({"id":op.id,"phase":phase,"mechanism":"usb_delete","device_serial_id":op.device_serial_id,"targets":op.targets,"error":if phase=="unresolved"{Some(first_error)}else{None}}).to_string());
        journal.push('\n');
    }
    std::fs::write(store.root.join("journal.jsonl"), journal).unwrap();
    op
}
#[test]
fn reviewed_legacy_pre_eb_refusal_resolves_not_performed_without_deleting() {
    let (mut p, m, _dir, s) = fixture();
    put(&m, "ZZT_TEST", OLD);
    put(&m, "SENTINEL", "keep");
    let op = deletion::prepare(&mut p, "ZZT_TEST", &s).unwrap();
    let refusal = "refused: `ZZT_TEST` exists on the calculator (its reply arrived late); it will not be overwritten";
    let op = legacy_attempt(&s, op, &format!("{refusal}; verification: {refusal}"));
    assert!(
        s.reject_pending(&op.device_serial_id, &op.targets().unwrap())
            .is_err()
    );
    let result = deletion::verify(&mut p, &s, &op.id).unwrap();
    assert_eq!(result.state, "not_submitted");
    assert_eq!(result.targets[0].status, "still_present");
    assert_eq!(
        result.submission,
        vec![ppl_device::delete::SubmissionState::NotSent]
    );
    s.reject_pending(&op.device_serial_id, &op.targets().unwrap())
        .unwrap();
    assert_eq!(
        deletion::verify(&mut p, &s, &op.id).unwrap().state,
        "not_submitted"
    );
    assert!(
        s.directory(&op.id)
            .unwrap()
            .join("legacy-no-send-evidence.json")
            .is_file()
    );
    assert!(
        s.directory(&op.id)
            .unwrap()
            .join("record-v1-original.json")
            .is_file()
    );
    assert!(
        !m.lock()
            .unwrap()
            .sent
            .iter()
            .any(|(cmd, _)| matches!(*cmd, 0xeb | 0xf7 | 0xec))
    );
    assert_eq!(
        m.lock().unwrap().objects[&(6, "SENTINEL".into())],
        utf16("keep")
    );
}
#[test]
fn verification_only_and_partial_write_errors_cannot_prove_no_submission() {
    for error in [
        "verification: refused: `ZZT_TEST` exists on the calculator (its reply arrived late); it will not be overwritten",
        "HID error: short write; verification: target still present",
    ] {
        let (mut p, m, _dir, s) = fixture();
        put(&m, "ZZT_TEST", OLD);
        let op = deletion::prepare(&mut p, "ZZT_TEST", &s).unwrap();
        let op = legacy_attempt(&s, op, error);
        assert_eq!(
            deletion::verify(&mut p, &s, &op.id).unwrap().state,
            "unresolved"
        );
        assert!(
            s.reject_pending(&op.device_serial_id, &op.targets().unwrap())
                .is_err()
        );
        assert!(
            !s.directory(&op.id)
                .unwrap()
                .join("legacy-no-send-evidence.json")
                .exists()
        );
    }
}
#[test]
fn changed_content_or_a_second_submission_keeps_legacy_attempt_blocked() {
    for second_attempt in [false, true] {
        let (mut p, m, _dir, s) = fixture();
        put(&m, "ZZT_TEST", OLD);
        let op = deletion::prepare(&mut p, "ZZT_TEST", &s).unwrap();
        let refusal = "refused: `ZZT_TEST` exists on the calculator (its reply arrived late); it will not be overwritten";
        let op = legacy_attempt(&s, op, &format!("{refusal}; verification: {refusal}"));
        if second_attempt {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(s.root.join("journal.jsonl"))
                .unwrap();
            writeln!(f,"{}",serde_json::json!({"id":op.id,"phase":"submitting","mechanism":"usb_delete","device_serial_id":op.device_serial_id,"targets":op.targets})).unwrap();
        } else {
            put(&m, "ZZT_TEST", NEW);
        }
        assert_eq!(
            deletion::verify(&mut p, &s, &op.id).unwrap().state,
            "unresolved"
        );
        assert!(
            s.reject_pending(&op.device_serial_id, &op.targets().unwrap())
                .is_err()
        );
    }
}
