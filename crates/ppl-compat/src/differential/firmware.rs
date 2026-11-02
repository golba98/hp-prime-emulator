use super::*;
use ppl_engine::{Cancellation, Operation, Status, firmware_options::Options, source::Source};
use serde_json::json;

/// One-shot adapter: never pretends separate helper invocations share state.
/// Stateful HLE scenarios require a future structured live-session adapter.
pub fn run_firmware(s: &LoadedScenario, options: &Options, cancel: &Cancellation) -> BackendRun {
    if s.scenario.initial.seed != 0 {
        return BackendRun::unsupported(
            BackendId::FirmwareHle,
            s,
            "Firmware RNG seeding has no verified PPL mapping; nondefault initial.seed cannot be established",
        );
    }
    if s.scenario.schema_version == 2
        || s.scenario
            .steps
            .iter()
            .filter(|x| matches!(x, Step::Run { .. }))
            .count()
            > 1
        || s.scenario.initial.setup_entry.is_some()
        || s.scenario.steps.iter().any(|x| {
            matches!(
                x,
                Step::PowerOff { .. }
                    | Step::Restart { .. }
                    | Step::Key { .. }
                    | Step::Touch { .. }
            )
        })
    {
        return run_live(s, options, cancel);
    }
    if let Err(reason) = s.verify_identity() {
        let mut result = BackendRun::new(BackendId::FirmwareHle, s);
        result.status = RunStatus::InfrastructureError;
        result.reason = Some(reason);
        return result;
    }
    let runs = s
        .scenario
        .steps
        .iter()
        .filter(|x| matches!(x, Step::Run { .. }))
        .count();
    if runs > 1
        || s.scenario.initial.setup_entry.is_some()
        || !s.scenario.initial.settings.is_empty()
        || s.scenario.steps.iter().any(|x| {
            matches!(
                x,
                Step::Observe { .. }
                    | Step::Key { .. }
                    | Step::Touch { .. }
                    | Step::PowerOff { .. }
                    | Step::Restart { .. }
            )
        })
    {
        return BackendRun::unsupported(
            BackendId::FirmwareHle,
            s,
            "One-shot firmware adapter cannot establish retained state, initial settings/setup, variable probes, physical input or power cycles; use live-session tests until a structured session adapter is available",
        );
    }
    if s.scenario
        .constraints
        .get(&BackendId::FirmwareHle)
        .is_some_and(|c| c.firmware_build.is_some())
    {
        return BackendRun::unsupported(
            BackendId::FirmwareHle,
            s,
            "Firmware Report identifies the ELF hash but not a verified build number; build constraint cannot be established",
        );
    }
    let actual_hash = options
        .firmware
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| crate::sha256_hex(&b));
    if let Some(required) = s
        .scenario
        .constraints
        .get(&BackendId::FirmwareHle)
        .and_then(|c| c.firmware_sha256.as_ref())
    {
        if actual_hash.as_ref() != Some(required) {
            return BackendRun::unsupported(
                BackendId::FirmwareHle,
                s,
                "configured firmware does not match the fixture's required SHA-256",
            );
        }
    }
    let mut report = BackendRun::new(BackendId::FirmwareHle, s);
    report.provenance.firmware_sha256 = actual_hash;
    let mut last_image = None;
    let temporary = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => return BackendRun::unavailable(BackendId::FirmwareHle, s, e.to_string()),
    };
    for step in &s.scenario.steps {
        let data = match step {
            Step::Check { .. } | Step::Run { .. } => {
                let mut opts = options.clone();
                let op = if let Step::Run { entry, args, .. } = step {
                    opts.entry = Some(entry.clone());
                    opts.args = serde_json::to_string(args).unwrap();
                    opts.screenshot = Some(temporary.path().join("screen.png"));
                    Operation::Run
                } else {
                    Operation::Check
                };
                let source = match Source::text(&s.source, &s.scenario.id) {
                    Ok(x) => x,
                    Err(e) => {
                        report.status = RunStatus::InfrastructureError;
                        report.reason = Some(e);
                        return report;
                    }
                };
                let raw = ppl_engine::firmware::execute(source, op, &opts, cancel);
                if let Some(hash) = raw.backend["report"]["firmware_sha256"].as_str() {
                    report.provenance.firmware_sha256 = Some(hash.to_owned());
                }
                if let Some(path) = &opts.screenshot {
                    last_image = std::fs::read(path)
                        .ok()
                        .and_then(|bytes| ppl_graphics::Grob::read_png(&bytes).ok())
                        .map(|g| Image::from(&g));
                }
                let status = match raw.status {
                    Status::Success => "success",
                    Status::InvalidSource => "compile_error",
                    Status::RuntimeError => "runtime_error",
                    Status::Unsupported => "unsupported",
                    Status::TimedOut => "timed_out",
                    Status::Cancelled => "cancelled",
                    Status::InvalidArguments => "invalid_arguments",
                    Status::Indeterminate => "unobserved",
                    _ => {
                        report.status = RunStatus::InfrastructureError;
                        "infrastructure_error"
                    }
                };
                let mut data = json!({"status":status});
                if op == Operation::Run {
                    data["return_value"] = raw.return_value.clone().unwrap_or_default();
                    data["output"] = json!(
                        raw.output
                            .iter()
                            .filter(|o| o.kind == "print")
                            .filter_map(|o| o.text.clone())
                            .collect::<Vec<_>>()
                    );
                    if raw.status == Status::Success && raw.return_value.is_none() {
                        data["status"] = json!("unobserved");
                        data["reason"] = json!(
                            "firmware result was not decoded; inspect retained raw representation"
                        );
                    }
                    if let Some(e) = &raw.error {
                        data["error"] = json!({"kind":format!("{:?}",e.kind),"message":e.message,"line":raw.runtime_errors.first().and_then(|e|e.line),"column":raw.runtime_errors.first().and_then(|e|e.column)});
                    }
                } else {
                    data["diagnostics"]=json!(raw.diagnostics.iter().map(|d|json!({"severity":d.severity,"code":d.code,"message":d.message,"line":d.line,"column":d.column})).collect::<Vec<_>>());
                }
                report.raw_reports.push(serde_json::to_value(raw).unwrap());
                data
            }
            Step::Capture { .. } => match &last_image {
                Some(i) => json!({"status":"success","image":i}),
                None => {
                    json!({"status":"unobserved","reason":"no decoded screenshot from preceding Run"})
                }
            },
            _ => unreachable!("preflight capability check"),
        };
        report.observations.push(Observation {
            step: step.id().into(),
            data,
        });
    }
    report
}

fn execution_data(o: &ppl_engine::session::ExecutionObservation) -> serde_json::Value {
    let status = match o.status.as_str() {
        "completed" if o.body_entered && o.completed => {
            if o.return_value.is_some() {
                "success"
            } else {
                "unobserved"
            }
        }
        "runtime_error" => "runtime_error",
        "cancelled" => "cancelled",
        "timed_out" => "timed_out",
        "invalid_arguments" => "invalid_arguments",
        _ => "infrastructure_error",
    };
    let mut value = json!({"status":status,"return_value":o.return_value,"output":o.output.iter().filter(|e|e.kind=="print").filter_map(|e|e.text.clone()).collect::<Vec<_>>()});
    if let Some(d) = o.diagnostics.first() {
        value["error"] =
            json!({"kind":"RuntimeError","message":d.message,"line":d.line,"column":d.column});
    }
    value
}
fn run_live(s: &LoadedScenario, options: &Options, cancel: &Cancellation) -> BackendRun {
    use ppl_engine::{
        live::{LiveSession, copy_profile},
        session::Command,
    };
    use std::{collections::BTreeMap, time::Duration};
    if let Err(reason) = s.verify_identity() {
        let mut r = BackendRun::new(BackendId::FirmwareHle, s);
        r.status = RunStatus::InfrastructureError;
        r.reason = Some(reason);
        return r;
    }
    let mut report = BackendRun::new(BackendId::FirmwareHle, s);
    if s.scenario
        .constraints
        .get(&BackendId::FirmwareHle)
        .is_some_and(|c| c.firmware_build.is_some())
    {
        return BackendRun::unsupported(
            BackendId::FirmwareHle,
            s,
            "Session identifies firmware by ELF hash; build-number constraint unavailable",
        );
    }
    if s.scenario
        .steps
        .iter()
        .any(|x| matches!(x, Step::Observe { .. }))
    {
        return BackendRun::unsupported(
            BackendId::FirmwareHle,
            s,
            "Use authored probe entries for retained variables; raw key/touch delivery is not yet correlated in the differential driver",
        );
    }
    let actual = options
        .firmware
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| crate::sha256_hex(&b));
    if s.scenario
        .constraints
        .get(&BackendId::FirmwareHle)
        .and_then(|c| c.firmware_sha256.as_ref())
        .is_some_and(|h| Some(h) != actual.as_ref())
    {
        return BackendRun::unsupported(
            BackendId::FirmwareHle,
            s,
            "Configured ELF differs from required exact firmware SHA",
        );
    }
    report.provenance.firmware_sha256 = actual;
    let temporary = match tempfile::tempdir() {
        Ok(t) => t,
        Err(e) => return BackendRun::unavailable(BackendId::FirmwareHle, s, e.to_string()),
    };
    let profile = temporary.path().join("profile");
    let Some(initial) = &options.init_data else {
        return BackendRun::unavailable(
            BackendId::FirmwareHle,
            s,
            "Stateful scenarios require initialized firmware storage (--firmware-init-data)",
        );
    };
    for drive in ["A", "C"] {
        if let Err(e) = copy_profile(&initial.join(drive), &profile.join(drive)) {
            return BackendRun::unavailable(BackendId::FirmwareHle, s, e);
        }
    }
    let mut live = match LiveSession::spawn(options, &profile, cancel) {
        Ok(l) => Some(l),
        Err(e) => return BackendRun::unavailable(BackendId::FirmwareHle, s, e),
    };
    let mut sources = s.sources.clone();
    if !s.source.is_empty() {
        sources.insert("ZZT_DFDEFAULT".into(), s.source.clone());
    }
    let mut imported = BTreeMap::<String, String>::new();
    let mut active = BTreeMap::<String, (u64, String, String, String)>::new();
    let mut id = 0u64;
    let mut last_image = None;
    let work = (|| -> Result<(), String> {
        if !s.scenario.initial.settings.is_empty() {
            let body = s
                .scenario
                .initial
                .settings
                .iter()
                .map(|(name, value)| format!("{name}:={value};"))
                .collect::<String>();
            let source = format!("EXPORT ZZTDIFSET() BEGIN {body} RETURN 1; END;");
            id += 1;
            let r = live.as_mut().unwrap().request(
                id,
                &Command::StoreProgram {
                    request: id,
                    name: "ZZT_DFSETUP".into(),
                    source: source.clone(),
                    expected_previous_sha256: None,
                },
                options.timeout,
                cancel,
            )?;
            if !r.success {
                return Err(r.message);
            }
            id += 1;
            let r = live.as_mut().unwrap().request(
                id,
                &Command::RunStored {
                    request: id,
                    name: "ZZT_DFSETUP".into(),
                    source_sha256: crate::sha256_hex(source.as_bytes()),
                    entry: "ZZTDIFSET".into(),
                    arguments: String::new(),
                },
                options.timeout,
                cancel,
            )?;
            if !r.success || r.execution.as_ref().is_none_or(|o| o.status != "completed") {
                return Err("Initial firmware settings routine failed".into());
            }
        }
        if let Some(entry) = &s.scenario.initial.setup_entry {
            let source = sources
                .get("ZZT_DFDEFAULT")
                .ok_or("initial setup_entry needs a default program")?
                .clone();
            id += 1;
            let r = live.as_mut().unwrap().request(
                id,
                &Command::StoreProgram {
                    request: id,
                    name: "ZZT_DFDEFAULT".into(),
                    source: source.clone(),
                    expected_previous_sha256: None,
                },
                options.timeout,
                cancel,
            )?;
            if !r.success {
                return Err(r.message);
            }
            imported.insert("ZZT_DFDEFAULT".into(), crate::sha256_hex(source.as_bytes()));
            id += 1;
            let r = live.as_mut().unwrap().request(
                id,
                &Command::RunStored {
                    request: id,
                    name: "ZZT_DFDEFAULT".into(),
                    source_sha256: imported["ZZT_DFDEFAULT"].clone(),
                    entry: entry.clone(),
                    arguments: String::new(),
                },
                options.timeout,
                cancel,
            )?;
            if !r.success {
                return Err(r.message);
            }
        }
        for step in &s.scenario.steps {
            if cancel.cancelled() {
                return Err("Scenario cancelled".into());
            }
            if !active.is_empty()
                && !matches!(
                    step,
                    Step::Await { .. }
                        | Step::Key { .. }
                        | Step::Touch { .. }
                        | Step::Capture { .. }
                )
            {
                return Err(
                    "Await the active invocation before another calculator operation".into(),
                );
            }
            let data=match step{
                Step::Checkpoint{wake,..}=>{
                    id+=1;let result=live.as_mut().ok_or("Calculator process is off; restart before checkpointing")?.checkpoint(id,*wake,Duration::from_secs(60),cancel)?;
                    let data=match &result{
                        ppl_engine::checkpoint::CheckpointResult::Saved{receipt}=>json!({"status":"success","state_kind":receipt.kind,"safe_point":receipt.safe_point,"restores_cpu":receipt.restores_cpu,"restores_framebuffer":receipt.restores_framebuffer}),
                        ppl_engine::checkpoint::CheckpointResult::Deferred{reason}=>json!({"status":"deferred","reason":reason}),
                        ppl_engine::checkpoint::CheckpointResult::Failed{message}=>json!({"status":"checkpoint_error","message":message}),
                    };
                    report.raw_reports.push(json!({"checkpoint_request":id,"result":result}));
                    data
                }
                Step::Start{program,entry,args,..}=>{
                    let hash=imported.get(program).ok_or("Start requires an imported program")?.clone();
                    id+=1;
                    let l=live.as_mut().ok_or("Calculator is off")?;
                    l.send(&Command::RunStored{request:id,name:program.clone(),source_sha256:hash.clone(),entry:entry.clone(),arguments:serde_json::to_string(args).unwrap().trim_start_matches('[').trim_end_matches(']').into()})?;
                    l.await_started(id,&hash,entry,options.compile_timeout,cancel)?;
                    active.insert(step.id().into(),(id,program.clone(),hash,entry.clone()));
                    json!({"status":"success"})
                }
                Step::Await{run,timeout_ms,..}=>{
                    let (request,_,hash,entry)=active.remove(run).ok_or("Await does not reference an active Start")?;
                    let l=live.as_mut().ok_or("Calculator is off")?;
                    let r=l.await_result(request,Duration::from_millis(*timeout_ms),cancel)?;
                    let o=r.execution.ok_or_else(||format!("Await has no runtime evidence: {}",r.message))?;
                    if o.entry!=entry||o.runtime["invocations"][0]["source_sha256"]!=hash{return Err("Await invocation identity mismatch".into());}
                    id+=1;let released=l.request(id,&Command::ReleaseInput{request:id},options.compile_timeout,cancel)?;
                    if !released.success{return Err(released.message);}
                    last_image=o.image.as_ref().map(|i|Image{width:i.width,height:i.height,pixels:i.pixels.clone()});
                    let data=execution_data(&o);report.raw_reports.push(serde_json::to_value(o).unwrap());data
                }
                Step::Key{code,down,..}=>{
                    let run_request=active.values().next().ok_or("Scripted keys require an active Start before Await")?.0;
                    id+=1;let r=live.as_mut().ok_or("Calculator is off")?.request(id,&Command::ScenarioInput{request:id,run_request,input:ppl_engine::session::CalculatorInput::Key{physical:*code as u8,down:*down}},options.timeout,cancel)?;
                    if !r.success{return Err(r.message);}json!({"status":"success","delivery_level":"queued"})
                }
                Step::Touch{status,x,y,..}=>{
                    let run_request=active.values().next().ok_or("Scripted touch requires an active Start before Await")?.0;
                    id+=1;let r=live.as_mut().ok_or("Calculator is off")?.request(id,&Command::ScenarioInput{request:id,run_request,input:ppl_engine::session::CalculatorInput::Touch{status:*status,x:*x,y:*y}},options.timeout,cancel)?;
                    if !r.success{return Err(r.message);}json!({"status":"success","delivery_level":"queued"})
                }
                Step::Import{program,..}|Step::Edit{program,..}=>{
                    let old=imported.get(program).cloned();
                    if matches!(step,Step::Import{..})&&old.is_some(){return Err(format!("{program} already imported"));}
                    if let Step::Edit{source,..}=step {if old.is_none(){return Err("Edit requires an imported program".into());}sources.insert(program.clone(),ppl_engine::source::Source::text(source,program)?.text);}
                    let source=sources.get(program).ok_or("Unknown named program")?.clone();
                    id+=1;let r=live.as_mut().ok_or("Calculator is off; restart first")?.request(id,&Command::StoreProgram{request:id,name:program.clone(),source:source.clone(),expected_previous_sha256:old},options.timeout,cancel)?;
                    if !r.success{return Err(r.message);}
                    let verdict=r.check.ok_or("Named assignment has no exact-source compiler verdict")?;
                    let hash=crate::sha256_hex(source.as_bytes());if !verdict.accepted||verdict.source_sha256!=hash{return Err("Named assignment verdict differs from fixture".into());}
                    imported.insert(program.clone(),hash);json!({"status":"success"})
                }
                Step::Run{entry,args,..}|Step::RunProgram{entry,args,..}=>{
                    let name=if let Step::RunProgram{program,..}=step{program.as_str()}else{"ZZT_DFDEFAULT"};
                    if !imported.contains_key(name){
                        if name!="ZZT_DFDEFAULT"{return Err(format!("{name} must be imported before Run"));}
                        let source=sources.get(name).ok_or("Missing default program")?.clone();
                        id+=1;let r=live.as_mut().ok_or("Calculator is off")?.request(id,&Command::StoreProgram{request:id,name:name.into(),source:source.clone(),expected_previous_sha256:None},options.timeout,cancel)?;
                        if !r.success{return Err(r.message);}imported.insert(name.into(),crate::sha256_hex(source.as_bytes()));
                    }
                    id+=1;let r=live.as_mut().ok_or("Calculator is off")?.request(id,&Command::RunStored{request:id,name:name.into(),source_sha256:imported[name].clone(),entry:entry.clone(),arguments:serde_json::to_string(args).unwrap().trim_start_matches('[').trim_end_matches(']').to_owned()},options.timeout,cancel)?;
                    let o=r.execution.ok_or_else(||format!("No structured execution result: {}",r.message))?;
                    if o.entry!=*entry || o.runtime["invocations"][0]["source_sha256"]!=imported[name]{return Err("Execution source/entry differs from requested program".into());}
                    last_image=o.image.as_ref().map(|i|Image{width:i.width,height:i.height,pixels:i.pixels.clone()});
                    let data=execution_data(&o);report.raw_reports.push(serde_json::to_value(o).unwrap());data
                }
                Step::Probe{program,entry,names,..}=>{
                    let hash=imported.get(program).ok_or("Probe requires an imported program")?.clone();
                    id+=1;let r=live.as_mut().ok_or("Calculator is off")?.request(id,&Command::RunStored{request:id,name:program.clone(),source_sha256:hash.clone(),entry:entry.clone(),arguments:String::new()},options.timeout,cancel)?;
                    let o=r.execution.ok_or_else(||format!("Probe has no structured runtime result: {}",r.message))?;
                    if o.entry!=*entry || o.runtime["invocations"][0]["source_sha256"]!=hash{return Err("Probe source/entry identity differs".into());}
                    let data=if o.status=="completed"&&o.body_entered&&o.completed{super::native::probe_data(o.return_value.as_ref().unwrap_or(&serde_json::Value::Null),names)}else{execution_data(&o)};
                    report.raw_reports.push(serde_json::to_value(o).unwrap());data
                }
                Step::Capture{..}=>last_image.as_ref().map(|i|json!({"status":"success","image":i})).unwrap_or_else(||json!({"status":"unobserved","reason":"no preceding program-output frame"})),
                Step::PowerOff{..}=>{live.as_mut().ok_or("Calculator is already off")?.power_down(Duration::from_secs(60),cancel)?;live.take();json!({"status":"success"})},
                Step::Restart{..}=>{if let Some(l)=live.as_mut(){l.power_down(Duration::from_secs(60),cancel)?;}live.take();live=Some(LiveSession::spawn(options,&profile,cancel)?);json!({"status":"success"})},
                Step::Check{..}=>return Err("Stateful Check requires explicit import verdict; one-shot Check remains available".into()),
                _=>unreachable!("capability preflight"),
            };
            report.observations.push(Observation {
                step: step.id().into(),
                data,
            });
        }
        if !active.is_empty() {
            return Err("Scenario ended with an unawaited invocation".into());
        }
        if let Some(l) = live.as_mut() {
            l.power_down(Duration::from_secs(60), cancel)?;
        }
        Ok(())
    })();
    if let Err(reason) = work {
        report.status = RunStatus::InfrastructureError;
        report.reason = Some(reason);
    }
    report
}
