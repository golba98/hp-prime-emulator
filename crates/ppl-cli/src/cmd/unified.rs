//! CLI presentation and engine dispatch. GUI/debug retain their existing paths.
use super::run::{self, Angle, CheckOpts, PrintMode, RunOpts};
use clap::{Args, ValueEnum};
use ppl_engine::{
    Cancellation, Engine, Operation, Report, Status, firmware_options::AssetPaths, source::Source,
};
use std::io::IsTerminal;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EngineChoice {
    Native,
    Firmware,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ArgsFormat {
    Json,
    Ppl,
}
#[derive(Debug, Args, Default)]
pub struct EngineOptions {
    /// Compiler/runtime engine. When omitted: the authentic firmware if its
    /// assets are configured (flags, PPL_* variables, or
    /// ~/.config/hp-prime-emulator/firmware.toml) and no native-only option
    /// is used; otherwise native. `PPL_ENGINE=native|firmware` overrides.
    #[arg(long, value_enum)]
    pub engine: Option<EngineChoice>,
    /// Emit one versioned JSON result on stdout.
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// Run only: override the legacy/explicit-engine argument convention.
    #[arg(long, value_enum)]
    pub args_format: Option<ArgsFormat>,
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
    #[arg(long)]
    pub program_name: Option<String>,
    /// Retain a unique firmware work directory (may contain proprietary assets).
    #[arg(long)]
    pub artifacts_dir: Option<PathBuf>,
    #[arg(long)]
    pub runtime_keys: Option<String>,
    /// Outer firmware request deadline, including startup (default 360 s).
    #[arg(long)]
    pub timeout_s: Option<f64>,
    #[arg(long)]
    pub boot_timeout_s: Option<f64>,
    /// Firmware Check/key settlement budget (default 30 s).
    #[arg(long)]
    pub compile_timeout_s: Option<f64>,
    #[arg(long)]
    pub firmware_max_instructions: Option<u64>,
}
impl EngineOptions {
    fn native(&self) -> bool {
        self.engine != Some(EngineChoice::Firmware)
    }
    fn legacy(&self) -> bool {
        self.engine.is_none()
            && !self.json
            && self.report.is_none()
            && self.args_format.is_none()
            && !self.firmware_flags()
    }
    fn firmware_flags(&self) -> bool {
        self.firmware.is_some()
            || self.os_rom.is_some()
            || self.factory_a.is_some()
            || self.init_data.is_some()
            || self.firmware_runner.is_some()
            || self.program_name.is_some()
            || self.artifacts_dir.is_some()
            || self.runtime_keys.is_some()
            || self.timeout_s.is_some()
            || self.boot_timeout_s.is_some()
            || self.compile_timeout_s.is_some()
            || self.firmware_max_instructions.is_some()
    }
    fn json_arguments(&self) -> bool {
        self.args_format.unwrap_or(if self.engine.is_some() {
            ArgsFormat::Json
        } else {
            ArgsFormat::Ppl
        }) == ArgsFormat::Json
    }
}
#[derive(Debug, Args)]
pub struct RunCommand {
    #[command(flatten)]
    pub native: RunOpts,
    #[command(flatten)]
    pub options: EngineOptions,
}
#[derive(Debug, Args)]
pub struct CheckCommand {
    /// Report independent native and configured local-firmware checks.
    #[arg(long, conflicts_with = "engine")]
    pub all_backends: bool,
    #[command(flatten)]
    pub native: CheckOpts,
    #[command(flatten)]
    pub options: EngineOptions,
}
fn load(file: Option<&Path>, code: Option<&str>) -> Result<Source, String> {
    if let Some(c) = code {
        let mut source = Source::text(c, file.and_then(|p| p.to_str()).unwrap_or("<inline>"))?;
        source.stem = file
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        return Ok(source);
    }
    if let Some(f) = file {
        return Source::file(f);
    }
    use std::io::Read;
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Source::text(&text, "<stdin>")
}
fn assets(o: &EngineOptions) -> Result<AssetPaths, String> {
    AssetPaths {
        runner: o.firmware_runner.clone(),
        firmware: o.firmware.clone(),
        os_rom: o.os_rom.clone(),
        factory_a: o.factory_a.clone(),
        init_data: o.init_data.clone(),
    }
    .resolve()
}
/// Engine for a request that named none. Returns `None` to keep the
/// historical native behaviour (and its output) unchanged.
fn default_engine(native_only: bool) -> Option<EngineChoice> {
    match std::env::var("PPL_ENGINE").ok().as_deref() {
        Some("native") => return None,
        Some("firmware") => return Some(EngineChoice::Firmware),
        _ => {}
    }
    if native_only {
        return None;
    }
    match AssetPaths::default().resolve() {
        Ok(paths) if paths.configured() => Some(EngineChoice::Firmware),
        Ok(_) => {
            if std::io::stderr().is_terminal() {
                eprintln!("Engine: native (firmware not configured)");
            }
            None
        }
        Err(error) => {
            eprintln!("warning: ignoring firmware configuration: {error}");
            None
        }
    }
}
fn duration(value: Option<f64>, default: f64) -> Result<Duration, String> {
    Duration::try_from_secs_f64(value.unwrap_or(default))
        .ok()
        .filter(|x| !x.is_zero())
        .ok_or_else(|| "timeouts must be positive and finite".into())
}
fn firmware_options(
    o: &EngineOptions,
    entry: Option<String>,
    args: String,
    png: Option<PathBuf>,
    max_time: Option<f64>,
) -> Result<ppl_engine::firmware::Options, String> {
    let paths = assets(o)?;
    Ok(ppl_engine::firmware::Options {
        runner: paths.runner,
        firmware: paths.firmware,
        os_rom: paths.os_rom,
        factory_a: paths.factory_a,
        init_data: paths.init_data,
        artifacts_dir: o.artifacts_dir.clone(),
        program_name: o.program_name.clone(),
        entry,
        args,
        runtime_keys: o.runtime_keys.clone(),
        screenshot: png,
        timeout: duration(o.timeout_s, 360.)?,
        boot_timeout: duration(o.boot_timeout_s, 60.)?,
        compile_timeout: duration(o.compile_timeout_s, 30.)?,
        execution_timeout: duration(max_time, 30.)?,
        max_instructions: o.firmware_max_instructions.unwrap_or(8_000_000_000),
    })
}
fn cancelled() -> Result<Cancellation, String> {
    let c = Cancellation::default();
    let signal = c.clone();
    ctrlc::set_handler(move || signal.cancel()).map_err(|e| e.to_string())?;
    Ok(c)
}
fn failure(o: &EngineOptions, op: Operation, status: Status, message: impl Into<String>) -> Report {
    let mut r = Report::new(
        if o.native() {
            Engine::Native
        } else {
            Engine::Firmware
        },
        op,
    );
    r.fail(status, "request", message);
    r
}
pub fn run(c: RunCommand) -> u8 {
    let frontend_started = Instant::now();
    let RunCommand {
        native: o,
        options: mut e,
    } = c;
    if e.engine.is_none() {
        let explicit_native_setting = std::env::args().any(|a| {
            a == "--angle" || a.starts_with("--angle=") || a == "--seed" || a.starts_with("--seed=")
        });
        // Native-only controls and legacy PPL-syntax arguments keep native.
        e.engine = default_engine(
            o.no_time_limit
                || o.max_steps.is_some()
                || o.max_depth.is_some()
                || o.keys.is_some()
                || !o.inputs.is_empty()
                || !o.choices.is_empty()
                || o.angle != Angle::Rad
                || o.seed != 0x5EED
                || o.warnings
                || explicit_native_setting
                || o.args.is_some() && e.args_format != Some(ArgsFormat::Json),
        );
    }
    if e.legacy() && !o.no_time_limit && o.max_time.is_some_and(|t| !t.is_finite()) {
        eprintln!("error: --max-time must be finite");
        return 2;
    }
    if e.legacy() {
        if let (Some(input), Some(output)) = (&o.file, &o.png) {
            if same_path(input, output) {
                eprintln!("error: screenshot path aliases input source");
                return 5;
            }
        }
        return run::run(o);
    }
    let mut result = execute_run(&o, &e)
        .unwrap_or_else(|(status, message)| failure(&e, Operation::Run, status, message));
    result.timings["cli_total_before_output_s"] =
        serde_json::json!(frontend_started.elapsed().as_secs_f64());
    finish(
        result,
        &e,
        o.quiet,
        !e.native() && o.print_mode != PrintMode::Screen,
        o.warnings,
        o.file.as_deref(),
    )
}
fn execute_run(o: &RunOpts, e: &EngineOptions) -> Result<Report, (Status, String)> {
    if e.native() && e.firmware_flags() {
        return Err((
            Status::InvalidRequest,
            "firmware options require --engine firmware".into(),
        ));
    }
    if !e.native() {
        let explicit_native_setting = std::env::args().any(|a| {
            a == "--angle" || a.starts_with("--angle=") || a == "--seed" || a.starts_with("--seed=")
        });
        if o.no_time_limit
            || o.max_steps.is_some()
            || o.max_depth.is_some()
            || o.keys.is_some()
            || !o.inputs.is_empty()
            || !o.choices.is_empty()
            || o.angle != Angle::Rad
            || o.seed != 0x5EED
            || o.warnings
            || explicit_native_setting
        {
            return Err((Status::Unsupported,"native VM controls cannot be applied to firmware; use --runtime-keys and firmware budgets".into()));
        }
        if e.args_format == Some(ArgsFormat::Ppl) {
            return Err((
                Status::Unsupported,
                "firmware arguments require JSON; native PPL argument evaluation is never used"
                    .into(),
            ));
        }
    }
    if !o.no_time_limit && o.max_time.is_some_and(|t| !t.is_finite()) {
        return Err((Status::InvalidArguments, "invalid --max-time".into()));
    }
    let source = load(o.file.as_deref(), o.code.as_deref()).map_err(|e| (Status::IoError, e))?;
    for output in [&e.report, &o.png].into_iter().flatten() {
        if source
            .identity
            .path
            .as_ref()
            .is_some_and(|input| same_path(input, output))
        {
            return Err((
                Status::IoError,
                "output path aliases the input source".into(),
            ));
        }
    }
    let cancel = cancelled().map_err(|e| (Status::InfrastructureError, e))?;
    if e.native() {
        let mut host = o
            .headless_host()
            .map_err(|e| (Status::InvalidArguments, e))?;
        host.echo = !e.json && o.print_mode != PrintMode::Screen;
        let options = ppl_engine::native::Options {
            entry: o.entry.clone(),
            args: o.args.clone().unwrap_or_else(|| {
                if e.json_arguments() {
                    "[]".into()
                } else {
                    String::new()
                }
            }),
            json_arguments: e.json_arguments(),
            host,
            settings: o.settings(),
            limits: o.limits(),
            seed: o.seed,
            png: o.png.clone(),
            deny_warnings: false,
            dump_bytecode: false,
        };
        Ok(ppl_engine::native::execute(
            source,
            Operation::Run,
            options,
            &cancel,
        ))
    } else {
        let options = firmware_options(
            e,
            o.entry.clone(),
            o.args.clone().unwrap_or("[]".into()),
            o.png.clone(),
            o.max_time,
        )
        .map_err(|e| (Status::InvalidArguments, e))?;
        Ok(ppl_engine::firmware::execute(
            source,
            Operation::Run,
            &options,
            &cancel,
        ))
    }
}

pub fn check(c: CheckCommand) -> u8 {
    let frontend_started = Instant::now();
    let CheckCommand {
        native: o,
        options: mut e,
        all_backends,
    } = c;
    if e.engine.is_none() && !all_backends {
        e.engine = default_engine(o.dump_bytecode || o.deny_warnings);
    }
    if all_backends {
        let result = (|| -> Result<ppl_engine::checking::CheckReport, String> {
            if let (Some(input), Some(output)) = (o.file.as_ref(), e.report.as_ref()) {
                if same_path(input, output) {
                    return Err("report path aliases source".into());
                }
            }
            if o.dump_bytecode
                || o.deny_warnings
                || e.runtime_keys.is_some()
                || e.args_format.is_some()
            {
                return Err("combined checking does not support bytecode, warning-policy, or runtime options".into());
            }
            let source = load(o.file.as_deref(), o.code.as_deref())?;
            let configured = if e.firmware_flags() {
                Some(firmware_options(&e, None, "[]".into(), None, None)?)
            } else {
                ppl_engine::checking::configured_firmware()
            };
            Ok(ppl_engine::checking::check(
                source,
                configured.as_ref(),
                &Cancellation::default(),
            ))
        })();
        return match result {
            Ok(report) => {
                let code = if report.accepted() { 0 } else { 1 };
                let json = serde_json::to_string_pretty(&report).unwrap();
                if let Some(path) = &e.report {
                    if let Err(error) = std::fs::write(path, &json) {
                        eprintln!("error: {error}");
                        return 5;
                    }
                }
                if e.json {
                    println!("{json}");
                } else {
                    println!(
                        "Native: {:?}; local firmware: {}; physical Check: not performed",
                        report.native.status, report.firmware_status
                    );
                    for result in std::iter::once(&report.native).chain(report.firmware.iter()) {
                        for d in &result.diagnostics {
                            eprintln!(
                                "{}:{}:{}: {}",
                                d.source,
                                d.line.map_or("?".into(), |v| v.to_string()),
                                d.column.map_or("?".into(), |v| v.to_string()),
                                d.message.as_deref().unwrap_or("diagnostic without message")
                            );
                        }
                        if let Some(e) = &result.error {
                            eprintln!("{:?}: {}", result.engine, e.message);
                        }
                    }
                }
                code
            }
            Err(error) => {
                eprintln!("error: {error}");
                2
            }
        };
    }
    if e.legacy() {
        return run::check(o);
    }
    let mut result = (|| -> Result<Report, (Status, String)> {
        if e.native() && e.firmware_flags() {
            return Err((
                Status::InvalidRequest,
                "firmware options require --engine firmware".into(),
            ));
        }
        if e.args_format.is_some() || e.runtime_keys.is_some() {
            return Err((
                Status::InvalidRequest,
                "argument format and runtime input apply only to run".into(),
            ));
        }
        if !e.native() && (o.dump_bytecode || o.deny_warnings) {
            return Err((
                Status::Unsupported,
                "native bytecode and warning policy are unavailable for firmware checking".into(),
            ));
        }
        let source =
            load(o.file.as_deref(), o.code.as_deref()).map_err(|e| (Status::IoError, e))?;
        if let (Some(input), Some(out)) = (&source.identity.path, &e.report) {
            if same_path(input, out) {
                return Err((Status::IoError, "report path aliases source".into()));
            }
        }
        let cancel = cancelled().map_err(|e| (Status::InfrastructureError, e))?;
        if e.native() {
            Ok(ppl_engine::native::execute(
                source,
                Operation::Check,
                ppl_engine::native::Options {
                    entry: None,
                    args: "[]".into(),
                    json_arguments: true,
                    host: ppl_vm::HeadlessHost::new(),
                    settings: Default::default(),
                    limits: Default::default(),
                    seed: 0x5EED,
                    png: None,
                    deny_warnings: o.deny_warnings,
                    dump_bytecode: o.dump_bytecode,
                },
                &cancel,
            ))
        } else {
            let opts = firmware_options(&e, None, "[]".into(), None, None)
                .map_err(|e| (Status::InvalidArguments, e))?;
            Ok(ppl_engine::firmware::execute(
                source,
                Operation::Check,
                &opts,
                &cancel,
            ))
        }
    })()
    .unwrap_or_else(|(s, m)| failure(&e, Operation::Check, s, m));
    result.timings["cli_total_before_output_s"] =
        serde_json::json!(frontend_started.elapsed().as_secs_f64());
    finish(result, &e, false, false, !o.no_warnings, o.file.as_deref())
}
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(unix)]
    if let (Ok(a), Ok(b)) = (a.metadata(), b.metadata()) {
        use std::os::unix::fs::MetadataExt;
        if a.dev() == b.dev() && a.ino() == b.ino() {
            return true;
        }
    }
    let key = |p: &Path| {
        p.canonicalize().ok().or_else(|| {
            p.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .canonicalize()
                .ok()
                .zip(p.file_name())
                .map(|(d, n)| d.join(n))
        })
    };
    key(a).zip(key(b)).is_some_and(|(a, b)| a == b)
}

fn finish(
    mut r: Report,
    o: &EngineOptions,
    quiet: bool,
    echo: bool,
    warnings: bool,
    input: Option<&Path>,
) -> u8 {
    r.requested_engine = o.engine.map(|e| {
        if e == EngineChoice::Native {
            "native"
        } else {
            "firmware"
        }
        .into()
    });
    let serialize_started = Instant::now();
    let probe = serde_json::to_vec(&r).expect("serializable engine report");
    r.timings["serialization_probe_s"] =
        serde_json::json!(serialize_started.elapsed().as_secs_f64());
    r.timings["json_bytes_before_timing"] = serde_json::json!(probe.len());
    if let Some(path) = &o.report {
        if input.is_some_and(|input| same_path(input, path)) {
            r.fail(
                Status::IoError,
                "report",
                "report path aliases input source",
            );
            let safe = EngineOptions {
                report: None,
                json: o.json,
                engine: o.engine,
                ..Default::default()
            };
            return finish(r, &safe, quiet, echo, warnings, None);
        }
        let write = (|| -> Result<(), String> {
            if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&r).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())
        })();
        if let Err(e) = write {
            r.fail(Status::IoError, "report", e)
        }
    }
    if o.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&r).expect("serializable engine result")
        );
    } else {
        if !quiet {
            eprintln!(
                "Engine: {}\nProgram: {}\nStatus: {:?}",
                r.engine
                    .map(|e| format!("{e:?}"))
                    .unwrap_or_else(|| "Unavailable".into()),
                r.program,
                r.status
            );
            if let Some(entry) = &r.entry {
                eprintln!("Entry: {entry}");
            }
        }
        if echo {
            for event in &r.output {
                if let Some(text) = &event.text {
                    println!("{text}")
                }
            }
        }
        for d in r
            .diagnostics
            .iter()
            .chain(&r.runtime_errors)
            .filter(|d| warnings || d.severity == "error")
        {
            eprintln!(
                "{}{}",
                d.message.as_deref().unwrap_or("diagnostic unavailable"),
                match (d.line, d.column) {
                    (Some(l), Some(c)) => format!(" (line {l}, column {c})"),
                    (Some(l), _) => format!(" (line {l})"),
                    _ => String::new(),
                }
            );
        }
        if let Some(error) = &r.error {
            eprintln!("error: {}", error.message)
        }
        if !quiet {
            if let Some(v) = &r.return_value {
                let display = r.backend["display_return"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        v["value"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string())
                    });
                eprintln!("Return: {display}");
            } else if r.return_representation["kind"] == "undecoded" {
                eprintln!(
                    "Return: not decoded by host (firmware tag {}/{})",
                    r.return_representation["tag"], r.return_representation["subtype"]
                );
            }
            for caveat in &r.caveats {
                eprintln!("Caveat: {caveat}");
            }
            eprintln!("Time: {:.3} s", r.timings["total_s"].as_f64().unwrap_or(0.));
        }
        if let Some(s) = r.backend["disassembly"].as_str() {
            print!("{s}");
        }
    }
    r.exit_code
}
