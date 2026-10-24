//! Guest virtual filesystem: drive-letter paths over a host directory,
//! FILE-handle table, directory search, and INI access.
//!
//! Conventions verified against the reference (PrimU2 `svc_common.cpp`,
//! `svc_filesystem.cpp`) — with one deliberate improvement: guest paths
//! are lexically normalized and jailed under the data root, which the
//! reference does not do for drive-absolute paths.

use std::collections::{HashMap, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

const FIRST_HANDLE: u32 = 0x0001_0001;

pub struct VFile {
    pub file: std::fs::File,
    pub host_path: PathBuf,
    pub readable: bool,
    pub writable: bool,
}

struct FindCtx {
    /// Entry names remaining to scan (collected eagerly: guest disks are
    /// small and this avoids holding a non-Send iterator across threads).
    remaining: VecDeque<String>,
    pattern: String,
    host_dir: PathBuf,
    attrib_mask: u32,
    /// Guest heap blocks holding the current SFN/LFN strings, freed on the
    /// next call (reference behavior).
    sfn_va: u32,
    lfn_va: u32,
}

pub struct Vfs {
    root: PathBuf,
    files: Mutex<HashMap<u32, VFile>>,
    next_handle: AtomicU64,
    finds: Mutex<HashMap<u32, FindCtx>>,
    next_find: AtomicU64,
    cwd: Mutex<[String; 26]>,
    cur_drive: Mutex<u8>,
}

impl Vfs {
    pub fn new(root: PathBuf) -> Vfs {
        let mut cwds: [String; 26] =
            std::array::from_fn(|i| format!("{}:", char::from(b'A' + i as u8)));
        for (i, c) in cwds.iter_mut().enumerate() {
            *c = format!("{}:", char::from(b'A' + i as u8));
        }
        Vfs {
            root,
            files: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(FIRST_HANDLE as u64),
            finds: Mutex::new(HashMap::new()),
            next_find: AtomicU64::new(1),
            cwd: Mutex::new(cwds),
            cur_drive: Mutex::new(b'A'),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Translate a guest path (ANSI or decoded wide) to a jailed host path.
    /// Returns None when the path escapes the data root.
    pub fn to_host(&self, guest: &str) -> Option<PathBuf> {
        let s = guest.trim().replace('/', "\\");
        let (drive, rest) = if s.len() >= 2 && s.as_bytes()[1] == b':' && s.len() > 2 {
            let d = s.as_bytes()[0].to_ascii_uppercase();
            if !d.is_ascii_uppercase() {
                return None;
            }
            (d - b'A', s[2..].to_owned())
        } else {
            (*self.cur_drive.lock().unwrap() - b'A', s)
        };
        if drive >= 26 {
            return None;
        }
        // The stored cwd is `X:` or `X:\\dir`: relative paths join its
        // directory part (the drive letter is added once, below).
        let cwd = self.cwd.lock().unwrap()[drive as usize].clone();
        let cwd_dir = cwd.get(2..).unwrap_or("");
        let joined = if rest.starts_with('\\') {
            rest
        } else {
            format!("{}\\{}", cwd_dir.trim_end_matches('\\'), rest)
        };
        let mut full = format!("{}:{}", char::from(b'A' + drive), joined);
        // Normalize to reject traversal outside the data root.
        let mut parts: Vec<&str> = Vec::new();
        for seg in full.split('\\') {
            if seg.is_empty() || seg == "." {
                continue;
            }
            if seg == ".." {
                // "A:" level: popping the drive segment is forbidden.
                if parts.len() <= 1 {
                    return None;
                }
                parts.pop();
                continue;
            }
            parts.push(seg);
        }
        full = parts.join("\\");
        let full = if full.len() == 2 {
            format!("{}\\", full)
        } else {
            full
        };
        Some(self.root.join(full.replacen(':', "", 1).replace('\\', "/")))
    }

    /// Resolve a path case-insensitively segment by segment (the guest's
    /// FAT is case-insensitive; a Linux host is not).
    pub fn resolve_case(&self, host: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        let mut comp = host.components().peekable();
        let mut prefixes: Vec<PathBuf> = Vec::new();
        let mut current = PathBuf::new();
        for c in comp.by_ref() {
            current = current.join(c);
            prefixes.push(current.clone());
        }
        let _ = &mut out;
        let _ = &mut prefixes;
        // Walk segments, matching each against the directory listing.
        let mut acc = PathBuf::new();
        for seg in host.components() {
            let cand = acc.join(seg);
            if cand.exists() {
                acc = cand;
                continue;
            }
            if let Some(name) = seg.as_os_str().to_str() {
                let mut found = false;
                if acc.is_dir() {
                    if let Ok(rd) = std::fs::read_dir(&acc) {
                        for e in rd.flatten() {
                            if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
                                acc = acc.join(e.file_name());
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if !found {
                    acc = cand; // keep the literal spelling (create case)
                }
            } else {
                acc = cand;
            }
        }
        acc
    }

    /// Open a guest file. Mode letters follow C `fopen`.
    pub fn open(&self, guest: &str, mode: &str) -> Option<u32> {
        let host = self.resolve_case(&self.to_host(guest)?);
        let readable = mode.contains('r') || mode.contains('+');
        let writable = mode.contains('w') || mode.contains('a') || mode.contains('+');
        if mode.contains('w') {
            if let Some(p) = host.parent() {
                std::fs::create_dir_all(p).ok()?;
            }
        }
        let file = std::fs::OpenOptions::new()
            .read(readable)
            .write(writable)
            .create(mode.contains('w') || mode.contains('a'))
            .truncate(mode.contains('w'))
            .open(&host)
            .ok()?;
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed) as u32;
        self.files.lock().unwrap().insert(
            handle,
            VFile {
                file,
                host_path: host,
                readable,
                writable,
            },
        );
        Some(handle)
    }

    pub fn close(&self, handle: u32) -> bool {
        self.files.lock().unwrap().remove(&handle).is_some()
    }

    fn with<T>(&self, handle: u32, f: impl FnOnce(&mut VFile) -> Option<T>) -> Option<T> {
        let mut files = self.files.lock().unwrap();
        files.get_mut(&handle).and_then(f)
    }

    pub fn read(&self, handle: u32, dest: &mut [u8]) -> Option<usize> {
        self.with(handle, |vf| Some(vf.file.read(dest).unwrap_or(0)))
    }

    pub fn read_at(&self, handle: u32, offset: u64, dest: &mut [u8]) -> Option<usize> {
        self.with(handle, |vf| {
            vf.file.seek(SeekFrom::Start(offset)).ok()?;
            vf.file.read(dest).ok()
        })
    }

    pub fn write(&self, handle: u32, src: &[u8]) -> Option<usize> {
        self.with(handle, |vf| Some(vf.file.write(src).unwrap_or(0)))
    }

    pub fn seek_set(&self, handle: u32, offset: u64) -> Option<()> {
        self.with(handle, |vf| {
            vf.file.seek(SeekFrom::Start(offset)).ok().map(|_| ())
        })
    }

    pub fn tell(&self, handle: u32) -> Option<u64> {
        self.with(handle, |vf| vf.file.stream_position().ok())
    }

    pub fn filesize(&self, handle: u32) -> Option<u64> {
        self.with(handle, |vf| {
            let cur = vf.file.stream_position().ok()?;
            let len = vf.file.seek(SeekFrom::End(0)).ok()?;
            vf.file.seek(SeekFrom::Start(cur)).ok()?;
            Some(len)
        })
    }

    pub fn host_path_of(&self, handle: u32) -> Option<PathBuf> {
        self.files
            .lock()
            .unwrap()
            .get(&handle)
            .map(|vf| vf.host_path.clone())
    }

    pub fn mkdir(&self, guest: &str) -> bool {
        match self.to_host(guest) {
            Some(h) => std::fs::create_dir_all(h).is_ok(),
            None => false,
        }
    }

    pub fn rmdir(&self, guest: &str) -> bool {
        match self.to_host(guest) {
            Some(h) => std::fs::remove_dir(h).is_ok(),
            None => false,
        }
    }

    pub fn chdir(&self, guest: &str) -> bool {
        let Some(h) = self.to_host(guest) else {
            return false;
        };
        if !h.is_dir() {
            return false;
        }
        let Some(rel) = h.strip_prefix(&self.root).ok() else {
            return false;
        };
        let segments: Vec<String> = rel
            .to_string_lossy()
            .replace('/', "\\")
            .split('\\')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let Some(first) = segments.first() else {
            return false; // cannot chdir to the data root itself
        };
        let d = first.chars().next().unwrap_or('?').to_ascii_uppercase();
        if !d.is_ascii_uppercase() {
            return false;
        }
        let rest = segments[1..].join("\\");
        let mut cwd = self.cwd.lock().unwrap();
        cwd[(d as u8 - b'A') as usize] = if rest.is_empty() {
            format!("{d}:")
        } else {
            format!("{d}:\\{rest}")
        };
        *self.cur_drive.lock().unwrap() = d as u8;
        true
    }

    pub fn current_path(&self) -> String {
        let d = *self.cur_drive.lock().unwrap();
        self.cwd.lock().unwrap()[(d - b'A') as usize].clone()
    }

    /// `_wfindfirst`: scan `guest_pattern` (may be a directory), store the
    /// host search handle in the guest context (+0), and run the first
    /// `_wfindnext`. Returns the first findnext result (0 found / -1 none).
    pub fn find_first(
        &self,
        guest_pattern: &str,
        _attrib_mask: u32,
    ) -> Option<(u32, PathBuf, Vec<String>, String)> {
        let host = self.to_host(guest_pattern)?;
        let (dir, pattern) = if host.is_dir() {
            (host, "*".to_owned())
        } else {
            (
                host.parent()?.to_path_buf(),
                host.file_name()?.to_string_lossy().into_owned(),
            )
        };
        if !dir.is_dir() {
            return None;
        }
        let mut names = Vec::new();
        for e in std::fs::read_dir(&dir).ok()?.flatten() {
            names.push(e.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Some((
            self.next_find.fetch_add(1, Ordering::Relaxed) as u32,
            dir,
            names,
            pattern,
        ))
    }

    /// (size, packed DOS datetime, FAT attributes) for a found entry.
    pub fn stat_entry(&self, handle: u32, name: &str) -> Option<(u32, u32, u8)> {
        let finds = self.finds.lock().unwrap();
        let ctx = finds.get(&handle)?;
        let path = ctx.host_dir.join(name);
        let md = std::fs::metadata(&path).ok()?;
        let attrib = if md.is_dir() { 0x10 } else { 0x20 };
        let size = if md.is_file() { md.len() as u32 } else { 0 };
        let t = md.modified().unwrap_or(std::time::UNIX_EPOCH);
        Some((size, dos_datetime(t), attrib))
    }

    /// Pop the next matching name. The caller must release the previous
    /// SFN/LFN strings first (`find_strings`).
    pub fn find_next(&self, handle: u32) -> Option<String> {
        let mut finds = self.finds.lock().unwrap();
        let ctx = finds.get_mut(&handle)?;
        while let Some(name) = ctx.remaining.pop_front() {
            if wildcard_match(&ctx.pattern, &name) {
                return Some(name);
            }
        }
        None
    }

    pub fn find_strings(&self, handle: u32) -> (u32, u32) {
        self.finds
            .lock()
            .unwrap()
            .get(&handle)
            .map(|c| (c.sfn_va, c.lfn_va))
            .unwrap_or((0, 0))
    }

    pub fn find_set_strings(&self, handle: u32, sfn_va: u32, lfn_va: u32) {
        if let Some(ctx) = self.finds.lock().unwrap().get_mut(&handle) {
            ctx.sfn_va = sfn_va;
            ctx.lfn_va = lfn_va;
        }
    }

    pub fn find_close(&self, handle: u32) -> Option<(u32, u32)> {
        self.finds
            .lock()
            .unwrap()
            .remove(&handle)
            .map(|c| (c.sfn_va, c.lfn_va))
    }

    pub fn find_register(
        &self,
        handle: u32,
        host_dir: PathBuf,
        names: Vec<String>,
        pattern: String,
        attrib_mask: u32,
    ) {
        self.finds.lock().unwrap().insert(
            handle,
            FindCtx {
                remaining: names.into(),
                pattern,
                host_dir,
                attrib_mask,
                sfn_va: 0,
                lfn_va: 0,
            },
        );
    }

    pub fn find_attrib_mask(&self, handle: u32) -> u32 {
        self.finds
            .lock()
            .unwrap()
            .get(&handle)
            .map(|c| c.attrib_mask)
            .unwrap_or(0)
    }
}

/// DOS `FindFirst`-style wildcard match (`*` and `?`), case-insensitive.
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    fn inner(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => inner(&p[1..], n) || (!n.is_empty() && inner(p, &n[1..])),
            (Some(b'?'), Some(_)) => inner(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => inner(&p[1..], &n[1..]),
            _ => false,
        }
    }
    inner(pattern.as_bytes(), name.as_bytes())
}

/// Pack a host mtime as a DOS date/time word pair (like the reference).
pub fn dos_datetime(t: std::time::SystemTime) -> u32 {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    let date = ((y - 1980) as u32) << 9 | m << 5 | d;
    let time =
        ((tod / 3600) as u32) << 11 | (((tod / 60) % 60) as u32) << 5 | ((tod % 60 / 2) as u32);
    date << 16 | time
}

fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod path_tests {
    use super::*;

    fn vfs() -> (Vfs, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("C/DATA")).unwrap();
        std::fs::create_dir_all(root.join("A")).unwrap();
        (Vfs::new(root.to_path_buf()), dir)
    }

    #[test]
    fn absolute_and_relative_paths_map_once_per_drive() {
        let (v, dir) = vfs();
        let root = dir.path();
        assert_eq!(
            v.to_host("C:\\DATA\\X.hpprgm"),
            Some(root.join("C/DATA/X.hpprgm"))
        );
        // As text too (`PathBuf` equality ignores a doubled separator).
        let host = v.to_host("C:\\DATA").unwrap();
        assert_eq!(
            host.to_string_lossy(),
            root.join("C/DATA").to_string_lossy()
        );
        // Relative to the current drive's cwd (initially its root).
        assert!(v.chdir("C:\\DATA"));
        assert_eq!(v.to_host("X.hpprgm"), Some(root.join("C/DATA/X.hpprgm")));
        // Drive-relative `C:name` uses drive C's cwd.
        assert_eq!(v.to_host("C:X.hpprgm"), Some(root.join("C/DATA/X.hpprgm")));
        // Another drive keeps its own cwd (its root).
        assert_eq!(v.to_host("A:Y"), Some(root.join("A/Y")));
        // Regression: `DATA` relative to `C:` must not become `C/C/DATA`.
        assert!(v.chdir("C:\\"));
        assert_eq!(v.to_host("DATA"), Some(root.join("C/DATA")));
    }

    #[test]
    fn drive_root_is_a_valid_cwd_and_traversal_is_jailed() {
        let (v, dir) = vfs();
        let root = dir.path();
        assert!(v.chdir("C:\\"), "the drive root is a directory");
        assert!(v.chdir("C:\\DATA"));
        assert_eq!(v.to_host("..\\DATA"), Some(root.join("C/DATA")));
        assert_eq!(v.to_host("C:\\..\\..\\etc"), None);
        assert_eq!(v.to_host("..\\..\\.."), None);
    }
}
