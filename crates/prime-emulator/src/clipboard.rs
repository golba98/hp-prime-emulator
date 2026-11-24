//! Read-only observation of the G1 2.4.2 firmware's own text clipboard.
//!
//! Verified on the installed 20260909 image by actual Shift+View, All, Copy
//! gestures. The named CopyToClipBoard SVC is not used by this path. The
//! internal setter at 0x30AFD6E4 owns the buffer at application+0x10A4.
//! At COMMITTED the UTF-16 payload and its byte-size header have been written,
//! and its type byte has been set to zero; the source may already be freed.
//! We copy that committed payload while the CPU owner still holds the memory
//! gate. No framebuffer, host input cache, guessed heap address or memory write
//! participates in extraction. Other firmware code is refused by its digest.
use crate::{memory::GuestMemory, sha256};
use std::sync::{Arc, Mutex};
use unicorn_engine::{RegisterARM, Unicorn, uc_error};
const SETTER: u32 = 0x30af_d6e4;
const END: u32 = 0x30af_d788;
const COMMITTED: u32 = 0x30af_d758;
const CODE_SHA256: &str = "3b9acc8de3b4e59374c41ff4499d85d8725c8820c86d2d44bfa74f4e45e77c76";
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
#[derive(Default)]
pub struct Clipboard {
    state: Mutex<State>,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    generation: u64,
    pending: Option<(u64, Result<String, String>)>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    version: u32,
    state: State,
}
impl Image {
    pub fn restore(&self) -> Result<Clipboard, String> {
        if self.version != 1
            || self
                .state
                .pending
                .as_ref()
                .is_some_and(|(generation, text)| {
                    *generation == 0
                        || *generation > self.state.generation
                        || text.as_ref().is_ok_and(|text| {
                            ppl_engine::session::validate_desktop_text(text).is_err()
                        })
                        || text.as_ref().err().is_some_and(|error| error.len() > 4096)
                })
        {
            return Err("Invalid clipboard image version, generation or text budget".into());
        }
        Ok(Clipboard {
            state: Mutex::new(self.state.clone()),
        })
    }
}
impl Clipboard {
    pub fn image(&self) -> Image {
        Image {
            version: 1,
            state: self.state.lock().unwrap().clone(),
        }
    }
    fn publish(&self, value: Result<String, String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.generation = state.generation.saturating_add(1);
        let generation = state.generation;
        // Clipboard assignment has last-writer semantics. Bound memory even
        // when the transport consumer is slower than guest Copy operations.
        state.pending = Some((generation, value));
    }
    pub fn take(&self) -> Option<(u64, Result<String, String>)> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .take()
    }
}
pub fn firmware_supported(path: &std::path::Path) -> bool {
    let Ok(image) = crate::firmware::ElfImage::load(path) else {
        return false;
    };
    image.segments.iter().any(|segment| {
        if segment.vaddr > SETTER
            || u64::from(END) > u64::from(segment.vaddr) + segment.file_size as u64
        {
            return false;
        }
        let start = (segment.offset + u64::from(SETTER - segment.vaddr)) as usize;
        image
            .bytes
            .get(start..start + (END - SETTER) as usize)
            .is_some_and(|bytes| sha256::hex(bytes) == CODE_SHA256)
    })
}
pub fn code_matches(mem: &GuestMemory) -> bool {
    mem.read_bytes_alloc(SETTER, (END - SETTER) as usize)
        .is_ok_and(|bytes| sha256::hex(&bytes) == CODE_SHA256)
}
/// Header size includes eight header bytes and the terminating UTF-16 NUL.
pub(crate) fn text_record(mem: &GuestMemory, record: u32) -> Result<String, String> {
    if record == 0 || record % 4 != 0 {
        return Err("Invalid firmware clipboard record".into());
    }
    let header = mem.read_bytes_alloc(record, 8).map_err(|e| e.to_string())?;
    if header[0] != 0 {
        return Err("Firmware clipboard selection is not plain text".into());
    }
    let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    if !(10..=MAX_TEXT_BYTES + 10).contains(&size) || size % 2 != 0 {
        return Err("Invalid firmware clipboard text length".into());
    }
    let bytes = mem
        .read_bytes_alloc(
            record.checked_add(8).ok_or("Clipboard address overflow")?,
            size - 8,
        )
        .map_err(|e| e.to_string())?;
    let mut units = bytes
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect::<Vec<_>>();
    if units.pop() != Some(0) || units.contains(&0) {
        return Err("Invalid firmware clipboard terminator".into());
    }
    String::from_utf16(&units).map_err(|_| {
        "Firmware clipboard contains invalid UTF-16; desktop clipboard was preserved".into()
    })
}
pub fn install(
    engine: &mut Unicorn<'static, ()>,
    mem: &Arc<GuestMemory>,
    clipboard: &Arc<Clipboard>,
) -> Result<(), uc_error> {
    if !code_matches(mem) {
        return Ok(());
    }
    let mem = mem.clone();
    let clipboard = clipboard.clone();
    engine.add_code_hook(
        u64::from(COMMITTED),
        u64::from(COMMITTED),
        Box::new(move |uc: &mut Unicorn<'_, ()>, _, _| {
            let read = |reg| {
                uc.reg_read(reg)
                    .map(|v| v as u32)
                    .map_err(|e| e.to_string())
            };
            let result = (|| {
                let owner = read(RegisterARM::R4)?;
                let slot = owner
                    .checked_add(0xa4)
                    .ok_or("Clipboard owner address overflow")?;
                let record = mem.read_u32(slot).map_err(|e| e.to_string())?;
                text_record(&mem, record)
            })();
            clipboard.publish(result);
        }),
    )?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restored_clipboard_uses_the_same_utf16_budget_as_delivery_and_guest_records() {
        let clipboard = Clipboard::default();
        let text = "漢".repeat(ppl_engine::session::MAX_TEXT_UTF16_UNITS);
        assert!(
            text.len() > MAX_TEXT_BYTES,
            "UTF-8 bytes are not guest UTF-16 bytes"
        );
        clipboard.publish(Ok(text.clone()));
        let restored = clipboard.image().restore().unwrap();
        assert_eq!(restored.take(), Some((1, Ok(text.clone()))));
        clipboard.publish(Ok(text + "漢"));
        assert!(clipboard.image().restore().is_err());
    }
    #[test]
    fn pending_copy_and_generation_resume_once_in_a_new_clipboard_owner() {
        let clipboard = Clipboard::default();
        clipboard.publish(Ok("first".into()));
        assert_eq!(clipboard.take().unwrap().0, 1);
        clipboard.publish(Ok("π\nsecond".into()));
        let image: Image =
            serde_json::from_slice(&serde_json::to_vec(&clipboard.image()).unwrap()).unwrap();
        let restored = image.restore().unwrap();
        assert_eq!(restored.take(), Some((2, Ok("π\nsecond".into()))));
        assert_eq!(restored.take(), None);
        restored.publish(Ok("third".into()));
        assert_eq!(restored.take(), Some((3, Ok("third".into()))));
    }
    fn image(units: &[u16]) -> GuestMemory {
        let mut mem = GuestMemory::new();
        mem.map("clipboard", 0x1000, 0x1000).unwrap();
        mem.write_u32(0x1004, (10 + units.len() * 2) as u32)
            .unwrap();
        for (i, u) in units.iter().chain(std::iter::once(&0)).enumerate() {
            mem.write_u16(0x1008 + i as u32 * 2, *u).unwrap();
        }
        mem
    }
    #[test]
    fn copied_unicode_multiline_and_empty_text_are_exact() {
        for text in [
            "",
            "135792468",
            "π×ⅈ−α",
            "EXPORT T()\r\nBEGIN\nEND;",
            "a😀b",
        ] {
            assert_eq!(
                text_record(&image(&text.encode_utf16().collect::<Vec<_>>()), 0x1000).unwrap(),
                text
            );
        }
    }
    #[test]
    fn corrupt_binary_oversized_and_surrogate_records_are_rejected() {
        let mem = image(&[65]);
        assert!(text_record(&mem, 0).is_err());
        assert!(text_record(&mem, 0x1001).is_err());
        mem.write_u16(0x1000, 1).unwrap();
        assert!(text_record(&mem, 0x1000).is_err());
        mem.write_u16(0x1000, 0).unwrap();
        for size in [0, 8, 11, u32::MAX] {
            mem.write_u32(0x1004, size).unwrap();
            assert!(text_record(&mem, 0x1000).is_err());
        }
        assert!(text_record(&image(&[0xd800]), 0x1000).is_err());
        assert!(text_record(&image(&[65, 0, 66]), 0x1000).is_err());
        let mem = image(&[65]);
        mem.write_u16(0x100a, 66).unwrap();
        assert!(text_record(&mem, 0x1000).is_err());
    }
    #[test]
    fn pending_updates_are_bounded_and_repeat_copy_advances_generation() {
        let clipboard = Clipboard::default();
        clipboard.publish(Ok("same".into()));
        clipboard.publish(Ok("same".into()));
        assert_eq!(clipboard.take(), Some((2, Ok("same".into()))));
        assert!(clipboard.take().is_none());
        assert!(!code_matches(&image(&[65])));
    }
}
