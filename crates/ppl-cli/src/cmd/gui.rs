//! `ppl gui`: run a program with the native winit / softbuffer GUI.

use std::path::PathBuf;
use std::time::Duration;

use clap::Args;
use ppl_gui::{GuiAppConfig, KeyScript, WorkerConfig, WorkerOutcome, run_gui_session};
use ppl_vm::{AngleMode, ErrorKind, Limits, Settings};

use super::common::{self, exit};
use super::run::Angle;

#[derive(Args, Debug, Clone)]
pub struct GuiOpts {
    /// PPL source file (.hpprgm, .ppl, .txt).
    pub file: Option<PathBuf>,

    /// Inline PPL source code.
    #[arg(short = 'c', long)]
    pub code: Option<String>,

    /// Function to run (default: the EXPORT function).
    #[arg(short, long)]
    pub entry: Option<String>,

    /// Arguments for the function, e.g. --args "50, {1,2}".
    #[arg(short, long)]
    pub args: Option<String>,

    /// Window width (default 640).
    #[arg(long, default_value_t = 640)]
    pub width: u32,

    /// Window height (default 480).
    #[arg(long, default_value_t = 480)]
    pub height: u32,

    /// Window title.
    #[arg(long, default_value = "HP Prime PPL Emulator")]
    pub title: String,

    /// Wall-clock limit in seconds (default: none in the GUI; time spent
    /// waiting for keys and dialogs is not counted).
    #[arg(long, value_name = "SECS")]
    pub max_time: Option<f64>,

    /// Disable the wall-clock limit (the GUI default; accepted for parity
    /// with `ppl run`, overrides --max-time).
    #[arg(long)]
    pub no_time_limit: bool,

    /// Instruction limit (host protection).
    #[arg(long, value_name = "N")]
    pub max_steps: Option<u64>,

    /// Angle mode (HAngle) at start.
    #[arg(long, value_enum, default_value_t = Angle::Rad)]
    pub angle: Angle,

    /// Random seed for RANDOM/RANDINT.
    #[arg(long, default_value_t = 0x5EED)]
    pub seed: u64,

    /// Scripted keys for GETKEY/WAIT/ISKEYDOWN/mouse.
    #[arg(long, value_name = "SCRIPT", allow_hyphen_values = true)]
    pub keys: Option<String>,

    /// Close the window as soon as the program finishes.
    #[arg(long)]
    pub exit_on_finish: bool,

    /// Close the window after SECS seconds, cancelling the program if it is
    /// still running (for scripted smoke tests).
    #[arg(long, value_name = "SECS")]
    pub exit_after: Option<f64>,

    /// Do not print the status line on stderr.
    #[arg(short, long)]
    pub quiet: bool,
}

impl GuiOpts {
    fn limits(&self) -> Limits {
        Limits {
            max_time: match self.max_time {
                Some(t) if !self.no_time_limit => Some(Duration::from_secs_f64(t.max(0.0))),
                _ => None, // Interactive GUI has no time limit by default.
            },
            max_steps: self.max_steps,
            ..Limits::default()
        }
    }

    fn settings(&self) -> Settings {
        Settings {
            angle: match self.angle {
                Angle::Rad => AngleMode::Radians,
                Angle::Deg => AngleMode::Degrees,
                Angle::Grad => AngleMode::Gradians,
            },
            ..Settings::default()
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct ShowOpts {
    /// `.hpprgm` or PPL source to open.
    pub file: PathBuf,
    /// Skip the start-up firmware Check.
    #[arg(long)]
    pub no_check: bool,
}

pub fn show(o: ShowOpts) -> u8 {
    if !o.file.is_file() {
        eprintln!("error: {} is not a file", o.file.display());
        return exit::IO;
    }
    if let Err(e) = ppl_gui::launch_ide_with(ppl_gui::LaunchOptions {
        file: Some(o.file),
        firmware: true,
        check: !o.no_check,
    }) {
        eprintln!("GUI error: {e}");
        return exit::IO;
    }
    exit::OK
}

pub fn run(o: GuiOpts) -> u8 {
    // If no headless/scripted batch flags are supplied, launch the full interactive desktop IDE.
    if o.code.is_none()
        && o.keys.is_none()
        && !o.exit_on_finish
        && o.exit_after.is_none()
        && o.args.is_none()
        && o.entry.is_none()
    {
        if let Err(e) = ppl_gui::launch_ide(o.file) {
            eprintln!("GUI error: {e}");
            return exit::IO;
        }
        return exit::OK;
    }

    let file = match common::load_source(o.file.as_deref(), o.code.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };

    let (prog, _) = common::compile(&file, false);
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
    // Validate --args up front so a typo is a usage error, not a GUI error.
    if let Err(e) = common::parse_args(o.args.as_deref(), &settings) {
        eprintln!("error: {e}");
        return exit::USAGE;
    }

    let exit_after = match o.exit_after {
        Some(s) if !(s.is_finite() && s >= 0.0) => {
            eprintln!("error: --exit-after must be a non-negative number of seconds");
            return exit::USAGE;
        }
        s => s.map(Duration::from_secs_f64),
    };

    let key_script = match o.keys.as_deref() {
        Some(s) => match KeyScript::parse(s) {
            Ok(ks) => Some(ks),
            Err(e) => {
                eprintln!("error: invalid --keys: {e}");
                return exit::USAGE;
            }
        },
        None => None,
    };

    let worker_config = WorkerConfig {
        prog: (*prog).clone(),
        entry: Some(entry.clone()),
        args: o.args.clone(),
        settings,
        limits: o.limits(),
        seed: o.seed,
        echo: !o.quiet,
        debug: None,
    };

    let app_config = GuiAppConfig {
        title: o.title.clone(),
        width: o.width,
        height: o.height,
        integer_scaling: true,
        bg_color: 0x0010_1014,
        exit_on_finish: o.exit_on_finish,
        exit_after,
        key_script,
    };

    let session = match run_gui_session(app_config, Some(worker_config)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("GUI error: {e}");
            return exit::IO;
        }
    };

    match session.outcome {
        Some(WorkerOutcome::Success {
            value_str,
            steps,
            elapsed,
        }) => {
            if !o.quiet {
                eprintln!(
                    "✓ {entry} returned {value_str} ({steps} steps, {:.1} ms)",
                    elapsed.as_secs_f64() * 1000.0
                );
            }
            exit::OK
        }
        // Closing the window, the break key or --exit-after: a user cancel,
        // not a program error.
        Some(WorkerOutcome::Error(e)) if e.kind == ErrorKind::Interrupted => {
            if !o.quiet {
                eprintln!("cancelled");
            }
            exit::LIMIT
        }
        Some(WorkerOutcome::Error(e)) => {
            common::report_runtime_error(&file, &e);
            common::exit_code_for(&e)
        }
        None if session.cancelled => {
            if !o.quiet {
                eprintln!("cancelled (the program did not stop in time)");
            }
            exit::LIMIT
        }
        None => exit::OK,
    }
}
