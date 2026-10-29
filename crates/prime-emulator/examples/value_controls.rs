//! Local firmware controls only. Imports into a fresh emulated filesystem;
//! no USB/device interface is used.
use clap::Parser;
use prime_emulator::{
    checkprog::CheckConfig,
    runprog::{self, ExecutionControl, RunProgramConfig},
    runtime::ExecutionRequest,
};
use std::{path::PathBuf, time::Duration};

#[derive(Parser, serde::Serialize)]
struct Args {
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    firmware: PathBuf,
    #[arg(long)]
    os_rom: PathBuf,
    #[arg(long)]
    factory_a: PathBuf,
    #[arg(long)]
    init_data: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, value_delimiter = ',')]
    entries: Vec<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    std::fs::create_dir(&args.out)?;
    std::fs::write(
        args.out.join("configuration.json"),
        serde_json::to_vec_pretty(&args)?,
    )?;
    std::fs::copy(&args.source, args.out.join("input.ppl"))?;
    let cfg = RunProgramConfig {
        check: CheckConfig {
            program: args.source,
            name: None,
            as_source: true,
            firmware: args.firmware,
            os_rom: Some(args.os_rom),
            factory_a: args.factory_a,
            out_dir: args.out.join("work"),
            init_data: Some(args.init_data),
            settle_ms: 300,
            boot_timeout: Duration::from_secs(60),
            key_timeout: Duration::from_secs(30),
            max_seconds: 180.,
            max_instructions: 8_000_000_000,
        },
        requests: args
            .entries
            .into_iter()
            .map(|entry| ExecutionRequest {
                entry,
                arguments: Vec::new(),
            })
            .collect(),
        runtime_keys: Vec::new(),
        execution_timeout: Duration::from_secs(3),
        control: ExecutionControl::default(),
        trace_runtime: false,
        screenshot: None,
    };
    let report = runprog::run(&cfg);
    std::fs::write(
        args.out.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{:?}: {} invocation(s)",
        report.execution_status,
        report.runtime.invocations.len()
    );
    Ok(())
}
