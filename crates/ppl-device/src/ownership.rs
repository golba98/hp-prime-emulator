//! An ephemeral capability for updating a note created and verified through
//! this exact open connection. It cannot authorize program/system writes,
//! cannot be serialized, and expires with the bounded experiment.
use crate::{DeviceError, FileType, Prime, ReadBack, Result, WriteGuard};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

/// Positive ownership is bounded independently of the five-minute absence guard.
/// Allows a manual start plus two short fixed-mailbox challenge windows.
const OWNED_NOTE_MAX_AGE: Duration = Duration::from_secs(900);

pub struct OwnedNote {
    name: String,
    owner: Weak<()>,
    created: Instant,
}
impl Prime {
    /// Create a new note through the ordinary guard and acquire a bounded
    /// update capability only after exact, CRC-valid read-back succeeds.
    pub fn create_owned_note(&mut self, guard: WriteGuard, text: &str) -> Result<OwnedNote> {
        let name = guard.program().to_string();
        let report = self.send_new_note(guard, text)?;
        if report.readback != ReadBack::Matches {
            return Err(DeviceError::Protocol(
                "owned note creation unverified; no update capability issued".into(),
            ));
        }
        Ok(OwnedNote {
            name,
            owner: Arc::downgrade(&self.owner),
            created: Instant::now(),
        })
    }
    /// Reopen a mailbox from a trusted creation record. The caller must bind
    /// that record to the current device and verified receiver source first.
    /// Exact current name, note type, CRC and content are required; differing
    /// content is never overwritten. This cannot bypass a busy F8 interface.
    pub fn reopen_owned_note(&mut self, name: &str, expected: &str) -> Result<OwnedNote> {
        crate::validate_test_name(name)?;
        if expected.len() != 75
            || !expected.starts_with("@@PPLMAIL|")
            || expected.as_bytes()[42] != b'|'
            || !expected.as_bytes()[10..42]
                .iter()
                .chain(expected.as_bytes()[43..].iter())
                .all(u8::is_ascii_hexdigit)
        {
            return Err(DeviceError::Refused(
                "only a fixed-mailbox creation receipt can reopen a note".into(),
            ));
        }
        match self.request_file(FileType::Note, name)? {
            crate::Probe::Present(file)
                if file.name == name
                    && file.ty == FileType::Note
                    && file.crc_ok
                    && file.text() == expected =>
            {
                Ok(OwnedNote {
                    name: name.into(),
                    owner: Arc::downgrade(&self.owner),
                    created: Instant::now(),
                })
            }
            _ => Err(DeviceError::Refused(
                "mailbox does not match its creation record, or is busy; nothing was written"
                    .into(),
            )),
        }
    }
    /// Update only the note created by create_owned_note in this still-open,
    /// exclusively held session. The receiver must be read-only with respect
    /// to this mailbox; other USB tools and manual note edits are excluded.
    /// This deliberately does not request F8 while PPL is running. A successful
    /// write is only transmission, never proof of receipt or execution.
    pub fn update_owned_note(&mut self, owned: &OwnedNote, text: &str) -> Result<()> {
        if owned.created.elapsed() > OWNED_NOTE_MAX_AGE
            || !owned
                .owner
                .upgrade()
                .is_some_and(|owner| Arc::ptr_eq(&owner, &self.owner))
        {
            return Err(DeviceError::Refused(
                "owned-note capability expired or belongs to another connection".into(),
            ));
        }
        crate::validate_test_name(&owned.name)?;
        if text.encode_utf16().count() > 4096 {
            return Err(DeviceError::Refused(
                "owned-note payload exceeds 4096 UTF-16 units".into(),
            ));
        }
        let start = self.log.len();
        self.drain();
        self.ensure_quiet(start, std::slice::from_ref(&owned.name))?;
        let data: Vec<_> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        self.send_message(&crate::files::encode_file_message(
            crate::CMD_FILE,
            FileType::Note,
            &owned.name,
            &data,
        ))
    }
}
