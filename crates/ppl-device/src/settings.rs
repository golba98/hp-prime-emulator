//! Official type-0 settings objects. Payloads are binary calculator state,
//! preserved opaquely; an empty liveness response is not a settings backup.
use crate::{DeviceError, FileType, Prime, PrimeFile, Result};

#[derive(Clone, Copy, Debug)]
pub enum SettingsKind {
    Home,
    Cas,
}
impl SettingsKind {
    pub fn object_name(self) -> &'static str {
        match self {
            Self::Home => "calc.hpsettings",
            Self::Cas => "cas.hpsettings",
        }
    }
}
impl Prime {
    pub fn read_settings(&mut self, kind: SettingsKind) -> Result<PrimeFile> {
        let before = self.info()?;
        if !crate::inventory::supported_profile(self.detected.product_id, &before) {
            return Err(DeviceError::Refused(
                "settings retrieval is unverified on this device profile".into(),
            ));
        }
        let name = kind.object_name();
        self.drain();
        self.send_message(&crate::files::encode_file_message(
            crate::CMD_REQ_FILE,
            FileType::Settings,
            name,
            &[],
        ))?;
        let raw = self.recv_message(crate::CMD_FILE, "settings reply", self.read_timeout)?;
        let file = crate::program::decode_record(&raw).inspect_err(|error| {
            self.retired = Some(error.to_string());
        })?;
        if file.ty != FileType::Settings || file.name != name {
            self.retired = Some("settings reply identity/type mismatch".into());
            return Err(DeviceError::Protocol(
                "settings reply identity/type mismatch".into(),
            ));
        }
        let after = self.info()?;
        if before.serial.is_none()
            || before.serial != after.serial
            || before.build != after.build
            || before.version != after.version
        {
            self.retired = Some("device identity changed during settings retrieval".into());
            return Err(DeviceError::Refused(
                "device identity changed during settings retrieval".into(),
            ));
        }
        Ok(file)
    }
}
