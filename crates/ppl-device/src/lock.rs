//! Advisory locks shared by all real HID connections in this crate.
use crate::{DeviceError, Result};
use std::fs::{File, OpenOptions};

pub(crate) fn acquire(device_path: &str) -> Result<File> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!(
        "ppl-prime-{:016x}.lock",
        crate::guard::fnv64(device_path.as_bytes())
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| {
            DeviceError::Hid(format!("cannot open device lock {}: {e}", path.display()))
        })?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            DeviceError::Busy("another process holds the HID connection".into())
        } else {
            DeviceError::Hid(format!("cannot lock device: {e}"))
        }
    })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    #[test]
    fn exclusive_and_released_on_close() {
        let id = format!("lock-test-{}", std::process::id());
        let first = super::acquire(&id).unwrap();
        assert!(matches!(
            super::acquire(&id),
            Err(crate::DeviceError::Busy(_))
        ));
        drop(first);
        assert!(super::acquire(&id).is_ok());
    }
}
