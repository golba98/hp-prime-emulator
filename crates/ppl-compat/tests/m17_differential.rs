//! Run the same checked-in multi-module scenarios without bespoke program code.
use ppl_compat::differential::{self, LoadedScenario};
use ppl_engine::{Cancellation, firmware_options::Options};
use std::path::Path;

fn fixtures() -> impl Iterator<Item = LoadedScenario> {
    ["initialization", "linkage", "typed_values"]
        .into_iter()
        .map(|name| {
            LoadedScenario::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../../tests/differential/m17/{name}.toml")),
            )
            .unwrap()
        })
}

#[test]
fn native_multi_module_initialization_and_edited_linkage() {
    for scenario in fixtures() {
        let result = differential::execute(
            &scenario,
            &[&differential::Native],
            &Cancellation::default(),
        );
        assert!(
            result.passed,
            "{}: {:?}",
            scenario.scenario.id, result.mismatches
        );
        assert!(result.assertions > 0);
    }
}

#[test]
fn real_firmware_multi_module_initialization_and_edited_linkage() {
    let (Some(firmware), Some(init), Some(runner)) = (
        std::env::var_os("PRIME_TEST_FIRMWARE"),
        std::env::var_os("PRIME_TEST_INIT_DATA"),
        std::env::var_os("PRIME_TEST_RUNNER"),
    ) else {
        eprintln!("local firmware unavailable: set PRIME_TEST_FIRMWARE/INIT_DATA/RUNNER");
        return;
    };
    let options = Options {
        runner: Some(runner.into()),
        firmware: Some(firmware.into()),
        init_data: Some(init.into()),
        os_rom: std::env::var_os("PRIME_TEST_OS_ROM").map(Into::into),
        ..Default::default()
    };
    for scenario in fixtures() {
        let result = differential::execute(
            &scenario,
            &[
                &differential::Native,
                &differential::FirmwareHle {
                    options: options.clone(),
                },
            ],
            &Cancellation::default(),
        );
        if let Some(dir) = std::env::var_os("PRIME_M17_EVIDENCE_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                Path::new(&dir).join(format!("{}.json", scenario.scenario.id)),
                serde_json::to_vec_pretty(&result).unwrap(),
            )
            .unwrap();
        }
        assert!(
            result.passed,
            "{}: {:?}",
            scenario.scenario.id, result.mismatches
        );
        // Each scenario creates and closes its own isolated retained session.
        assert_eq!(result.runs.len(), 2);
        assert!(result.assertions > 0);
    }
}
