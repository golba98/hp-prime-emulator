//! `ppl debug`: interactive step debugger and inspector.
//!
//! Commands are read line by line from stdin, so a session can be scripted:
//! `printf 'b 5\nc\nl\nc\n' | ppl debug prog.hpprgm`. End of input quits.
//!
//! Output lines (stable, machine-checkable):
//! - `[STOP] FN() at FILE:LINE:COL (frame depth D)` when execution stops,
//!   preceded by `Breakpoint hit at line LINE` for breakpoint stops;
//! - `  NAME = VALUE` for `locals`, `globals` and `p`;
//! - `  #I FN() at line LINE:COL` for `bt`; `Frame #I FN() at line LINE:COL`
//!   after `up`/`down`/`frame`;
//! - `Program exited normally with value: VALUE` on normal termination,
//!   `Execution terminated: ERROR` (stderr, followed by `  --> FILE:LINE:COL
//!   in FN   SOURCE` trace lines) on a runtime error, and
//!   `Debugger quit; program aborted.` after `q` or end of input.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use clap::Args;
use ppl_syntax::{SourceFile, Span};
use ppl_vm::{
    AngleMode, DebugAction, Debugger, HeadlessHost, Limits, Machine, Settings, Value,
    parse_key_script,
};

use super::common::{self, exit};
use super::run::Angle;

#[derive(Args, Debug, Clone)]
pub struct DebugOpts {
    /// PPL source file (.hpprgm, .ppl, .txt).
    pub file: Option<PathBuf>,

    /// Inline PPL source code.
    #[arg(short = 'c', long)]
    pub code: Option<String>,

    /// Function to run (default: the EXPORT function).
    #[arg(short, long)]
    pub entry: Option<String>,

    /// Set a breakpoint at LINE before starting (repeatable).
    #[arg(short = 'b', long = "break", value_name = "LINE")]
    pub breakpoints: Vec<usize>,

    /// Run to the first breakpoint instead of stopping at the first statement.
    #[arg(long)]
    pub run: bool,

    /// Arguments for the function, e.g. --args "50, {1,2}".
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

    /// Wall-clock limit in seconds (default: none, since time spent at the
    /// debugger prompt would otherwise count against the program).
    #[arg(long, value_name = "SECS")]
    pub max_time: Option<f64>,

    /// Disable the wall-clock limit (the default; overrides --max-time).
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
}

impl DebugOpts {
    fn limits(&self) -> Limits {
        let mut l = Limits {
            max_time: match self.max_time {
                Some(t) if !self.no_time_limit => Some(Duration::from_secs_f64(t.max(0.0))),
                _ => None,
            },
            max_steps: self.max_steps,
            ..Limits::default()
        };
        if let Some(d) = self.max_depth {
            l.max_depth = d;
        }
        l
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

    fn host(&self) -> Result<HeadlessHost, String> {
        let mut h = HeadlessHost::new();
        // PRINT goes to stdout exactly as with `ppl run`.
        h.echo = true;
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

/// When to stop next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Stop at the next statement anywhere.
    Step,
    /// Stop at the next statement at this frame depth or shallower.
    Next(usize),
    /// Stop at the next statement shallower than this depth.
    Finish(usize),
    /// Only stop at breakpoints.
    Continue,
}

struct CliDebugger {
    source: SourceFile,
    breakpoints: BTreeSet<usize>,
    mode: Mode,
    /// `(line, depth)` of the previous statement executed (stopped or not):
    /// a breakpoint fires when execution *enters* its line.
    prev_stmt: Option<(usize, usize)>,
    input: Box<dyn BufRead>,
    quit: Rc<Cell<bool>>,
}

/// Whether execution should stop at a statement on `line` at `depth`.
fn should_stop(mode: Mode, depth: usize) -> bool {
    match mode {
        Mode::Step => true,
        Mode::Next(d) => depth <= d,
        Mode::Finish(d) => depth < d,
        Mode::Continue => false,
    }
}

fn find_var(vars: Vec<(String, Value)>, name: &str) -> Option<(String, Value)> {
    vars.into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
}

impl CliDebugger {
    /// `(function, line, col)` of frame `frame` (0 = innermost), using the
    /// current stop position for the innermost frame.
    fn frame_info(
        &self,
        machine: &Machine,
        frame: usize,
        here: (&str, usize, usize),
    ) -> Option<(String, usize, usize)> {
        if frame == 0 {
            return Some((here.0.to_string(), here.1, here.2));
        }
        let stack = machine.call_stack();
        let (name, span) = stack.get(frame)?;
        let lc = self.source.line_col(span.start);
        Some((name.clone(), lc.line as usize, lc.col as usize))
    }

    fn print_frame(&self, machine: &Machine, frame: usize, here: (&str, usize, usize)) {
        if let Some((f, l, c)) = self.frame_info(machine, frame, here) {
            println!("Frame #{frame} {f}() at line {l}:{c}");
        }
    }

    fn read_command(&mut self) -> Option<String> {
        print!("(ppldb) ");
        let _ = io::stdout().flush();
        let mut input = String::new();
        match self.input.read_line(&mut input) {
            Ok(0) => {
                println!();
                println!("End of input: quitting debugger.");
                None
            }
            Ok(_) => Some(input),
            Err(e) => {
                println!();
                println!("Cannot read debugger input ({e}): quitting debugger.");
                None
            }
        }
    }
}

impl Debugger for CliDebugger {
    fn on_statement(
        &mut self,
        machine: &Machine,
        function: &str,
        span: Span,
        depth: usize,
    ) -> DebugAction {
        let lc = self.source.line_col(span.start);
        let (line, col) = (lc.line as usize, lc.col as usize);
        let prev = self.prev_stmt.replace((line, depth));
        let at_breakpoint = self.breakpoints.contains(&line) && prev != Some((line, depth));

        if !at_breakpoint && !should_stop(self.mode, depth) {
            return DebugAction::Continue;
        }

        if at_breakpoint {
            println!("\nBreakpoint hit at line {line}");
        } else {
            println!();
        }
        println!(
            "[STOP] {}() at {}:{}:{} (frame depth {})",
            function, self.source.name, line, col, depth
        );
        print_source_context(&self.source, line);

        let here = (function, line, col);
        let max_frame = machine.call_stack().len().saturating_sub(1);
        let mut frame = 0usize;
        let fmt = machine.settings.format();

        loop {
            let Some(input) = self.read_command() else {
                self.quit.set(true);
                return DebugAction::Abort;
            };
            let line_cmd = input.trim();
            if line_cmd.is_empty() {
                continue;
            }
            let mut parts = line_cmd.split_whitespace();
            let cmd = parts.next().unwrap_or_default();

            match cmd {
                "q" | "quit" => {
                    self.quit.set(true);
                    return DebugAction::Abort;
                }
                "s" | "step" => {
                    self.mode = Mode::Step;
                    return DebugAction::Continue;
                }
                "n" | "next" => {
                    self.mode = Mode::Next(depth);
                    return DebugAction::Continue;
                }
                "fin" | "finish" | "out" => {
                    self.mode = Mode::Finish(depth);
                    return DebugAction::Continue;
                }
                "c" | "cont" | "continue" => {
                    self.mode = Mode::Continue;
                    return DebugAction::Continue;
                }
                "b" | "break" => {
                    if let Some(target) = parts.next() {
                        match target.parse::<usize>() {
                            Ok(ln) if ln >= 1 => {
                                self.breakpoints.insert(ln);
                                println!("Breakpoint set at line {ln}");
                            }
                            _ => println!("Usage: b <line_number>"),
                        }
                    } else {
                        println!("Active breakpoints: {:?}", self.breakpoints);
                    }
                }
                "d" | "delete" => {
                    if let Some(target) = parts.next() {
                        if let Ok(ln) = target.parse::<usize>() {
                            self.breakpoints.remove(&ln);
                            println!("Removed breakpoint at line {ln}");
                        } else {
                            println!("Usage: d [line_number]");
                        }
                    } else {
                        self.breakpoints.clear();
                        println!("Cleared all breakpoints");
                    }
                }
                "l" | "locals" => {
                    let locs = machine.locals(frame);
                    if locs.is_empty() {
                        println!("(no locals)");
                    } else {
                        for (name, val) in locs {
                            println!("  {} = {}", name, val.display(fmt));
                        }
                    }
                }
                "g" | "globals" => {
                    let globs = machine.globals();
                    if globs.is_empty() {
                        println!("(no globals)");
                    }
                    for (name, val) in globs {
                        println!("  {} = {}", name, val.display(fmt));
                    }
                }
                "bt" | "stack" | "backtrace" => {
                    for i in 0..=max_frame {
                        if let Some((f, l, c)) = self.frame_info(machine, i, here) {
                            println!("  #{i} {f}() at line {l}:{c}");
                        }
                    }
                }
                "up" => {
                    if frame < max_frame {
                        frame += 1;
                        self.print_frame(machine, frame, here);
                    } else {
                        println!("Already at the outermost frame");
                    }
                }
                "down" => {
                    if frame > 0 {
                        frame -= 1;
                        self.print_frame(machine, frame, here);
                    } else {
                        println!("Already at the innermost frame");
                    }
                }
                "f" | "frame" => match parts.next().map(str::parse::<usize>) {
                    None => self.print_frame(machine, frame, here),
                    Some(Ok(n)) if n <= max_frame => {
                        frame = n;
                        self.print_frame(machine, frame, here);
                    }
                    _ => println!("Usage: frame [0..={max_frame}]"),
                },
                "p" | "print" => {
                    if let Some(var) = parts.next() {
                        match find_var(machine.locals(frame), var)
                            .or_else(|| find_var(machine.globals(), var))
                        {
                            Some((name, val)) => println!("  {} = {}", name, val.display(fmt)),
                            None => {
                                println!("Variable `{var}` not found in current scope or globals.")
                            }
                        }
                    } else {
                        println!("Usage: p <var_name>");
                    }
                }
                "list" => {
                    let l = self
                        .frame_info(machine, frame, here)
                        .map_or(line, |(_, l, _)| l);
                    print_source_context(&self.source, l);
                }
                "h" | "help" => {
                    println!("Debugger commands:");
                    println!("  s, step               Step to the next statement (into calls)");
                    println!("  n, next               Step over calls to the next statement here");
                    println!("  fin, finish, out      Run until the current function returns");
                    println!("  c, cont, continue     Continue running to a breakpoint or the end");
                    println!("  b, break [line]       Set or list breakpoints");
                    println!("  d, delete [line]      Delete breakpoint(s)");
                    println!("  l, locals             Show local variables of the selected frame");
                    println!("  g, globals            Show program and calculator globals");
                    println!(
                        "  p, print <var>        Print a variable (selected frame, then globals)"
                    );
                    println!("  bt, stack, backtrace  Show call stack (#0 = innermost)");
                    println!("  up, down, frame [n]   Select a caller / callee frame");
                    println!("  list                  Show source around the selected frame");
                    println!("  q, quit               Abort execution (also on end of input)");
                }
                other => {
                    println!("Unknown command `{other}`. Type `h` for help.");
                }
            }
        }
    }
}

fn print_source_context(source: &SourceFile, current_line: usize) {
    let lines: Vec<&str> = source.text.lines().collect();
    let start = current_line.saturating_sub(3);
    let end = (current_line + 2).min(lines.len());

    for (idx, line_text) in lines.iter().enumerate().take(end).skip(start) {
        let line_num = idx + 1;
        let prefix = if line_num == current_line { "=>" } else { "  " };
        println!("{} {:>4} | {}", prefix, line_num, line_text);
    }
}

pub fn run(o: DebugOpts) -> u8 {
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
    let args = match common::parse_args(o.args.as_deref(), &settings) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let host = match o.host() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };

    println!(
        "Starting debugger on `{}` entry function `{}`",
        file.name, entry
    );
    println!("Type `s` to step, `c` to continue, `h` for help.");

    let quit = Rc::new(Cell::new(false));
    let breakpoints: BTreeSet<usize> = o.breakpoints.iter().copied().collect();
    for ln in &breakpoints {
        println!("Breakpoint set at line {ln}");
    }
    let debugger = Box::new(CliDebugger {
        source: file.clone(),
        breakpoints,
        mode: if o.run { Mode::Continue } else { Mode::Step },
        prev_stmt: None,
        input: Box::new(io::stdin().lock()),
        quit: quit.clone(),
    });

    let mut m = Machine::new(prog, Box::new(host), settings, o.limits());
    m.set_seed(o.seed);
    m.set_debugger(Some(debugger));

    let result = m.call_function(&entry, args);
    let _ = io::stdout().flush();
    match result {
        Ok(val) => {
            println!(
                "\nProgram exited normally with value: {}",
                val.display(m.settings.format())
            );
            exit::OK
        }
        Err(_) if quit.get() => {
            println!("Debugger quit; program aborted.");
            exit::LIMIT
        }
        Err(e) => {
            eprintln!("\nExecution terminated: {e}");
            for (i, f) in e.trace.iter().enumerate() {
                let lc = file.line_col(f.span.start);
                let text = file.line_text(lc.line).trim();
                let arrow = if i == 0 { "-->" } else { "   " };
                eprintln!(
                    "  {arrow} {}:{}:{} in {}   {}",
                    file.name, lc.line, lc.col, f.function, text
                );
            }
            common::exit_code_for(&e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_source_context() {
        let file = SourceFile::new(
            "test.ppl",
            "EXPORT A()\nBEGIN\n  PRINT(1);\nEND;".to_string(),
        );
        print_source_context(&file, 3);
    }

    #[test]
    fn test_stop_modes() {
        assert!(should_stop(Mode::Step, 3));
        assert!(should_stop(Mode::Next(2), 2));
        assert!(should_stop(Mode::Next(2), 1));
        assert!(!should_stop(Mode::Next(2), 3));
        assert!(should_stop(Mode::Finish(2), 1));
        assert!(!should_stop(Mode::Finish(2), 2));
        assert!(!should_stop(Mode::Continue, 1));
    }

    #[test]
    fn test_default_limits_have_no_time_limit() {
        use clap::Parser;
        #[derive(Parser)]
        struct W {
            #[command(flatten)]
            o: DebugOpts,
        }
        let w = W::parse_from(["x", "f.ppl"]);
        assert_eq!(w.o.limits().max_time, None);
        let w = W::parse_from(["x", "f.ppl", "--max-time", "2"]);
        assert_eq!(w.o.limits().max_time, Some(Duration::from_secs(2)));
        let w = W::parse_from(["x", "f.ppl", "--max-time", "2", "--no-time-limit"]);
        assert_eq!(w.o.limits().max_time, None);
    }
}
