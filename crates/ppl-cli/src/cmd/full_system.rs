//! Synthetic hardware research only; never a fallback for PPL execution.
use clap::{Args, Subcommand};
use std::path::PathBuf;
#[derive(Debug, Args)]
pub struct Options {
    #[command(subcommand)]
    pub action: Action,
}
#[derive(Debug, Subcommand)]
pub enum Action {
    /// Validate a pinned research build without booting or accessing USB.
    Info {
        #[arg(long)]
        build_root: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Execute a self-authored synthetic ARM hardware case in isolated QEMU.
    Run {
        #[arg(long)]
        build_root: PathBuf,
        #[arg(long, value_parser=clap::builder::PossibleValuesParser::new(ppl_engine::full_system::CASES.iter().copied()))]
        case: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t=30, value_parser=clap::value_parser!(u64).range(1..=120))]
        timeout: u64,
        #[arg(long)]
        json: bool,
    },
}
pub fn run(options: Options) -> u8 {
    use ppl_engine::full_system;
    match options.action {
        Action::Info { build_root, json } => {
            let report = full_system::inspect(build_root.as_deref());
            if json {
                println!("{}", serde_json::to_string(&report).unwrap());
            } else {
                println!(
                    "FullSystem research: {}\n{}",
                    report.availability,
                    report.reason.as_deref().unwrap_or("")
                );
            }
            if report.availability == "configured" {
                0
            } else {
                2
            }
        }
        Action::Run {
            build_root,
            case,
            output,
            timeout,
            json,
        } => {
            let cancel = ppl_engine::Cancellation::default();
            let interrupt = cancel.clone();
            if let Err(error) = ctrlc::set_handler(move || interrupt.cancel()) {
                eprintln!("Cannot install research cancellation handler: {error}");
                return 2;
            }
            match full_system::execute(&build_root, &case, &output, timeout, &cancel) {
                Ok(report) => {
                    let passed = report["status"] == "passed";
                    if json {
                        println!("{report}");
                    } else {
                        println!(
                            "FullSystem {case}: {} (synthetic hardware; no kernel or physical verification)",
                            report["status"]
                        );
                    }
                    if passed { 0 } else { 1 }
                }
                Err(error) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"schema_version":1,"backend":"full_system","status":if cancel.cancelled() { "cancelled" } else { "infrastructure_error" },"kernel_boot_verified":false,"physical_hardware_verified":false,"error":error})
                        );
                    } else {
                        eprintln!("{error}");
                    }
                    if cancel.cancelled() { 130 } else { 2 }
                }
            }
        }
    }
}
