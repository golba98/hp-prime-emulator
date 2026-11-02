use super::*;
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn program(thumb: bool) -> SyntheticProgram {
    let bytes = if thumb {
        [
            0x2011u16, 0x6020, 0xb401, 0x2811, 0xd103, 0xbc01, 0x3007, 0x6060, 0x3205, 0xe7fe,
        ]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect::<Vec<_>>()
    } else {
        [
            0xe3a0_1011u32,
            0xe588_1000,
            0xe52d_1004,
            0xe351_0011,
            0x0282_2005,
            0xe49d_3004,
            0xe283_0007,
            0xe588_0004,
            0xeaff_fffe,
        ]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect::<Vec<_>>()
    };
    SyntheticProgram::new(&bytes, thumb).unwrap()
}
fn guest(program: &SyntheticProgram, thumb: bool) -> SyntheticGuest {
    let mut g = SyntheticGuest::new(program).unwrap();
    g.set_register(if thumb { 4 } else { 8 }, DATA_BASE)
        .unwrap();
    g
}
fn assert_finished(g: &SyntheticGuest) {
    let s = g.state().unwrap();
    assert_eq!(s.registers[0], 24);
    assert_eq!(
        s.registers[2], 5,
        "conditional suffix executed exactly once"
    );
    assert_eq!(s.registers[13], STACK_BASE + REGION_BYTES as u32);
    assert_eq!(g.read_word(DATA_BASE).unwrap(), 17);
    assert_eq!(g.read_word(DATA_BASE + 4).unwrap(), 24);
}
fn bounded_child(path: &Path, thumb: bool, role: &str) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "synthetic_snapshot::tests::fresh_process_child",
            "--nocapture",
        ])
        .env("PRIME_SYNTHETIC_CHILD", path)
        .env("PRIME_SYNTHETIC_ROLE", role)
        .env("PRIME_SYNTHETIC_THUMB", if thumb { "1" } else { "0" })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let at = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if at.elapsed() > Duration::from_secs(20) {
            child.kill().unwrap();
            panic!("synthetic fresh-process continuation timed out");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "child: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn fresh_process_child() {
    let Some(path) = std::env::var_os("PRIME_SYNTHETIC_CHILD") else {
        return;
    };
    let role = std::env::var("PRIME_SYNTHETIC_ROLE").unwrap();
    if role == "staging" {
        let path = Path::new(&path);
        let p = program(false);
        let snapshot = SyntheticSnapshot::load(path, &p).unwrap();
        snapshot.save(path, &p).unwrap();
        panic!("staging fault injector returned unexpectedly");
    }
    let thumb = std::env::var("PRIME_SYNTHETIC_THUMB").unwrap() == "1";
    let p = program(thumb);
    if role == "rotation" {
        let mut store = SyntheticSnapshotStore::new(Path::new(&path)).unwrap();
        let selected = store.load(&p).unwrap();
        let mut g = selected.snapshot().restore(&p).unwrap();
        g.run_slice(1).unwrap();
        store.save(&g.capture(&p).unwrap(), &p).unwrap();
        panic!("rotation fault injector returned unexpectedly");
    }
    let snap = if role == "recovery" {
        let store = SyntheticSnapshotStore::new(Path::new(&path)).unwrap();
        match store.load(&p).unwrap() {
            SnapshotSelection::Previous {
                snapshot,
                current_error,
            } => {
                assert!(!current_error.is_empty());
                snapshot
            }
            SnapshotSelection::Current(_) => panic!("corrupt current unexpectedly selected"),
        }
    } else {
        SyntheticSnapshot::load(Path::new(&path), &p).unwrap()
    };
    if role == "descriptors" {
        let count = || fs::read_dir("/proc/self/fd").unwrap().count();
        let before = count();
        for _ in 0..32 {
            let mut g = snap.restore(&p).unwrap();
            g.run_slice(4).unwrap();
            assert_finished(&g);
        }
        assert_eq!(count(), before);
        return;
    }
    assert_eq!(snap.state().instructions, 4);
    assert_ne!(
        snap.state().cpsr & (1 << 30),
        0,
        "NZCV preserved before conditional branch"
    );
    assert_eq!(snap.state().cpsr & 0x20 != 0, thumb);
    assert_eq!(
        snap.state().registers[13],
        STACK_BASE + REGION_BYTES as u32 - 4
    );
    let mut g = snap.restore(&p).unwrap();
    g.run_slice(if thumb { 5 } else { 4 }).unwrap();
    assert_finished(&g);
    let mut uninterrupted = guest(&p, thumb);
    uninterrupted.run_slice(if thumb { 9 } else { 8 }).unwrap();
    assert_eq!(g.state().unwrap(), uninterrupted.state().unwrap());
}
#[test]
fn arm_and_thumb_continue_in_fresh_process_with_flags_and_stack() {
    let dir = tempfile::tempdir().unwrap();
    for thumb in [false, true] {
        let p = program(thumb);
        let mut g = guest(&p, thumb);
        g.run_slice(4).unwrap();
        let snapshot = g.capture(&p).unwrap();
        let file = dir
            .path()
            .join(if thumb { "thumb.snap" } else { "arm.snap" });
        snapshot.save(&file, &p).unwrap();
        drop(g);
        bounded_child(&file, thumb, "resume");
    }
}
#[test]
fn boundaries_and_repeated_cycles_equal_uninterrupted_execution() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state");
    for thumb in [false, true] {
        let p = program(thumb);
        let total = if thumb { 9 } else { 8 };
        let mut continuous = guest(&p, thumb);
        continuous.run_slice(total).unwrap();
        for boundary in 1..total {
            let mut g = guest(&p, thumb);
            g.run_slice(boundary).unwrap();
            for _ in 0..4 {
                g.capture(&p).unwrap().save(&file, &p).unwrap();
                drop(g);
                g = SyntheticSnapshot::load(&file, &p)
                    .unwrap()
                    .restore(&p)
                    .unwrap();
            }
            g.run_slice(total - boundary).unwrap();
            assert_eq!(g.state().unwrap(), continuous.state().unwrap());
            assert_finished(&g);
        }
    }
}
#[test]
fn unsupported_instructions_and_faults_poison_capture() {
    let cases: &[(&[u8], bool)] = &[
        (&0xef00_0000u32.to_le_bytes(), false), // SVC
        (&0xee01_0f10u32.to_le_bytes(), false), // MCR CP15
        (&0xe10f_0000u32.to_le_bytes(), false), // MRS
        (&0xe12f_ff30u32.to_le_bytes(), false), // BLX
        (&0xe581_0000u32.to_le_bytes(), false), // unmapped STR
        (&0xdf00u16.to_le_bytes(), true),       // SVC
        (&0x4780u16.to_le_bytes(), true),       // BLX
        (&0xbc00u16.to_le_bytes(), true),       // empty POP (unpredictable)
    ];
    for &(bytes, thumb) in cases {
        let p = SyntheticProgram::new(bytes, thumb).unwrap();
        let mut g = SyntheticGuest::new(&p).unwrap();
        assert!(g.run_slice(1).is_err(), "accepted {bytes:x?}");
        assert!(g.capture(&p).is_err());
        assert!(g.run_slice(1).is_err());
    }
    // Code is RX: even an ordinary supported store cannot alter the program.
    let p = SyntheticProgram::new(&0xe581_0000u32.to_le_bytes(), false).unwrap();
    let mut g = SyntheticGuest::new(&p).unwrap();
    g.set_register(1, CODE_BASE).unwrap();
    assert!(g.run_slice(1).is_err());
    assert!(g.capture(&p).is_err());
}
#[test]
fn validates_resource_state_layout_and_backend_before_restore() {
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(4).unwrap();
    let changes: &[fn(&mut Manifest)] = &[
        |m| m.kind = "calculator".into(),
        |m| m.version += 1,
        |m| m.calculator_live_resume = true,
        |m| m.cpu = "ARM1176".into(),
        |m| m.unicorn_version ^= 1,
        |m| m.resource_identity = "wrong".into(),
        |m| m.regions[1].0 = CODE_BASE,
        |m| m.regions[0].2 = "rwx".into(),
        |m| m.state.cpsr = 0x13,
        |m| m.state.cpsr |= 1 << 9,
        |m| m.state.cpsr |= 1 << 27,
        |m| m.state.registers[15] = DATA_BASE,
        |m| m.state.registers[15] += 2,
        |m| m.state.registers[13] = 0,
        |m| m.state.registers[13] += 1,
    ];
    for change in changes {
        let mut snapshot = g.capture(&p).unwrap();
        change(&mut snapshot.manifest);
        snapshot.manifest.manifest_sha256 = snapshot.manifest.digest().unwrap();
        assert!(snapshot.restore(&p).is_err());
    }
    let mut other = program(false);
    other.code[0] ^= 1;
    assert!(g.capture(&other).is_err());
    let other = SyntheticProgram::new(&other.code, false).unwrap();
    assert!(g.capture(&p).unwrap().restore(&other).is_err());
}
#[test]
fn invalid_files_and_partial_staging_preserve_the_good_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("good");
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(4).unwrap();
    g.capture(&p).unwrap().save(&path, &p).unwrap();
    let original = fs::read(&path).unwrap();
    let broken = dir.path().join("broken");
    for count in [0, 7, 16, original.len() - 1] {
        fs::write(&broken, &original[..count]).unwrap();
        assert!(SyntheticSnapshot::load(&broken, &p).is_err());
    }
    let mut bytes = original.clone();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&broken, &bytes).unwrap();
    assert!(SyntheticSnapshot::load(&broken, &p).is_err());
    let mut bytes = original.clone();
    bytes[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    fs::write(&broken, &bytes).unwrap();
    assert!(SyntheticSnapshot::load(&broken, &p).is_err());
    let mut bytes = original.clone();
    bytes.push(0);
    fs::write(&broken, &bytes).unwrap();
    assert!(SyntheticSnapshot::load(&broken, &p).is_err());
    let mut staging = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    staging.write_all(b"HPUSYN01partial").unwrap();
    drop(staging);
    assert_eq!(fs::read(&path).unwrap(), original);
    bounded_child(&path, false, "resume");
}
#[cfg(unix)]
#[test]
fn symlink_and_fifo_inputs_are_refused_before_open() {
    let dir = tempfile::tempdir().unwrap();
    let p = program(false);
    let file = dir.path().join("file");
    fs::write(&file, []).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&file, &link).unwrap();
    assert!(SyntheticSnapshot::load(&link, &p).is_err());
    let fifo = dir.path().join("fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    assert!(SyntheticSnapshot::load(&fifo, &p).is_err());
}
#[test]
fn limits_and_host_register_admission_are_explicit() {
    assert!(SyntheticProgram::new(&[], false).is_err());
    assert!(SyntheticProgram::new(&[1], true).is_err());
    assert!(SyntheticProgram::new(&vec![0; REGION_BYTES + 4], false).is_err());
    let p = program(false);
    let mut g = guest(&p, false);
    assert!(g.run_slice(0).is_err());
    assert!(g.run_slice(MAX_SLICE + 1).is_err());
    assert!(g.set_register(15, CODE_BASE).is_err());
    g.set_register(13, 0).unwrap();
    assert!(g.capture(&p).is_err());
    assert!(g.run_slice(1).is_err());
}
#[cfg(target_os = "linux")]
#[test]
fn repeated_restore_does_not_leak_descriptors() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state");
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(4).unwrap();
    g.capture(&p).unwrap().save(&file, &p).unwrap();
    bounded_child(&file, false, "descriptors");
}
#[test]
fn terminated_staging_writer_cannot_damage_valid_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(4).unwrap();
    g.capture(&p).unwrap().save(&path, &p).unwrap();
    let original = fs::read(&path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "synthetic_snapshot::tests::fresh_process_child"])
        .env("PRIME_SYNTHETIC_CHILD", &path)
        .env("PRIME_SYNTHETIC_ROLE", "staging")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ready = path.with_extension("ready");
    let at = Instant::now();
    while !ready.exists() && at.elapsed() < Duration::from_secs(10) {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        ready.exists(),
        "child never reached interrupted-write stage"
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    bounded_child(&path, false, "resume");
}

// Only present in the test binary. The child runs the real save path, pauses
// after its bounded header reaches the private staging file, and is killed.
pub(super) fn interrupt_staged_save(path: &Path) -> Result<()> {
    if std::env::var("PRIME_SYNTHETIC_ROLE").as_deref() == Ok("staging") {
        fs::write(path.with_extension("ready"), b"save header staged").map_err(string)?;
        loop {
            std::thread::park();
        }
    }
    Ok(())
}

// Only the isolated child pauses here: backup has been synchronized and the
// primary still contains the old valid generation when the parent kills it.
pub(super) fn interrupt_rotated_save(path: &Path) -> Result<()> {
    if std::env::var("PRIME_SYNTHETIC_ROLE").as_deref() == Ok("rotation") {
        fs::write(path.with_extension("ready"), b"previous generation durable").map_err(string)?;
        loop {
            std::thread::park();
        }
    }
    Ok(())
}

#[test]
fn current_previous_rotation_and_corrupt_fallback_resume_in_fresh_child() {
    let dir = tempfile::tempdir().unwrap();
    let p = program(false);
    let mut g = guest(&p, false);
    let path = dir.path().join("state");
    let mut store = SyntheticSnapshotStore::new(&path).unwrap();
    g.run_slice(3).unwrap();
    store.save(&g.capture(&p).unwrap(), &p).unwrap();
    g.run_slice(1).unwrap();
    let fourth = g.capture(&p).unwrap();
    store.save(&fourth, &p).unwrap();
    let previous = fs::read(store.previous_path()).unwrap();
    store.save(&fourth, &p).unwrap();
    assert_eq!(
        fs::read(store.previous_path()).unwrap(),
        previous,
        "unchanged save retains older backup"
    );
    g.run_slice(1).unwrap();
    store.save(&g.capture(&p).unwrap(), &p).unwrap();
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        2,
        "only two published generations"
    );
    assert!(matches!(
        store.load(&p).unwrap(),
        SnapshotSelection::Current(_)
    ));
    assert_eq!(store.load(&p).unwrap().snapshot().state().instructions, 5);
    fs::write(&path, b"interrupted/corrupt current").unwrap();
    bounded_child(&path, false, "recovery");
    assert_eq!(
        fs::read(&path).unwrap(),
        b"interrupted/corrupt current",
        "loading never silently promotes"
    );
}

#[test]
fn invalid_current_never_replaces_valid_backup_and_wrong_program_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(4).unwrap();
    let fourth = g.capture(&p).unwrap();
    let mut store = SyntheticSnapshotStore::new(&path).unwrap();
    store.save(&fourth, &p).unwrap();
    g.run_slice(1).unwrap();
    let fifth = g.capture(&p).unwrap();
    store.save(&fifth, &p).unwrap();
    let good_previous = fs::read(store.previous_path()).unwrap();
    fs::write(&path, b"bad current").unwrap();
    store.save(&fifth, &p).unwrap();
    assert_eq!(fs::read(store.previous_path()).unwrap(), good_previous);
    assert!(matches!(
        store.load(&p).unwrap(),
        SnapshotSelection::Current(_)
    ));
    let other = program(true);
    assert!(store.load(&other).is_err());
    assert!(store.save(&fifth, &other).is_err());
    assert_eq!(fs::read(store.previous_path()).unwrap(), good_previous);
    // An incompatible (but intrinsically valid) current is not a candidate
    // for promoting over this program's compatible prior generation either.
    let mut other_guest = guest(&other, true);
    other_guest.run_slice(4).unwrap();
    other_guest
        .capture(&other)
        .unwrap()
        .save(&path, &other)
        .unwrap();
    assert!(matches!(
        store.load(&p).unwrap(),
        SnapshotSelection::Previous { .. }
    ));
    store.save(&fifth, &p).unwrap();
    assert_eq!(fs::read(store.previous_path()).unwrap(), good_previous);
    fs::write(&path, b"bad current").unwrap();
    fs::write(store.previous_path(), b"bad previous").unwrap();
    let error = store.load(&p).err().unwrap();
    assert!(error.contains("current:") && error.contains("previous:"));
}

#[test]
fn interrupted_rotation_preserves_current_and_validated_fresh_child_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(3).unwrap();
    let mut store = SyntheticSnapshotStore::new(&path).unwrap();
    store.save(&g.capture(&p).unwrap(), &p).unwrap();
    g.run_slice(1).unwrap();
    store.save(&g.capture(&p).unwrap(), &p).unwrap();
    let old_current = fs::read(&path).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "synthetic_snapshot::tests::fresh_process_child"])
        .env("PRIME_SYNTHETIC_CHILD", &path)
        .env("PRIME_SYNTHETIC_THUMB", "0")
        .env("PRIME_SYNTHETIC_ROLE", "rotation")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ready = path.with_extension("ready");
    let at = Instant::now();
    while !ready.exists() && at.elapsed() < Duration::from_secs(10) {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        ready.exists(),
        "child never reached durable-backup boundary"
    );
    assert_eq!(fs::read(&path).unwrap(), old_current);
    assert_eq!(fs::read(store.previous_path()).unwrap(), old_current);
    fs::write(&path, b"corrupt primary after interrupted rotation").unwrap();
    bounded_child(&path, false, "recovery");
}

#[test]
fn backup_publication_failure_preserves_the_current_generation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state");
    let p = program(false);
    let mut g = guest(&p, false);
    g.run_slice(4).unwrap();
    let mut store = SyntheticSnapshotStore::new(&path).unwrap();
    store.save(&g.capture(&p).unwrap(), &p).unwrap();
    let original = fs::read(&path).unwrap();
    fs::create_dir(store.previous_path()).unwrap();
    g.run_slice(1).unwrap();
    assert!(store.save(&g.capture(&p).unwrap(), &p).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(matches!(
        store.load(&p).unwrap(),
        SnapshotSelection::Current(_)
    ));
}
