//! Thin native adapter; compiler and VM semantics remain in their existing crates.
use crate::{
    Cancellation, Diagnostic, Engine, Operation, OutputEvent, Report, Status, arguments,
    source::Source,
};
use ppl_graphics::Grob;
use ppl_vm::host::Touch;
use ppl_vm::{
    Dialog, DialogResult, DisplayMode, ErrorKind, HeadlessHost, Host, Limits, Machine,
    RuntimeError, Settings,
};
use serde_json::json;
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};
pub struct Options {
    pub entry: Option<String>,
    pub args: String,
    pub json_arguments: bool,
    pub host: HeadlessHost,
    pub settings: Settings,
    pub limits: Limits,
    pub seed: u64,
    pub png: Option<PathBuf>,
    pub deny_warnings: bool,
    pub dump_bytecode: bool,
}
struct Capture {
    inner: HeadlessHost,
    events: Arc<Mutex<Vec<OutputEvent>>>,
}
impl Host for Capture {
    fn print(&mut self, t: &str) {
        self.inner.print(t);
        self.events.lock().unwrap().push(OutputEvent {
            kind: "print".into(),
            text: Some(t.into()),
            value: None,
        });
    }
    fn clear_terminal(&mut self) {
        self.inner.clear_terminal();
        self.events.lock().unwrap().push(OutputEvent {
            kind: "clear".into(),
            text: None,
            value: None,
        });
    }
    fn present(&mut self, g: &Grob, t: Option<&[String]>) {
        self.inner.present(g, t)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.inner.poll_key()
    }
    fn is_key_down(&mut self, k: i64) -> Result<bool, RuntimeError> {
        self.inner.is_key_down(k)
    }
    fn wait_key(&mut self, t: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.inner.wait_key(t)
    }
    fn sleep(&mut self, s: f64) -> Result<(), RuntimeError> {
        self.inner.sleep(s)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.inner.touch()
    }
    fn dialog(&mut self, d: &Dialog, g: &Grob) -> Result<DialogResult, RuntimeError> {
        self.inner.dialog(d, g)
    }
    fn now_ms(&mut self) -> f64 {
        self.inner.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.inner.local_time()
    }
    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        self.inner.interrupt_flag()
    }
}
pub fn execute(source: Source, op: Operation, mut opts: Options, cancel: &Cancellation) -> Report {
    let start = Instant::now();
    let mut r = Report::new(Engine::Native, op);
    r.program = if source.stem.is_empty() {
        source.identity.display_name.clone()
    } else {
        source.stem.clone()
    };
    r.source = Some(source.identity.clone());
    r.requested_entry = opts.entry.clone();
    r.arguments =
        json!({"format":if opts.json_arguments{"json"}else{"ppl"},"raw":opts.args,"resolved":null});
    if cancel.cancelled() {
        r.fail(Status::Cancelled, "compile", "cancelled before compilation");
        return r;
    }
    let file = ppl_syntax::SourceFile::new(&source.identity.display_name, &source.text);
    let compile_start = Instant::now();
    let (program, diags) = ppl_compiler::compile_source(&file, &Default::default());
    let compile_s = compile_start.elapsed().as_secs_f64();
    r.diagnostics=diags.items.iter().map(|d|{let lc=file.line_col(d.span.start);Diagnostic{source:"native-compiler".into(),severity:if d.is_error(){"error"}else{"warning"}.into(),code:Some(d.code.into()),message:Some(d.message.clone()),line:Some(lc.line),column:Some(lc.col),raw_position:Some(json!({"start_byte":d.span.start,"end_byte":d.span.end,"hint":d.hint,"unverified":d.unverified,"class":format!("{:?}",d.class())}))}}).collect();
    r.compilation_status = Some(
        if program.is_some() && !source.text.trim().is_empty() {
            "valid"
        } else {
            "invalid"
        }
        .into(),
    );
    let warnings = diags.warnings().count();
    r.backend = json!({"function_count":program.as_ref().map(|p|p.functions.len()),"warning_count":warnings});
    let finish = |r: &mut Report, execution_s: f64| {
        r.timings = json!({"compilation_s":compile_s,"execution_s":execution_s,"total_s":start.elapsed().as_secs_f64()});
    };
    if cancel.cancelled() {
        r.fail(Status::Cancelled, "compile", "cancelled after compilation");
        finish(&mut r, 0.);
        return r;
    }
    if source.text.trim().is_empty() || program.is_none() {
        r.fail(
            Status::InvalidSource,
            "compile",
            "native compiler rejected source",
        );
        finish(&mut r, 0.);
        return r;
    }
    let program = program.unwrap();
    if opts.dump_bytecode {
        r.backend["disassembly"] = json!(program.disassemble());
    }
    if opts.deny_warnings && warnings > 0 {
        r.fail(Status::InvalidSource, "compile", "warnings denied");
        finish(&mut r, 0.);
        return r;
    }
    if op == Operation::Check {
        r.status = Status::Success;
        r.exit_code = 0;
        finish(&mut r, 0.);
        return r;
    }
    let entry = match program.pick_entry_with_stem(opts.entry.as_deref(), Some(&source.stem)) {
        Ok(x) => x,
        Err(e) => {
            r.fail(Status::InvalidEntry, "entry", e);
            finish(&mut r, 0.);
            return r;
        }
    };
    r.entry = Some(entry.clone());
    let args = if opts.json_arguments {
        arguments::json_args(&opts.args, &opts.settings)
    } else {
        arguments::ppl_args(Some(&opts.args), &opts.settings)
    };
    let args = match args {
        Ok(x) => x,
        Err(e) => {
            r.fail(Status::InvalidArguments, "arguments", e);
            finish(&mut r, 0.);
            return r;
        }
    };
    r.arguments["resolved"] = json!(args.iter().map(arguments::encode).collect::<Vec<_>>());
    let index = program.function_index(&entry).unwrap();
    if args.len() != program.functions[index].params as usize {
        r.fail(
            Status::InvalidArguments,
            "invocation",
            format!(
                "{} expects {} argument(s), got {}",
                entry,
                program.functions[index].params,
                args.len()
            ),
        );
        r.exit_code = 3;
        finish(&mut r, 0.);
        return r;
    }
    opts.host.interrupt = Some(cancel.0.clone());
    let events = Arc::new(Mutex::new(vec![]));
    let host = Capture {
        inner: opts.host,
        events: events.clone(),
    };
    let mut m = Machine::new(Rc::new(program), Box::new(host), opts.settings, opts.limits);
    m.set_seed(opts.seed);
    let executing = Instant::now();
    let result = m.call_function(&entry, args);
    let execution_s = executing.elapsed().as_secs_f64();
    r.output = events.lock().unwrap().clone();
    r.backend["steps"] = json!(m.steps());
    match result {
        Ok(value) => {
            r.status = Status::Success;
            r.exit_code = 0;
            r.execution_status = Some("completed".into());
            r.return_value = Some(arguments::encode(&value));
            r.backend["display_return"] = json!(value.display(m.settings.format()));
        }
        Err(e) => {
            let status = match e.kind {
                ErrorKind::Interrupted => Status::Cancelled,
                ErrorKind::ResourceLimit(_) | ErrorKind::ScriptExhausted => Status::TimedOut,
                ErrorKind::Unsupported => Status::Unsupported,
                _ => Status::RuntimeError,
            };
            r.runtime_errors=e.trace.iter().map(|f|{let lc=file.line_col(f.span.start);Diagnostic{source:"native-runtime".into(),severity:"error".into(),code:None,message:Some(e.to_string()),line:Some(lc.line),column:Some(lc.col),raw_position:Some(json!({"function":f.function,"start_byte":f.span.start,"end_byte":f.span.end}))}}).collect();
            r.fail(status, "runtime", e.to_string());
            if status == Status::Unsupported {
                r.exit_code = 3;
            }
            r.execution_status = serde_json::to_value(status)
                .unwrap()
                .as_str()
                .map(str::to_owned);
            r.backend["runtime_kind"] = json!(format!("{:?}", e.kind));
        }
    }
    if let Some(path) = opts.png {
        let image = if m.display_mode() == DisplayMode::Terminal {
            ppl_graphics::text::render_terminal(&m.terminal)
        } else {
            m.screen().clone()
        };
        if let Err(e) = image.save_png(&path) {
            r.fail(Status::IoError, "screenshot", e.to_string())
        } else {
            r.artifacts.push(path)
        }
    }
    finish(&mut r, execution_s);
    r
}
