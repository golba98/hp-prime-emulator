//! `ppl run` and `ppl check`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum};
use ppl_graphics::text::render_terminal;
use ppl_vm::{AngleMode, DisplayMode, HeadlessHost, Limits, Machine, Settings, parse_key_script};

use super::common::{self, exit};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PrintMode {
    /// PRINT goes to the emulated screen only.
    Screen,
    /// PRINT goes to stdout only.
    Terminal,
    /// Both (default).
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Angle {
    Rad,
    Deg,
    Grad,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum StringVersion {
    #[value(name = "15157")]
    Legacy15157,
    #[default]
    #[value(name = "15751")]
    Current15751,
}
impl StringVersion {
    fn setting(self) -> ppl_vm::StringFormatVersion {
        match self {
            Self::Legacy15157 => ppl_vm::StringFormatVersion::Legacy15157,
            Self::Current15751 => ppl_vm::StringFormatVersion::Current15751,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum NumberVersion {
    #[default]
    #[value(name = "15157")]
    Legacy15157,
    #[value(name = "15751")]
    Current15751,
}
impl NumberVersion {
    fn setting(self) -> ppl_math::NumericVersion {
        match self {
            Self::Legacy15157 => ppl_math::NumericVersion::Legacy15157,
            Self::Current15751 => ppl_math::NumericVersion::Current15751,
        }
    }
}

/// Options shared by `run`, `gui`, `debug` and `compare`.
#[derive(Args, Debug, Clone)]
pub struct RunOpts {
    /// PPL source file (.hpprgm, .ppl, .txt); stdin when omitted.
    pub file: Option<PathBuf>,
    /// Inline PPL source instead of a file.
    #[arg(short = 'c', long)]
    pub code: Option<String>,
    /// Function to run (default: the EXPORT function).
    #[arg(short, long)]
    pub entry: Option<String>,
    /// Function arguments: legacy PPL by default; JSON with explicit --engine.
    #[arg(short, long)]
    pub args: Option<String>,
    /// Value typed into the next INPUT field (repeatable); `<ESC>` cancels
    /// the dialog.
    #[arg(long = "input", value_name = "TEXT")]
    pub inputs: Vec<String>,
    /// Choice for the next CHOOSE (1-based; 0 = cancel; repeatable).
    #[arg(long = "choose", value_name = "N")]
    pub choices: Vec<usize>,
    /// Scripted keys for GETKEY/WAIT/ISKEYDOWN: "-1*30,4" or "touch:10:20".
    #[arg(long, value_name = "SCRIPT", allow_hyphen_values = true)]
    pub keys: Option<String>,
    /// Where PRINT output appears.
    #[arg(long, value_enum, default_value_t = PrintMode::Both)]
    pub print_mode: PrintMode,
    /// Save the final screen as PNG.
    #[arg(short = 'o', long = "png", value_name = "PATH")]
    pub png: Option<PathBuf>,
    /// Execution limit in seconds (default 30; excludes firmware boot).
    #[arg(long, value_name = "SECS")]
    pub max_time: Option<f64>,
    /// Disable the wall-clock limit.
    #[arg(long)]
    pub no_time_limit: bool,
    /// Native VM instruction limit.
    #[arg(long, value_name = "N")]
    pub max_steps: Option<u64>,
    /// Native VM call-depth limit (default 1000).
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,
    /// Native engine: initial angle mode (HAngle).
    #[arg(long, value_enum, default_value_t = Angle::Rad)]
    pub angle: Angle,
    /// Native STRING fraction-format version (does not change BCD arithmetic).
    #[arg(long, value_enum, default_value_t = StringVersion::Current15751)]
    pub string_format_version: StringVersion,
    /// Native numeric model: legacy12-digit BCD or build15751 exact reals.
    #[arg(long, value_enum, default_value_t = NumberVersion::Legacy15157)]
    pub numeric_version: NumberVersion,
    /// Native engine: seed for RANDOM/RANDINT.
    #[arg(long, default_value_t = 0x5EED)]
    pub seed: u64,
    /// Show compiler warnings.
    #[arg(short = 'W', long)]
    pub warnings: bool,
    /// Do not print the status line on stderr.
    #[arg(short, long)]
    pub quiet: bool,
}

impl RunOpts {
    pub fn settings(&self) -> Settings {
        Settings {
            numeric_version: self.numeric_version.setting(),
            string_format_version: self.string_format_version.setting(),
            angle: match self.angle {
                Angle::Rad => AngleMode::Radians,
                Angle::Deg => AngleMode::Degrees,
                Angle::Grad => AngleMode::Gradians,
            },
            ..Settings::default()
        }
    }

    pub fn limits(&self) -> Limits {
        let mut l = Limits::default();
        if self.no_time_limit {
            l.max_time = None;
        } else if let Some(t) = self.max_time {
            l.max_time = Some(Duration::from_secs_f64(t.max(0.0)));
        }
        l.max_steps = self.max_steps;
        if let Some(d) = self.max_depth {
            l.max_depth = d;
        }
        l
    }

    pub fn headless_host(&self) -> Result<HeadlessHost, String> {
        let mut h = HeadlessHost::new();
        h.echo = self.print_mode != PrintMode::Screen;
        if let Some(k) = &self.keys {
            h.keys = parse_key_script(k)?.into();
        }
        h.inputs = self
            .inputs
            .iter()
            .map(|t| ppl_vm::ScriptedInput::parse(t))
            .collect();
        h.choices = self.choices.iter().copied().collect();
        Ok(h)
    }
}

#[derive(Args, Debug)]
pub struct CheckOpts {
    /// PPL source file; stdin when omitted.
    pub file: Option<PathBuf>,
    #[arg(short = 'c', long)]
    pub code: Option<String>,
    /// Treat warnings as errors.
    #[arg(long)]
    pub deny_warnings: bool,
    /// Hide warnings.
    #[arg(long)]
    pub no_warnings: bool,
    /// Print the compiled bytecode.
    #[arg(long)]
    pub dump_bytecode: bool,
}

pub fn check(o: CheckOpts) -> u8 {
    let file = match common::load_source(o.file.as_deref(), o.code.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let (prog, diags) = common::compile(&file, !o.no_warnings);
    let Some(prog) = prog else {
        return exit::COMPILE;
    };
    if o.dump_bytecode {
        println!("{}", prog.disassemble());
    }
    let warnings = diags.warnings().count();
    if o.deny_warnings && warnings > 0 {
        return exit::COMPILE;
    }
    eprintln!(
        "✓ {} is valid PPL ({} function(s), {} warning(s))",
        file.name,
        prog.functions.len(),
        warnings
    );
    exit::OK
}

pub fn run(o: RunOpts) -> u8 {
    let file = match common::load_source(o.file.as_deref(), o.code.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let (prog, _) = common::compile(&file, o.warnings);
    let Some(prog) = prog else {
        return exit::COMPILE;
    };
    let stem = o
        .file
        .as_deref()
        .and_then(|p| p.file_stem().and_then(|s| s.to_str()));
    let entry = match common::pick_entry(&prog, o.entry.as_deref(), stem) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let settings = o.settings();
    let args = match common::parse_args(o.args.as_deref(), &settings) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let host = match o.headless_host() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let fonts = match ppl_engine::fonts::configured() {
        Ok(fonts) => fonts,
        Err(error) => {
            eprintln!("error: {error}");
            return exit::IO;
        }
    };
    let mut m = Machine::new(prog, Box::new(host), settings, o.limits());
    m.set_seed(o.seed);
    let t0 = Instant::now();
    let draw = || {
        let result = m.call_function(&entry, args);
        let image = o.png.as_ref().map(|_| {
            if m.display_mode() == DisplayMode::Terminal {
                render_terminal(&m.terminal)
            } else {
                m.screen().clone()
            }
        });
        (result, image)
    };
    let rendered = match fonts {
        Some(fonts) => ppl_graphics::text::with_prime_fonts(fonts, draw),
        None => Ok(draw()),
    };
    let (result, image) = match rendered {
        Ok(result) => result,
        Err(error) => {
            eprintln!("error: {error}");
            return exit::IO;
        }
    };
    let elapsed = t0.elapsed();
    if let (Some(path), Some(img)) = (&o.png, image) {
        if let Err(e) = img.save_png(path) {
            eprintln!("error: cannot write {}: {e}", path.display());
            return exit::IO;
        }
        if !o.quiet {
            eprintln!("→ screen saved to {}", path.display());
        }
    }
    match result {
        Ok(v) => {
            if !o.quiet {
                eprintln!(
                    "✓ {entry} returned {} ({} steps, {:.1} ms)",
                    v.display(m.settings.format()),
                    m.steps(),
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            exit::OK
        }
        Err(e) => {
            common::report_runtime_error(&file, &e);
            common::exit_code_for(&e)
        }
    }
}
