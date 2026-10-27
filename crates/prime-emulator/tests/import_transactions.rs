//! Regressions for reproduced host-side data loss, independent of firmware.
use prime_emulator::{
    import::{self, ImportRequest},
    storage,
};

fn setup() -> (tempfile::TempDir, ImportRequest) {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("data/C/DATA")).unwrap();
    let source = t.path().join("HELLO.ppl");
    std::fs::write(&source, "EXPORT HELLO() BEGIN RETURN 1; END;\n").unwrap();
    let req = ImportRequest {
        program: source,
        data_dir: t.path().join("data"),
        name: None,
        replace: false,
        as_source: false,
    };
    (t, req)
}

#[cfg(unix)]
#[test]
fn stale_temp_symlink_cleanup_preserves_its_target() {
    use std::os::unix::fs::symlink;
    let (t, req) = setup();
    let unrelated = t.path().join("unrelated");
    std::fs::write(&unrelated, b"must survive").unwrap();
    let old_temp = req.data_dir.join(format!(
        "C/DATA/{}HELLO-{}.tmp",
        import::TEMP_PREFIX,
        std::process::id()
    ));
    symlink(&unrelated, &old_temp).unwrap();
    import::import(&req).unwrap();
    assert_eq!(std::fs::read(&unrelated).unwrap(), b"must survive");
}

#[test]
fn ambiguous_case_aliases_are_refused_without_deleting_either_program() {
    let (_t, mut req) = setup();
    let dir = req.data_dir.join("C/DATA");
    std::fs::write(dir.join("HELLO.hpprgm"), b"first").unwrap();
    std::fs::write(dir.join("hello.HPPRGM"), b"second").unwrap();
    if std::fs::read(dir.join("HELLO.hpprgm")).unwrap() == b"second" {
        return;
    }
    let before = storage::snapshot(&req.data_dir);
    req.replace = true;
    assert!(
        import::import(&req).is_err(),
        "ambiguous identity must not pick an arbitrary file"
    );
    assert_eq!(storage::snapshot(&req.data_dir), before);
}

#[test]
fn concurrent_imports_never_remove_or_overwrite_the_winner() {
    let (_t, req) = setup();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(12));
    let threads: Vec<_> = (0..12)
        .map(|i| {
            let mut req = req.clone();
            req.name = Some(if i % 2 == 0 { "HELLO" } else { "hello" }.into());
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                import::import(&req)
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(import::list_programs(&req.data_dir).len(), 1);
    assert!(!req.data_dir.join(".prime-import-lock").exists());
}

#[test]
fn existing_profile_lock_fails_closed_without_touching_programs() {
    let (_t, req) = setup();
    let lock = req.data_dir.join(".prime-import-lock");
    std::fs::create_dir(&lock).unwrap();
    let before = storage::snapshot(&req.data_dir);
    assert!(matches!(
        import::import(&req),
        Err(import::ImportError::Busy(_))
    ));
    assert_eq!(before, storage::snapshot(&req.data_dir));
    // Explicit recovery after the owner stopped; never guessed stale.
    std::fs::remove_dir(&lock).unwrap();
    assert!(import::import(&req).unwrap().other_files_unchanged);
}

#[test]
fn separate_process_imports_respect_case_insensitive_no_replace() {
    let (_t, req) = setup();
    let mut children = Vec::new();
    for i in 0..8 {
        children.push(
            std::process::Command::new(env!("CARGO_BIN_EXE_prime-emulator"))
                .arg("import")
                .arg("--program")
                .arg(&req.program)
                .arg("--data-dir")
                .arg(&req.data_dir)
                .arg("--name")
                .arg(if i % 2 == 0 { "HELLO" } else { "hello" })
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    let successes = children
        .into_iter()
        .filter_map(|mut c| c.wait().unwrap().success().then_some(()))
        .count();
    assert_eq!(successes, 1);
    assert_eq!(import::list_programs(&req.data_dir).len(), 1);
}
