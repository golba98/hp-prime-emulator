use super::*;
use ppl_engine::{Cancellation, arguments};
use ppl_graphics::Grob;
use ppl_vm::{
    Dialog, DialogResult, HeadlessHost, Host, Limits, RuntimeError, SessionContext, SourceIdentity,
};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeMap,
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

struct SharedHost(Arc<Mutex<HeadlessHost>>);
impl Host for SharedHost {
    fn print(&mut self, t: &str) {
        self.0.lock().unwrap().print(t)
    }
    fn clear_terminal(&mut self) {
        self.0.lock().unwrap().clear_terminal()
    }
    fn present(&mut self, g: &Grob, t: Option<&[String]>) {
        self.0.lock().unwrap().present(g, t)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.0.lock().unwrap().poll_key()
    }
    fn is_key_down(&mut self, k: i64) -> Result<bool, RuntimeError> {
        self.0.lock().unwrap().is_key_down(k)
    }
    fn wait_key(&mut self, t: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.0.lock().unwrap().wait_key(t)
    }
    fn sleep(&mut self, s: f64) -> Result<(), RuntimeError> {
        self.0.lock().unwrap().sleep(s)
    }
    fn touch(&mut self) -> Result<Option<ppl_vm::host::Touch>, RuntimeError> {
        self.0.lock().unwrap().touch()
    }
    fn dialog(&mut self, d: &Dialog, g: &Grob) -> Result<DialogResult, RuntimeError> {
        self.0.lock().unwrap().dialog(d, g)
    }
    fn now_ms(&mut self) -> f64 {
        self.0.lock().unwrap().now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.0.lock().unwrap().local_time()
    }
    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        self.0.lock().unwrap().interrupt_flag()
    }
}

pub fn run_native(s: &LoadedScenario, cancel: &Cancellation) -> BackendRun {
    if s.scenario.schema_version == 2 {
        return run_native_stateful(s, cancel);
    }
    if let Err(reason) = s.verify_identity() {
        let mut result = BackendRun::new(BackendId::Native, s);
        result.status = RunStatus::InfrastructureError;
        result.reason = Some(reason);
        return result;
    }
    if s.scenario
        .constraints
        .get(&BackendId::Native)
        .is_some_and(|c| c.firmware_build.is_some() || c.firmware_sha256.is_some())
    {
        return BackendRun::unsupported(
            BackendId::Native,
            s,
            "Native has no firmware build; this constraint cannot be established",
        );
    }
    if s.scenario.steps.iter().any(|step| {
        matches!(
            step,
            Step::Key { .. } | Step::Touch { .. } | Step::PowerOff { .. } | Step::Restart { .. }
        )
    }) {
        return BackendRun::unsupported(
            BackendId::Native,
            s,
            "Native differential runner supports Check, Run, Observe and Capture; physical key/touch and power-cycle semantics require Firmware HLE",
        );
    }
    let mut report = BackendRun::new(BackendId::Native, s);
    let file = ppl_syntax::SourceFile::new(&s.scenario.id, &s.source);
    let (program, diags) = ppl_compiler::compile_source(&file, &Default::default());
    let diagnostics:Vec<_>=diags.items.iter().map(|d|{let lc=file.line_col(d.span.start);json!({"severity":if d.is_error(){"error"}else{"warning"},"code":d.code,"message":d.message,"line":lc.line,"column":lc.col})}).collect();
    let compiled = program.map(Rc::new);
    let mut settings = ppl_vm::Settings::default();
    if let Some(v) = s.scenario.initial.settings.get("HAngle") {
        settings.angle = ppl_vm::AngleMode::from_code(*v).unwrap();
    }
    let format = *s
        .scenario
        .initial
        .settings
        .get("HFormat")
        .unwrap_or(&settings.hformat);
    let digits = *s
        .scenario
        .initial
        .settings
        .get("HDigits")
        .unwrap_or(&settings.hdigits);
    settings.set_format(format, digits);
    let mut session = SessionContext::new(settings, s.scenario.initial.seed);
    let host = Arc::new(Mutex::new(HeadlessHost::new()));
    host.lock().unwrap().interrupt = Some(cancel.0.clone());
    let identity = SourceIdentity::new(&s.scenario.id, &s.source_sha256);
    let mut globals = BTreeMap::new();
    let mut screen = Grob::screen();
    let limits = Limits {
        max_steps: Some(5_000_000),
        ..Default::default()
    };
    if let Some(entry) = &s.scenario.initial.setup_entry {
        let Some(program) = &compiled else {
            report.status = RunStatus::InfrastructureError;
            report.reason = Some("initial setup cannot run: source did not compile".into());
            return report;
        };
        let setup = session.run(
            identity.clone(),
            program.clone(),
            Box::new(SharedHost(host.clone())),
            limits.clone(),
            false,
            |m| {
                let result = m.call_function(entry, vec![]);
                globals = m
                    .globals()
                    .into_iter()
                    .map(|(n, v)| (n, arguments::encode(&v)))
                    .collect();
                screen = if m.display_mode() == ppl_vm::DisplayMode::Terminal {
                    ppl_graphics::text::render_terminal(&m.terminal)
                } else {
                    m.screen().clone()
                };
                result
            },
        );
        if let Err(e) = setup.result {
            report.status = RunStatus::InfrastructureError;
            report.reason = Some(format!("initial setup failed: {e}"));
            return report;
        }
        host.lock().unwrap().output.clear();
    }
    for step in &s.scenario.steps {
        if cancel.cancelled() {
            report.status = RunStatus::InfrastructureError;
            report.reason = Some("scenario cancelled".into());
            break;
        }
        let data = match step {
            Step::Check { .. } => {
                json!({"status":if compiled.is_some(){"success"}else{"compile_error"},"diagnostics":diagnostics})
            }
            Step::Run { entry, args, .. } => {
                let Some(program) = &compiled else {
                    report.observations.push(Observation {
                        step: step.id().into(),
                        data: json!({"status":"compile_error","diagnostics":diagnostics}),
                    });
                    continue;
                };
                let args = match arguments::json_args(
                    &serde_json::to_string(args).unwrap(),
                    &session.settings,
                ) {
                    Ok(a) => a,
                    Err(e) => {
                        report.observations.push(Observation {
                            step: step.id().into(),
                            data: json!({"status":"invalid_arguments","error":e}),
                        });
                        continue;
                    }
                };
                host.lock().unwrap().output.clear();
                let run = session.run(
                    identity.clone(),
                    program.clone(),
                    Box::new(SharedHost(host.clone())),
                    limits.clone(),
                    true,
                    |m| {
                        let result = m.call_function(entry, args);
                        globals = m
                            .globals()
                            .into_iter()
                            .map(|(n, v)| (n, arguments::encode(&v)))
                            .collect();
                        screen = if m.display_mode() == ppl_vm::DisplayMode::Terminal {
                            ppl_graphics::text::render_terminal(&m.terminal)
                        } else {
                            m.screen().clone()
                        };
                        result
                    },
                );
                let output = host.lock().unwrap().output.clone();
                match run.result {
                    Ok(v) => {
                        json!({"status":"success","return_value":arguments::encode(&v),"output":output})
                    }
                    Err(e) => {
                        let lc = e.trace.first().map(|f| file.line_col(f.span.start));
                        json!({"status":match e.kind{ppl_vm::ErrorKind::Unsupported=>"unsupported",ppl_vm::ErrorKind::Interrupted=>"cancelled",ppl_vm::ErrorKind::ResourceLimit(_)=>"timed_out",_=>"runtime_error"},"error":{"kind":format!("{:?}",e.kind),"message":e.to_string(),"line":lc.map(|p|p.line),"column":lc.map(|p|p.col)},"output":output})
                    }
                }
            }
            Step::Observe { names, .. } => {
                let snapshot = session.snapshot();
                let mut values = BTreeMap::<String, Json>::new();
                let mut missing = vec![];
                for name in names {
                    let setting = match name.as_str() {
                        "HAngle" => Some(snapshot.settings.angle.code()),
                        "HFormat" => Some(snapshot.settings.hformat),
                        "HDigits" => Some(snapshot.settings.hdigits),
                        _ => None,
                    };
                    if let Some(v) = setting {
                        values.insert(name.clone(), json!({"type":"real","value":v.to_string()}));
                    } else if let Some(v) = globals.get(name) {
                        values.insert(name.clone(), v.clone());
                    } else if let Some(v) = snapshot.globals.get(name) {
                        values.insert(name.clone(), arguments::encode(&v.to_value()));
                    } else {
                        missing.push(name.clone());
                    }
                }
                json!({"status":if missing.is_empty(){"success"}else{"unobserved"},"variables":values,"missing":missing})
            }
            Step::Capture { .. } => json!({"status":"success","image":Image::from(&screen)}),
            _ => unreachable!("preflight capability check"),
        };
        report.observations.push(Observation {
            step: step.id().into(),
            data,
        });
    }
    report
}

fn run_native_stateful(s: &LoadedScenario, cancel: &Cancellation) -> BackendRun {
    let mut report = BackendRun::new(BackendId::Native, s);
    if let Err(reason) = s.verify_identity() {
        report.status = RunStatus::InfrastructureError;
        report.reason = Some(reason);
        return report;
    }
    if s.scenario.initial.setup_entry.is_some()
        || s.scenario
            .constraints
            .get(&BackendId::Native)
            .is_some_and(|c| c.firmware_build.is_some() || c.firmware_sha256.is_some())
        || s.scenario.steps.iter().any(|step| {
            matches!(
                step,
                Step::Key { .. }
                    | Step::Touch { .. }
                    | Step::PowerOff { .. }
                    | Step::Restart { .. }
            )
        })
    {
        return BackendRun::unsupported(
            BackendId::Native,
            s,
            "Native has no firmware identity, physical event loop or firmware power persistence; use authored setup steps",
        );
    }
    let mut settings = ppl_vm::Settings::default();
    if let Some(v) = s.scenario.initial.settings.get("HAngle") {
        settings.angle = ppl_vm::AngleMode::from_code(*v).unwrap();
    }
    if let Some(v) = s.scenario.initial.settings.get("HFormat") {
        settings.hformat = *v;
    }
    if let Some(v) = s.scenario.initial.settings.get("HDigits") {
        settings.hdigits = *v;
    }
    let mut session = SessionContext::new(settings, s.scenario.initial.seed);
    let host = Arc::new(Mutex::new(HeadlessHost::default()));
    host.lock().unwrap().interrupt = Some(cancel.0.clone());
    let mut sources = BTreeMap::<String, String>::new();
    let mut screen = Grob::screen();
    for step in &s.scenario.steps {
        if cancel.cancelled() {
            report.status = RunStatus::InfrastructureError;
            report.reason = Some("scenario cancelled".into());
            break;
        }
        let data = match step {
            Step::Import { program, .. } => {
                if sources.contains_key(program) {
                    json!({"status":"infrastructure_error","reason":"program already imported"})
                } else {
                    sources.insert(program.clone(), s.sources[program].clone());
                    json!({"status":"success"})
                }
            }
            Step::Edit {
                program, source, ..
            } => {
                if sources.contains_key(program) {
                    sources.insert(program.clone(), source.clone());
                    json!({"status":"success"})
                } else {
                    json!({"status":"infrastructure_error","reason":"program was not imported"})
                }
            }
            Step::RunProgram { program, entry, .. } | Step::Probe { program, entry, .. } => {
                let args = match step {
                    Step::RunProgram { args, .. } => args.clone(),
                    _ => vec![],
                };
                let Some(source) = sources.get(program) else {
                    report.status = RunStatus::InfrastructureError;
                    report.reason = Some(format!("{program} was not imported"));
                    return report;
                };
                let file = ppl_syntax::SourceFile::new(program, source);
                let (compiled, diags) = ppl_compiler::compile_source(&file, &Default::default());
                match compiled {
                    None => {
                        json!({"status":"compile_error","diagnostics":diags.items.iter().map(|d|json!({"code":d.code,"message":d.message})).collect::<Vec<_>>()})
                    }
                    Some(compiled) => {
                        let args = match arguments::json_args(
                            &serde_json::to_string(&args).unwrap(),
                            &session.settings,
                        ) {
                            Ok(a) => a,
                            Err(e) => {
                                report.status = RunStatus::InfrastructureError;
                                report.reason = Some(e);
                                return report;
                            }
                        };
                        host.lock().unwrap().output.clear();
                        let run = session.run(
                            SourceIdentity::new(program, crate::sha256_hex(source.as_bytes())),
                            Rc::new(compiled),
                            Box::new(SharedHost(host.clone())),
                            Limits::default(),
                            true,
                            |m| {
                                let value = m.call_function(entry, args);
                                screen = if m.display_mode() == ppl_vm::DisplayMode::Terminal {
                                    ppl_graphics::text::render_terminal(&m.terminal)
                                } else {
                                    m.screen().clone()
                                };
                                value
                            },
                        );
                        match run.result {
                            Ok(v) => {
                                let value = arguments::encode(&v);
                                if let Step::Probe { names, .. } = step {
                                    probe_data(&value, names)
                                } else {
                                    json!({"status":"success","return_value":value,"output":host.lock().unwrap().output})
                                }
                            }
                            Err(e) => {
                                json!({"status":if e.kind==ppl_vm::ErrorKind::Unsupported{"unsupported"}else{"runtime_error"},"error":{"kind":format!("{:?}",e.kind),"message":e.to_string()}})
                            }
                        }
                    }
                }
            }
            Step::Capture { .. } => json!({"status":"success","image":Image::from(&screen)}),
            _ => {
                return BackendRun::unsupported(
                    BackendId::Native,
                    s,
                    "Schema 2 Native supports import, edit, run_program, authored probe and capture; use schema 1 for legacy default-program steps",
                );
            }
        };
        report.observations.push(Observation {
            step: step.id().into(),
            data,
        });
    }
    report
}
/// Authored probes return one typed list item per name; no display parsing or memory injection.
pub(super) fn probe_data(value: &Json, names: &[String]) -> Json {
    let Some(items) = value
        .get("value")
        .and_then(Json::as_array)
        .filter(|_| value["type"] == "list")
    else {
        return json!({"status":"unobserved","reason":"probe did not return a typed list"});
    };
    if items.len() != names.len() {
        return json!({"status":"unobserved","reason":"probe item count differs from declared names"});
    }
    let variables: BTreeMap<_, _> = names.iter().cloned().zip(items.iter().cloned()).collect();
    json!({"status":"success","variables":variables,"missing":[]})
}
