//! Fresh-process restoration of actual VFS handles and directory searches.
//! The matching storage copy and RAM image are explicit dependencies. This is
//! resource reconstruction evidence, not whole-calculator live-resume evidence.
use prime_emulator::{
    memory::{GuestMemory, image::MemoryImage},
    services::fs::{Vfs, image::VfsImage},
    storage::image::StorageImage,
};
use std::path::Path;
const ID: &str = "writable-vfs-test-v1";

fn memory() -> GuestMemory {
    let mut memory = GuestMemory::new();
    memory.map("heap", 0x20000000, 65536).unwrap();
    memory.set_heap(0x20000000, 65536);
    memory
}
fn storage(root: &Path) -> Vfs {
    std::fs::create_dir_all(root.join("C/DATA")).unwrap();
    std::fs::write(root.join("C/DATA/base.dat"), b"abcdef").unwrap();
    Vfs::new(root.to_owned())
}

#[test]
fn writable_files_cursors_append_mode_and_searches_resume_in_a_fresh_process() {
    let root = tempfile::tempdir().unwrap();
    let memory = memory();
    let source = storage(&root.path().join("source"));
    let rw = source.open("C:/DATA/base.dat", "r+").unwrap();
    source.seek_set(rw, 2).unwrap();
    let append = source.open("C:/DATA/base.dat", "a+").unwrap();
    source.seek_set(append, 0).unwrap();
    let writer = source.open("C:/DATA/output.dat", "w").unwrap();
    assert_eq!(source.write(writer, b"AB"), Some(2));
    source.seek_set(writer, 10).unwrap(); // A valid sparse write beyond EOF.
    let reader = source.open("C:/DATA/base.dat", "r").unwrap();
    source.seek_set(reader, 1).unwrap();
    assert!(source.chdir("C:/DATA"));
    let (find, directory, names, pattern) = source.find_first("*.dat", 0x20).unwrap();
    source.find_register(find, directory, names, pattern, 0x20);
    assert_eq!(source.find_next(find).as_deref(), Some("base.dat"));
    let short = memory.heap_alloc(32).unwrap();
    let long = memory.heap_alloc(32).unwrap();
    memory.write_bytes(short, b"base.dat\0").unwrap();
    memory.write_bytes(long, b"base.dat\0").unwrap();
    source.find_set_strings(find, short, long);
    let image = VfsImage::capture(&source, &memory, ID).unwrap();
    assert_eq!(
        source.tell(rw),
        Some(2),
        "capture must not move a shared stream cursor"
    );
    image.save(&root.path().join("vfs.json"), &memory).unwrap();
    MemoryImage::capture(&memory, ID)
        .unwrap()
        .save(&root.path().join("ram.bin"))
        .unwrap();
    StorageImage::capture(source.root(), &root.path().join("storage.bin"), ID).unwrap();
    // The child must bind to its storage copy, not reopen source-process paths.
    std::fs::write(
        root.path().join("source/C/DATA/base.dat"),
        b"changed in source",
    )
    .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "restored_vfs_child", "--nocapture"])
        .env("PRIME_VFS_RESTORE_CHILD", root.path())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(String::from_utf8_lossy(&child.stdout).contains("writable-vfs-resumed"));
}

#[cfg(unix)]
#[test]
fn restored_directory_search_remains_within_a_symlinked_profile_root() {
    let root = tempfile::tempdir().unwrap();
    let memory = memory();
    let source = storage(&root.path().join("source"));
    let (find, directory, names, pattern) = source.find_first("C:/DATA/*.dat", 0x20).unwrap();
    source.find_register(find, directory, names, pattern, 0x20);
    let image = VfsImage::capture(&source, &memory, ID).unwrap();

    let actual = root.path().join("actual");
    storage(&actual);
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(actual, &alias).unwrap();
    let mut target = Vfs::new(alias);
    image.restore(&mut target, &memory, ID).unwrap();

    let recaptured = VfsImage::capture(&target, &memory, ID).unwrap();
    assert_eq!(recaptured.searches[0].directory, "C/DATA");
    assert_eq!(target.find_next(find).as_deref(), Some("base.dat"));
}

#[test]
fn restored_vfs_child() {
    let Some(root) = std::env::var_os("PRIME_VFS_RESTORE_CHILD") else {
        return;
    };
    let root = Path::new(&root);
    let memory = MemoryImage::load(&root.join("ram.bin"), ID)
        .unwrap()
        .restore(ID)
        .unwrap();
    let image = VfsImage::load(&root.join("vfs.json"), &memory, ID).unwrap();
    let restored = StorageImage::open(&root.join("storage.bin"), ID)
        .unwrap()
        .restore(root)
        .unwrap();
    let mut vfs = Vfs::new(restored.path().into());
    image.restore(&mut vfs, &memory, ID).unwrap();
    let handles = image.files.iter().map(|f| f.handle).collect::<Vec<_>>();
    assert_eq!(vfs.current_path(), "C:\\DATA");
    let mut buffer = [0; 8];
    assert_eq!(vfs.read(handles[0], &mut buffer), Some(4));
    assert_eq!(&buffer[..4], b"cdef");
    assert_eq!(vfs.read(handles[3], &mut buffer), Some(5));
    assert_eq!(&buffer[..5], b"bcdef");
    assert_eq!(vfs.write(handles[1], b"Z"), Some(1));
    assert_eq!(
        std::fs::read(restored.path().join("C/DATA/base.dat")).unwrap(),
        b"abcdefZ"
    );
    vfs.seek_set(handles[0], 1).unwrap();
    assert_eq!(vfs.write(handles[0], b"!"), Some(1));
    assert_eq!(
        std::fs::read(restored.path().join("C/DATA/base.dat")).unwrap(),
        b"a!cdefZ"
    );
    assert_eq!(vfs.write(handles[2], b"Q"), Some(1));
    assert_eq!(
        std::fs::read(restored.path().join("C/DATA/output.dat")).unwrap(),
        b"AB\0\0\0\0\0\0\0\0Q"
    );
    assert_eq!(
        vfs.read(handles[2], &mut buffer),
        Some(0),
        "write-only descriptor must retain access mode"
    );
    assert_eq!(
        vfs.write(handles[3], b"!"),
        Some(0),
        "read-only descriptor must retain access mode"
    );
    let search = &image.searches[0];
    assert_eq!(vfs.find_next(search.handle).as_deref(), Some("output.dat"));
    assert_eq!(vfs.find_next(search.handle), None);
    assert_eq!(vfs.find_attrib_mask(search.handle), 0x20);
    assert_eq!(
        vfs.find_strings(search.handle),
        (search.short_name, search.long_name)
    );
    assert_eq!(
        memory.read_bytes_alloc(search.short_name, 9).unwrap(),
        b"base.dat\0"
    );
    assert_eq!(
        vfs.find_close(search.handle),
        Some((search.short_name, search.long_name))
    );
    assert!(memory.heap_free(search.short_name));
    assert!(memory.heap_free(search.long_name));
    assert_eq!(vfs.open("base.dat", "r"), Some(image.next_handle as u32));
    assert_eq!(
        vfs.find_first("*.dat", 0).unwrap().0,
        image.next_find as u32
    );
    println!("writable-vfs-resumed");
}

#[test]
fn changed_storage_or_manifest_leaves_the_target_unmodified() {
    let root = tempfile::tempdir().unwrap();
    let memory = memory();
    let source = storage(root.path());
    source.open("C:/DATA/base.dat", "r+").unwrap();
    let mut image = VfsImage::capture(&source, &memory, ID).unwrap();
    let mut target = Vfs::new(root.path().into());
    image.files[0].path = "../base.dat".into();
    assert!(image.restore(&mut target, &memory, ID).is_err());
    assert_eq!(target.inspection()["open_files"], 0);
    let image = VfsImage::capture(&source, &memory, ID).unwrap();
    std::fs::write(root.path().join("C/DATA/base.dat"), b"changed").unwrap();
    assert!(image.restore(&mut target, &memory, ID).is_err());
    assert_eq!(target.inspection()["open_files"], 0);
    assert_eq!(target.open("C:/DATA/base.dat", "r"), Some(0x10001));
}

#[test]
fn append_writes_at_eof_even_after_seek_and_does_not_truncate() {
    let root = tempfile::tempdir().unwrap();
    let vfs = storage(root.path());
    let handle = vfs.open("C:/DATA/base.dat", "a+").unwrap();
    vfs.seek_set(handle, 1).unwrap();
    assert_eq!(vfs.write(handle, b"Z"), Some(1));
    assert_eq!(
        std::fs::read(root.path().join("C/DATA/base.dat")).unwrap(),
        b"abcdefZ"
    );
    assert_eq!(vfs.tell(handle), Some(7));
}

#[cfg(unix)]
#[test]
fn identical_replacement_bytes_cannot_rebind_an_open_descriptor_to_another_inode() {
    let root = tempfile::tempdir().unwrap();
    let vfs = storage(root.path());
    vfs.open("C:/DATA/base.dat", "w").unwrap();
    let path = root.path().join("C/DATA/base.dat");
    std::fs::rename(&path, root.path().join("old.dat")).unwrap();
    std::fs::write(&path, []).unwrap(); // Same bytes, different file object.
    let error = VfsImage::capture(&vfs, &memory(), ID).unwrap_err();
    assert!(error.contains("differs from its storage path"), "{error}");
}
