//! Offline preparation, sealing and comparison of physical scenario evidence.
//! No command in this module opens a USB device or sends calculator input.
use std::path::PathBuf;

use clap::{Args, Subcommand};
use ppl_compat::{capture, differential};

#[derive(Debug, Args)]
pub struct CaptureOpts {
    #[command(subcommand)]
    pub action: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Prepare an immutable plan from a shared scenario, without device access.
    Prepare {
        fixture: PathBuf,
        #[arg(long)]
        capture_id: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Validate manual/mock records and publish a new immutable evidence bundle.
    Seal {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        recording: PathBuf,
        #[arg(long)]
        artifacts: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify bundle integrity, actions, confirmations and optional fixture identity.
    Validate {
        bundle: PathBuf,
        #[arg(long)]
        fixture: Option<PathBuf>,
    },
    /// Compare a sealed capture against an executing backend's BackendRun JSON.
    /// This command replays evidence offline; it does not run the calculator.
    Compare {
        fixture: PathBuf,
        bundle: PathBuf,
        #[arg(long)]
        backend_run: PathBuf,
    },
}

pub fn run(opts: CaptureOpts) -> u8 {
    match perform(opts.action) {
        Ok((report, passed)) => {
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            if passed { 0 } else { 1 }
        }
        Err(error) => {
            eprintln!("capture: {error}");
            2
        }
    }
}

fn perform(action: Action) -> Result<(serde_json::Value, bool), String> {
    use serde_json::json;
    match action {
        Action::Prepare {
            fixture,
            capture_id,
            output,
        } => {
            let scenario = differential::LoadedScenario::load(&fixture)?;
            let plan = capture::CapturePlan::prepare(&scenario, &capture_id)?;
            capture::save_plan(&plan, &output)?;
            Ok((
                json!({"schema_version":1,"status":"prepared","plan":output,"plan_sha256":plan.sha256()?,"device_access":false}),
                true,
            ))
        }
        Action::Seal {
            plan,
            recording,
            artifacts,
            output,
        } => {
            let plan: capture::CapturePlan = capture::read_json(&plan)?;
            let recording: capture::Recording = capture::read_json(&recording)?;
            let bundle = capture::seal(&plan, &recording, &artifacts, &output)?;
            Ok((
                json!({"schema_version":1,"status":"sealed","bundle":output,"capture_sha256":bundle.sha256(),"origin":recording.origin,"device_access":false}),
                true,
            ))
        }
        Action::Validate { bundle, fixture } => {
            let capture = capture::SealedCapture::load(&bundle)?;
            if let Some(fixture) = fixture {
                capture
                    .plan()
                    .verify_scenario(&differential::LoadedScenario::load(&fixture)?)?;
            }
            Ok((
                json!({"schema_version":1,"status":"valid","capture_sha256":capture.sha256(),"origin":capture.recording().origin,"steps":capture.recording().steps.len(),"device_access":false}),
                true,
            ))
        }
        Action::Compare {
            fixture,
            bundle,
            backend_run,
        } => {
            let scenario = differential::LoadedScenario::load(&fixture)?;
            let capture = capture::SealedCapture::load(&bundle)?;
            let backend: differential::BackendRun = capture::read_json(&backend_run)?;
            let comparison = capture::compare_capture(&scenario, &capture, backend)?;
            let passed = comparison.comparison.passed;
            Ok((
                serde_json::to_value(comparison).map_err(|e| e.to_string())?,
                passed,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_is_offline_immutable_and_reports_no_device_access() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/differential/arithmetic.toml");
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("plan.json");
        let action = || Action::Prepare {
            fixture: fixture.clone(),
            capture_id: "CLI_MOCK_PLAN".into(),
            output: output.clone(),
        };
        let (report, passed) = perform(action()).unwrap();
        assert!(passed);
        assert_eq!(report["device_access"], false);
        assert!(perform(action()).is_err());
        let plan: capture::CapturePlan = capture::read_json(&output).unwrap();
        plan.validate().unwrap();
    }

    #[test]
    fn incomplete_bundle_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            perform(Action::Validate {
                bundle: tmp.path().to_path_buf(),
                fixture: None
            })
            .is_err()
        );
    }
}
