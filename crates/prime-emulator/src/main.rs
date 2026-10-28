//! `prime-emulator` — headless firmware-execution CLI (M2).
//!
//! Exit codes:
//! - 0: verified firmware heartbeat
//! - 1: guest fault
//! - 2: stopped on an unimplemented service (strict policy)
//! - 3: execution limit reached without a heartbeat
//! - 4: configuration or load error
//! - 5: key script failed (boot or a key did not settle in time)
//!
//! With `--keys`, 0 means the key script completed (every key delivered
//! and settled); the heartbeat is still recorded in the report.

use clap::Parser;
use prime_emulator::keys;
use prime_emulator::runner::{self, Outcome, RunConfig};
use prime_emulator::script::ScriptConfig;
use prime_emulator::services::UnsupportedPolicy;
use prime_emulator::trace::TraceConfig;

#[derive(Debug, Parser)]
#[command(
    name = "prime-emulator",
    version,
    about = "Headless HP Prime G1 firmware emulator (executes the original armfir.elf)"
)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, clap::Subcommand)]
enum Cmd {
    /// Load and execute a firmware ELF.
    Run(RunArgs),
    /// Long-lived local IDE calculator session (versioned stdin/stdout protocol).
    Session {
        #[arg(long)]
        firmware: std::path::PathBuf,
        #[arg(long)]
        os_rom: Option<std::path::PathBuf>,
        #[arg(long)]
        data_dir: std::path::PathBuf,
    },
    /// Check imported PPL using the original firmware Program Editor.
    CheckProgram(CheckProgramArgs),
    /// Execute an exported PPL function through the original firmware runtime.
    RunProgram(RunProgramArgs),
    /// M4-C: three firmware runs (2+2, 4, 2+3) with differential
    /// result-glyph comparison.
    VerifyCalc(VerifyCalcArgs),
    /// M5-B: import a PPL program into a guest data directory as
    /// `C:\DATA\<NAME>.hpprgm` (transactional; never touches other files).
    Import(ImportArgs),
    /// Decode an `.hpprgm` container (strict) and print its structure.
    InspectProgram(InspectArgs),
    /// M5-C/D: import programs into an initialised data directory and
    /// verify them in the firmware's Program Catalog and Program Editor.
    VerifyProgram(VerifyProgramArgs),
    /// Run the application compatibility suite (tests/runtime) against the
    /// original firmware and write per-application reports.
    AppSuite(AppSuiteArgs),
    /// Compare two framebuffer PNGs: exact pixel parity, then similarity.
    FrameDiff(FrameDiffArgs),
}

#[derive(Debug, clap::Args)]
struct AppSuiteArgs {
    #[command(flatten)]
    load: LoadArgs,
    #[arg(long)]
    factory_a: std::path::PathBuf,
    /// Program-free initialized storage, copied before use.
    #[arg(long)]
    init_data: Option<std::path::PathBuf>,
    /// Suite root holding `cases/` and `reference/`.
    #[arg(long, default_value = "tests/runtime")]
    suite: std::path::PathBuf,
    #[arg(long)]
    out_dir: std::path::PathBuf,
    /// Run only this case id.
    #[arg(long)]
    only: Option<String>,
    /// Record local-firmware references from runs matching their
    /// expectations (never physical evidence).
    #[arg(long)]
    record: bool,
}

#[derive(Debug, clap::Args)]
struct FrameDiffArgs {
    actual: std::path::PathBuf,
    reference: std::path::PathBuf,
    /// PNG whose non-black pixels are ignored.
    #[arg(long)]
    mask: Option<std::path::PathBuf>,
    /// Write a visualization (changed pixels in magenta).
    #[arg(long)]
    out: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Args)]
struct RunProgramArgs {
    #[command(flatten)]
    check: CheckProgramArgs,
    #[arg(long)]
    entry: Option<String>,
    /// Original source stem used only for automatic export selection.
    #[arg(long, conflicts_with = "entry")]
    entry_stem: Option<String>,
    /// JSON array of numbers or {"type":"real","value":"1e400"} objects.
    #[arg(long, default_value = "[]")]
    args: String,
    /// Normal calculator key/touch script, replayed only during execution.
    #[arg(long)]
    runtime_keys: Option<String>,
    #[arg(long, default_value_t = 30.0)]
    execution_timeout_s: f64,
    /// Capture the runtime's display before Home redraws over graphics.
    #[arg(long)]
    screenshot: Option<std::path::PathBuf>,
    /// Include framebuffer writer counters (no full-instruction tracing).
    #[arg(long)]
    trace_runtime: bool,
}

#[derive(Debug, clap::Args)]
struct CheckProgramArgs {
    /// Caller correlation token; echoed without affecting guest execution.
    #[arg(long)]
    request_id: Option<String>,
    #[command(flatten)]
    load: LoadArgs,
    #[arg(long)]
    program: std::path::PathBuf,
    #[arg(long)]
    name: Option<String>,
    /// Treat plain-text .hpprgm input as source (M5 import semantics).
    #[arg(long)]
    as_source: bool,
    #[arg(long)]
    factory_a: std::path::PathBuf,
    /// Parent for unique retained run directories; never cleaned.
    #[arg(long)]
    out_dir: Option<std::path::PathBuf>,
    /// Program-free initialized storage, copied before use.
    #[arg(long)]
    init_data: Option<std::path::PathBuf>,
    #[arg(long)]
    report: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = 300)]
    settle_ms: u64,
    #[arg(long, default_value_t = 60.0)]
    boot_timeout_s: f64,
    #[arg(long, default_value_t = 20.0)]
    key_timeout_s: f64,
    #[arg(long, default_value_t = 300.0)]
    max_seconds: f64,
    #[arg(long, default_value_t = 8_000_000_000)]
    max_instructions: u64,
}

#[derive(Debug, clap::Args)]
struct VerifyProgramArgs {
    #[command(flatten)]
    load: LoadArgs,
    /// Factory `A:` tree (for the initialising first boot).
    #[arg(long)]
    factory_a: std::path::PathBuf,
    /// Base output directory (`init/`, `control/`, `session1/`, `session2/`).
    #[arg(long)]
    out_dir: std::path::PathBuf,
    /// Program to import (repeatable; at most 8). `NAME=PATH` sets the name.
    #[arg(long = "program", required = true)]
    programs: Vec<String>,
    /// Treat inputs as source text even with a .hpprgm extension.
    #[arg(long)]
    as_source: bool,
    /// Start from this initialised data directory (copied, not modified)
    /// instead of a first boot.
    #[arg(long)]
    init_data: Option<std::path::PathBuf>,
    /// Skip the second boot (persistence check).
    #[arg(long)]
    no_persistence: bool,
    /// First-run wizard navigation keys.
    #[arg(long, default_value = prime_emulator::calcverify::DEFAULT_SETUP_KEYS)]
    setup_keys: String,
    #[arg(long, default_value_t = 300)]
    settle_ms: u64,
    #[arg(long, default_value_t = 60.0)]
    boot_timeout_s: f64,
    #[arg(long, default_value_t = 20.0)]
    key_timeout_s: f64,
    #[arg(long, default_value_t = 8_000_000_000)]
    max_instructions: u64,
    #[arg(long, default_value_t = 600.0)]
    max_seconds: f64,
    #[arg(short = 'v', long, default_value_t = 0)]
    verbosity: u8,
    /// Write the report (JSON) here as well (default: OUT_DIR/report.json).
    #[arg(long)]
    report: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Args)]
struct ImportArgs {
    /// PPL source (.ppl/.txt, UTF-8 or UTF-16 with BOM) or a genuine
    /// .hpprgm container.
    #[arg(long)]
    program: std::path::PathBuf,
    /// Guest data directory, initialised by one firmware boot.
    #[arg(long)]
    data_dir: std::path::PathBuf,
    /// Program name (default: the input file's stem).
    #[arg(long)]
    name: Option<String>,
    /// Replace an existing program of the same name (case-insensitive).
    #[arg(long)]
    replace: bool,
    /// Treat the input as source text even with a .hpprgm extension.
    #[arg(long)]
    as_source: bool,
    /// Write the JSON report here as well.
    #[arg(long)]
    report: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Args)]
struct InspectArgs {
    /// The .hpprgm file.
    file: std::path::PathBuf,
}

/// Shared loader options.
#[derive(Debug, clap::Args)]
struct LoadArgs {
    /// Path to armfir.elf (user-supplied; never bundled).
    #[arg(long)]
    firmware: std::path::PathBuf,
    /// Optional PRIME_OS.ROM, mapped inert at 0x30000000.
    #[arg(long)]
    os_rom: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Args)]
struct VerifyCalcArgs {
    #[command(flatten)]
    load: LoadArgs,
    /// Factory `A:` tree (device flash contents) copied into each run's
    /// fresh data directory; the wizard needs the firmware's own
    /// `armfir.dat`/DLLs to render its pages.
    #[arg(long)]
    factory_a: std::path::PathBuf,
    /// Base output directory (`A/`, `B/`, `C/` are created inside).
    #[arg(long)]
    out_dir: std::path::PathBuf,
    /// First-run wizard navigation keys.
    #[arg(long, default_value = prime_emulator::calcverify::DEFAULT_SETUP_KEYS)]
    setup_keys: String,
    /// Screen-unchanged period that counts as settled.
    #[arg(long, default_value_t = 300)]
    settle_ms: u64,
    #[arg(long, default_value_t = 60.0)]
    boot_timeout_s: f64,
    #[arg(long, default_value_t = 20.0)]
    key_timeout_s: f64,
    #[arg(long, default_value_t = 2_000_000_000)]
    max_instructions: u64,
    #[arg(long, default_value_t = 600.0)]
    max_seconds: f64,
    /// Record framebuffer stores and guest PCs on run A.
    #[arg(long)]
    lcd_watch: bool,
    #[arg(short = 'v', long, default_value_t = 1)]
    verbosity: u8,
    /// Write the differential report (JSON) to this path.
    #[arg(long)]
    report: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum UiTimers {
    Auto,
    On,
    Off,
}

#[derive(Debug, clap::Args)]
struct RunArgs {
    /// Path to armfir.elf (user-supplied; never bundled).
    #[arg(long)]
    firmware: std::path::PathBuf,
    /// Optional PRIME_OS.ROM, mapped inert at 0x30000000.
    #[arg(long)]
    os_rom: Option<std::path::PathBuf>,
    /// Host directory serving guest drive letters (default ./prime_data).
    #[arg(long, default_value = "prime_data")]
    data_dir: std::path::PathBuf,
    /// Strict (stop at first unimplemented service) or zero (log and
    /// return 0; diagnostic mode — never reported as success).
    #[arg(long, default_value = "stop")]
    on_unsupported: String,
    /// Total instruction budget.
    #[arg(long, default_value_t = 20_000_000)]
    max_instructions: u64,
    /// Wall-clock budget in seconds.
    #[arg(long, default_value_t = 30.0)]
    max_seconds: f64,
    /// Trace every SVC invocation.
    #[arg(long)]
    trace_svc: bool,
    /// Trace every instruction (heavy; enables fault-context dumps).
    #[arg(long)]
    trace_code: bool,
    /// Write JSONL diagnostics to this path.
    #[arg(long)]
    jsonl: Option<std::path::PathBuf>,
    /// Write the final run report (JSON) to this path.
    #[arg(long)]
    report: Option<std::path::PathBuf>,
    /// Verbosity: 0 quiet, 1 stages/faults, 2 +svc, 3 everything.
    #[arg(short = 'v', long, default_value_t = 1)]
    verbosity: u8,
    /// Inject this key code (hex, e.g. 0x01) every N ms as a heartbeat
    /// probe, e.g. `--inject-key 0x01:250`.
    #[arg(long, conflicts_with = "keys")]
    inject_key: Option<String>,
    /// Minimum delivered input events before declaring a heartbeat.
    #[arg(long, default_value_t = 3)]
    heartbeat_min_events: u32,
    /// Virtual IRQ tick interval in ms (large = effectively disabled).
    #[arg(long, default_value_t = 10)]
    irq_tick_ms: u64,
    /// Guest write watchpoint `0xADDR,SIZE` (repeatable, diagnostics).
    #[arg(long = "watch")]
    watch: Vec<String>,
    /// Key script, e.g. "2,+,2,ENTER" (names or 0xNN codes). Each key is
    /// sent once the firmware has processed the previous one and the
    /// screen has settled; the run ends when the script completes.
    #[arg(long)]
    keys: Option<String>,
    /// Screen-unchanged period (ms) that counts as settled.
    #[arg(long, default_value_t = 300)]
    settle_ms: u64,
    /// Idle observation (ms) after boot, before the first key.
    #[arg(long, default_value_t = 1500)]
    idle_observe_ms: u64,
    /// Seconds to wait for a stable screen after boot.
    #[arg(long, default_value_t = 60.0)]
    boot_timeout_s: f64,
    /// Seconds to wait for each key to be processed.
    #[arg(long, default_value_t = 20.0)]
    key_timeout_s: f64,
    /// Save the final firmware framebuffer as PNG.
    #[arg(long)]
    screenshot: Option<std::path::PathBuf>,
    /// Save initial.png and one PNG per scripted key in this directory.
    #[arg(long)]
    screenshot_dir: Option<std::path::PathBuf>,
    /// Record framebuffer stores and their guest PCs (diagnostics; slows
    /// execution).
    #[arg(long)]
    lcd_watch: bool,
    /// Periodic UI timer events through GetEvent (SYS_TIMER ~183 ms,
    /// DEVICE_TIMER ~1 s, reference cadence): `auto` = on with `--keys`
    /// (the interactive UI needs them), off otherwise (reproduces M3).
    #[arg(long, value_enum, default_value_t = UiTimers::Auto)]
    ui_timers: UiTimers,
    /// Record every guest file-service call (paths, modes, byte counts,
    /// directory searches) in the report.
    #[arg(long)]
    trace_fs: bool,
    /// Passively observe the verified G1 compiler path (version-pinned).
    #[arg(long)]
    observe_compiler: bool,
    /// Report files the run created, changed or deleted in --data-dir.
    #[arg(long)]
    storage_diff: bool,
}

fn main() {
    let args = Args::parse();
    let code = match args.cmd {
        Cmd::Run(a) => run(a),
        Cmd::Session {
            firmware,
            os_rom,
            data_dir,
        } => prime_emulator::session::run(firmware, os_rom, data_dir),
        Cmd::CheckProgram(a) => check_program(a),
        Cmd::RunProgram(a) => run_program(a),
        Cmd::VerifyCalc(a) => verify_calc(a),
        Cmd::Import(a) => import(a),
        Cmd::InspectProgram(a) => inspect_program(a),
        Cmd::VerifyProgram(a) => verify_program(a),
        Cmd::AppSuite(a) => app_suite(a),
        Cmd::FrameDiff(a) => frame_diff(a),
    };
    std::process::exit(code);
}

/// Exit 0 when every case passes against local-firmware references, 1 when
/// any differs, 4 on configuration errors.
fn app_suite(a: AppSuiteArgs) -> i32 {
    use prime_emulator::appsuite;
    let cfg = appsuite::SuiteConfig {
        root: a.suite,
        firmware: a.load.firmware,
        os_rom: a.load.os_rom,
        factory_a: a.factory_a,
        init_data: a.init_data,
        out_dir: a.out_dir,
        record: a.record,
    };
    match appsuite::run_suite(&cfg, a.only.as_deref()) {
        Ok(results) => {
            print!("{}", appsuite::summary(&results));
            if results.iter().all(|r| r.as_expected()) {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("app-suite: {e}");
            4
        }
    }
}

/// Exit 0 for exact parity, 1 for any difference, 4 on errors.
fn frame_diff(a: FrameDiffArgs) -> i32 {
    use prime_emulator::framediff;
    let result = (|| -> Result<framediff::Comparison, String> {
        let actual = framediff::load(&a.actual)?;
        let reference = framediff::load(&a.reference)?;
        let mask = a.mask.as_deref().map(framediff::load_mask).transpose()?;
        let c = framediff::compare(&actual, &reference, mask.as_deref())?;
        if let Some(out) = &a.out {
            framediff::visualize(&actual, &reference)
                .save_png(out)
                .map_err(|e| e.to_string())?;
        }
        Ok(c)
    })();
    match result {
        Ok(c) => {
            println!("{}", serde_json::to_string_pretty(&c).unwrap());
            if c.exact { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("frame-diff: {e}");
            4
        }
    }
}

fn run_program(a: RunProgramArgs) -> i32 {
    use prime_emulator::{
        runprog,
        runtime::{ExecutionRequest, ExecutionStatus},
        runtime_value,
    };
    let duration = |s: f64| {
        std::time::Duration::try_from_secs_f64(s)
            .ok()
            .filter(|d| !d.is_zero())
    };
    let boot = duration(a.check.boot_timeout_s);
    let key = duration(a.check.key_timeout_s);
    let execution = duration(a.execution_timeout_s);
    let cfg = prime_emulator::checkprog::CheckConfig {
        program: a.check.program,
        name: a.check.name,
        as_source: a.check.as_source,
        firmware: a.check.load.firmware,
        os_rom: a.check.load.os_rom,
        factory_a: a.check.factory_a,
        out_dir: a
            .check
            .out_dir
            .unwrap_or_else(|| std::env::temp_dir().join("prime-firmware-runs")),
        init_data: a.check.init_data,
        settle_ms: a.check.settle_ms,
        boot_timeout: boot.unwrap_or_default(),
        key_timeout: key.unwrap_or_default(),
        max_seconds: a.check.max_seconds,
        max_instructions: a.check.max_instructions,
    };
    let mut report = (|| -> Result<runprog::ProgramReport, String> {
        if boot.is_none() || key.is_none() || execution.is_none() {
            return Err("timeouts must be positive finite durations".into());
        }
        let arguments = runtime_value::parse_arguments(&a.args)?;
        let runtime_keys = a
            .runtime_keys
            .as_deref()
            .map(prime_emulator::keys::parse_keys)
            .transpose()?
            .unwrap_or_default();
        let control = runprog::ExecutionControl::default();
        let interrupted = control.clone();
        if let Err(e) = ctrlc::set_handler(move || interrupted.cancel()) {
            return Ok(runprog::ProgramReport::rejected(
                &cfg,
                ExecutionStatus::InfrastructureError,
                e.to_string(),
            ));
        }
        let run_cfg = runprog::RunProgramConfig {
            check: cfg.clone(),
            requests: vec![ExecutionRequest {
                entry: a.entry.clone().unwrap_or_default(),
                arguments,
            }],
            runtime_keys,
            execution_timeout: execution.unwrap(),
            control,
            trace_runtime: a.trace_runtime,
            screenshot: a.screenshot,
        };
        Ok(if a.entry.is_some() {
            runprog::run(&run_cfg)
        } else {
            let stem = a
                .entry_stem
                .as_deref()
                .or_else(|| cfg.program.file_stem().and_then(|s| s.to_str()))
                .unwrap_or("");
            runprog::run_auto_entry(&run_cfg, stem)
        })
    })()
    .unwrap_or_else(|e| {
        runprog::ProgramReport::rejected(&cfg, ExecutionStatus::InvalidArguments, e)
    });
    report.request_id = a.check.request_id;
    if let Some(path) = a.check.report {
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, serde_json::to_vec_pretty(&report)?)
        })();
        if let Err(e) = result {
            report.execution_status = ExecutionStatus::InfrastructureError;
            report.evidence = format!("report write failed: {e}");
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("serializable runtime report")
    );
    report.execution_status.exit_code()
}

fn check_program(a: CheckProgramArgs) -> i32 {
    let duration = |s: f64| {
        std::time::Duration::try_from_secs_f64(s)
            .ok()
            .filter(|d| !d.is_zero())
    };
    let (Some(boot_timeout), Some(key_timeout)) =
        (duration(a.boot_timeout_s), duration(a.key_timeout_s))
    else {
        eprintln!("error: boot and key timeouts must be positive, finite durations");
        return 4;
    };
    let cfg = prime_emulator::checkprog::CheckConfig {
        program: a.program,
        name: a.name,
        as_source: a.as_source,
        firmware: a.load.firmware,
        os_rom: a.load.os_rom,
        factory_a: a.factory_a,
        out_dir: a
            .out_dir
            .unwrap_or_else(|| std::env::temp_dir().join("prime-firmware-checks")),
        init_data: a.init_data,
        settle_ms: a.settle_ms,
        boot_timeout,
        key_timeout,
        max_seconds: a.max_seconds,
        max_instructions: a.max_instructions,
    };
    let mut report = prime_emulator::checkprog::check(&cfg);
    report.request_id = a.request_id;
    let json = serde_json::to_string_pretty(&report).expect("serializable compiler report");
    if let Some(path) = a.report {
        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &json)
        })();
        if let Err(e) = result {
            eprintln!("error writing {}: {e}", path.display());
            println!("{json}");
            return 4;
        }
    }
    println!("{json}");
    report.status.exit_code()
}

fn write_json(path: &std::path::Path, value: &impl serde::Serialize) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(value).unwrap()) {
        eprintln!("warning: could not write report: {e}");
    }
}

/// Exit codes: 0 imported, 4 configuration error, 7 rejected (name,
/// conflict, invalid input; nothing written).
fn import(a: ImportArgs) -> i32 {
    use prime_emulator::import::{ImportError, ImportRequest};
    let req = ImportRequest {
        program: a.program,
        data_dir: a.data_dir,
        name: a.name,
        replace: a.replace,
        as_source: a.as_source,
    };
    match prime_emulator::import::import(&req) {
        Ok(report) => {
            if let Some(path) = &a.report {
                write_json(path, &report);
            }
            println!("{}", serde_json::to_string_pretty(&report).unwrap());
            0
        }
        Err(e) => {
            let value = serde_json::json!({ "imported": false, "error": e.to_string() });
            if let Some(path) = &a.report {
                write_json(path, &value);
            }
            eprintln!("import failed: {e}");
            match e {
                ImportError::Io(_) | ImportError::NotInitialized(_) => 4,
                _ => 7,
            }
        }
    }
}

/// Exit codes: 0 every program passed, 6 a check failed, 4 bad arguments.
fn verify_program(a: VerifyProgramArgs) -> i32 {
    use prime_emulator::progverify::{ProgramSpec, ProgramVerifyConfig};
    use std::time::Duration;
    let programs = a
        .programs
        .iter()
        .map(|p| match p.split_once('=') {
            Some((name, path)) if !name.contains(['/', '\\', '.']) => ProgramSpec {
                path: path.into(),
                name: Some(name.to_owned()),
                as_source: a.as_source,
            },
            _ => ProgramSpec {
                path: p.into(),
                name: None,
                as_source: a.as_source,
            },
        })
        .collect();
    let cfg = ProgramVerifyConfig {
        firmware_path: a.load.firmware,
        os_rom_path: a.load.os_rom,
        factory_a: a.factory_a,
        out_dir: a.out_dir.clone(),
        programs,
        init_data: a.init_data,
        setup_keys: a.setup_keys,
        settle_ms: a.settle_ms,
        boot_timeout: Duration::from_secs_f64(a.boot_timeout_s),
        key_timeout: Duration::from_secs_f64(a.key_timeout_s),
        max_instructions: a.max_instructions,
        max_seconds: a.max_seconds,
        persistence: !a.no_persistence,
        verbosity: a.verbosity,
    };
    let report = prime_emulator::progverify::verify(&cfg);
    write_json(&a.out_dir.join("report.json"), &report);
    if let Some(path) = &a.report {
        write_json(path, &report);
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if report.status == "passed" { 0 } else { 6 }
}

fn inspect_program(a: InspectArgs) -> i32 {
    use prime_emulator::hpprgm::{self, Body, Object};
    let bytes = match std::fs::read(&a.file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}: {e}", a.file.display());
            return 4;
        }
    };
    let c = match hpprgm::parse(&bytes) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", a.file.display());
            return 7;
        }
    };
    fn show(objs: &[Object], depth: usize) {
        for o in objs {
            let pad = "  ".repeat(depth);
            match &o.body {
                Body::Data(p) => {
                    let text = match hpprgm::kind(o.ty) {
                        0x0B => hpprgm::name64(p).map(|s| format!(" {s:?}")),
                        0x05 if p.len() == 4 => Some(format!(
                            " {}",
                            u32::from_le_bytes(p[..4].try_into().unwrap())
                        )),
                        0x1B => Some(format!(" ({} bytes of UTF-16)", p.len())),
                        _ => None,
                    };
                    let hex: String = p.iter().take(16).map(|b| format!("{b:02x}")).collect();
                    println!(
                        "{pad}@{:#06x} {:08x} [{}]{}",
                        o.offset,
                        o.ty,
                        p.len(),
                        text.unwrap_or_else(|| format!(" {hex}"))
                    );
                }
                Body::List(groups) => {
                    println!(
                        "{pad}@{:#06x} {:08x} list of {} group(s)",
                        o.offset,
                        o.ty,
                        groups.len()
                    );
                    for g in groups {
                        show(g, depth + 1);
                    }
                }
            }
        }
    }
    println!(
        "{} ({} bytes, sha256 {})",
        a.file.display(),
        bytes.len(),
        prime_emulator::sha256::hex(&bytes)
    );
    show(&c.objects, 0);
    println!("header ints: {:?}", c.header_ints());
    println!("exports: {:?}", c.exports());
    match c.source() {
        Ok(s) => {
            println!(
                "file {:?} int {} source {} chars:",
                s.name,
                s.int,
                s.source.chars().count()
            );
            println!("{}", s.source);
            0
        }
        Err(e) => {
            println!("source: {e}");
            7
        }
    }
}

fn verify_calc(a: VerifyCalcArgs) -> i32 {
    use prime_emulator::calcverify::{CalcVerifyConfig, CalcVerifyOutcome};
    use std::time::Duration;
    let cfg = CalcVerifyConfig {
        firmware_path: a.load.firmware,
        os_rom_path: a.load.os_rom,
        factory_a: a.factory_a,
        out_dir: a.out_dir,
        setup_keys: a.setup_keys,
        settle_ms: a.settle_ms,
        boot_timeout: Duration::from_secs_f64(a.boot_timeout_s),
        key_timeout: Duration::from_secs_f64(a.key_timeout_s),
        max_instructions: a.max_instructions,
        max_seconds: a.max_seconds,
        lcd_watch: a.lcd_watch,
        verbosity: a.verbosity,
    };
    let (report, outcome) = prime_emulator::calcverify::verify(&cfg);
    if let Some(path) = &a.report {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&report).unwrap()) {
            eprintln!("warning: could not write report: {e}");
        }
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    match outcome {
        CalcVerifyOutcome::Passed => 0,
        CalcVerifyOutcome::Failed(_) => 6,
        CalcVerifyOutcome::ConfigError(_) => 4,
    }
}

fn run(a: RunArgs) -> i32 {
    let policy = match a.on_unsupported.as_str() {
        "stop" => UnsupportedPolicy::Stop,
        "zero" => UnsupportedPolicy::ZeroReturn,
        other => {
            eprintln!("invalid --on-unsupported '{other}' (use stop|zero)");
            return 4;
        }
    };
    let inject_key = a.inject_key.and_then(|s| {
        let (code, ms) = s.split_once(':')?;
        let key = u16::from_str_radix(code.trim_start_matches("0x"), 16).ok()?;
        let interval: u64 = ms.parse().ok()?;
        Some((key, interval))
    });
    let script = match &a.keys {
        None => None,
        Some(k) => match keys::parse_keys(k) {
            Ok(keys) => Some(ScriptConfig {
                keys,
                settle_ms: a.settle_ms,
                idle_observe_ms: a.idle_observe_ms,
                boot_timeout: std::time::Duration::from_secs_f64(a.boot_timeout_s),
                key_timeout: std::time::Duration::from_secs_f64(a.key_timeout_s),
                screenshot_dir: a.screenshot_dir.clone(),
                start_with_keys: false,
            }),
            Err(e) => {
                eprintln!("invalid --keys: {e}");
                return 4;
            }
        },
    };
    if script.is_none() && a.screenshot_dir.is_some() {
        eprintln!("--screenshot-dir needs --keys (use --screenshot for the final frame)");
        return 4;
    }
    if let Some(fw) = a.firmware.parent() {
        let _ = std::fs::create_dir_all(fw);
    }
    let cfg = RunConfig {
        firmware_path: a.firmware,
        os_rom_path: a.os_rom,
        data_dir: a.data_dir,
        firmware_guest_path: "A:\\programs\\misc\\armfir.elf".to_owned(),
        slice_instructions: 200_000,
        max_instructions: a.max_instructions,
        max_seconds: a.max_seconds,
        on_unsupported: policy,
        heartbeat_min_events: a.heartbeat_min_events,
        irq_tick_ms: a.irq_tick_ms,
        inject_key,
        watches: a
            .watch
            .iter()
            .filter_map(|s| {
                let (addr, size) = s.split_once(',')?;
                let addr = u32::from_str_radix(addr.trim_start_matches("0x"), 16).ok()?;
                let size = size.trim().parse::<u32>().ok()?;
                Some((addr, size))
            })
            .collect(),
        trace: TraceConfig {
            svc_trace: a.trace_svc,
            code_trace: a.trace_code,
            jsonl_path: a.jsonl,
            verbosity: a.verbosity,
            ring_capacity: 4096,
            fs_trace: a.trace_fs,
            compiler: a.observe_compiler,
            runtime: false,
        },
        script,
        screenshot: a.screenshot,
        lcd_watch: a.lcd_watch,
        storage_diff: a.storage_diff,
        program: None,
        ui_timers: match a.ui_timers {
            UiTimers::Auto => a.keys.is_some(),
            UiTimers::On => true,
            UiTimers::Off => false,
        },
    };
    let summary = match runner::run(&cfg) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 4;
        }
    };
    if let Some(path) = &a.report {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&summary.report).unwrap())
        {
            eprintln!("warning: could not write report: {e}");
        }
    }
    println!("{}", serde_json::to_string_pretty(&summary.report).unwrap());
    match summary.outcome {
        Outcome::Heartbeat | Outcome::ScriptComplete | Outcome::ProgramComplete => 0,
        Outcome::Fault => 1,
        Outcome::UnsupportedStop => 2,
        Outcome::LimitReached => 3,
        Outcome::ScriptFailed => 5,
    }
}
