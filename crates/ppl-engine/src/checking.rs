//! Independent native and genuine local-firmware checking. Physical Check and
//! physical execution are deliberately not part of this service.
use crate::{Cancellation, Operation, Report, Status, firmware, native, source::Source};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckReport {
    pub schema_version: u32,
    pub source_sha256: String,
    pub native: Report,
    pub firmware: Option<Report>,
    pub firmware_status: String,
    pub physical_check_executed: bool,
}
impl CheckReport {
    pub fn accepted(&self) -> bool {
        self.native.status == Status::Success
            && self
                .firmware
                .as_ref()
                .is_none_or(|r| r.status == Status::Success)
    }
}
pub fn configured_firmware() -> Option<firmware::Options> {
    let paths = crate::firmware_options::AssetPaths::default()
        .resolve()
        .ok()?;
    let options = firmware::Options {
        runner: paths.runner,
        firmware: paths.firmware,
        os_rom: paths.os_rom,
        factory_a: paths.factory_a,
        init_data: paths.init_data,
        ..Default::default()
    };
    (options.runner.is_some()
        || options.firmware.is_some()
        || options.os_rom.is_some()
        || options.factory_a.is_some())
    .then_some(options)
}
pub fn check(
    source: Source,
    options: Option<&firmware::Options>,
    cancel: &Cancellation,
) -> CheckReport {
    let hash = source.identity.source_sha256.clone();
    let native = native::execute(
        source.clone(),
        Operation::Check,
        native::Options {
            entry: None,
            args: "[]".into(),
            json_arguments: true,
            host: ppl_vm::HeadlessHost::new(),
            settings: Default::default(),
            limits: Default::default(),
            seed: 0x5eed,
            png: None,
            deny_warnings: false,
            dump_bytecode: false,
        },
        cancel,
    );
    // Native rejection must not suppress the independent firmware verdict.
    let firmware = options.map(|o| firmware::execute(source, Operation::Check, o, cancel));
    let firmware_status = match &firmware {
        None => "not_configured".into(),
        Some(r) => serde_json::to_value(r.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "infrastructure_error".into()),
    };
    CheckReport {
        schema_version: 1,
        source_sha256: hash,
        native,
        firmware,
        firmware_status,
        physical_check_executed: false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_firmware_is_not_a_native_success() {
        let source = Source::text("EXPORT DEMO()\nBEGIN\nRETURN 1;\nEND;", "DEMO").unwrap();
        let r = check(
            source,
            Some(&firmware::Options::default()),
            &Cancellation::default(),
        );
        assert_eq!(r.native.status, Status::Success);
        assert!(r.firmware.is_some());
        assert!(!r.accepted());
        assert!(!r.physical_check_executed);
    }
    #[test]
    fn syntax_location_and_provenance_survive_json() {
        let source = Source::text("EXPORT DEMO()\nBEGIN\nLOCAL := ;\nEND;", "DEMO").unwrap();
        let r = check(source, None, &Cancellation::default());
        assert_eq!(r.native.status, Status::InvalidSource);
        assert!(
            r.native
                .diagnostics
                .iter()
                .any(|d| d.line == Some(3) && d.source == "native-compiler")
        );
        let json = serde_json::to_value(r).unwrap();
        assert_eq!(json["firmware_status"], "not_configured");
        assert_eq!(json["physical_check_executed"], false);
    }
}
