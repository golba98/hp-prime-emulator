//! Complete portable storage restoration through production archive/VFS code.
use prime_emulator::{
    services::fs::Vfs,
    storage::image::{CaptureProgress, Kind, StorageImage},
};
use std::{
    fs::{self, FileTimes},
    path::Path,
    time::{Duration, UNIX_EPOCH},
};
const ID: &str = "storage-tree-test-v1";
fn source(root: &Path) {
    fs::create_dir_all(root.join("A/WINDOW/SYSTEM")).unwrap();
    fs::create_dir_all(root.join("C/DATA/empty")).unwrap();
    fs::write(root.join("A/WINDOW/SYSTEM/test.dll"), [0, 1, 2, 255]).unwrap();
    fs::write(
        root.join("C/DATA/program.hpprgm"),
        "EXPORT TEST() BEGIN RETURN \"π 😀\"; END;",
    )
    .unwrap();
    fs::write(root.join("C/DATA/settings"), [0, 255, 0, 65, 0, 0]).unwrap();
    // A sibling with punctuation sorts before this directory's children.
    fs::create_dir_all(root.join("C/DATA/a")).unwrap();
    fs::write(root.join("C/DATA/a/child"), b"child").unwrap();
    fs::hard_link(root.join("C/DATA/a/child"), root.join("C/DATA/a.backup")).unwrap();
    fs::create_dir_all(root.join(".recovery/host-bookkeeping")).unwrap();
    fs::write(
        root.join(".recovery/host-bookkeeping/skip"),
        b"not a guest drive",
    )
    .unwrap();
}
#[test]
fn complete_drive_tree_restores_unopened_files_empty_directories_metadata_and_links() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("source");
    source(&original);
    let settings = original.join("C/DATA/settings");
    let modified = UNIX_EPOCH + Duration::new(1_600_000_000, 123_456_789);
    fs::File::options()
        .write(true)
        .open(&settings)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o640)).unwrap();
    }
    fs::hard_link(&settings, original.join("C/DATA/linked-settings")).unwrap();
    let empty = original.join("C/DATA/empty");
    #[cfg(unix)]
    fs::File::open(&empty)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    let archive = root.path().join("tree.bin");
    StorageImage::capture(&original, &archive, ID).unwrap();
    let mut image = StorageImage::open(&archive, ID).unwrap();
    assert_eq!(
        image
            .manifest()
            .entries
            .iter()
            .filter(|entry| matches!(entry.object, Kind::HardLink { .. }))
            .count(),
        2
    );
    assert!(
        !image
            .manifest()
            .entries
            .iter()
            .any(|entry| entry.path.starts_with('.'))
    );
    fs::write(&settings, b"source changed").unwrap();
    let restored = image.restore(root.path()).unwrap();
    let actual = restored.path().join("C/DATA/settings");
    assert_eq!(fs::read(&actual).unwrap(), [0, 255, 0, 65, 0, 0]);
    assert_eq!(fs::metadata(&actual).unwrap().modified().unwrap(), modified);
    assert!(restored.path().join("C/DATA/empty").is_dir());
    assert_eq!(
        fs::read_to_string(restored.path().join("C/DATA/program.hpprgm")).unwrap(),
        "EXPORT TEST() BEGIN RETURN \"π 😀\"; END;"
    );
    assert_eq!(
        fs::read(restored.path().join("A/WINDOW/SYSTEM/test.dll")).unwrap(),
        [0, 1, 2, 255]
    );
    assert!(
        same_file::is_same_file(&actual, restored.path().join("C/DATA/linked-settings")).unwrap()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&actual).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            fs::metadata(restored.path().join("C/DATA/empty"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
    }
    let vfs = Vfs::new(restored.path().into());
    let h = vfs.open("C:/DATA/linked-settings", "r+").unwrap();
    assert_eq!(vfs.write(h, b"X"), Some(1));
    assert_eq!(fs::read(&actual).unwrap(), b"X\xff\0A\0\0");
    assert_eq!(fs::read(&settings).unwrap(), b"source changed");
    restored.close().unwrap();
}
#[test]
fn invalid_identity_corruption_truncation_and_trailing_bytes_are_refused() {
    let root = tempfile::tempdir().unwrap();
    source(&root.path().join("source"));
    let archive = root.path().join("tree.bin");
    StorageImage::capture(&root.path().join("source"), &archive, ID).unwrap();
    assert!(StorageImage::open(&archive, "other firmware").is_err());
    let bytes = fs::read(&archive).unwrap();
    for candidate in [
        bytes[..bytes.len() - 1].to_vec(),
        [bytes.clone(), vec![0]].concat(),
        {
            let mut corrupt = bytes.clone();
            *corrupt.last_mut().unwrap() ^= 1;
            corrupt
        },
    ] {
        fs::write(&archive, candidate).unwrap();
        assert!(StorageImage::open(&archive, ID).is_err());
    }
    fs::write(&archive, &bytes).unwrap();
    let mut image = StorageImage::open(&archive, ID).unwrap();
    let before = fs::read_dir(root.path()).unwrap().count();
    let mut corrupt = bytes;
    *corrupt.last_mut().unwrap() ^= 1;
    fs::write(&archive, corrupt).unwrap();
    assert!(image.restore(root.path()).is_err());
    assert_eq!(
        fs::read_dir(root.path()).unwrap().count(),
        before,
        "partial restored tree must not escape staging"
    );
}
#[test]
fn refused_capture_keeps_the_previous_image_and_never_writes_inside_a_drive() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("source");
    source(&original);
    let archive = root.path().join("tree.bin");
    StorageImage::capture(&original, &archive, ID).unwrap();
    let before = fs::read(&archive).unwrap();
    assert!(StorageImage::capture(&original, &original.join("C/DATA/snapshot.bin"), ID).is_err());
    assert!(!original.join("C/DATA/snapshot.bin").exists());
    assert!(StorageImage::capture(&original, &archive, "").is_err());
    assert_eq!(fs::read(&archive).unwrap(), before);
    fs::create_dir(original.join("C/DATA/case")).unwrap();
    #[cfg(unix)]
    {
        fs::create_dir(original.join("C/DATA/CASE")).unwrap();
        assert!(StorageImage::capture(&original, &archive, ID).is_err());
        assert_eq!(fs::read(&archive).unwrap(), before);
    }
}
#[cfg(unix)]
#[test]
fn symlinks_and_special_files_inside_guest_drives_are_never_followed() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("source");
    source(&original);
    fs::write(root.path().join("outside"), b"must remain private").unwrap();
    std::os::unix::fs::symlink(root.path().join("outside"), original.join("C/DATA/link")).unwrap();
    let archive = root.path().join("tree.bin");
    let error = StorageImage::capture(&original, &archive, ID).unwrap_err();
    assert!(error.contains("symbolic links"));
    assert!(!archive.exists());
    assert_eq!(
        fs::read(root.path().join("outside")).unwrap(),
        b"must remain private"
    );
    fs::remove_file(original.join("C/DATA/link")).unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(original.join("C/DATA/socket")).unwrap();
    assert!(
        StorageImage::capture(&original, &archive, ID)
            .unwrap_err()
            .contains("special files")
    );
    assert!(!archive.exists());
}

#[test]
fn readonly_attributes_survive_and_cancellation_never_replaces_an_archive() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("source");
    source(&original);
    let file = original.join("C/DATA/program.hpprgm");
    let mut permissions = fs::metadata(&file).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&file, permissions).unwrap();
    let archive = root.path().join("tree.bin");
    StorageImage::capture(&original, &archive, ID).unwrap();
    let before = fs::read(&archive).unwrap();
    for publishing in [false, true] {
        let error = StorageImage::capture_with_progress(&original, &archive, ID, |progress| {
            if (publishing && progress == CaptureProgress::Publishing)
                || (!publishing && matches!(progress, CaptureProgress::Writing { .. }))
            {
                Err("cancelled by caller".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error, "cancelled by caller");
        assert_eq!(fs::read(&archive).unwrap(), before);
        assert!(
            !fs::read_dir(root.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tmp"))
        );
    }
    let restored = StorageImage::open(&archive, ID)
        .unwrap()
        .restore(root.path())
        .unwrap();
    assert!(
        fs::metadata(restored.path().join("C/DATA/program.hpprgm"))
            .unwrap()
            .permissions()
            .readonly()
    );
    restored.close().unwrap();
}

#[test]
fn actual_process_termination_during_capture_preserves_the_previous_archive() {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        sync::mpsc,
    };
    let root = tempfile::tempdir().unwrap();
    source(&root.path().join("source"));
    let archive = root.path().join("tree.bin");
    StorageImage::capture(&root.path().join("source"), &archive, ID).unwrap();
    let before = fs::read(&archive).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "storage_capture_child", "--nocapture"])
        .env("PRIME_STORAGE_CAPTURE_CHILD", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if line.unwrap().contains("capture-unpublished") {
                tx.send(()).unwrap();
                break;
            }
        }
    });
    let ready = rx.recv_timeout(Duration::from_secs(10));
    child.kill().unwrap();
    let output = child.wait_with_output().unwrap();
    reader.join().unwrap();
    assert!(
        ready.is_ok(),
        "capture never reached unpublished output: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.status.success());
    assert_eq!(fs::read(&archive).unwrap(), before);
    let restored = StorageImage::open(&archive, ID)
        .unwrap()
        .restore(root.path())
        .unwrap();
    assert_eq!(
        fs::read(restored.path().join("C/DATA/settings")).unwrap(),
        [0, 255, 0, 65, 0, 0]
    );
}

#[test]
fn storage_capture_child() {
    use std::io::{Read, Write};
    let Some(root) = std::env::var_os("PRIME_STORAGE_CAPTURE_CHILD") else {
        return;
    };
    let root = Path::new(&root);
    StorageImage::capture_with_progress(
        &root.join("source"),
        &root.join("tree.bin"),
        ID,
        |progress| {
            if matches!(progress, CaptureProgress::Writing { completed: 0, .. }) {
                println!("capture-unpublished");
                std::io::stdout().flush().unwrap();
                let mut resume = [0];
                // A normal progress consumer may block; termination at this point
                // must preserve the last complete published archive.
                std::io::stdin().read_exact(&mut resume).unwrap();
            }
            Ok(())
        },
    )
    .unwrap();
}
