//! Filesystem-only process/thread contention tests. Never opens USB.
use ppl_programs::delete_store::Store;
use std::{path::Path, process::Command, sync::Barrier};

#[test]
fn child_lock_probe() {
    let Some(path) = std::env::var_os("PPL_TEST_JOURNAL_PATH") else {
        return;
    };
    let result = Store::new(Path::new(&path)).lock();
    let expect_busy = std::env::var_os("PPL_TEST_JOURNAL_BUSY").is_some();
    assert_eq!(
        result.is_err(),
        expect_busy,
        "unexpected lock outcome: {result:?}"
    );
}

fn probe(path: &Path, busy: bool) {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "child_lock_probe", "--nocapture"])
        .env("PPL_TEST_JOURNAL_PATH", path)
        .env_remove("PPL_TEST_JOURNAL_BUSY");
    if busy {
        child.env("PPL_TEST_JOURNAL_BUSY", "1");
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn independent_process_observes_contention_then_owner_release() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::new(directory.path());
    let guard = store.lock().unwrap();
    let error = store.lock().unwrap_err().to_string();
    assert!(error.contains("operation journal is locked"));
    assert!(error.contains("operations.lock") && error.contains("pid="));
    probe(directory.path(), true);
    drop(guard);
    probe(directory.path(), false);
    assert!(store.lock().is_ok());
}

#[test]
fn distinct_journals_remain_independent_under_parallel_ownership() {
    let directories: Vec<_> = (0..8).map(|_| tempfile::tempdir().unwrap()).collect();
    let barrier = Barrier::new(directories.len());
    std::thread::scope(|scope| {
        for directory in &directories {
            let barrier = &barrier;
            scope.spawn(move || {
                let store = Store::new(directory.path());
                let guard = store.lock().unwrap();
                barrier.wait();
                assert!(store.lock().is_err());
                drop(guard);
                assert!(store.lock().is_ok());
            });
        }
    });
}
