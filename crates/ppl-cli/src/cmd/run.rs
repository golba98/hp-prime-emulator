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
    /// Arguments for the function, e.g. --args "50, {1,2}".
    #[arg(short, long)]
    pub args: Option<String>,
    /// Value typed into the next INPUT field (repeatable).
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
    /// Wall-clock limit in seconds (host protection, default 30).
    #[arg(long, value_name = "SECS")]
    pub max_time: Option<f64>,
    /// Disable the wall-clock limit.
    #[arg(long)]
    pub no_time_limit: bool,
    /// Instruction limit (host protection).
    #[arg(long, value_name = "N")]
    pub max_steps: Option<u64>,
    /// Call-depth limit (host protection, default 1000).
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,
    /// Angle mode (HAngle) at start.
    #[arg(long, value_enum, default_value_t = Angle::Rad)]
    pub angle: Angle,
    /// Random seed for RANDOM/RANDINT.
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
        h.inputs = self.inputs.iter().cloned().collect();
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
    let entry = match common::pick_entry(&prog, o.entry.as_deref()) {
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
    let mut m = Machine::new(prog, Box::new(host), settings, o.limits());
    m.set_seed(o.seed);
    let t0 = Instant::now();
    let result = m.call_function(&entry, args);
    let elapsed = t0.elapsed();
    if let Some(path) = &o.png {
        let img = if m.display_mode() == DisplayMode::Terminal {
            render_terminal(&m.terminal)
        } else {
            m.screen().clone()
        };
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
