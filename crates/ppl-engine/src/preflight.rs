//! Read-only setup checks. These checks never boot firmware or open USB.
use crate::{Engine, firmware_options::AssetPaths};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    NotChecked,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupCheck {
    pub component: String,
    pub status: CheckStatus,
    pub path: Option<PathBuf>,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightReport {
    pub schema_version: u32,
    pub engine: Engine,
    pub checks: Vec<SetupCheck>,
    pub resolved_paths: Option<AssetPaths>,
}
impl PreflightReport {
    pub fn passed(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.status == CheckStatus::Failed)
    }
    pub fn summary(&self) -> String {
        if self.engine == Engine::Native {
            "Native PPL is available; firmware resources are not required".into()
        } else if self.passed() {
            "Firmware files and helper are available; startup still checks session compatibility"
                .into()
        } else {
            format!(
                "Firmware setup needs attention: {} failed check(s)",
                self.checks
                    .iter()
                    .filter(|check| check.status == CheckStatus::Failed)
                    .count()
            )
        }
    }
    fn add(&mut self, component: &str, path: Option<PathBuf>, result: Result<String, String>) {
        let (status, message) = match result {
            Ok(message) => (CheckStatus::Passed, message),
            Err(message) => (CheckStatus::Failed, message),
        };
        self.checks.push(SetupCheck {
            component: component.into(),
            status,
            path,
            message,
        });
    }
    fn unchecked(&mut self, component: &str, path: Option<PathBuf>, message: &str) {
        self.checks.push(SetupCheck {
            component: component.into(),
            status: CheckStatus::NotChecked,
            path,
            message: message.into(),
        });
    }
}

/// Resolve the same flag/environment/config precedence used by execution.
/// Native checking deliberately does not read optional firmware configuration.
pub fn inspect(engine: Engine, overrides: AssetPaths) -> PreflightReport {
    let resolved = if engine == Engine::Native {
        Ok(overrides)
    } else {
        overrides.resolve()
    };
    inspect_resolved(engine, resolved, cfg!(target_os = "linux"))
}
fn inspect_resolved(
    engine: Engine,
    paths: Result<AssetPaths, String>,
    linux: bool,
) -> PreflightReport {
    let mut report = PreflightReport {
        schema_version: 1,
        engine,
        checks: Vec::new(),
        resolved_paths: None,
    };
    if engine == Engine::Native {
        report.add(
            "native",
            None,
            Ok("Native PPL execution is available".into()),
        );
        return report;
    }
    report.add(
        "platform",
        None,
        if linux {
            Ok("Firmware process supervision is available on Linux".into())
        } else {
            Err("Firmware mode currently requires Linux; Native PPL remains available".into())
        },
    );
    let paths = match paths {
        Ok(paths) => paths,
        Err(error) => {
            report.add("configuration", None, Err(error));
            return report;
        }
    };
    report.resolved_paths = Some(paths.clone());
    report.add("firmware", paths.firmware.clone(), match &paths.firmware {
        Some(path) => crate::firmware_options::validate_firmware_elf(path).map(|()| "Little-endian ARM ELF32 header is valid; executable loading is checked at startup".into()),
        None => Err("Select the G1 firmware ELF file (--firmware or PPL_FIRMWARE)".into()),
    });
    let resource = paths
        .factory_a
        .as_ref()
        .map(|path| path.join("programs/misc/armfir.dat"));
    report.add(
        "factory_resources",
        resource.clone(),
        match &resource {
            Some(path) if path.is_file() => Ok("Factory resource file is present".into()),
            Some(path) => Err(format!(
                "Factory resources are incomplete: missing {}",
                path.display()
            )),
            None => Err(
                "Select the extracted factory A directory (--factory-a or PPL_FACTORY_A)".into(),
            ),
        },
    );
    match &paths.os_rom {
        Some(path) => report.add(
            "os_rom",
            Some(path.clone()),
            if path.is_file() {
                Ok("OS ROM file is present".into())
            } else {
                Err(format!("OS ROM is missing: {}", path.display()))
            },
        ),
        None => report.unchecked("os_rom", None, "No optional OS ROM selected"),
    }
    match &paths.init_data {
        Some(path) => {
            for drive in ["A", "C"] {
                let directory = path.join(drive);
                report.add(&format!("initialized_{drive}"), Some(directory.clone()), if directory.is_dir() { Ok(format!("Initialized drive {drive} is present")) } else { Err(format!("Initialized storage is missing drive {drive}: {}", path.display())) });
            }
        }
        None => report.unchecked("initialized_storage", None, "No initialized storage selected; a new calculator profile may open the firmware's first-run setup"),
    }
    match crate::firmware::discover_runner(paths.runner.as_deref()) {
        Ok(path) => report.add("helper", Some(path), Ok("Helper executable is present; protocol compatibility is checked at session startup".into())),
        Err(error) => report.add("helper", paths.runner.clone(), Err(error)),
    }
    report.unchecked("session_compatibility", None, "Session protocol and firmware observation capabilities are negotiated at startup; this check does not boot firmware");
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_all_missing_resources_without_creating_any_files() {
        let root = tempfile::tempdir().unwrap();
        let paths = AssetPaths {
            firmware: Some(root.path().join("missing.elf")),
            factory_a: Some(root.path().join("factory")),
            os_rom: Some(root.path().join("missing.rom")),
            init_data: Some(root.path().join("initialized")),
            runner: Some(root.path().join("missing-helper")),
        };
        let report = inspect_resolved(Engine::Firmware, Ok(paths), true);
        for component in [
            "firmware",
            "factory_resources",
            "os_rom",
            "initialized_A",
            "initialized_C",
            "helper",
        ] {
            assert!(
                report.checks.iter().any(
                    |check| check.component == component && check.status == CheckStatus::Failed
                ),
                "{component}: {report:?}"
            );
        }
        assert!(!report.passed());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn native_is_independent_of_firmware_configuration_and_platform() {
        let report = inspect_resolved(Engine::Native, Err("bad optional config".into()), false);
        assert!(report.passed());
        assert!(report.summary().contains("Native PPL"));
        assert!(report.resolved_paths.is_none());
    }
    #[test]
    fn unsupported_platform_and_bad_configuration_are_actionable() {
        let report = inspect_resolved(
            Engine::Firmware,
            Err("config.toml: unknown field".into()),
            false,
        );
        assert_eq!(report.checks.len(), 2);
        assert!(
            report.checks[0]
                .message
                .contains("Native PPL remains available")
        );
        assert_eq!(report.checks[1].component, "configuration");
        assert!(!report.passed());
    }
    #[test]
    #[cfg(target_os = "linux")]
    fn valid_files_do_not_claim_runtime_or_protocol_verification() {
        let root = tempfile::tempdir().unwrap();
        let firmware = root.path().join("firmware.elf");
        let mut header = [0u8; 52];
        header[..6].copy_from_slice(b"\x7fELF\x01\x01");
        header[18] = 40;
        std::fs::write(&firmware, header).unwrap();
        std::fs::create_dir_all(root.path().join("factory/programs/misc")).unwrap();
        std::fs::write(root.path().join("factory/programs/misc/armfir.dat"), []).unwrap();
        let paths = AssetPaths {
            firmware: Some(firmware),
            factory_a: Some(root.path().join("factory")),
            runner: Some(std::env::current_exe().unwrap()),
            ..Default::default()
        };
        let report = inspect_resolved(Engine::Firmware, Ok(paths), true);
        assert!(report.passed(), "{report:?}");
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.component == "session_compatibility"
                    && check.status == CheckStatus::NotChecked)
        );
        assert!(report.summary().contains("startup still checks"));
        let encoded = serde_json::to_vec(&report).unwrap();
        let decoded: PreflightReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.checks, report.checks);
    }
}
