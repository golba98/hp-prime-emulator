//! Offline native replay with explicitly captured Home settings. Never opens USB.
//! Usage: matched_settings SOURCE HANGLE HFORMAT HDIGITS [INPUT ...]
use std::{path::Path, time::Duration};

use ppl_engine::{Cancellation, Operation, native, source::Source};
use ppl_vm::{AngleMode, HeadlessHost, Limits, ScriptedInput, Settings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 5 {
        return Err("usage: matched_settings SOURCE HANGLE HFORMAT HDIGITS [INPUT ...]".into());
    }
    let angle_code: i64 = args[2].parse()?;
    let hformat: i64 = args[3].parse()?;
    let hdigits: i64 = args[4].parse()?;
    if !(0..=5).contains(&hformat) || !(0..=11).contains(&hdigits) {
        return Err("HFormat must be 0..5 and HDigits 0..11".into());
    }
    let mut settings = Settings {
        angle: AngleMode::from_code(angle_code).ok_or("HAngle must be 0..2")?,
        ..Settings::default()
    };
    settings.set_format(hformat, hdigits);
    let host = HeadlessHost {
        inputs: args[5..].iter().map(|s| ScriptedInput::parse(s)).collect(),
        ..HeadlessHost::new()
    };
    let mut report = native::execute(
        Source::file(Path::new(&args[1]))?,
        Operation::Run,
        native::Options {
            entry: None,
            args: "[]".into(),
            json_arguments: true,
            host,
            settings,
            limits: Limits {
                max_time: Some(Duration::from_secs(3)),
                max_steps: Some(500_000),
                ..Limits::default()
            },
            seed: 0x5EED,
            png: None,
            deny_warnings: false,
            dump_bytecode: false,
        },
        &Cancellation::default(),
    );
    report.backend["initial_settings"] = serde_json::json!({
        "HAngle": angle_code, "HFormat": hformat, "HDigits": hdigits,
        "remaining_settings": "native defaults; physical reference not captured"
    });
    let exit_code = report.exit_code;
    println!("{}", serde_json::to_string_pretty(&report)?);
    if exit_code != 0 {
        std::process::exit(exit_code.into());
    }
    Ok(())
}
