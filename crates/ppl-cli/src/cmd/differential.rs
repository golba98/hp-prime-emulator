//! Offline multi-backend scenarios and qualified physical-evidence replay.
use clap::{Args, ValueEnum};
use ppl_compat::differential::{self, Backend, BackendId, LoadedScenario};
use ppl_engine::{
    Cancellation,
    firmware_options::{AssetPaths, Options},
};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Choice {
    Native,
    FirmwareHle,
    PhysicalCapture,
    FullSystem,
}
#[derive(Debug, Args)]
pub struct DifferentialOpts {
    /// Versioned TOML scenario. Omit only with all three replay options.
    pub fixture: Option<PathBuf>,
    #[arg(long, value_enum, value_delimiter = ',', default_value = "native")]
    pub backend: Vec<Choice>,
    /// Structured JSON report (stdout is always JSON).
    #[arg(long)]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub firmware: Option<PathBuf>,
    #[arg(long)]
    pub os_rom: Option<PathBuf>,
    #[arg(long)]
    pub factory_a: Option<PathBuf>,
    #[arg(long)]
    pub init_data: Option<PathBuf>,
    #[arg(long)]
    pub firmware_runner: Option<PathBuf>,
    /// Existing physical EvidenceV2 JSON; replay performs no USB I/O.
    #[arg(long)]
    pub replay_evidence: Option<PathBuf>,
    #[arg(long)]
    pub replay_manifest: Option<PathBuf>,
    #[arg(long)]
    pub replay_suite: Option<PathBuf>,
    #[arg(long, default_value = "tests/compat/evidence")]
    pub evidence_dir: PathBuf,
}
pub fn run(o: DifferentialOpts) -> u8 {
    match perform(&o) {
        Ok((report, passed)) => {
            let text = serde_json::to_string_pretty(&report).unwrap() + "\n";
            if let Some(path) = o.output {
                if let Err(e) = std::fs::write(path, &text) {
                    eprintln!("cannot write differential report: {e}");
                    return 2;
                }
            }
            print!("{text}");
            if passed { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("differential: {e}");
            2
        }
    }
}
fn perform(o: &DifferentialOpts) -> Result<(serde_json::Value, bool), String> {
    let replay =
        o.replay_evidence.is_some() || o.replay_manifest.is_some() || o.replay_suite.is_some();
    if replay {
        if o.backend.iter().any(|b| !matches!(b, Choice::Native)) {
            return Err("legacy physical replay compares qualified TYPE/STRING with Native; scenario backend flags cannot be combined with replay".into());
        }
        if o.fixture.is_some() {
            return Err("scenario execution and legacy physical replay are separate scopes; omit the fixture for replay".into());
        }
        let evidence = o
            .replay_evidence
            .as_ref()
            .ok_or("replay requires --replay-evidence")?;
        let manifest = o
            .replay_manifest
            .as_ref()
            .ok_or("replay requires --replay-manifest")?;
        let suite = ppl_compat::load_suite(
            o.replay_suite
                .as_deref()
                .ok_or("replay requires --replay-suite")?,
        )?;
        let capture =
            differential::load_physical_replay(&suite, evidence, manifest, &o.evidence_dir)?;
        let report = differential::replay_legacy_cases(&suite, &capture);
        let passed = report.passed;
        return Ok((serde_json::to_value(report).unwrap(), passed));
    }
    let scenario = LoadedScenario::load(
        o.fixture
            .as_deref()
            .ok_or("a scenario fixture is required")?,
    )?;
    let mut backends: Vec<Box<dyn Backend>> = vec![];
    for choice in &o.backend {
        backends.push(match choice {
            Choice::Native=>Box::new(differential::Native),
            Choice::FirmwareHle=>{
                let paths=AssetPaths{runner:o.firmware_runner.clone(),firmware:o.firmware.clone(),os_rom:o.os_rom.clone(),factory_a:o.factory_a.clone(),init_data:o.init_data.clone()}.resolve()?;
                Box::new(differential::FirmwareHle{options:Options{runner:paths.runner,firmware:paths.firmware,os_rom:paths.os_rom,factory_a:paths.factory_a,init_data:paths.init_data,..Default::default()}})
            }
            Choice::PhysicalCapture=>Box::new(differential::Unavailable{backend:BackendId::PhysicalCapture,reason:"No live physical backend; use qualified --replay-evidence/--replay-manifest/--replay-suite for existing TYPE/STRING captures".into()}),
            Choice::FullSystem=>Box::new(differential::Unavailable{backend:BackendId::FullSystem,reason:"Full-system emulator is not implemented; no kernel/hardware execution evidence is available".into()}),
        });
    }
    let refs: Vec<_> = backends.iter().map(|b| b.as_ref()).collect();
    let report = differential::execute(&scenario, &refs, &Cancellation::default());
    let passed = report.passed;
    Ok((serde_json::to_value(report).unwrap(), passed))
}
