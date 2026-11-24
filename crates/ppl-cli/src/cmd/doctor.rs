//! Non-mutating checks for Native or firmware setup; no calculator interaction.
use clap::Args;
use ppl_engine::{Engine, firmware_options::AssetPaths, preflight};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct DoctorOpts {
    #[arg(long, value_enum, default_value = "firmware")]
    pub engine: super::unified::EngineChoice,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub firmware: Option<PathBuf>,
    #[arg(long)]
    pub factory_a: Option<PathBuf>,
    #[arg(long)]
    pub os_rom: Option<PathBuf>,
    #[arg(long)]
    pub init_data: Option<PathBuf>,
    #[arg(long)]
    pub firmware_runner: Option<PathBuf>,
}
pub fn run(options: DoctorOpts) -> u8 {
    let engine = match options.engine {
        super::unified::EngineChoice::Native => Engine::Native,
        super::unified::EngineChoice::Firmware => Engine::Firmware,
    };
    let report = preflight::inspect(
        engine,
        AssetPaths {
            runner: options.firmware_runner,
            firmware: options.firmware,
            factory_a: options.factory_a,
            os_rom: options.os_rom,
            init_data: options.init_data,
        },
    );
    if options.json {
        println!("{}", serde_json::to_string(&report).unwrap());
    } else {
        println!("{}", report.summary());
        for check in &report.checks {
            let status = match check.status {
                preflight::CheckStatus::Passed => "PASS",
                preflight::CheckStatus::Failed => "FAIL",
                preflight::CheckStatus::NotChecked => "NOT CHECKED",
            };
            println!("{status} {}: {}", check.component, check.message);
            if let Some(path) = &check.path {
                println!("  {}", path.display());
            }
        }
    }
    if report.passed() { 0 } else { 2 }
}
