//! Platform-independent configuration for the optional firmware helper.
use std::{path::PathBuf, time::Duration};
#[derive(Debug, Clone)]
pub struct Options {
    pub runner: Option<PathBuf>,
    pub firmware: Option<PathBuf>,
    pub os_rom: Option<PathBuf>,
    pub factory_a: Option<PathBuf>,
    pub init_data: Option<PathBuf>,
    pub artifacts_dir: Option<PathBuf>,
    pub program_name: Option<String>,
    pub entry: Option<String>,
    pub args: String,
    pub runtime_keys: Option<String>,
    pub screenshot: Option<PathBuf>,
    pub timeout: Duration,
    pub boot_timeout: Duration,
    pub compile_timeout: Duration,
    pub execution_timeout: Duration,
    pub max_instructions: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            runner: None,
            firmware: None,
            os_rom: None,
            factory_a: None,
            init_data: None,
            artifacts_dir: None,
            program_name: None,
            entry: None,
            args: "[]".into(),
            runtime_keys: None,
            screenshot: None,
            timeout: Duration::from_secs(360),
            boot_timeout: Duration::from_secs(60),
            compile_timeout: Duration::from_secs(30),
            execution_timeout: Duration::from_secs(30),
            max_instructions: 8_000_000_000,
        }
    }
}

/// Firmware asset locations, resolved per field as flag, then environment
/// variable, then the user's config file
/// (`$XDG_CONFIG_HOME/hp-prime-emulator/firmware.toml`, or `PPL_CONFIG`).
/// Relative paths in the config file are relative to the file's directory,
/// so resolution never depends on the current working directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetPaths {
    pub runner: Option<PathBuf>,
    pub firmware: Option<PathBuf>,
    pub os_rom: Option<PathBuf>,
    pub factory_a: Option<PathBuf>,
    pub init_data: Option<PathBuf>,
}
const ENV: [&str; 5] = [
    "PPL_FIRMWARE_RUNNER",
    "PPL_FIRMWARE",
    "PPL_OS_ROM",
    "PPL_FACTORY_A",
    "PPL_FIRMWARE_INIT_DATA",
];
impl AssetPaths {
    fn fields(&mut self) -> [&mut Option<PathBuf>; 5] {
        [
            &mut self.runner,
            &mut self.firmware,
            &mut self.os_rom,
            &mut self.factory_a,
            &mut self.init_data,
        ]
    }
    pub fn config_file() -> Option<PathBuf> {
        let var = |k| {
            std::env::var_os(k)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        var("PPL_CONFIG").or_else(|| {
            var("XDG_CONFIG_HOME")
                .or_else(|| var("HOME").map(|h| h.join(".config")))
                .map(|d| d.join("hp-prime-emulator/firmware.toml"))
        })
    }
    /// The config file's paths; absent file is empty, a malformed one an error.
    pub fn from_config(path: &std::path::Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut paths: Self =
            toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let base = path.parent().unwrap_or(std::path::Path::new("."));
        for field in paths.fields() {
            if let Some(p) = field.as_mut().filter(|p| p.is_relative()) {
                *p = base.join(&*p);
            }
        }
        Ok(paths)
    }
    /// Fill unset fields from the environment, then the config file.
    pub fn resolve(mut self) -> Result<Self, String> {
        for (field, key) in self.fields().into_iter().zip(ENV) {
            if field.is_none() {
                *field = std::env::var_os(key)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from);
            }
        }
        if let Some(file) = Self::config_file() {
            let mut config = Self::from_config(&file)?;
            for (field, value) in self.fields().into_iter().zip(config.fields()) {
                if field.is_none() {
                    *field = value.take();
                }
            }
        }
        Ok(self)
    }
    /// Validate local resources before spawning a helper or modifying a profile.
    pub fn validate_files(&self) -> Result<(), String> {
        let firmware = self
            .firmware
            .as_ref()
            .ok_or("Select the G1 firmware ELF file")?;
        validate_firmware_elf(firmware)?;
        let factory = self
            .factory_a
            .as_ref()
            .ok_or("Select the extracted factory A directory")?;
        let resource = factory.join("programs/misc/armfir.dat");
        if !resource.is_file() {
            return Err(format!(
                "Factory resources are incomplete: missing {}",
                resource.display()
            ));
        }
        if let Some(rom) = &self.os_rom {
            if !rom.is_file() {
                return Err(format!("OS ROM is missing: {}", rom.display()));
            }
        }
        if let Some(initial) = &self.init_data {
            for drive in ["A", "C"] {
                if !initial.join(drive).is_dir() {
                    return Err(format!(
                        "Initialized storage is missing drive {drive}: {}",
                        initial.display()
                    ));
                }
            }
        }
        Ok(())
    }

    /// Firmware image and factory drive are the minimum to run the helper.
    pub fn configured(&self) -> bool {
        self.firmware.is_some() && self.factory_a.is_some()
    }
}

/// Header-only check shared by startup validation and read-only setup reports.
pub(crate) fn validate_firmware_elf(firmware: &std::path::Path) -> Result<(), String> {
    let mut file = std::fs::File::open(firmware)
        .map_err(|e| format!("Cannot read firmware {}: {e}", firmware.display()))?;
    let mut header = [0u8; 52];
    std::io::Read::read_exact(&mut file, &mut header).map_err(|e| {
        format!(
            "Firmware {} has a truncated ELF header: {e}",
            firmware.display()
        )
    })?;
    if &header[..4] != b"\x7fELF"
        || header[4] != 1
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != 40
    {
        return Err(format!(
            "{} is not a little-endian ARM ELF32 firmware image",
            firmware.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_arm_header_and_reports_exact_missing_resource() {
        let dir = tempfile::tempdir().unwrap();
        let firmware = dir.path().join("armfir.elf");
        let factory = dir.path().join("factory");
        let paths = AssetPaths {
            firmware: Some(firmware.clone()),
            factory_a: Some(factory.clone()),
            ..Default::default()
        };
        std::fs::write(&firmware, b"not an ELF").unwrap();
        assert!(paths.validate_files().unwrap_err().contains("truncated"));
        let mut header = [0u8; 52];
        header[..6].copy_from_slice(b"\x7fELF\x01\x01");
        header[18] = 40;
        std::fs::write(&firmware, header).unwrap();
        assert!(
            paths
                .validate_files()
                .unwrap_err()
                .contains("programs/misc/armfir.dat")
        );
        std::fs::create_dir_all(factory.join("programs/misc")).unwrap();
        std::fs::write(factory.join("programs/misc/armfir.dat"), []).unwrap();
        assert!(paths.validate_files().is_ok());
        header[18] = 62;
        std::fs::write(&firmware, header).unwrap();
        assert!(paths.validate_files().unwrap_err().contains("ARM ELF32"));
    }

    #[test]
    fn config_paths_are_relative_to_the_file_not_the_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("firmware.toml");
        assert_eq!(
            AssetPaths::from_config(&file).unwrap(),
            AssetPaths::default()
        );
        let absolute = dir.path().join("absolute-A");
        let encoded = toml::Value::String(absolute.to_string_lossy().into_owned());
        std::fs::write(
            &file,
            format!("firmware = \"fw/armfir.elf\"\nfactory_a = {encoded}\n"),
        )
        .unwrap();
        let paths = AssetPaths::from_config(&file).unwrap();
        assert_eq!(paths.firmware, Some(dir.path().join("fw/armfir.elf")));
        assert_eq!(paths.factory_a, Some(absolute));
        assert!(paths.configured());
        std::fs::write(&file, "firmwre = \"typo\"\n").unwrap();
        assert!(AssetPaths::from_config(&file).is_err());
    }
}
