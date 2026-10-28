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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
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
    /// Firmware image and factory drive are the minimum to run the helper.
    pub fn configured(&self) -> bool {
        self.firmware.is_some() && self.factory_a.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_paths_are_relative_to_the_file_not_the_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("firmware.toml");
        assert_eq!(
            AssetPaths::from_config(&file).unwrap(),
            AssetPaths::default()
        );
        std::fs::write(
            &file,
            "firmware = \"fw/armfir.elf\"\nfactory_a = \"/abs/A\"\n",
        )
        .unwrap();
        let paths = AssetPaths::from_config(&file).unwrap();
        assert_eq!(paths.firmware, Some(dir.path().join("fw/armfir.elf")));
        assert_eq!(paths.factory_a, Some(PathBuf::from("/abs/A")));
        assert!(paths.configured());
        std::fs::write(&file, "firmwre = \"typo\"\n").unwrap();
        assert!(AssetPaths::from_config(&file).is_err());
    }
}
