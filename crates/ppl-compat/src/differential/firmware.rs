use super::*;
use ppl_engine::{Cancellation, Operation, Status, firmware_options::Options, source::Source};
use serde_json::json;

/// One-shot adapter: never pretends separate helper invocations share state.
/// Stateful HLE scenarios require a future structured live-session adapter.
pub fn run_firmware(s: &LoadedScenario, options: &Options, cancel: &Cancellation) -> BackendRun {
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
