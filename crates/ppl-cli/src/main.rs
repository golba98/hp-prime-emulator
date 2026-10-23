//! `ppl`: native HP Prime PPL emulator.

mod cmd;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ppl", version, about = "HP Prime G1 PPL emulator", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a program headlessly (PRINT to stdout, optional PNG).
    Run(cmd::run::RunOpts),
    /// Validate a program without running it.
    Check(cmd::run::CheckOpts),
    /// List built-in commands and their emulation status.
    Commands(cmd::commands::CommandsOpts),
    /// Detect and talk to a physical HP Prime over USB.
    Device(cmd::device::DeviceOpts),
    /// Compare emulation with hardware or evidence.
    Compare(cmd::compare::CompareOpts),
    /// Run an interactive step debugger.
    Debug(cmd::debug::DebugOpts),
    /// Launch the native graphical calculator window.
    Gui(cmd::gui::GuiOpts),
    /// Generate test corpora, run campaigns, import real-world programs.
    Test(cmd::test::TestOpts),
}

const SUBCOMMANDS: &[&str] = &[
    "run",
    "check",
    "commands",
    "gui",
    "debug",
    "device",
    "compare",
    "test",
    "help",
    "-h",
    "--help",
    "-V",
    "--version",
];

fn main() -> ExitCode {
    // `ppl file.hpprgm …` (the Python-era form) means `ppl run file.hpprgm …`.
    let mut argv: Vec<String> = std::env::args().collect();
    if argv.len() > 1 && !SUBCOMMANDS.contains(&argv[1].as_str()) {
        argv.insert(1, "run".into());
    }
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(error)
            if error.use_stderr()
                && argv.get(1).is_some_and(|s| s == "device")
                && argv.iter().any(|s| s == "delete")
                && argv.iter().any(|s| s == "--json") =>
        {
            println!(
                "{}",
                serde_json::json!({"schema_version":1,"operation":"delete","backup_created":false,"deletion_confirmed":false,"execution_confirmed":false,"error":error.to_string()})
            );
            return ExitCode::from(2);
        }
        Err(error) => error.exit(),
    };
    let code = match cli.command {
        Command::Run(o) => cmd::run::run(o),
        Command::Check(o) => cmd::run::check(o),
        Command::Commands(o) => cmd::commands::list(o),
        Command::Device(o) => cmd::device::run(o),
        Command::Compare(o) => cmd::compare::run(o),
        Command::Debug(o) => cmd::debug::run(o),
        Command::Gui(o) => cmd::gui::run(o),
        Command::Test(o) => cmd::test::run(o),
    };
    ExitCode::from(code)
}
