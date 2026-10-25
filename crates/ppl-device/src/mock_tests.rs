//! Tests against a scripted mock calculator. No HID device is opened.
//!
//! The mock reassembles outgoing framed reports into messages, checks the
//! host sequence numbers, answers `FF` and `F8` from a scripted object map
//! and stores `F7` uploads. Time is virtual: a read that finds nothing
//! advances the mock clock by its timeout instead of sleeping, so 8 s
//! probes run instantly; replies can be scheduled late.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::*;
use crate::files::{decode_file_message, encode_file_message};

const CTRL: &str = "ZZT_CTRL";
const CTRL_TEXT: &str = "EXPORT ZZT_CTRL()\r\nBEGIN\r\nEND;";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnWrite {
    /// Store the uploaded data unchanged.
    Store,
    /// Store it with CRLF line ends and trailing NULs (as the calculator might).
    StoreCrlf,
    /// Store different text.
    StoreMutated,
    /// Accept the message but store nothing.
    Drop,
}

struct State {
    report_size: usize,
    clock: u64,
    rx: Vec<u8>,
    rx_seq: u8,
    outbox: VecDeque<(u64, Vec<u8>)>,
    objects: HashMap<String, (FileType, Vec<u8>)>,
    ff_reply: Option<u8>,
    /// Raw `FA` reply (identity); `None` = silent.
    fa_reply: Option<Vec<u8>>,
    fc_reply: Option<Vec<u8>>,
    reply_delay: u64,
    /// Per-name `F8` reply delay (ms) overriding `reply_delay`.
    late: HashMap<String, u64>,
    /// Names whose `F8` is answered with a non-`F7` message.
    bogus: HashSet<String>,
    corrupt: HashSet<String>,
    ignored_eb: HashSet<String>,
    /// Programs the mock "executes" when their result note is first
    /// requested: program name -> (objects to remove, result-note
    /// content). Simulates the user having pressed Run on a deletion
    /// helper by the time the host polls for its note (after the upload
    /// read-back).
    run_on_upload: HashMap<String, (Vec<String>, Option<String>)>,
    /// Program names actually stored by an `F7` upload.
    uploaded: HashSet<String>,
    /// Firmware simulation for an old revision that ignores `EB`.
    ignore_eb: bool,
    /// Names answered only this many more times, then silent.
    budget: HashMap<String, usize>,
    on_write: OnWrite,
    messages: Vec<Vec<u8>>,
}

#[derive(Clone)]
struct Mock(Arc<Mutex<State>>);

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn detected(path: &str) -> Detected {
    Detected {
        vendor_id: VID_HP,
        product_id: 0x1541,
        model: "mock G1",
        path: path.into(),
        manufacturer: None,
        product: None,
        serial: None,
        release: 0,
        interface: 0,
    }
}

impl Mock {
    fn new() -> Mock {
        Mock(Arc::new(Mutex::new(State {
            report_size: 64,
            clock: 0,
            rx: Vec::new(),
            rx_seq: 0,
            outbox: VecDeque::new(),
            objects: HashMap::new(),
            ff_reply: Some(READY_REPLY),
            fa_reply: Some(fa_raw("0000000000")),
            fc_reply: None,
            reply_delay: 5,
            late: HashMap::new(),
            bogus: HashSet::new(),
            corrupt: HashSet::new(),
            ignored_eb: HashSet::new(),
            run_on_upload: HashMap::new(),
            uploaded: HashSet::new(),
            ignore_eb: false,
            budget: HashMap::new(),
            on_write: OnWrite::Store,
            messages: Vec::new(),
        })))
    }

    /// A ready calculator holding the default control program.
    fn with_control() -> Mock {
        Mock::new().object(FileType::Program, CTRL, CTRL_TEXT)
    }

    fn object(self, ty: FileType, name: &str, text: &str) -> Mock {
        self.st().objects.insert(name.into(), (ty, utf16(text)));
        self
    }

    fn screenshot(self, png: Vec<u8>) -> Mock {
        self.st().fc_reply = Some(png);
        self
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap()
    }

    fn advance(&self, ms: u64) {
        self.st().clock += ms;
    }

    fn prime(&self) -> Prime {
        Prime::with_transport(detected("mock:0"), Box::new(self.clone()))
    }

    fn cmds(&self) -> Vec<u8> {
        self.st().messages.iter().map(|m| m[0]).collect()
    }

    fn f7_messages(&self) -> Vec<Vec<u8>> {
        self.st()
            .messages
            .iter()
            .filter(|m| m[0] == CMD_FILE)
            .cloned()
            .collect()
    }

    fn f8_names(&self) -> Vec<String> {
        self.st()
            .messages
            .iter()
            .filter(|m| m[0] == CMD_REQ_FILE)
            .map(|m| decode_file_message(m).unwrap().name)
            .collect()
    }
}

impl State {
    /// Frames a calc → host message: `[seq][payload]`, padded to full reports.
    fn queue_reply(&mut self, at: u64, msg: &[u8]) {
        let chunk = self.report_size - 1;
        for (count, part) in msg.chunks(chunk).enumerate() {
            let count = count as u32;
            let mut r = vec![((count + count / 0xFF) & 0xFF) as u8];
            r.extend_from_slice(part);
            r.resize(self.report_size, 0);
            self.outbox.push_back((at, r));
        }
        self.outbox.make_contiguous().sort_by_key(|(t, _)| *t);
    }

    fn handle(&mut self, msg: Vec<u8>) {
        self.messages.push(msg.clone());
        let now = self.clock;
        match msg[0] {
            CMD_CHECK_READY => {
                if let Some(b) = self.ff_reply {
                    self.queue_reply(now + self.reply_delay, &[b]);
                }
            }
            crate::CMD_GET_INFO => {
                if let Some(raw) = self.fa_reply.clone() {
                    self.queue_reply(now + self.reply_delay, &raw);
                }
            }
            CMD_SCREENSHOT => {
                if let Some(png) = self.fc_reply.clone() {
                    let mut rep = vec![CMD_SCREENSHOT, 0x01];
                    rep.extend_from_slice(&(png.len() as u32).to_be_bytes());
                    rep.extend_from_slice(&png);
                    self.queue_reply(now + self.reply_delay, &rep);
                }
            }
            CMD_REQ_FILE => {
                let req = decode_file_message(&msg).expect("well-formed F8");
                assert!(req.crc_ok, "F8 CRC");
                assert!(req.data.is_empty(), "F8 carries no data");
                // One-shot "user pressed Run" simulation: the first poll
                // for a configured helper's result note performs its
                // deletions.
                if let Some(prog) = req.name.strip_suffix('R') {
                    if self.uploaded.contains(prog) {
                        if let Some((remove, note)) = self.run_on_upload.remove(prog) {
                            for r in remove {
                                self.objects.remove(&r);
                            }
                            if let Some(text) = note {
                                self.objects
                                    .insert(req.name.clone(), (FileType::Note, utf16(&text)));
                            }
                        }
                    }
                }
                if let Some(left) = self.budget.get_mut(&req.name) {
                    if *left == 0 {
                        return;
                    }
                    *left -= 1;
                }
                let at = now
                    + self
                        .late
                        .get(&req.name)
                        .copied()
                        .unwrap_or(self.reply_delay);
                if self.bogus.contains(&req.name) {
                    self.queue_reply(at, &[0xF9, 0x01, 0, 0, 0, 2, 0xAA, 0xBB]);
                } else if let Some((ty, data)) = self.objects.get(&req.name).cloned() {
                    let mut reply = encode_file_message(CMD_FILE, ty, &req.name, &data);
                    if self.corrupt.contains(&req.name) {
                        reply[8] ^= 1;
                    }
                    self.queue_reply(at, &reply);
                }
                // Absent: no reply, as observed on the G1.
            }
            crate::CMD_DELETE_OBJECT if self.ignore_eb => {}
            crate::CMD_DELETE_OBJECT => {
                // Firmware simulation: an EB message deletes the named
                // object (as observed from the Connectivity Kit).
                let f = decode_file_message(&msg).expect("well-formed EB");
                assert!(f.crc_ok, "EB CRC");
                if self.ignored_eb.contains(&f.name) {
                    return;
                }
                assert!(f.data.is_empty(), "EB carries no data");
                match self.objects.remove(&f.name) {
                    Some((ty, _)) if ty == f.ty => {}
                    Some((ty, _)) => {
                        panic!("EB type mismatch for {}: {:?}", f.name, ty);
                    }
                    None => panic!("EB for absent object {}", f.name),
                }
            }
            CMD_FILE => {
                let f = decode_file_message(&msg).expect("well-formed F7");
                assert!(f.crc_ok, "F7 CRC");
                let text = f.text();
                let data = match self.on_write {
                    OnWrite::Store => f.data.clone(),
                    OnWrite::StoreCrlf => {
                        let mut d = utf16(&format!("{}  \r\n", text.replace('\n', "\r\n")));
                        d.extend([0, 0, 0, 0]);
                        d
                    }
                    OnWrite::StoreMutated => utf16(&format!("{text}\nX:=1;")),
                    OnWrite::Drop => return,
                };
                self.objects.insert(f.name.clone(), (f.ty, data));
                self.uploaded.insert(f.name);
            }
            _ => {}
        }
    }
}

impl Transport for Mock {
    fn write(&mut self, report: &[u8]) -> Result<usize> {
        let mut s = self.st();
        assert_eq!(report[0], 0, "report id");
        assert!(report.len() <= s.report_size + 1, "report too long");
        assert_eq!(report[1], s.rx_seq, "host sequence number");
        s.rx_seq = s.rx_seq.wrapping_add(1);
        if s.rx_seq == 0xFF {
            s.rx_seq = 0;
        }
        s.rx.extend_from_slice(&report[2..]);
        let short = report.len() < s.report_size + 1;
        let prefixed = s.rx.len() >= 6 && s.rx[1] == 0x01;
        let total =
            prefixed.then(|| u32::from_be_bytes([s.rx[2], s.rx[3], s.rx[4], s.rx[5]]) as usize + 6);
        let complete = match total {
            Some(t) => s.rx.len() >= t,
            None => short,
        };
        if complete {
            let mut msg = std::mem::take(&mut s.rx);
            if let Some(t) = total {
                msg.truncate(t);
            }
            s.rx_seq = 0;
            s.handle(msg);
        }
        Ok(report.len())
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize> {
        let mut s = self.st();
        let t = timeout_ms.max(0) as u64;
        match s.outbox.front() {
            Some(&(at, _)) if at <= s.clock + t => {
                s.clock = s.clock.max(at);
                let (_, r) = s.outbox.pop_front().unwrap();
                let n = r.len().min(buf.len());
                buf[..n].copy_from_slice(&r[..n]);
                Ok(n)
            }
            _ => {
                s.clock += t;
                Ok(0)
            }
        }
    }
}

/// A G1 build-15157 `FA` reply with the given 10-character serial.
fn fa_raw(serial: &str) -> Vec<u8> {
    let mut raw = vec![0xFA, 0x01, 0, 0, 0, 0x6C];
    let mut body = vec![0u8; 108];
    body[65] = 0x30;
    body[68..76].copy_from_slice(&[0x24, 0x20, 0x01, 0x39, 0x35, 0x3B, 0, 0]);
    body[76..86].copy_from_slice(b"V0.050.640");
    body[92..102].copy_from_slice(serial.as_bytes());
    raw.extend(body);
    raw
}

fn anchor() -> ControlObject {
    ControlObject::parse(crate::DEFAULT_SETTINGS_ANCHOR).unwrap()
}

fn baseline(p: &mut Prime) -> crate::delete::Baseline {
    crate::delete::Baseline::from_info(&p.info().unwrap()).unwrap()
}

fn control() -> ControlObject {
    ControlObject::parse(DEFAULT_CONTROL).unwrap()
}

fn targets(program: &str) -> Vec<(FileType, String)> {
    vec![
        (FileType::Program, program.to_string()),
        (FileType::Note, result_note_name(program)),
    ]
}

fn preflight(p: &mut Prime, program: &str) -> Result<WriteGuard> {
    let t = targets(program);
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    p.preflight_new_names(&refs, Some(&control()))
}

fn drained(p: &Prime) -> Vec<&Exchange> {
    p.log.iter().filter(|e| e.drained).collect()
}

const SRC: &str = "EXPORT ZZT_T()\nBEGIN\n  RETURN 1;\nEND;\n";

// (i) late replies

#[test]
fn late_reply_between_commands_is_drained_not_misread() {
    let mock = Mock::new().object(FileType::Note, "ZZT_X", "late");
    mock.st().late.insert("ZZT_X".into(), 8_500);
    let mut p = mock.prime();
    let probe = p
        .probe_file(FileType::Note, "ZZT_X", PROBE_TIMEOUT)
        .unwrap();
    assert!(matches!(probe, Probe::NoReply { waited } if waited == PROBE_TIMEOUT));
    assert!(drained(&p).is_empty(), "nothing has arrived yet");

    // The F7 reply arrives while the host is idle; the next command must
    // not take its first report (seq 00, payload F7 …) as the FF reply.
    mock.advance(2_000);
    let ready = p.check_ready().unwrap();
    assert_eq!(ready, vec![READY_REPLY]);
    let d = drained(&p);
    assert!(!d.is_empty());
    assert_eq!(&d[0].bytes[..2], &[0x00, CMD_FILE]);
    assert!(p.log_text().contains("(drained)"));
}

#[test]
fn late_reply_inside_post_timeout_window_is_drained_immediately() {
    let mock = Mock::new().object(FileType::Note, "ZZT_X", "late");
    mock.st().late.insert("ZZT_X".into(), 8_020);
    let mut p = mock.prime();
    let probe = p
        .probe_file(FileType::Note, "ZZT_X", PROBE_TIMEOUT)
        .unwrap();
    assert!(matches!(probe, Probe::NoReply { .. }));
    assert!(!drained(&p).is_empty(), "post-timeout drain caught it");
    assert_eq!(p.check_ready().unwrap(), vec![READY_REPLY]);
}

#[test]
fn late_target_reply_during_preflight_refuses() {
    // The target exists but answers 8.5 s late: round 1 sees silence, the
    // drain before the next probe catches the late F7 naming the target.
    let mock = Mock::with_control().object(FileType::Program, "ZZT_ABC", SRC);
    mock.st().late.insert("ZZT_ABC".into(), 8_500);
    let mut p = mock.prime();
    let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
    assert!(
        matches!(&err, DeviceError::Refused(m) if m.contains("exists")),
        "{err}"
    );
    assert!(mock.f7_messages().is_empty());
}

// (ii) control silent / not ready

#[test]
fn preflight_succeeds_without_control_object_or_when_control_absent() {
    let mock = Mock::new(); // Zero user programs on device
    let mut p = mock.prime();
    // 1. With an absent control object, preflight falls back to device-level transport verification
    let guard = preflight(&mut p, "ZZT_ABC").expect("preflight should succeed without control");
    assert_eq!(guard.program(), "ZZT_ABC");
    assert!(
        guard
            .log()
            .iter()
            .any(|l| l.contains("using device-level transport verification"))
    );

    // 2. With no control object specified at all (control = None), preflight succeeds directly
    let t = targets("ZZT_DEF");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard2 = p
        .preflight_new_names(&refs, None)
        .expect("preflight without control");
    assert_eq!(guard2.program(), "ZZT_DEF");
    assert!(
        guard2
            .log()
            .iter()
            .any(|l| l.contains("no control object specified"))
    );
}

#[test]
fn preflight_inconclusive_when_control_silent_on_second_probe() {
    // The control answers once, then the calculator stops answering (e.g.
    // the user left the Home screen): the targets' silence proves nothing.
    let mock = Mock::with_control();
    mock.st().budget.insert(CTRL.into(), 1);
    let mut p = mock.prime();
    let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
    assert!(
        matches!(&err, DeviceError::Inconclusive(m) if m.contains("probe 2")),
        "{err}"
    );
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn preflight_inconclusive_when_not_ready() {
    for reply in [None, Some(0x00)] {
        let mock = Mock::with_control();
        mock.st().ff_reply = reply;
        let mut p = mock.prime();
        let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
        assert!(
            matches!(&err, DeviceError::Inconclusive(m) if m.contains("FF")),
            "{reply:?}: {err}"
        );
        assert!(mock.f8_names().is_empty());
    }
}

#[test]
fn preflight_inconclusive_on_stale_input() {
    let mock = Mock::with_control();
    mock.st().queue_reply(0, &[CMD_FILE, 0x01, 0, 0, 0, 0]);
    let mut p = mock.prime();
    let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
    assert!(
        matches!(&err, DeviceError::Inconclusive(m) if m.contains("late")),
        "{err}"
    );
}

// (iii) target exists

#[test]
fn preflight_refuses_existing_program() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_ABC", "user's");
    let mut p = mock.prime();
    let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
    assert!(
        matches!(&err, DeviceError::Refused(m) if m.contains("exists")),
        "{err}"
    );
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn preflight_refuses_existing_result_note() {
    let mock = Mock::with_control().object(FileType::Note, "ZZT_ABCR", "old results");
    let mut p = mock.prime();
    let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
    assert!(
        matches!(&err, DeviceError::Refused(m) if m.contains("ZZT_ABCR")),
        "{err}"
    );
    assert!(mock.f7_messages().is_empty());
}

// (iv) success path

#[test]
fn preflight_then_send_writes_framed_f7_and_reads_back() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let guard = preflight(&mut p, "ZZT_ABC").unwrap();
    assert_eq!(guard.program(), "ZZT_ABC");
    assert_eq!(guard.names(), vec!["ZZT_ABC", "ZZT_ABCR"]);
    assert!(guard.log().iter().any(|l| l.contains("round 2")));
    assert!(
        guard
            .log()
            .iter()
            .any(|l| l.contains("control program:ZZT_CTRL"))
    );
    assert!(
        !guard.log().iter().any(|l| l.contains("BEGIN")),
        "control contents are never logged"
    );
    assert_eq!(
        mock.f8_names(),
        [
            "ZZT_CTRL", "ZZT_ABC", "ZZT_ABCR", "ZZT_CTRL", "ZZT_ABC", "ZZT_ABCR"
        ]
    );
    assert!(mock.f7_messages().is_empty(), "preflight never writes");

    let report = p.send_new_program(guard, SRC).unwrap();
    assert_eq!(report.name, "ZZT_ABC");
    assert_eq!(report.bytes_sent, SRC.encode_utf16().count() * 2);
    assert_eq!(report.readback, ReadBack::Matches);
    assert!(!report.preflight_log.is_empty());

    let f7 = mock.f7_messages();
    assert_eq!(f7.len(), 1, "exactly one F7, never retried");
    let f = decode_file_message(&f7[0]).unwrap();
    assert_eq!(f.name, "ZZT_ABC");
    assert_eq!(f.ty, FileType::Program);
    assert!(f.crc_ok);
    assert_eq!(f.text(), SRC);
    // Multi-report message: reports were checked for id 0 and sequence by
    // the mock; confirm the framing actually spanned several reports.
    assert!(f7[0].len() > 63, "F7 needs more than one 64-byte report");
    let full_reports = p
        .log
        .iter()
        .filter(|e| e.outgoing && e.bytes.len() == 65)
        .count();
    assert!(full_reports >= 1);
    assert_eq!(*mock.cmds().last().unwrap(), CMD_REQ_FILE, "read-back last");
}

#[test]
fn preflight_and_send_on_clean_device_without_any_programs() {
    // Zero user programs on device: no ZZT_CTRL, no ZZT_* programs.
    let mock = Mock::new();
    let mut p = mock.prime();
    let t = targets("ZZT_CLEAN");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = p.preflight_new_names(&refs, None).expect("clean preflight");
    assert_eq!(guard.program(), "ZZT_CLEAN");
    assert!(
        guard
            .log()
            .iter()
            .any(|l| l.contains("no control object specified"))
    );

    let report = p
        .send_new_program(guard, SRC)
        .expect("send on clean device");
    assert_eq!(report.name, "ZZT_CLEAN");
    assert_eq!(report.readback, ReadBack::Matches);
}

#[test]
fn preflight_and_send_with_unrelated_user_programs() {
    // Device has unrelated user programs and notes.
    let mock = Mock::new()
        .object(FileType::Program, "PHYSICS", "EXPORT RUN()...")
        .object(FileType::Note, "MYNOTES", "Some personal notes");
    let mut p = mock.prime();
    let t = targets("ZZT_APP");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = p
        .preflight_new_names(&refs, None)
        .expect("preflight with user programs");
    let report = p.send_new_program(guard, SRC).expect("send");
    assert_eq!(report.readback, ReadBack::Matches);

    // Unrelated programs remain untouched
    assert!(matches!(
        p.request_file(FileType::Program, "PHYSICS"),
        Ok(Probe::Present(_))
    ));
    assert!(matches!(
        p.request_file(FileType::Note, "MYNOTES"),
        Ok(Probe::Present(_))
    ));
}

#[test]
fn preflight_refuses_existing_zzt_bstvis() {
    // Device has existing ZZT_BSTVIS.
    let mock = Mock::new().object(FileType::Program, "ZZT_BSTVIS", "EXPORT BSTVisualizer()...");
    let mut p = mock.prime();
    let t = targets("ZZT_BSTVIS");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let err = p.preflight_new_names(&refs, None).unwrap_err();
    assert!(matches!(err, DeviceError::Refused(m) if m.contains("exists on the calculator")));
}

#[test]
fn readback_normalises_crlf_and_trailing_nuls() {
    let mock = Mock::with_control();
    mock.st().on_write = OnWrite::StoreCrlf;
    let mut p = mock.prime();
    let guard = preflight(&mut p, "ZZT_ABC").unwrap();
    let report = p.send_new_program(guard, SRC).unwrap();
    assert_eq!(report.readback, ReadBack::Matches);
}

// (v) read-back problems

#[test]
fn readback_differs_is_reported() {
    let mock = Mock::with_control();
    mock.st().on_write = OnWrite::StoreMutated;
    let mut p = mock.prime();
    let guard = preflight(&mut p, "ZZT_ABC").unwrap();
    let report = p.send_new_program(guard, SRC).unwrap();
    match report.readback {
        ReadBack::Differs {
            sent_chars,
            got_chars,
            got_text,
        } => {
            assert_eq!(sent_chars, normalize_program_text(SRC).chars().count());
            assert!(got_chars > sent_chars);
            assert!(got_text.ends_with("X:=1;"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.f7_messages().len(), 1);
}

#[test]
fn readback_no_reply_is_reported() {
    let mock = Mock::with_control();
    mock.st().on_write = OnWrite::Drop;
    let mut p = mock.prime();
    let guard = preflight(&mut p, "ZZT_ABC").unwrap();
    let report = p.send_new_program(guard, SRC).unwrap();
    assert_eq!(report.readback, ReadBack::NoReply);
    assert_eq!(mock.f7_messages().len(), 1, "no automatic retry");
}

#[test]
fn readback_accepts_slow_reply_without_resending_f8() {
    // The calculator takes 3.5 s to answer the read-back. The host must
    // keep listening to the single F8 instead of re-asking every 2 s and
    // draining the reply that is already on its way.
    let mock = Mock::new();
    mock.st().late.insert("ZZT_SLOW".into(), 3500);
    let mut p = mock.prime();
    let t = targets("ZZT_SLOW");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = p.preflight_new_names(&refs, None).unwrap();
    let before = mock.f8_names().iter().filter(|n| *n == "ZZT_SLOW").count();
    let report = p.send_new_program(guard, SRC).unwrap();
    assert_eq!(report.readback, ReadBack::Matches);
    let after = mock.f8_names().iter().filter(|n| *n == "ZZT_SLOW").count();
    assert_eq!(after - before, 1, "exactly one read-back F8");
    assert_eq!(mock.f7_messages().len(), 1);
}

#[test]
fn large_program_uploads_in_exact_report_count_and_reads_back() {
    // Just under the proven G1 limit (about 15.9 KB of UTF-16).
    let mut src = String::from("EXPORT ZZT_BIG()\nBEGIN\n");
    while src.len() < 7_900 {
        src.push_str("  LOCAL a; a:=a+1; // padding line\n");
    }
    src.push_str("END;\n");
    let mock = Mock::new();
    let mut p = mock.prime();
    let t = targets("ZZT_BIG");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = p.preflight_new_names(&refs, None).unwrap();
    let report = p.send_new_program(guard, &src).unwrap();
    assert_eq!(report.readback, ReadBack::Matches);
    assert_eq!(report.bytes_sent, src.len() * 2);
    let f7 = mock.f7_messages();
    assert_eq!(f7.len(), 1);
    let expected = Prime::frame(&f7[0], 64).len();
    let sent_reports = p.log.iter().filter(|e| e.outgoing).count();
    // F7 reports plus the FF/F8 probes (one report each).
    let small = mock.cmds().iter().filter(|c| **c != CMD_FILE).count();
    assert_eq!(sent_reports, expected + small);
}

#[test]
fn oversize_program_is_refused_before_any_f7() {
    let mock = Mock::new();
    let mut p = mock.prime();
    let t = targets("ZZT_HUGE");
    let refs: Vec<(FileType, &str)> = t.iter().map(|(ty, n)| (*ty, n.as_str())).collect();
    let guard = p.preflight_new_names(&refs, None).unwrap();
    let src = "A".repeat(MAX_PROGRAM_PAYLOAD / 2 + 1);
    let err = p.send_new_program(guard, &src).unwrap_err();
    assert!(matches!(err, DeviceError::Refused(m) if m.contains("silently drops")));
    assert!(mock.f7_messages().is_empty());
}

// (vi) expired guards, invalid names

#[test]
fn expired_guard_is_refused() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let mut guard = preflight(&mut p, "ZZT_ABC").unwrap();
    guard.max_age = Duration::ZERO;
    std::thread::sleep(Duration::from_millis(2));
    assert!(guard.is_expired());
    let err = p.send_new_program(guard, SRC).unwrap_err();
    assert!(
        matches!(&err, DeviceError::Refused(m) if m.contains("old")),
        "{err}"
    );
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn guard_for_another_device_is_refused() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let guard = preflight(&mut p, "ZZT_ABC").unwrap();
    let mut other = Prime::with_transport(detected("mock:1"), Box::new(mock.clone()));
    let err = other.send_new_program(guard, SRC).unwrap_err();
    assert!(matches!(err, DeviceError::Refused(_)), "{err}");
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn invalid_names_are_refused_without_traffic() {
    for bad in [
        "ABC",
        "ZZT_",
        "zzt_abc",
        "ZZT_ABCDEFGHIJKLM",
        "ZZT_A-B",
        "ZZT_É",
        "ZZT_A B",
        "",
    ] {
        assert!(validate_test_name(bad).is_err(), "{bad:?}");
        let mock = Mock::with_control();
        let mut p = mock.prime();
        let err = p
            .preflight_new_names(&[(FileType::Program, bad)], Some(&control()))
            .unwrap_err();
        assert!(matches!(err, DeviceError::Refused(_)), "{bad:?}: {err}");
        assert!(mock.cmds().is_empty(), "{bad:?}: no traffic");
    }
    for good in ["ZZT_A", "ZZT_ABC123", "ZZT_ABCDEFGHIJKL", "ZZT_a_b"] {
        assert!(validate_test_name(good).is_ok(), "{good:?}");
    }
}

#[test]
fn preflight_target_set_rules() {
    let cases: Vec<Vec<(FileType, &str)>> = vec![
        vec![],
        vec![(FileType::Note, "ZZT_A"), (FileType::Note, "ZZT_B")],
        vec![(FileType::Program, "ZZT_A"), (FileType::Program, "ZZT_B")],
        vec![(FileType::Program, "ZZT_A"), (FileType::Note, "ZZT_A")],
        vec![(FileType::Program, "ZZT_CTRL")],
    ];
    for t in cases {
        let mock = Mock::with_control();
        let mut p = mock.prime();
        let err = p.preflight_new_names(&t, Some(&control())).unwrap_err();
        assert!(matches!(err, DeviceError::Refused(_)), "{t:?}: {err}");
        assert!(mock.cmds().is_empty());
    }
}

// (vii) non-F7 replies

#[test]
fn non_f7_reply_is_protocol_error_not_absence() {
    let mock = Mock::new();
    mock.st().bogus.insert("ZZT_Q".into());
    let mut p = mock.prime();
    let err = p
        .probe_file(FileType::Program, "ZZT_Q", PROBE_TIMEOUT)
        .unwrap_err();
    assert!(matches!(err, DeviceError::Protocol(_)), "{err}");
    let err = p.request_file(FileType::Program, "ZZT_Q").unwrap_err();
    assert!(matches!(err, DeviceError::Protocol(_)), "{err}");
}

#[test]
fn non_f7_reply_during_preflight_is_inconclusive() {
    let mock = Mock::with_control();
    mock.st().bogus.insert("ZZT_ABCR".into());
    let mut p = mock.prime();
    let err = preflight(&mut p, "ZZT_ABC").unwrap_err();
    assert!(matches!(err, DeviceError::Inconclusive(_)), "{err}");
    assert!(mock.f7_messages().is_empty());
}

// helpers

#[test]
fn request_file_reads_existing_object() {
    let mock = Mock::new().object(FileType::Note, "ZZT_R", "1,2,3");
    let mut p = mock.prime();
    match p.request_file(FileType::Note, "ZZT_R").unwrap() {
        Probe::Present(f) => {
            assert_eq!(f.text(), "1,2,3");
            assert!(f.crc_ok);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        p.request_file(FileType::Note, "ZZT_NONE").unwrap(),
        Probe::NoReply { .. }
    ));
}

#[test]
fn fresh_names_are_valid_and_vary() {
    let a = fresh_test_name();
    assert_eq!(a.len(), 10);
    assert!(a.starts_with("ZZT_"));
    assert!(
        a[4..]
            .chars()
            .all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c))
    );
    validate_test_name(&a).unwrap();
    validate_test_name(&result_note_name(&a)).unwrap();
    assert_eq!(result_note_name("ZZT_1"), "ZZT_1R");
    let distinct: HashSet<String> = (0..16).map(|_| fresh_test_name()).collect();
    assert!(distinct.len() > 1);
}

#[test]
fn control_object_parsing() {
    let c = ControlObject::parse("program:ZZT_CTRL").unwrap();
    assert_eq!(c.ty, FileType::Program);
    assert_eq!(c.name, "ZZT_CTRL");
    assert_eq!(c.to_string(), "program:ZZT_CTRL");
    assert_eq!(
        "note:MyNote".parse::<ControlObject>().unwrap().ty,
        FileType::Note
    );
    for bad in [
        "ZZT_CTRL",
        "bogus:ZZT_CTRL",
        "program:",
        "program:1X",
        "program:A-B",
    ] {
        assert!(ControlObject::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn normalisation() {
    assert_eq!(normalize_program_text("A\r\nB  \r\n\0\0"), "A\nB");
    assert_eq!(normalize_program_text("A\n"), "A");
}

// (j) EC key injection (never automatic: requires a session-authorized caller)

#[test]
fn press_keys_frames_one_ec_message_per_call() {
    let mock = Mock::new();
    let mut p = mock.prime();
    p.press_keys(&[5]).unwrap();
    p.press_keys(&[4, 30]).unwrap();
    let ec: Vec<Vec<u8>> = mock
        .st()
        .messages
        .iter()
        .filter(|m| m[0] == CMD_KEY)
        .cloned()
        .collect();
    assert_eq!(
        ec,
        vec![
            vec![CMD_KEY, 0x01, 0, 0, 0, 1, 5],
            vec![CMD_KEY, 0x01, 0, 0, 0, 2, 4, 30],
        ]
    );
    // nothing else was sent
    assert_eq!(mock.cmds(), vec![CMD_KEY, CMD_KEY]);
}

#[test]
fn press_keys_rejects_codes_outside_the_grid() {
    let mock = Mock::new();
    let mut p = mock.prime();
    assert!(matches!(p.press_keys(&[51]), Err(DeviceError::Refused(_))));
    assert!(matches!(p.press_keys(&[255]), Err(DeviceError::Refused(_))));
    assert!(mock.st().messages.is_empty(), "nothing is sent on refusal");
}

#[test]
fn press_keys_sends_nothing_when_drain_finds_late_data_but_keeps_logs() {
    // EC is a *drive* command: it must tolerate a quiet line but must not
    // swallow queued replies into a wrong read later.
    let mock = Mock::new().object(FileType::Note, "ZZT_X", "late");
    mock.st().late.insert("ZZT_X".into(), 8_500);
    let mut p = mock.prime();
    // provoke a late F7 that arrives during the EC call
    let _ = p.request_file(FileType::Note, "ZZT_X");
    p.press_keys(&[5]).unwrap();
    let ec: Vec<Vec<u8>> = mock
        .st()
        .messages
        .iter()
        .filter(|m| m[0] == CMD_KEY)
        .cloned()
        .collect();
    assert_eq!(ec, vec![vec![CMD_KEY, 0x01, 0, 0, 0, 1, 5]]);
    // the next read must still be able to observe the late F7
    assert!(p.request_file(FileType::Note, "ZZT_X").is_ok());
}

#[test]
fn session_authorized_press_path_end_to_end() {
    use crate::keys::parse_key_sequence;
    use crate::session::DeviceSession;

    let mut session = DeviceSession::start("test", None, 4, 3600).unwrap();
    let mock = Mock::new();
    let mut p = mock.prime();
    // a scripted sequence: home, enter, esc — all navigation keys
    let codes = parse_key_sequence("home enter esc").unwrap();
    for name in ["home", "enter", "esc"] {
        let code = session.authorize_press(name).unwrap();
        p.press_keys(&[code]).unwrap();
    }
    assert_eq!(codes, vec![5, 30, 4]);
    // refused keys never reach the device
    assert!(session.authorize_press("1").is_err());
    let ec: Vec<Vec<u8>> = mock
        .st()
        .messages
        .iter()
        .filter(|m| m[0] == CMD_KEY)
        .cloned()
        .collect();
    assert_eq!(
        ec,
        vec![
            vec![CMD_KEY, 0x01, 0, 0, 0, 1, 5],
            vec![CMD_KEY, 0x01, 0, 0, 0, 1, 30],
            vec![CMD_KEY, 0x01, 0, 0, 0, 1, 4],
        ]
    );
    // session bookkeeping: 3 of 4 used, all navigation
    assert_eq!(session.presses.len(), 3);
    assert!(session.status_line().contains("used=3/4"));
}

#[test]
fn note_only_preflight_and_send_roundtrip() {
    let mock = Mock::with_control().object(FileType::Note, "ZZT_KEPT", "existing");
    let mut p = mock.prime();
    let guard = p
        .preflight_new_names(&[(FileType::Note, "ZZT_NEWNT")], Some(&control()))
        .expect("note-only preflight");
    assert_eq!(guard.names(), vec!["ZZT_NEWNT"]);
    let report = p
        .send_new_note(guard, "roundtrip probe text")
        .expect("send");
    assert_eq!(report.name, "ZZT_NEWNT");
    assert_eq!(report.readback, ReadBack::Matches);
    // existing note untouched
    assert!(matches!(
        p.request_file(FileType::Note, "ZZT_KEPT"),
        Ok(Probe::Present(_))
    ));
    // program guard rules still enforced
    let err = p.preflight_new_names(
        &[(FileType::Note, "ZZT_A"), (FileType::Note, "ZZT_B")],
        Some(&control()),
    );
    assert!(matches!(err, Err(DeviceError::Refused(_))));
    let err = p.preflight_new_names(&[], Some(&control()));
    assert!(matches!(err, Err(DeviceError::Refused(_))));
}

#[test]
fn note_guard_may_not_send_a_program() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let guard = p
        .preflight_new_names(
            &[(FileType::Program, "ZZT_NP"), (FileType::Note, "ZZT_NPR")],
            Some(&control()),
        )
        .unwrap();
    let err = p.send_new_note(guard, "text");
    assert!(matches!(err, Err(DeviceError::Refused(_))));
    assert!(mock.f7_messages().is_empty(), "nothing was written");
}

#[test]
fn program_only_preflight_does_not_panic() {
    // Regression: `unwrap_or(notes[0])` eagerly indexed the empty notes
    // list and panicked even though the guard names the program. The
    // cleanup PoC (a program upload with no result note) hit this.
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let guard = p
        .preflight_new_names(&[(FileType::Program, "ZZT_ONLYP")], Some(&control()))
        .expect("program-only preflight");
    assert_eq!(guard.program(), "ZZT_ONLYP");
    let report = p
        .send_new_program(guard, "EXPORT ZZT_ONLYP() BEGIN END;")
        .unwrap();
    assert_eq!(report.name, "ZZT_ONLYP");
    assert_eq!(report.readback, ReadBack::Matches);
}

// ---- run controller (manual-run orchestration) ------------------------------

const RUN_NOTE_TEXT: &str = "@@ZZT|ZZT_TRUN|deadbeef|12|0|0\nc1|0|1.5\n@@END";

fn run_cfg(program: &str) -> RunConfig {
    let mut cfg = RunConfig::new(program, control());
    cfg.timeout = Duration::from_secs(40);
    cfg.poll = Duration::from_secs(2);
    cfg.settle = Duration::from_secs(2);
    cfg
}

#[test]
fn run_captures_note_after_program_completes() {
    // The program exists; its result note only starts answering 30 s into
    // the polling window (a "user pressed Run" simulation).
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TRUN", SRC);
    mock.st()
        .objects
        .insert("ZZT_TRUNR".into(), (FileType::Note, utf16(RUN_NOTE_TEXT)));
    mock.st().late.insert("ZZT_TRUNR".into(), 30_000);
    let mut p = mock.prime();
    let cfg = run_cfg("ZZT_TRUN");
    let mut polls = 0u32;
    let out = run_program(&mut p, &cfg, |e| {
        if matches!(e, RunEvent::Poll { .. }) {
            polls += 1;
        }
    })
    .expect("run");
    match out {
        RunOutcome::Captured { text, polls: n } => {
            assert!(text.starts_with("@@ZZT|ZZT_TRUN|"));
            assert!(text.ends_with("@@END"));
            assert!((1..=8).contains(&n), "unexpected poll count {n}");
            assert!(polls >= 1, "expected at least one silent poll event");
        }
        other => panic!("expected Captured, got {other:?}"),
    }
    // The whole flow is read-only: only F8 requests were sent.
    assert!(
        mock.cmds().iter().all(|c| *c == CMD_REQ_FILE),
        "run sent non-F8 traffic: {:02X?}",
        mock.cmds()
    );
}

#[test]
fn run_refuses_stale_result_note() {
    // A leftover result note from an earlier run must block the run instead
    // of mixing evidence.
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TRUN", SRC)
        .object(FileType::Note, "ZZT_TRUNR", RUN_NOTE_TEXT);
    let mut p = mock.prime();
    let err = run_program(&mut p, &run_cfg("ZZT_TRUN"), |_| {}).unwrap_err();
    assert!(
        matches!(&err, RunPreflight::StaleNote(n) if n == "ZZT_TRUNR"),
        "{err}"
    );
}

#[test]
fn run_times_out_when_note_never_appears() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TRUN", SRC);
    let mut p = mock.prime();
    let mut cfg = run_cfg("ZZT_TRUN");
    cfg.timeout = Duration::from_secs(6);
    cfg.poll = Duration::from_secs(2);
    let out = run_program(&mut p, &cfg, |_| {}).expect("run");
    assert_eq!(
        out,
        RunOutcome::TimedOut {
            note: "ZZT_TRUNR".into(),
            polls: 3
        }
    );
}

#[test]
fn run_reports_header_mismatch() {
    let note_text = "some other note entirely";
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TRUN", SRC)
        .object(FileType::Note, "ZZT_TRUNR", note_text);
    // The note only starts answering mid-run (program "completes" after the
    // preflight): preflight must not see it as stale evidence.
    mock.st().late.insert("ZZT_TRUNR".into(), 25_000);
    let mut p = mock.prime();
    let out = run_program(&mut p, &run_cfg("ZZT_TRUN"), |_| {}).expect("run");
    assert_eq!(
        out,
        RunOutcome::HeaderMismatch {
            note: "ZZT_TRUNR".into(),
            expected: "@@ZZT|ZZT_TRUN|".into()
        }
    );
}

#[test]
fn run_header_check_can_be_disabled() {
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TRUN", SRC)
        .object(FileType::Note, "ZZT_TRUNR", "arbitrary");
    mock.st().late.insert("ZZT_TRUNR".into(), 25_000);
    let mut p = mock.prime();
    let mut cfg = run_cfg("ZZT_TRUN");
    cfg.expect_header = None;
    let out = run_program(&mut p, &cfg, |_| {}).expect("run");
    assert_eq!(
        out,
        RunOutcome::Captured {
            text: "arbitrary".into(),
            polls: 5
        }
    );
}

#[test]
fn run_preflight_requires_program_presence() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let err = crate::run::preflight(&mut p, &run_cfg("ZZT_GONE")).unwrap_err();
    assert!(
        matches!(&err, RunPreflight::MissingProgram(n) if n == "ZZT_GONE"),
        "{err}"
    );
}

#[test]
fn run_preflight_detects_dead_device() {
    // Control answers once, then falls silent: note silence must not be
    // mistaken for a free-and-clear result slot.
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TRUN", SRC);
    mock.st().budget.insert(CTRL.into(), 1);
    let mut p = mock.prime();
    let err = crate::run::preflight(&mut p, &run_cfg("ZZT_TRUN")).unwrap_err();
    assert!(matches!(err, RunPreflight::ControlLost(_)), "{err}");
}

// Mailbox tests use the scripted HID transport, never physical evidence.
#[test]
fn mailbox_reserves_request_receipt_result_and_sends_only_request() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let request = remote::Request::new("ABC123", "hello", true).unwrap();
    let mut persisted = false;
    remote::publish(&mut p, &request, &control(), || {
        persisted = true;
        Ok(())
    })
    .unwrap();
    assert!(persisted);
    let files = mock.f7_messages();
    assert_eq!(files.len(), 1);
    assert_eq!(
        files::decode_file_message(&files[0]).unwrap().name,
        request.request_note
    );
    for n in [
        &request.request_note,
        &request.receipt_note,
        &request.result_note,
    ] {
        assert!(mock.f8_names().iter().filter(|s| *s == n).count() >= 2);
    }
}
#[test]
fn mailbox_collision_and_journal_failure_never_send() {
    let request = remote::Request::new("ABC123", "hello", true).unwrap();
    for name in [
        &request.request_note,
        &request.receipt_note,
        &request.result_note,
    ] {
        let mock = Mock::with_control().object(FileType::Note, name, "existing");
        let mut p = mock.prime();
        assert!(remote::publish(&mut p, &request, &control(), || Ok(())).is_err());
        assert!(mock.f7_messages().is_empty());
    }
    let mock = Mock::with_control();
    let mut p = mock.prime();
    assert!(
        remote::publish(&mut p, &request, &control(), || Err(DeviceError::Hid(
            "disk full".into()
        )))
        .is_err()
    );
    assert!(mock.f7_messages().is_empty());
}
#[test]
fn mailbox_poll_rejects_wrong_crc_and_unrelated_notes() {
    let request = remote::Request::new("ABC123", "2+2", false).unwrap();
    let text = format!(
        "@@PPLREMOTE|1|{}|{}|ok|0\n4\n@@END",
        request.request_note, request.nonce
    );
    let mock = Mock::with_control().object(FileType::Note, &request.result_note, &text);
    let mut p = mock.prime();
    let reply = remote::poll(
        &mut p,
        &request,
        std::time::Instant::now() + Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(reply.text, "4");
    let mock = Mock::with_control().object(FileType::Program, &request.result_note, &text);
    let mut p = mock.prime();
    assert!(matches!(
        remote::poll(
            &mut p,
            &request,
            std::time::Instant::now() + Duration::from_secs(1)
        ),
        Err(DeviceError::Protocol(_))
    ));
}
#[test]
fn mailbox_timeout_sends_no_execution_and_disconnect_is_propagated() {
    let request = remote::Request::new("ABC123", "2", false).unwrap();
    let mock = Mock::with_control();
    let mut p = mock.prime();
    assert!(matches!(
        remote::poll(&mut p, &request, std::time::Instant::now()),
        Err(DeviceError::Timeout(_))
    ));
    assert!(mock.cmds().is_empty());
    struct Disconnected;
    impl Transport for Disconnected {
        fn write(&mut self, _: &[u8]) -> Result<usize> {
            Err(DeviceError::Hid("disconnected".into()))
        }
        fn read_timeout(&mut self, _: &mut [u8], _: i32) -> Result<usize> {
            Err(DeviceError::Hid("disconnected".into()))
        }
    }
    let mut p = Prime::with_transport(detected("disconnect"), Box::new(Disconnected));
    assert!(matches!(
        remote::poll(
            &mut p,
            &request,
            std::time::Instant::now() + Duration::from_secs(1)
        ),
        Err(DeviceError::Hid(_))
    ));
}

#[test]
fn partial_hid_writes_are_never_accepted_or_retried() {
    struct Partial;
    impl Transport for Partial {
        fn write(&mut self, r: &[u8]) -> Result<usize> {
            Ok(r.len() - 1)
        }
        fn read_timeout(&mut self, _: &mut [u8], _: i32) -> Result<usize> {
            Ok(0)
        }
    }
    let mut prime = Prime::with_transport(detected("partial"), Box::new(Partial));
    assert!(matches!(prime.check_ready(), Err(DeviceError::Hid(_))));
    assert_eq!(prime.log.iter().filter(|e| e.outgoing).count(), 1);
}

#[test]
fn mailbox_corrupt_crc_is_not_completion_evidence() {
    let request = remote::Request::new("ABC123", "2+2", false).unwrap();
    let text = format!(
        "@@PPLREMOTE|1|{}|{}|ok|0\n4\n@@END",
        request.request_note, request.nonce
    );
    let mut raw = files::encode_file_message(
        CMD_FILE,
        FileType::Note,
        &request.result_note,
        &utf16(&text),
    );
    raw[8] ^= 1;
    let mock = Mock::with_control();
    // Arrives after the initial drain, simulating a corrupt F8 response.
    mock.st().queue_reply(100, &raw);
    let mut prime = mock.prime();
    assert!(matches!(
        remote::poll(
            &mut prime,
            &request,
            std::time::Instant::now() + Duration::from_secs(1)
        ),
        Err(DeviceError::Protocol(_))
    ));
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn owned_note_capability_is_connection_bound_and_never_writes_programs() {
    let mock = Mock::with_control();
    let mut prime = mock.prime();
    let guard = prime
        .preflight_new_names(&[(FileType::Note, "ZZT_OWNED")], Some(&control()))
        .unwrap();
    let owned = prime.create_owned_note(guard, "initial").unwrap();
    let mut other = mock.prime();
    let before = mock.f7_messages().len();
    assert!(matches!(
        other.update_owned_note(&owned, "wrong connection"),
        Err(DeviceError::Refused(_))
    ));
    assert_eq!(before, mock.f7_messages().len());
    prime.update_owned_note(&owned, "challenge").unwrap();
    let uploads = mock.f7_messages();
    assert_eq!(uploads.len(), 2);
    for raw in uploads {
        let file = files::decode_file_message(&raw).unwrap();
        assert_eq!(file.ty, FileType::Note);
        assert_eq!(file.name, "ZZT_OWNED");
    }
    drop(prime);
    assert!(other.update_owned_note(&owned, "expired owner").is_err());
}
#[test]
fn unverified_note_creation_never_issues_owned_update_capability() {
    let mock = Mock::with_control();
    mock.st().on_write = OnWrite::Drop;
    let mut prime = mock.prime();
    let guard = prime
        .preflight_new_names(&[(FileType::Note, "ZZT_OWNED")], Some(&control()))
        .unwrap();
    assert!(prime.create_owned_note(guard, "initial").is_err());
    assert_eq!(mock.f7_messages().len(), 1);
}

#[test]
fn write_guard_cannot_cross_connections_even_with_same_path() {
    let mock = Mock::with_control();
    let mut original = mock.prime();
    let guard = original
        .preflight_new_names(&[(FileType::Program, "ZZT_BOUND")], Some(&control()))
        .unwrap();
    let mut other = mock.prime();
    assert!(matches!(
        other.send_new_program(guard, "EXPORT ZZT_BOUND() BEGIN END;"),
        Err(DeviceError::Refused(_))
    ));
    assert!(mock.f7_messages().is_empty());
}
#[test]
fn owned_mailbox_resume_requires_exact_current_tagged_content() {
    let text = format!("@@PPLMAIL|{}|{}", "A".repeat(32), "0".repeat(32));
    let mock = Mock::with_control().object(FileType::Note, "ZZT_BOX", &text);
    let mut prime = mock.prime();
    let owned = prime.reopen_owned_note("ZZT_BOX", &text).unwrap();
    assert!(
        prime
            .reopen_owned_note("ZZT_BOX", &text.replace('A', "B"))
            .is_err()
    );
    assert!(prime.reopen_owned_note("ZZT_BOX", "user content").is_err());
    assert!(mock.f7_messages().is_empty());
    prime.update_owned_note(&owned, "new nonce").unwrap();
    assert_eq!(mock.f7_messages().len(), 1);
}

// ---- deletion of tool-owned objects ----------------------------------------

const DEL_SRC: &str = "EXPORT ZZT_TDEL1()\r\nBEGIN\r\n  RETURN 1;\r\nEND;";

/// Simulates the calculator running the uploaded helper: every target and
/// the helper itself disappear, and the helper's result note appears.
fn run_helper_on_mock(mock: &Mock, del: &Deletion) {
    let mut st = mock.st();
    for b in &del.backups {
        st.objects.remove(&b.name);
    }
    st.objects.remove(&del.helper);
    st.objects.insert(
        del.helper_note.clone(),
        (
            FileType::Note,
            utf16(&format!("@@PPLDELETE|{}|done", del.helper)),
        ),
    );
}

/// Configures the mock to "run" the deletion helper once its result note
/// is first polled: `apply_deletions` controls whether the helper's
/// deletions actually happen; `note` is the result-note content.
fn autorun_helper(mock: &Mock, del: &Deletion, apply_deletions: bool, note: Option<&str>) {
    let mut remove: Vec<String> = if apply_deletions {
        del.backups.iter().map(|b| b.name.clone()).collect()
    } else {
        Vec::new()
    };
    remove.push(del.helper.clone());
    mock.st().run_on_upload.insert(
        del.helper.clone(),
        (
            remove,
            note.map(|text| {
                if text == "done" {
                    format!("@@PPLDELETE|{}|done", del.helper)
                } else {
                    text.into()
                }
            }),
        ),
    );
}

fn del_opts() -> DeleteOptions {
    DeleteOptions {
        timeout: Duration::from_secs(30),
        poll: Duration::from_secs(2),
        settle: Duration::from_secs(1),
    }
}

#[test]
fn delete_flow_confirms_removal_of_program_and_note() {
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TDEL1", DEL_SRC)
        .object(FileType::Note, "ZZT_TDELN", "note body");
    let mut p = mock.prime();
    let targets = [
        DeleteTarget::program("ZZT_TDEL1"),
        DeleteTarget::note("ZZT_TDELN"),
    ];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    // Backups capture the exact object data.
    assert_eq!(del.backups.len(), 2);
    assert_eq!(del.backups[0].name, "ZZT_TDEL1");
    assert_eq!(del.backups[1].name, "ZZT_TDELN");
    assert_eq!(del.backups[0].data, utf16(DEL_SRC));
    assert_eq!(del.backups[1].data, utf16("note body"));
    // The helper source deletes exactly the targets, then itself.
    assert!(del.helper.starts_with("ZZT_D"));
    assert_eq!(del.helper_note, format!("{}R", del.helper));
    assert!(del.source.contains("Programs(\"ZZT_TDEL1\"):=\"\";"));
    assert!(del.source.contains("Notes(\"ZZT_TDELN\"):=\"\";"));
    assert!(
        del.source
            .contains(&format!("Programs(\"{}\"):=\"\";", del.helper))
    );
    assert!(!del.source.contains("ZZT_CTRL"));
    assert_eq!(
        del.allowlist(),
        vec![
            DeleteTarget::program("ZZT_TDEL1"),
            DeleteTarget::note("ZZT_TDELN"),
            DeleteTarget::program(&del.helper),
        ]
    );
    // Upload through the guarded preflight; read-back must match.
    let report = upload_deletion_helper(&mut p, &del, &control()).expect("upload");
    assert_eq!(report.name, del.helper);
    assert_eq!(report.readback, ReadBack::Matches);
    // "User presses Run": the calculator performs the deletions.
    run_helper_on_mock(&mock, &del);
    let w = await_deletion_note(&mut p, &del, &del_opts(), |_| {}).expect("await");
    assert_eq!(w.expect("note").text, "done");
    // Two probe rounds with the control answering: all gone.
    let mut all: Vec<DeleteTarget> = targets.to_vec();
    all.push(DeleteTarget::program(&del.helper));
    let statuses = verify_deleted(&mut p, &all, &control()).expect("verify");
    assert!(
        statuses
            .iter()
            .all(|s| *s == TargetStatus::ConfirmedDeleted)
    );
}

#[test]
fn delete_flow_reports_still_present_when_helper_was_not_run() {
    // Nothing runs the helper: the result note never appears and every
    // object is still there after the timeout.
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TDEL1", DEL_SRC)
        .object(FileType::Note, "ZZT_TDELN", "note body");
    let mut p = mock.prime();
    let targets = [
        DeleteTarget::program("ZZT_TDEL1"),
        DeleteTarget::note("ZZT_TDELN"),
    ];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    let report = run_deletion(&mut p, &del, &control(), &del_opts(), |_| {}).expect("run");
    assert_eq!(
        report.completion,
        Completion::TimedOut { polls: 15 },
        "expected a timeout outcome"
    );
    assert!(
        report
            .targets
            .iter()
            .all(|t| t.status == TargetStatus::StillPresent)
    );
    assert_eq!(report.helper_removed, Some(false));
}

#[test]
fn delete_flow_reports_failed_note_content() {
    // The helper "runs" but its result note says something unexpected:
    // the deletion must be reported as failed and unconfirmed.
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TDEL1", DEL_SRC)
        .object(FileType::Note, "ZZT_TDELN", "note body");
    let mut p = mock.prime();
    let targets = [DeleteTarget::program("ZZT_TDEL1")];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    autorun_helper(&mock, &del, false, Some("garbage"));
    let report = run_deletion(&mut p, &del, &control(), &del_opts(), |_| {}).expect("run");
    assert_eq!(
        report.completion,
        Completion::Failed {
            got: "garbage".into()
        }
    );
    assert_eq!(report.targets[0].status, TargetStatus::StillPresent);
    // The helper still removed itself; only the target survived.
    assert_eq!(report.helper_removed, Some(true));
}

#[test]
fn delete_flow_confirm_end_to_end_when_helper_runs() {
    // Full composed flow with the helper "running" right after upload.
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TDEL1", DEL_SRC)
        .object(FileType::Note, "ZZT_TDELN", "note body");
    let mut p = mock.prime();
    let targets = [
        DeleteTarget::program("ZZT_TDEL1"),
        DeleteTarget::note("ZZT_TDELN"),
    ];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    autorun_helper(&mock, &del, true, Some("done"));
    let mut events = Vec::new();
    let report =
        run_deletion(&mut p, &del, &control(), &del_opts(), |e| events.push(e)).expect("run");
    assert_eq!(report.completion, Completion::Done);
    assert_eq!(report.note_text.as_deref(), Some("done"));
    assert!(
        report
            .targets
            .iter()
            .all(|t| t.status == TargetStatus::ConfirmedDeleted)
    );
    assert_eq!(report.helper_removed, Some(true));
    assert!(events.contains(&DeleteEvent::AwaitingRun {
        helper: del.helper.clone()
    }));
    // Terminal evidence: the helper's result note remains on the device.
    assert_eq!(mock.st().objects.len(), 2); // control + helper result note
}

#[test]
fn delete_prepare_refuses_missing_object() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    let err =
        prepare_deletion(&mut p, &[DeleteTarget::program("ZZT_GONE")], &control()).unwrap_err();
    assert!(
        matches!(err, DeviceError::Absent(ref m) if m.contains("not present")),
        "{err}"
    );
}

#[test]
fn delete_prepare_refuses_wrong_type() {
    // Only a program exists, but a note deletion is requested.
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TDEL1", DEL_SRC);
    let mut p = mock.prime();
    let err = prepare_deletion(&mut p, &[DeleteTarget::note("ZZT_TDEL1")], &control()).unwrap_err();
    assert!(
        matches!(err, DeviceError::Refused(ref m) if m.contains("exists as a program")),
        "{err}"
    );
}

#[test]
fn delete_prepare_refuses_protected_and_invalid_names() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TDEL1", DEL_SRC);
    let mut p = mock.prime();
    for bad in [
        vec![DeleteTarget::program("ZZT_CTRL")],
        vec![DeleteTarget::program("tester")],
        vec![DeleteTarget::program("ZZT_A B")],
        vec![
            DeleteTarget::program("ZZT_TDEL1"),
            DeleteTarget::program("zzt_tdel1"),
        ],
        vec![DeleteTarget::note("ZZT_CTRL")],
    ] {
        let err = prepare_deletion(&mut p, &bad, &control()).unwrap_err();
        assert!(matches!(err, DeviceError::Refused(_)), "{err} for {bad:?}");
    }
    // The configured control is protected even under another name.
    let custom = ControlObject {
        ty: FileType::Program,
        name: "ZZT_TDEL1".into(),
    };
    let err = prepare_deletion(&mut p, &[DeleteTarget::program("ZZT_TDEL1")], &custom).unwrap_err();
    assert!(matches!(err, DeviceError::Refused(_)), "{err}");
}

#[test]
fn delete_prepare_requires_living_control() {
    // The control answers once, then falls silent: target silence must
    // not be interpreted as "not present".
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TDEL1", DEL_SRC);
    mock.st().budget.insert(CTRL.to_string(), 1);
    let mut p = mock.prime();
    let err =
        prepare_deletion(&mut p, &[DeleteTarget::program("ZZT_TDEL1")], &control()).unwrap_err();
    assert!(matches!(err, DeviceError::Inconclusive(_)), "{err}");
}

#[test]
fn delete_verify_distinguishes_presence_and_control_loss() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TDEL1", DEL_SRC);
    let mut p = mock.prime();
    // Present object: not deleted.
    let st = verify_deleted(&mut p, &[DeleteTarget::program("ZZT_TDEL1")], &control()).unwrap();
    assert_eq!(st[0], TargetStatus::StillPresent);
    // Dead control: silence means nothing.
    mock.st().budget.insert(CTRL.to_string(), 0);
    let err = verify_deleted(&mut p, &[], &control()).unwrap_err();
    assert!(matches!(err, DeviceError::Inconclusive(_)), "{err}");
}

#[test]
fn delete_helper_source_is_deterministic() {
    let targets = [
        DeleteTarget::program("ZZT_TDEL1"),
        DeleteTarget::note("ZZT_TDELN"),
    ];
    let src = generate_deletion_helper("ZZT_DABCDEF", &targets);
    assert_eq!(
        src,
        "// Deletion helper generated by ppl (exact allowlist; run once).\n\
         // It deletes only the objects named below, then itself.\n\
         EXPORT ZZT_DABCDEF()\n\
         BEGIN\n\
         \x20 Notes(\"ZZT_DABCDEFR\"):=\"working\";\n\
         \x20 Programs(\"ZZT_TDEL1\"):=\"\";\n\
         \x20 Notes(\"ZZT_TDELN\"):=\"\";\n\
         \x20 Notes(\"ZZT_DABCDEFR\"):=\"done\";\n\
         \x20 Programs(\"ZZT_DABCDEF\"):=\"\";\n\
         \x20 RETURN 0;\n\
         END;\n"
    );
}

#[test]
fn delete_flow_survives_disconnect_during_verify() {
    // A transport error during verification surfaces as a device error,
    // never as a false "confirmed".
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TDEL1", DEL_SRC)
        .object(FileType::Note, "ZZT_TDELN", "note body");
    let mut p = mock.prime();
    let targets = [DeleteTarget::program("ZZT_TDEL1")];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    upload_deletion_helper(&mut p, &del, &control()).expect("upload");
    run_helper_on_mock(&mock, &del);
    // Swap in a dead transport before verification.
    struct Dead;
    impl Transport for Dead {
        fn write(&mut self, _: &[u8]) -> Result<usize> {
            Err(DeviceError::Hid("disconnected".into()))
        }
        fn read_timeout(&mut self, _: &mut [u8], _: i32) -> Result<usize> {
            Err(DeviceError::Hid("disconnected".into()))
        }
    }
    let mut dead = Prime::with_transport(detected("disconnect"), Box::new(Dead));
    let mut all: Vec<DeleteTarget> = targets.to_vec();
    all.push(DeleteTarget::program(&del.helper));
    let err = verify_deleted(&mut dead, &all, &control());
    assert!(err.is_err());
}

#[test]
fn usb_delete_removes_objects_without_any_upload() {
    // The EB path: presence proof, direct delete, absence verification.
    // No F7 upload, no helper program, no result note.
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_TDEL1", DEL_SRC)
        .object(FileType::Note, "ZZT_TDELN", "note body");
    let mut p = mock.prime();
    let targets = [
        DeleteTarget::program("ZZT_TDEL1"),
        DeleteTarget::note("ZZT_TDELN"),
    ];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    let guard = authorize_usb_deletion(&p, &del, &control()).unwrap();
    let report = delete_via_usb(&mut p, guard, |_| {}).expect("usb delete");
    assert!(
        report
            .targets
            .iter()
            .all(|t| t.status == TargetStatus::ConfirmedDeleted)
    );
    // Only F8 probes and the EB messages themselves were sent.
    let cmds = mock.cmds();
    assert!(
        cmds.iter()
            .all(|c| *c == CMD_REQ_FILE || *c == crate::CMD_DELETE_OBJECT),
        "unexpected traffic: {:02X?}",
        cmds
    );
    assert_eq!(
        cmds.iter()
            .filter(|c| **c == crate::CMD_DELETE_OBJECT)
            .count(),
        2
    );
    // The EB message is F7-shaped: type, length-prefixed UTF-16 name, CRC.
    let st = mock.st();
    let eb = st
        .messages
        .iter()
        .find(|m| m[0] == crate::CMD_DELETE_OBJECT)
        .expect("EB message");
    let expect = crate::files::encode_file_message(
        crate::CMD_DELETE_OBJECT,
        FileType::Program,
        "ZZT_TDEL1",
        &[],
    );
    assert_eq!(eb, &expect);
    // No object remains except the control.
    assert_eq!(st.objects.len(), 1);
}

#[test]
fn usb_delete_reports_still_present_when_ignored() {
    // Simulate firmware that ignores EB (e.g. an older revision): the
    // objects must be reported still present, never confirmed.
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TDEL1", DEL_SRC);
    mock.st().ignore_eb = true;
    let mut p = mock.prime();
    let targets = [DeleteTarget::program("ZZT_TDEL1")];
    let del = prepare_deletion(&mut p, &targets, &control()).expect("prepare");
    let guard = authorize_usb_deletion(&p, &del, &control()).unwrap();
    let report = delete_via_usb(&mut p, guard, |_| {}).expect("usb delete");
    assert_eq!(report.targets[0].status, TargetStatus::StillPresent);
    assert!(mock.st().objects.contains_key("ZZT_TDEL1"));
}

#[test]
fn deletion_guard_rejects_another_connection_even_with_the_same_path() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TEST", DEL_SRC);
    let mut first = mock.prime();
    let d = prepare_deletion(&mut first, &[DeleteTarget::program("ZZT_TEST")], &control()).unwrap();
    let guard = authorize_usb_deletion(&first, &d, &control()).unwrap();
    let mut second = mock.prime();
    assert!(delete_via_usb(&mut second, guard, |_| {}).is_err());
    assert!(!mock.cmds().contains(&CMD_DELETE_OBJECT));
}

#[test]
fn deletion_revalidates_content_after_backup_and_never_submits_changed_data() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TEST", DEL_SRC);
    let mut p = mock.prime();
    let d = prepare_deletion(&mut p, &[DeleteTarget::program("ZZT_TEST")], &control()).unwrap();
    let guard = authorize_usb_deletion(&p, &d, &control()).unwrap();
    mock.st()
        .objects
        .insert("ZZT_TEST".into(), (FileType::Program, utf16("changed")));
    assert!(delete_via_usb(&mut p, guard, |_| {}).is_err());
    assert!(!mock.cmds().contains(&CMD_DELETE_OBJECT));
}

#[test]
fn deletion_absence_rejects_corrupt_control_and_control_loss_after_last_probe() {
    let mock = Mock::with_control();
    let mut p = mock.prime();
    mock.st().corrupt.insert(CTRL.into());
    assert!(verify_deleted(&mut p, &[DeleteTarget::program("ZZT_GONE")], &control()).is_err());
    mock.st().corrupt.clear();
    mock.st().budget.insert(CTRL.into(), 2);
    assert!(verify_deleted(&mut p, &[DeleteTarget::program("ZZT_GONE")], &control()).is_err());
}

#[test]
fn deletion_preserves_partial_outcomes_and_never_retries_or_uploads_a_helper() {
    let mock = Mock::with_control()
        .object(FileType::Program, "ZZT_ONE", DEL_SRC)
        .object(FileType::Note, "ZZT_TWO", "note");
    let mut p = mock.prime();
    let d = prepare_deletion(
        &mut p,
        &[
            DeleteTarget::program("ZZT_ONE"),
            DeleteTarget::note("ZZT_TWO"),
        ],
        &control(),
    )
    .unwrap();
    mock.st().ignored_eb.insert("ZZT_TWO".into());
    let guard = authorize_usb_deletion(&p, &d, &control()).unwrap();
    let report = delete_via_usb(&mut p, guard, |_| {}).unwrap();
    assert_eq!(report.targets[0].status, TargetStatus::ConfirmedDeleted);
    assert_eq!(report.targets[1].status, TargetStatus::StillPresent);
    assert_eq!(
        mock.cmds()
            .iter()
            .filter(|&&c| c == CMD_DELETE_OBJECT)
            .count(),
        2
    );
    assert!(!mock.cmds().contains(&CMD_FILE));
    assert!(!mock.cmds().contains(&CMD_KEY));
}

#[test]
fn result_note_identity_crc_and_type_are_required() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_TEST", DEL_SRC);
    let mut p = mock.prime();
    let d = prepare_deletion(&mut p, &[DeleteTarget::program("ZZT_TEST")], &control()).unwrap();
    mock.st()
        .objects
        .insert(d.helper_note.clone(), (FileType::Note, utf16("done")));
    mock.st().corrupt.insert(d.helper_note.clone());
    assert!(await_deletion_note(&mut p, &d, &DeleteOptions::default(), |_| {}).is_err());
}

#[test]
fn deletion_absence_rejects_late_target_replies_drained_beside_live_control() {
    let mock = Mock::with_control().object(FileType::Program, "ZZT_SLOW", DEL_SRC);
    mock.st().late.insert("ZZT_SLOW".into(), 8020);
    let mut p = mock.prime();
    assert!(verify_deleted(&mut p, &[DeleteTarget::program("ZZT_SLOW")], &control()).is_err());
    assert!(mock.st().objects.contains_key("ZZT_SLOW"));
}

// ---- ordinary user programs (no ZZT_ prefix) ----

#[test]
fn user_program_names_accepted_and_rejected() {
    for good in [
        "FIREWORKS",
        "HELLO",
        "SQUARE",
        "BSTVisualizer",
        "MyProgram",
        "ZZT_ABC",
        "A_1",
        "ABCDEFGHIJKLMNOP",
    ] {
        assert!(validate_program_name(good).is_ok(), "{good:?}");
    }
    for bad in [
        "",
        "1ABC",
        "_ABC",
        "MY PROG",
        "MY-PROG",
        "A/B",
        "..",
        "a.b",
        "ÉCOLE",
        "AB\nC",
        "AB\0C",
        "ABCDEFGHIJKLMNOPQ",
        "A",
        "L1",
        "G0",
        "Ans",
        "BEGIN",
        "end",
        "ZZT_CTRL",
    ] {
        let err = validate_program_name(bad).unwrap_err();
        assert!(
            matches!(&err, DeviceError::Refused(m) if !m.contains("test object")),
            "{bad:?}: {err}"
        );
    }
    // The test-object policy is unchanged.
    assert!(validate_test_name("FIREWORKS").is_err());
    assert!(validate_test_name("ZZT_FIREWORKS").is_ok());
}

#[test]
fn user_program_preflight_and_send_need_no_control_and_no_result_note() {
    let mock = Mock::new(); // no ZZT_CTRL, no other objects
    let mut p = mock.prime();
    let guard = p.preflight_new_user_program("FIREWORKS", None).unwrap();
    assert_eq!(guard.program(), "FIREWORKS");
    assert_eq!(guard.policy(), NamePolicy::User);
    assert_eq!(guard.names(), vec!["FIREWORKS"], "no result note target");
    let f8 = mock.f8_names();
    assert!(!f8.iter().any(|n| n == "FIREWORKSR" || n == CTRL), "{f8:?}");
    // exact name plus its lower-case variant, two rounds each
    assert_eq!(f8, ["FIREWORKS", "fireworks", "FIREWORKS", "fireworks"]);
    assert!(mock.f7_messages().is_empty());

    let report = p.send_new_program(guard, SRC).unwrap();
    assert_eq!(report.name, "FIREWORKS");
    assert_eq!(report.readback, ReadBack::Matches);
    let f7 = mock.f7_messages();
    assert_eq!(f7.len(), 1, "exactly one F7, never retried");
    assert_eq!(decode_file_message(&f7[0]).unwrap().name, "FIREWORKS");
    assert_eq!(*mock.cmds().last().unwrap(), CMD_REQ_FILE, "read-back last");
}

#[test]
fn existing_user_program_is_never_overwritten() {
    for existing in ["FIREWORKS", "fireworks"] {
        let mock = Mock::new().object(FileType::Program, existing, "EXPORT X()\nBEGIN\nEND;");
        let mut p = mock.prime();
        let err = p.preflight_new_user_program("FIREWORKS", None).unwrap_err();
        assert!(
            matches!(&err, DeviceError::Refused(m) if m.contains("will not be overwritten")),
            "{existing}: {err}"
        );
        assert!(mock.f7_messages().is_empty(), "{existing}: nothing written");
    }
}

#[test]
fn case_variant_of_existing_program_is_refused() {
    // Uploading `Fireworks` must not shadow an existing `FIREWORKS`.
    let mock = Mock::new().object(FileType::Program, "FIREWORKS", "EXPORT X()\nBEGIN\nEND;");
    let mut p = mock.prime();
    let err = p.preflight_new_user_program("Fireworks", None).unwrap_err();
    assert!(matches!(err, DeviceError::Refused(_)), "{err}");
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn silent_device_is_not_proof_of_absence_for_user_programs() {
    let mock = Mock::new();
    mock.st().ff_reply = None;
    let mut p = mock.prime();
    let err = p.preflight_new_user_program("FIREWORKS", None).unwrap_err();
    assert!(matches!(err, DeviceError::Inconclusive(_)), "{err}");
    assert!(mock.f7_messages().is_empty());
}

#[test]
fn user_program_indeterminate_upload_is_not_retried() {
    for mode in [OnWrite::Drop, OnWrite::StoreMutated] {
        let mock = Mock::new();
        mock.st().on_write = mode;
        let mut p = mock.prime();
        let guard = p.preflight_new_user_program("FIREWORKS", None).unwrap();
        let report = p.send_new_program(guard, SRC).unwrap();
        assert_ne!(report.readback, ReadBack::Matches, "{mode:?}");
        assert_eq!(mock.f7_messages().len(), 1, "{mode:?}: exactly one F7");
    }
}

#[test]
fn user_program_payload_limit_is_enforced_before_any_f7() {
    let at_limit = "A".repeat(MAX_PROGRAM_PAYLOAD / 2);
    assert_eq!(
        check_program_payload("X", &at_limit).unwrap(),
        MAX_PROGRAM_PAYLOAD
    );
    let over = "A".repeat(MAX_PROGRAM_PAYLOAD / 2 + 1);
    assert!(matches!(
        check_program_payload("X", &over),
        Err(DeviceError::Refused(m)) if m.contains("silently drops")
    ));

    let mock = Mock::new();
    let mut p = mock.prime();
    let guard = p.preflight_new_user_program("BIGPROG", None).unwrap();
    let err = p.send_new_program(guard, &over).unwrap_err();
    assert!(matches!(err, DeviceError::Refused(_)), "{err}");
    assert!(mock.f7_messages().is_empty());

    let guard = p.preflight_new_user_program("EDGEPROG", None).unwrap();
    assert_eq!(
        p.send_new_program(guard, &at_limit).unwrap().readback,
        ReadBack::Matches
    );
}

#[test]
fn guards_are_bound_to_their_policy_and_exact_name() {
    // A user guard names only its own program and cannot send a note.
    let mock = Mock::new();
    let mut p = mock.prime();
    let guard = p.preflight_new_user_program("HELLO", None).unwrap();
    assert!(matches!(
        p.send_new_note(guard, "text"),
        Err(DeviceError::Refused(_))
    ));
    assert!(mock.f7_messages().is_empty());

    // The test-object entry points still refuse ordinary names ...
    assert!(matches!(
        p.preflight_new_names(&[(FileType::Program, "FIREWORKS")], None),
        Err(DeviceError::Refused(_))
    ));
    // ... and a user guard sends exactly the program it was issued for.
    let guard = p.preflight_new_user_program("SQUARE", None).unwrap();
    let report = p.send_new_program(guard, SRC).unwrap();
    assert_eq!(report.name, "SQUARE");
    let f7 = mock.f7_messages();
    assert_eq!(f7.len(), 1);
    assert_eq!(decode_file_message(&f7[0]).unwrap().name, "SQUARE");
}

#[test]
fn invalid_user_names_are_refused_without_traffic() {
    for bad in ["", "A B", "../x", "ABCDEFGHIJKLMNOPQ", "Home", "ZZT_CTRL"] {
        let mock = Mock::new();
        let mut p = mock.prime();
        let err = p.preflight_new_user_program(bad, None).unwrap_err();
        assert!(matches!(err, DeviceError::Refused(_)), "{bad:?}: {err}");
        assert!(mock.cmds().is_empty(), "{bad:?}: no traffic");
    }
}

// ---- ordinary (non-ZZT) object deletion ----

fn eb_count(mock: &Mock) -> usize {
    mock.cmds()
        .iter()
        .filter(|c| **c == crate::CMD_DELETE_OBJECT)
        .count()
}

#[test]
fn object_names_accept_ordinary_and_test_names_and_reject_the_rest() {
    use crate::validate_object_name;
    for n in [
        "FIREWORKS",
        "HELLO",
        "BSTVisualizer",
        "ADVPMT",
        "Leasing20",
        "ZZT_BSTVIS",
    ] {
        validate_object_name(FileType::Program, n).unwrap_or_else(|e| panic!("{n}: {e}"));
        validate_object_name(FileType::Note, n).unwrap();
    }
    for n in [
        "",
        "../x",
        "a/b",
        "A B",
        "1ABC",
        "_ABC",
        "ABCDEFGHIJKLMNOPQ",
        "ZZT_CTRL",
        "zzt_ctrl",
        "END",
        "NA\0ME",
        "naïve",
    ] {
        assert!(
            matches!(
                validate_object_name(FileType::Program, n),
                Err(DeviceError::Refused(_))
            ),
            "`{n}` must be refused"
        );
    }
    for ty in [
        FileType::List,
        FileType::Matrix,
        FileType::Real,
        FileType::Complex,
        FileType::Settings,
        FileType::App,
    ] {
        assert!(matches!(
            validate_object_name(ty, "FIREWORKS"),
            Err(DeviceError::Unsupported(_))
        ));
    }
}

#[test]
fn ordinary_program_deletion_sends_one_eb_and_leaves_other_programs() {
    let mock = Mock::with_control()
        .object(FileType::Program, "FIREWORKS", DEL_SRC)
        .object(FileType::Program, "HELLO", "EXPORT HELLO()\nBEGIN\nEND;\n")
        .object(FileType::Note, "MyNote", "keep me");
    let mut p = mock.prime();
    let t = [DeleteTarget::program("FIREWORKS")];
    let del = crate::prepare_deletion_with(&mut p, &t, &control(), NamePolicy::User, None)
        .expect("prepare");
    assert_eq!(del.backups.len(), 1);
    let guard = authorize_usb_deletion(&p, &del, &control()).unwrap();
    let report = delete_via_usb(&mut p, guard, |_| {}).expect("usb delete");
    assert_eq!(report.targets[0].status, TargetStatus::ConfirmedDeleted);
    assert_eq!(eb_count(&mock), 1);
    let st = mock.st();
    assert!(!st.objects.contains_key("FIREWORKS"));
    assert!(st.objects.contains_key("HELLO"));
    assert!(st.objects.contains_key("MyNote"));
    assert!(st.objects.contains_key("ZZT_CTRL"));
}

#[test]
fn test_policy_still_refuses_ordinary_names() {
    let mock = Mock::with_control().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let t = [DeleteTarget::program("FIREWORKS")];
    assert!(prepare_deletion(&mut p, &t, &control()).is_err());
    assert_eq!(eb_count(&mock), 0);
}

#[test]
fn user_policy_refuses_batches_protected_and_control_names() {
    let mock = Mock::with_control()
        .object(FileType::Program, "FIREWORKS", DEL_SRC)
        .object(FileType::Program, "HELLO", DEL_SRC);
    let mut p = mock.prime();
    let two = [
        DeleteTarget::program("FIREWORKS"),
        DeleteTarget::program("HELLO"),
    ];
    assert!(
        crate::prepare_deletion_with(&mut p, &two, &control(), NamePolicy::User, None).is_err()
    );
    let ctrl = [DeleteTarget::program("zzt_ctrl")];
    assert!(
        crate::prepare_deletion_with(&mut p, &ctrl, &control(), NamePolicy::User, None).is_err()
    );
    assert_eq!(eb_count(&mock), 0);
}

#[test]
fn ordinary_deletion_refuses_wrong_type_corrupt_crc_and_changed_content() {
    // A program named FIREWORKS is not a note named FIREWORKS.
    let mock = Mock::with_control().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let note = [DeleteTarget::note("FIREWORKS")];
    assert!(
        crate::prepare_deletion_with(&mut p, &note, &control(), NamePolicy::User, None).is_err()
    );

    // Corrupt CRC on the target.
    mock.st().corrupt.insert("FIREWORKS".into());
    let prog = [DeleteTarget::program("FIREWORKS")];
    assert!(
        crate::prepare_deletion_with(&mut p, &prog, &control(), NamePolicy::User, None).is_err()
    );
    mock.st().corrupt.clear();

    // Content changes after the backup was taken.
    let del = crate::prepare_deletion_with(&mut p, &prog, &control(), NamePolicy::User, None)
        .expect("prepare");
    let guard = authorize_usb_deletion(&p, &del, &control()).unwrap();
    let data = crate::mock_tests::utf16("CHANGED");
    mock.st()
        .objects
        .insert("FIREWORKS".into(), (FileType::Program, data));
    assert!(delete_via_usb(&mut p, guard, |_| {}).is_err());
    assert_eq!(eb_count(&mock), 0);
    assert!(mock.st().objects.contains_key("FIREWORKS"));
}

#[test]
fn ordinary_deletion_ignored_eb_is_still_present_and_never_resent() {
    let mock = Mock::with_control().object(FileType::Program, "FIREWORKS", DEL_SRC);
    mock.st().ignore_eb = true;
    let mut p = mock.prime();
    let t = [DeleteTarget::program("FIREWORKS")];
    let del = crate::prepare_deletion_with(&mut p, &t, &control(), NamePolicy::User, None).unwrap();
    let guard = authorize_usb_deletion(&p, &del, &control()).unwrap();
    let report = delete_via_usb(&mut p, guard, |_| {}).unwrap();
    assert_eq!(report.targets[0].status, TargetStatus::StillPresent);
    assert_eq!(eb_count(&mock), 1);
}

#[test]
fn ordinary_deletion_with_a_silent_control_is_inconclusive_before_any_eb() {
    let mock = Mock::new().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let t = [DeleteTarget::program("FIREWORKS")];
    assert!(matches!(
        crate::prepare_deletion_with(&mut p, &t, &control(), NamePolicy::User, None),
        Err(DeviceError::Inconclusive(_))
    ));
    assert_eq!(eb_count(&mock), 0);
}

// ---- control-free (settings anchor + FA/FF beat) deletion ----

fn anchored() -> Mock {
    Mock::new().object(FileType::Settings, "calc.settings", "settings")
}

#[test]
fn settings_anchors_are_a_closed_allowlist() {
    assert!(ControlObject::parse("settings:calc.settings").is_ok());
    assert!(ControlObject::parse("settings:cas.settings").is_ok());
    for bad in [
        "settings:evil",
        "settings:../calc.settings",
        "list:calc.settings",
        "program:calc.settings",
    ] {
        assert!(ControlObject::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn deletion_without_any_control_program_uses_the_settings_anchor() {
    let mock = anchored()
        .object(FileType::Program, "FIREWORKS", DEL_SRC)
        .object(FileType::Program, "HELLO", DEL_SRC);
    let mut p = mock.prime();
    let base = baseline(&mut p);
    let t = [DeleteTarget::program("FIREWORKS")];
    let del = crate::prepare_deletion_with(&mut p, &t, &anchor(), NamePolicy::User, Some(&base))
        .expect("prepare");
    let guard = authorize_usb_deletion(&p, &del, &anchor()).unwrap();
    let report = delete_via_usb(&mut p, guard, |_| {}).expect("delete");
    assert_eq!(report.targets[0].status, TargetStatus::ConfirmedDeleted);
    assert_eq!(eb_count(&mock), 1);
    let cmds = mock.cmds();
    assert!(cmds.contains(&crate::CMD_GET_INFO) && cmds.contains(&crate::CMD_CHECK_READY));
    assert!(
        cmds.iter().all(|c| matches!(*c, 0xF8 | 0xEB | 0xFA | 0xFF)),
        "{cmds:02X?}"
    );
    let st = mock.st();
    assert!(st.objects.contains_key("HELLO") && st.objects.contains_key("calc.settings"));
    assert!(!st.objects.contains_key("FIREWORKS"));
}

#[test]
fn missing_program_is_not_found_and_sends_no_eb() {
    let mock = anchored().object(FileType::Program, "HELLO", DEL_SRC);
    let mut p = mock.prime();
    let base = baseline(&mut p);
    let t = [DeleteTarget::program("FIREWORKS")];
    let err = crate::prepare_deletion_with(&mut p, &t, &anchor(), NamePolicy::User, Some(&base))
        .unwrap_err();
    assert!(matches!(err, DeviceError::Absent(_)), "{err}");
    assert_eq!(eb_count(&mock), 0);
}

#[test]
fn silent_anchor_or_silent_identity_never_counts_as_deletion() {
    // Anchor missing: silence proves nothing -> inconclusive before any EB.
    let mock = Mock::new().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let base = baseline(&mut p);
    let t = [DeleteTarget::program("FIREWORKS")];
    assert!(matches!(
        crate::prepare_deletion_with(&mut p, &t, &anchor(), NamePolicy::User, Some(&base)),
        Err(DeviceError::Inconclusive(_))
    ));
    assert_eq!(eb_count(&mock), 0);

    // Identity silent after the baseline: refused as unanswered, no EB.
    let mock = anchored().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let base = baseline(&mut p);
    mock.st().fa_reply = None;
    match crate::prepare_deletion_with(&mut p, &t, &anchor(), NamePolicy::User, Some(&base)) {
        Err(DeviceError::Inconclusive(m)) => assert!(m.contains("did not answer"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(eb_count(&mock), 0);
}

#[test]
fn device_swap_after_baseline_is_refused_before_eb() {
    let mock = anchored().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let base = baseline(&mut p);
    let t = [DeleteTarget::program("FIREWORKS")];
    let del =
        crate::prepare_deletion_with(&mut p, &t, &anchor(), NamePolicy::User, Some(&base)).unwrap();
    let guard = authorize_usb_deletion(&p, &del, &anchor()).unwrap();
    mock.st().fa_reply = Some(fa_raw("9999999999"));
    assert!(matches!(
        delete_via_usb(&mut p, guard, |_| {}),
        Err(DeviceError::Refused(_))
    ));
    assert_eq!(eb_count(&mock), 0);
    assert!(mock.st().objects.contains_key("FIREWORKS"));
}

#[test]
fn anchor_lost_after_eb_is_inconclusive_not_deleted() {
    let mock = anchored().object(FileType::Program, "FIREWORKS", DEL_SRC);
    let mut p = mock.prime();
    let base = baseline(&mut p);
    let t = [DeleteTarget::program("FIREWORKS")];
    let del =
        crate::prepare_deletion_with(&mut p, &t, &anchor(), NamePolicy::User, Some(&base)).unwrap();
    let guard = authorize_usb_deletion(&p, &del, &anchor()).unwrap();
    // The anchor answers the pre-send probes, then goes silent for good.
    mock.st().budget.insert("calc.settings".into(), 3);
    let report = delete_via_usb(&mut p, guard, |_| {}).unwrap();
    assert_eq!(report.targets[0].status, TargetStatus::Unknown);
    assert_eq!(eb_count(&mock), 1);
}

#[test]
fn screenshot_fetches_png_without_modifying_calculator_state() {
    let mock_png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xAA, 0xBB];
    let mock = Mock::new()
        .object(
            FileType::Program,
            "FIREWORKS",
            "EXPORT FIREWORKS() BEGIN END;",
        )
        .screenshot(mock_png.clone());
    let mut p = mock.prime();

    let bytes = p.screenshot(8).expect("screenshot succeeds");
    assert_eq!(bytes, mock_png);

    let cmds = mock.cmds();
    assert_eq!(cmds, vec![CMD_SCREENSHOT]);
    // Verifies no F7 writes, no EB deletes, no key presses, program retained intact
    assert!(mock.st().objects.contains_key("FIREWORKS"));
    assert_eq!(mock.st().uploaded.len(), 0);
}

#[test]
fn screenshot_timeout_reported_when_calculator_silent() {
    let mock = Mock::new(); // fc_reply is None, so calculator stays silent
    let mut p = mock.prime();

    let err = p
        .screenshot_timeout(8, Duration::from_millis(10))
        .unwrap_err();
    assert!(matches!(err, DeviceError::Timeout("screenshot")));
}

#[test]
fn screenshot_protocol_error_when_reply_lacks_png() {
    let non_png = vec![0x01, 0x02, 0x03, 0x04];
    let mock = Mock::new().screenshot(non_png);
    let mut p = mock.prime();

    let err = p.screenshot(8).unwrap_err();
    assert!(matches!(err, DeviceError::Protocol(_)));
}
