use super::*;

fn program(receive: bool) -> SyntheticProgram {
    let svc = if receive { 0xef19_0002u32 } else { 0xef19_0001 };
    let words = [svc, 0xe288_8001, 0xe584_8000, 0xeaff_fffe];
    let code: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    SyntheticProgram::new(&code, false).unwrap()
}
fn guest(p: &SyntheticProgram, receive: bool) -> ResumableGuest {
    let mut g = ResumableGuest::new(p, "event-port-v1", 50, 1).unwrap();
    g.set_register(0, if receive { DATA_BASE + 4 } else { 100 })
        .unwrap();
    g.set_register(1, if receive { 100 } else { 1 }).unwrap();
    g.set_register(4, DATA_BASE).unwrap();
    g
}
pub(super) fn interrupt_save(path: &Path) -> Result<()> {
    if std::env::var("PRIME_SYNTH_SVC_ROLE").as_deref() == Ok("save") {
        fs::write(path.with_extension("ready"), b"staging header written").map_err(string)?;
        loop {
            std::thread::park();
        }
    }
    Ok(())
}
pub(super) fn interrupt_restore() -> Result<()> {
    if std::env::var("PRIME_SYNTH_SVC_ROLE").as_deref() == Ok("restore") {
        let path = std::env::var("PRIME_SYNTH_SVC_PATH").map_err(string)?;
        fs::write(
            Path::new(&path).with_extension("ready"),
            b"private CPU restored before service rebinding",
        )
        .map_err(string)?;
        loop {
            std::thread::park();
        }
    }
    Ok(())
}

#[test]
fn actual_svc_traps_and_completes_outside_callback() {
    for receive in [false, true] {
        let p = program(receive);
        let mut g = guest(&p, receive);
        let ServiceStop::Waiting(ticket) = g.run_slice(4).unwrap() else {
            panic!("SVC did not wait");
        };
        assert_eq!(g.state().unwrap().registers[15], CODE_BASE + 4);
        assert_eq!(g.state().unwrap().instructions, 1);
        assert_eq!(g.run_slice(4).unwrap(), ServiceStop::Waiting(ticket));
        let snapshot = g.capture(&p).unwrap();
        drop(g);
        let mut restored = snapshot.restore(&p, "event-port-v1", 50, 2).unwrap();
        assert!(restored.deliver(ticket, event(), 50).is_err());
        let ticket = restored.pending().unwrap();
        restored
            .deliver(
                ticket,
                SyntheticEvent {
                    kind: 19,
                    value: 42,
                },
                50,
            )
            .unwrap();
        assert!(
            restored
                .deliver(
                    ticket,
                    SyntheticEvent {
                        kind: 19,
                        value: 99
                    },
                    50
                )
                .is_err()
        );
        restored.run_slice(2).unwrap();
        assert_eq!(restored.read_word(DATA_BASE).unwrap(), 1);
        if receive {
            assert_eq!(restored.read_word(DATA_BASE + 4).unwrap(), 19);
            assert_eq!(restored.read_word(DATA_BASE + 8).unwrap(), 42);
        }
    }
}

fn event() -> SyntheticEvent {
    SyntheticEvent {
        kind: 19,
        value: 42,
    }
}
fn uninterrupted(receive: bool) -> CpuState {
    let p = program(receive);
    let mut g = guest(&p, receive);
    let ServiceStop::Waiting(ticket) = g.run_slice(4).unwrap() else {
        panic!("not blocked")
    };
    g.deliver(ticket, event(), 50).unwrap();
    g.run_slice(2).unwrap();
    g.state().unwrap()
}

fn spawn_child(path: &Path, receive: bool, role: &str) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "synthetic_snapshot::resumable::tests::fresh_process_child",
            "--nocapture",
        ])
        .env("PRIME_SYNTH_SVC_PATH", path)
        .env("PRIME_SYNTH_SVC_RECEIVE", if receive { "1" } else { "0" })
        .env("PRIME_SYNTH_SVC_ROLE", role)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}
fn wait_child(mut child: std::process::Child) {
    let start = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > std::time::Duration::from_secs(20) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("service child timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fresh_process_child() {
    let Ok(path) = std::env::var("PRIME_SYNTH_SVC_PATH") else {
        return;
    };
    let receive = std::env::var("PRIME_SYNTH_SVC_RECEIVE").as_deref() == Ok("1");
    let p = program(receive);
    let snapshot = ResumableSnapshot::load(Path::new(&path), &p).unwrap();
    let mut g = snapshot.restore(&p, "event-port-v1", 50, 2).unwrap();
    if std::env::var("PRIME_SYNTH_SVC_ROLE").as_deref() == Ok("save") {
        g.capture(&p).unwrap().save(Path::new(&path), &p).unwrap();
        panic!("interrupted save unexpectedly completed");
    }
    let ticket = g.pending().expect("pending operation lost");
    assert_eq!(g.state().unwrap().instructions, 1);
    g.deliver(ticket, event(), 50).unwrap();
    g.run_slice(2).unwrap();
    assert_eq!(g.state().unwrap(), uninterrupted(receive));
    assert_eq!(g.read_word(DATA_BASE).unwrap(), 1);
    assert!(g.deliver(ticket, event(), 50).is_err());
    if receive {
        assert_eq!(g.read_word(DATA_BASE + 4).unwrap(), 19);
        assert_eq!(g.read_word(DATA_BASE + 8).unwrap(), 42);
    }
}

#[test]
fn wait_and_receive_continue_in_new_process_without_native_continuation() {
    for receive in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("svc-state");
        let p = program(receive);
        let mut g = guest(&p, receive);
        assert!(matches!(g.run_slice(4).unwrap(), ServiceStop::Waiting(_)));
        let snapshot = g.capture(&p).unwrap();
        eprintln!(
            "synthetic SVC fresh-process receive={receive} cpu=ARM926 unicorn={} program={} post_svc_pc={:#x} instructions={} format=HPUSVC01",
            runtime_version(),
            p.identity(),
            g.state().unwrap().registers[15],
            g.state().unwrap().instructions
        );
        snapshot.save(&path, &p).unwrap();
        drop(g);
        wait_child(spawn_child(&path, receive, "resume"));
    }
}

#[test]
fn wait_preserves_one_logical_event_for_a_later_get_event() {
    let words = [
        0xef19_0001u32,
        0xe3a0_0064,
        0xe3a0_1001,
        0xef19_0001,
        0xe1a0_0005,
        0xe3a0_1064,
        0xef19_0002,
        0xe288_8001,
        0xe584_8000,
        0xeaff_fffe,
    ];
    let code: Vec<_> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let p = SyntheticProgram::new(&code, false).unwrap();
    let mut g = guest(&p, false);
    g.set_register(5, DATA_BASE + 4).unwrap();
    let ServiceStop::Waiting(ticket) = g.run_slice(4).unwrap() else {
        panic!("wait")
    };
    g.deliver(ticket, event(), 50).unwrap();
    let snapshot = g.capture(&p).unwrap();
    drop(g);
    let mut restored = snapshot.restore(&p, "event-port-v1", 50, 2).unwrap();
    assert_eq!(restored.run_slice(4).unwrap(), ServiceStop::Runnable);
    assert!(restored.pending().is_none());
    assert!(
        restored.services.queued.is_some(),
        "repeated wait must preserve its event"
    );
    assert_eq!(restored.run_slice(4).unwrap(), ServiceStop::Runnable);
    restored.run_slice(2).unwrap();
    assert_eq!(restored.read_word(DATA_BASE).unwrap(), 1);
    assert_eq!(restored.read_word(DATA_BASE + 4).unwrap(), 19);
    assert_eq!(restored.read_word(DATA_BASE + 8).unwrap(), 42);
    assert_eq!(restored.services.completed, 3);
    assert!(restored.services.queued.is_none());
}

#[test]
fn stale_double_completion_and_timer_disagreement_are_rejected() {
    let p = program(true);
    let mut g = guest(&p, true);
    let ServiceStop::Waiting(ticket) = g.run_slice(4).unwrap() else {
        panic!("wait")
    };
    assert!(
        g.deliver(
            WaitTicket {
                sequence: ticket.sequence + 1,
                binding_epoch: ticket.binding_epoch
            },
            event(),
            50
        )
        .is_err()
    );
    assert!(g.deliver(ticket, event(), 51).is_err());
    assert!(g.set_register(0, 0).is_err());
    let snapshot = g.capture(&p).unwrap();
    assert!(snapshot.restore(&p, "wrong-resource", 50, 2).is_err());
    assert!(snapshot.restore(&p, "event-port-v1", 51, 2).is_err());
    assert!(snapshot.restore(&p, "event-port-v1", 50, 1).is_err());
    let mut restored = snapshot.restore(&p, "event-port-v1", 50, 2).unwrap();
    assert!(!restored.advance_time(149).unwrap());
    let timed = restored.capture(&p).unwrap();
    let mut restored = timed.restore(&p, "event-port-v1", 149, 3).unwrap();
    assert!(restored.advance_time(150).unwrap());
    assert_eq!(restored.state().unwrap().registers[0], 0);
    assert!(restored.deliver(ticket, event(), 150).is_err());
    assert!(restored.advance_time(149).is_err());
    restored.run_slice(2).unwrap();
    assert_eq!(restored.read_word(DATA_BASE).unwrap(), 1);
    assert_eq!(restored.read_word(DATA_BASE + 4).unwrap(), 0);
}

#[test]
fn unsupported_service_context_and_invalid_pending_state_fail_closed() {
    let unsupported = SyntheticProgram::new(&0xef19_0099u32.to_le_bytes(), false).unwrap();
    let mut g = guest(&unsupported, false);
    assert!(g.run_slice(1).is_err());
    assert!(g.capture(&unsupported).is_err());
    let p = program(true);
    let mut g = guest(&p, true);
    g.set_register(0, DATA_BASE + REGION_BYTES as u32 - 4)
        .unwrap();
    assert!(g.run_slice(1).is_err());
    assert!(g.capture(&p).is_err());
    for change in 0..7 {
        let mut g = guest(&p, true);
        g.run_slice(1).unwrap();
        let mut snapshot = g.capture(&p).unwrap();
        let pending = snapshot.services.pending.as_mut().unwrap();
        match change {
            0 => pending.operation = 0x190099,
            1 => pending.return_pc += 4,
            2 => pending.arguments[0] += 4,
            3 => pending.deadline = 50,
            4 => snapshot.services.next_ticket += 1,
            5 => snapshot.services.queued = Some(event()),
            _ => pending.deadline += 1,
        }
        assert!(snapshot.restore(&p, "event-port-v1", 50, 2).is_err());
    }
}

#[test]
fn separate_file_identity_corruption_and_restore_failures_preserve_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("svc");
    let p = program(true);
    let mut g = guest(&p, true);
    g.run_slice(1).unwrap();
    let snapshot = g.capture(&p).unwrap();
    snapshot.save(&path, &p).unwrap();
    let original = fs::read(&path).unwrap();
    assert!(SyntheticSnapshot::load(&path, &p).is_err());
    assert!(ResumableSnapshot::load(&path, &program(false)).is_err());
    assert!(ResumableSnapshot::load(&path.with_extension("missing"), &p).is_err());
    assert!(snapshot.save(&path.join("missing"), &p).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    for offset in [0, 8, 32, original.len() - 1] {
        let mut broken = original.clone();
        broken[offset] ^= 1;
        fs::write(&path, broken).unwrap();
        assert!(ResumableSnapshot::load(&path, &p).is_err());
    }
    fs::write(&path, &original[..original.len() - 1]).unwrap();
    assert!(ResumableSnapshot::load(&path, &p).is_err());
    fs::write(&path, original).unwrap();
    let loaded = ResumableSnapshot::load(&path, &p).unwrap();
    assert_eq!(loaded.pending(), snapshot.pending());
    #[cfg(unix)]
    {
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(ResumableSnapshot::load(&link, &p).is_err());
    }
}

#[test]
fn terminated_save_or_restore_cannot_partially_publish_or_consume_checkpoint() {
    for role in ["save", "restore"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("svc");
        let p = program(true);
        let mut g = guest(&p, true);
        g.run_slice(1).unwrap();
        g.capture(&p).unwrap().save(&path, &p).unwrap();
        let original = fs::read(&path).unwrap();
        drop(g);
        let mut child = spawn_child(&path, true, role);
        let ready = path.with_extension("ready");
        let start = std::time::Instant::now();
        while !ready.exists() && start.elapsed() < std::time::Duration::from_secs(10) {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(ready.exists(), "child missed {role} boundary");
        assert_eq!(fs::read(&path).unwrap(), original);
        wait_child(spawn_child(&path, true, "resume"));
    }
}
