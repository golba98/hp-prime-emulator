//! FSGetDiskRoomState's 0x3c-byte ABI, established from executable OS ROM
//! code at 0x3003F0CC. Byte fields are 64-bit; the first five fields retain
//! the low 32 bits of their KiB forms, as the original ARM shifts do.
use std::{io, path::Path};
pub const SIZE: usize = 0x3c;
pub fn encode(total: u64, free: u64) -> [u8; SIZE] {
    let free = free.min(total);
    let values = [total, total, total, total - free, free];
    let mut bytes = [0; SIZE];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&((value >> 10) as u32).to_le_bytes());
        bytes[0x14 + index * 8..0x1c + index * 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}
/// Logical occupancy of this host-backed virtual volume. Never follow links
/// outside the profile or silently report unreadable files as free storage.
pub fn used_bytes(root: &Path) -> io::Result<u64> {
    let metadata = match root.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "virtual volume root is not an owned directory",
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut bytes = 0u64;
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let metadata = entry.path().symlink_metadata()?;
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other("virtual volume contains a symbolic link"));
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| io::Error::other("virtual volume size overflow"))?;
            } else {
                return Err(io::Error::other("virtual volume contains a special file"));
            }
        }
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_room_state_tracks_real_file_mutations_and_64_bit_totals() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/program"), [1u8; 3000]).unwrap();
        let used = used_bytes(root.path()).unwrap();
        assert_eq!(used, 3000);
        let total = 1u64 << 40;
        let data = encode(total, total - used);
        assert_eq!(
            u64::from_le_bytes(data[0x2c..0x34].try_into().unwrap()),
            3000
        );
        assert_eq!(
            u64::from_le_bytes(data[0x34..0x3c].try_into().unwrap()),
            total - 3000
        );
        std::fs::write(root.path().join("nested/program"), [0u8; 5000]).unwrap();
        assert_eq!(used_bytes(root.path()).unwrap(), 5000);
        std::fs::remove_file(root.path().join("nested/program")).unwrap();
        assert_eq!(used_bytes(root.path()).unwrap(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn unreadable_volume_shapes_do_not_traverse_host_links() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("cycle")).unwrap();
        assert!(used_bytes(root.path()).is_err());
    }
}
