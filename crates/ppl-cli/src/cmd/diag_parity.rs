//! `ppl diag-parity`: compare the editor's native diagnostics with
//! authentic HP Prime Check results over the diagnostic corpus.
//!
//! `capture` runs the original firmware (local emulator, never the physical
//! calculator) and writes a reference file, either one fresh checker per case
//! or through one long-lived firmware session. `report` compares the native
//! analyzer with a reference file and `diff` compares two reference files;
//! neither needs firmware.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand, ValueEnum};
use ppl_compat::diag_parity::{
    self, BASELINE_FILE, Baseline, Corpus, DEFAULT_CORPUS_DIR, LOCAL_FIRMWARE_REFERENCE,
    ReferenceFile, ReferenceResult, Verdict,
};
use ppl_engine::session::{self as wire, CheckVerdict, Event, Packet};
use ppl_engine::{Cancellation, Operation, Report, Status, source::Source};

#[derive(Debug, Args)]
pub struct DiagParityOpts {
    #[command(subcommand)]
    pub action: DiagParityAction,
}

#[derive(Debug, Subcommand)]
pub enum DiagParityAction {
    /// Compare the native analyzer with a reference file.
    Report {
        /// Corpus directory.
        #[arg(long, default_value = DEFAULT_CORPUS_DIR)]
        corpus: PathBuf,
        /// Reference file (default: the local-firmware reference in the corpus).
        #[arg(long)]
        reference: Option<PathBuf>,
        /// Emit JSON instead of a Markdown table.
        #[arg(long)]
        json: bool,
        /// Exit 1 when outcomes differ from the corpus baseline, or when
        /// any verdict disagrees with the reference.
        #[arg(long)]
        check_baseline: bool,
    },
    /// Compare two reference files case by case (verdict, status, message,
    /// line, column, UTF-16 offset). Exit 1 on any difference.
    Diff {
        first: PathBuf,
        second: PathBuf,
        /// Compare only these case ids (default: all).
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    },
    /// Check every corpus case with the original firmware (local emulator)
    /// and write a reference file. Configure with PPL_FIRMWARE, PPL_OS_ROM,
    /// PPL_FACTORY_A, PPL_FIRMWARE_INIT_DATA and PPL_FIRMWARE_RUNNER.
    Capture {
        #[arg(long, default_value = DEFAULT_CORPUS_DIR)]
        corpus: PathBuf,
        /// Output reference file (default: the local-firmware reference in the corpus).
        #[arg(long)]
        output: Option<PathBuf>,
        /// Directory that retains each case's firmware evidence.
        #[arg(long)]
        artifacts_dir: PathBuf,
        /// Replace an existing reference file.
        #[arg(long)]
        overwrite: bool,
        /// `fresh`: a new firmware boot per case. `session`: every case
        /// through one long-lived firmware session, as the IDE uses it.
        #[arg(long, value_enum, default_value_t = Engine::Fresh)]
        engine: Engine,
        /// Session only: check the cases in reverse order.
        #[arg(long)]
        reverse: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Engine {
    Fresh,
    Session,
}

pub fn run(o: DiagParityOpts) -> u8 {
    let result = match o.action {
        DiagParityAction::Report {
            corpus,
            reference,
            json,
            check_baseline,
        } => report(&corpus, reference, json, check_baseline),
        DiagParityAction::Diff {
            first,
            second,
            only,
        } => diff(&first, &second, &only),
        DiagParityAction::Capture {
            corpus,
            output,
            artifacts_dir,
            overwrite,
            engine,
            reverse,
        } => capture(&corpus, output, &artifacts_dir, overwrite, engine, reverse),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn report(
    corpus_dir: &Path,
    reference: Option<PathBuf>,
    json: bool,
    check_baseline: bool,
) -> Result<u8, String> {
    let corpus = diag_parity::load_corpus(corpus_dir)?;
    let reference_path = reference.unwrap_or_else(|| corpus_dir.join(LOCAL_FIRMWARE_REFERENCE));
    let reference = ReferenceFile::load(&reference_path)?;
    let report = diag_parity::run_parity(&corpus, &reference);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        print!("{}", report.to_markdown());
    }
    if !check_baseline {
        return Ok(0);
    }
    let baseline = Baseline::load(&corpus_dir.join(BASELINE_FILE))?;
    let differences = diag_parity::baseline_differences(&report, &baseline);
    for d in &differences {
        eprintln!("baseline: {d}");
    }
    let disagreements = diag_parity::verdict_disagreements(&report);
    for d in &disagreements {
        eprintln!("verdict: {d}");
    }
    Ok(if differences.is_empty() && disagreements.is_empty() {
        0
    } else {
        1
    })
}

fn diff(first: &Path, second: &Path, only: &[String]) -> Result<u8, String> {
    let mut a = ReferenceFile::load(first)?;
    let mut b = ReferenceFile::load(second)?;
    if !only.is_empty() {
        a.results.retain(|r| only.contains(&r.id));
        b.results.retain(|r| only.contains(&r.id));
    }
    let differences = diag_parity::reference_differences(&a, &b);
    if a.firmware_sha256 != b.firmware_sha256 {
        println!(
            "firmware: {:?} ≠ {:?}",
            a.firmware_sha256, b.firmware_sha256
        );
    }
    for d in &differences {
        println!("{d}");
    }
    println!(
        "{} ({}) vs {} ({}): {} case(s) compared, {} difference(s)",
        first.display(),
        a.evidence,
        second.display(),
        b.evidence,
        a.results.len().max(b.results.len()),
        differences.len()
    );
    Ok(
        if differences.is_empty() && a.firmware_sha256 == b.firmware_sha256 {
            0
        } else {
            1
        },
    )
}

fn capture(
    corpus_dir: &Path,
    output: Option<PathBuf>,
    artifacts_dir: &Path,
    overwrite: bool,
    engine: Engine,
    reverse: bool,
) -> Result<u8, String> {
    let output = output.unwrap_or_else(|| corpus_dir.join(LOCAL_FIRMWARE_REFERENCE));
    if output.exists() && !overwrite {
        return Err(format!(
            "{} exists; pass --overwrite to replace this reference",
            output.display()
        ));
    }
    let Some(base) = ppl_engine::checking::configured_firmware() else {
        return Err(
            "original firmware is not configured (PPL_FIRMWARE, PPL_OS_ROM, PPL_FACTORY_A, \
             PPL_FIRMWARE_RUNNER); there is no native fallback for a reference"
                .into(),
        );
    };
    let corpus = diag_parity::load_corpus(corpus_dir)?;
    std::fs::create_dir_all(artifacts_dir).map_err(|e| e.to_string())?;
    if engine == Engine::Session {
        return capture_session(&corpus, corpus_dir, &output, &base, artifacts_dir, reverse);
    }
    if reverse {
        return Err("--reverse applies to --engine session".into());
    }
    let cancel = Cancellation::default();
    let mut firmware_sha256: Option<String> = None;
    let mut results = Vec::new();
    for case in &corpus.cases {
        let source = Source::text(&case.text, &case.spec.id)?;
        if source.identity.source_sha256 != case.source_sha256 {
            return Err(format!("{}: engine and corpus hashes differ", case.spec.id));
        }
        let dir = artifacts_dir.join(&case.spec.id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Case ids are valid program names apart from length (at most 31);
        // each check runs on a fresh drive, so truncation cannot collide.
        let program: String = case.spec.id.chars().take(31).collect();
        let options = ppl_engine::firmware::Options {
            artifacts_dir: Some(dir.clone()),
            program_name: Some(program),
            ..base.clone()
        };
        eprintln!("checking {} …", case.spec.id);
        let r = ppl_engine::firmware::execute(source, Operation::Check, &options, &cancel);
        let report_path = dir.join("report.json");
        std::fs::write(
            &report_path,
            serde_json::to_string_pretty(&r).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let Some(sha) = r.backend["report"]["firmware_sha256"].as_str() {
            match &firmware_sha256 {
                Some(prev) if prev != sha => {
                    return Err(format!(
                        "{}: firmware changed during capture ({prev} → {sha})",
                        case.spec.id
                    ));
                }
                _ => firmware_sha256 = Some(sha.to_owned()),
            }
        }
        let result = reference_result(&case.spec.id, &case.source_sha256, &r, &report_path);
        eprintln!(
            "  {:?} {} {:?}:{:?}",
            result.verdict,
            result.message.as_deref().unwrap_or(""),
            result.line,
            result.column
        );
        results.push(result);
    }
    let file = ReferenceFile {
        schema_version: 1,
        evidence: "local-firmware".into(),
        description: "Original HP Prime G1 firmware 2.4.2 (armfir.elf) Program Editor Check, \
                      run in the local emulator. Not a physical-calculator observation."
            .into(),
        firmware_sha256,
        captured_at: Some(ppl_compat::utc_now()),
        capture_command: Some(format!(
            "ppl diag-parity capture --corpus {} --artifacts-dir {}",
            corpus_dir.display(),
            artifacts_dir.display()
        )),
        results,
    };
    let header = "# Generated by `ppl diag-parity capture` from authentic firmware.\n\
                  # Do not edit by hand: re-capture instead.\n\n";
    std::fs::write(&output, format!("{header}{}", file.to_toml()?)).map_err(|e| e.to_string())?;
    eprintln!("wrote {}", output.display());
    let failed = file
        .results
        .iter()
        .filter(|r| r.verdict == Verdict::InfrastructureError)
        .count();
    Ok(if failed == 0 { 0 } else { 1 })
}

/// Converts a firmware Check report. Only `success` and `invalid_source`
/// are verdicts; every other status is recorded as no verdict.
fn reference_result(id: &str, sha: &str, r: &Report, artifact: &Path) -> ReferenceResult {
    let verdict = match r.status {
        Status::Success => Verdict::Accepted,
        Status::InvalidSource => Verdict::Rejected,
        _ => Verdict::InfrastructureError,
    };
    let status = serde_json::to_value(r.status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let d = r
        .diagnostics
        .iter()
        .find(|d| d.source == "guest-firmware" && verdict == Verdict::Rejected);
    ReferenceResult {
        id: id.into(),
        source_sha256: sha.into(),
        verdict,
        status,
        message: d.and_then(|d| d.message.clone()).or_else(|| {
            (verdict == Verdict::InfrastructureError)
                .then(|| r.error.as_ref().map(|e| e.message.clone()))
                .flatten()
        }),
        line: d.and_then(|d| d.line),
        column: d.and_then(|d| d.column),
        utf16_offset: d
            .and_then(|d| d.raw_position.as_ref())
            .and_then(|p| p["offset_utf16"].as_u64())
            .map(|v| v as u32),
        artifact: Some(artifact.display().to_string()),
    }
}

/// `capture --engine session`: every case through one long-lived firmware
/// session on a new profile, the path the IDE's firmware backend uses.
fn capture_session(
    corpus: &Corpus,
    corpus_dir: &Path,
    output: &Path,
    base: &ppl_engine::firmware::Options,
    artifacts_dir: &Path,
    reverse: bool,
) -> Result<u8, String> {
    let runner = ppl_engine::firmware::discover_runner(base.runner.as_deref())?;
    let firmware = base
        .firmware
        .clone()
        .ok_or("PPL_FIRMWARE is required for a session capture")?;
    let firmware_sha256 =
        diag_parity::sha256_of(&std::fs::read(&firmware).map_err(|e| e.to_string())?);
    // A new profile, never the user's: the same initialized data as the
    // fresh checker when configured, otherwise factory A as the IDE seeds it.
    let profile = artifacts_dir.join("session-profile");
    if profile.exists() {
        return Err(format!(
            "{} exists; use a new --artifacts-dir",
            profile.display()
        ));
    }
    let seed = if let Some(init) = &base.init_data {
        copy_tree(init, &profile)?;
        format!("initialized data {}", init.display())
    } else if let Some(factory) = &base.factory_a {
        copy_tree(factory, &profile.join("A"))?;
        format!("factory A {}", factory.display())
    } else {
        return Err("PPL_FIRMWARE_INIT_DATA or PPL_FACTORY_A is required".into());
    };
    let log =
        std::fs::File::create(artifacts_dir.join("session.log")).map_err(|e| e.to_string())?;
    let mut command = Command::new(&runner);
    command
        .arg("session")
        .arg("--firmware")
        .arg(&firmware)
        .arg("--data-dir")
        .arg(&profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log));
    if let Some(rom) = &base.os_rom {
        command.arg("--os-rom").arg(rom);
    }
    let spawned = Instant::now();
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("no session stdin")?;
    let mut stdout = child.stdout.take().ok_or("no session stdout")?;
    let (tx, events) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        // Frames are drained and discarded; only events matter here.
        while let Ok(packet) = wire::read_packet(&mut stdout) {
            if let Packet::Event(e) = packet {
                if tx.send(e).is_err() {
                    break;
                }
            }
        }
    });
    let mut cases: Vec<_> = corpus.cases.iter().collect();
    if reverse {
        cases.reverse();
    }
    let outcome = (|| -> Result<(Vec<ReferenceResult>, Vec<serde_json::Value>), String> {
        let mut results = Vec::new();
        let mut timings = Vec::new();
        for (i, case) in cases.iter().enumerate() {
            let request = i as u64 + 1;
            eprintln!("checking {} …", case.spec.id);
            let sent = Instant::now();
            wire::write_command(
                &mut stdin,
                &wire::Command::Compile {
                    request,
                    source: case.text.clone(),
                },
            )
            .map_err(|e| format!("session write: {e}"))?;
            let (success, message, check) = loop {
                match events.recv_timeout(Duration::from_secs(240)) {
                    Ok(Event::Hello { version }) if version != wire::VERSION => {
                        return Err(format!("session protocol {version} ≠ {}", wire::VERSION));
                    }
                    Ok(Event::Result {
                        request: r,
                        success,
                        message,
                        check,
                    }) if r == request => break (success, message, check),
                    Ok(Event::Closed { message }) => {
                        return Err(format!("session closed: {message}"));
                    }
                    Ok(Event::Error { message }) => eprintln!("  session error: {message}"),
                    Ok(_) => {}
                    Err(_) => return Err(format!("{}: no session result", case.spec.id)),
                }
            };
            let elapsed = sent.elapsed();
            let dir = artifacts_dir.join(&case.spec.id);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let artifact = dir.join("session-result.json");
            std::fs::write(
                &artifact,
                serde_json::to_string_pretty(&serde_json::json!({
                    "request": request, "success": success, "message": message,
                    "check": check, "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
                }))
                .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let result = session_result(case, check.as_ref(), &message, &artifact);
            eprintln!(
                "  {:?} {} {:?}:{:?} ({:.0} ms)",
                result.verdict,
                result.message.as_deref().unwrap_or(""),
                result.line,
                result.column,
                elapsed.as_secs_f64() * 1000.0
            );
            timings.push(serde_json::json!({
                "id": case.spec.id,
                "order": i,
                "compile_to_result_ms": elapsed.as_secs_f64() * 1000.0,
                "spawn_to_result_ms": spawned.elapsed().as_secs_f64() * 1000.0,
            }));
            results.push(result);
        }
        Ok((results, timings))
    })();
    let _ = wire::write_command(&mut stdin, &wire::Command::Shutdown);
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = reader.join();
    let (mut results, mut timings) = outcome?;
    // The session's own per-request report breaks each Check into its UI
    // stages; keep it with the case and fold its stage durations in.
    let mut stages: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for (i, t) in timings.iter_mut().enumerate() {
        let report = profile.join(format!("ide-reports/request-{}.json", i + 1));
        let Ok(raw) = std::fs::read_to_string(&report) else {
            continue;
        };
        if let Some(id) = t["id"].as_str() {
            let _ = std::fs::write(artifacts_dir.join(id).join("session-report.json"), &raw);
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let mut previous = 0.0;
        let mut breakdown = serde_json::Map::new();
        for step in v["timeline"].as_array().into_iter().flatten() {
            let (Some(name), Some(done)) = (step["stage"].as_str(), step["done_ms"].as_f64())
            else {
                continue;
            };
            breakdown.insert(name.into(), (done - previous).into());
            if i > 0 {
                stages.entry(name.into()).or_default().push(done - previous);
            }
            previous = done;
        }
        if let Some(total) = v["elapsed_ms"].as_f64() {
            breakdown.insert("verdict".into(), (total - previous).into());
        }
        t["stages_ms"] = breakdown.into();
    }
    // Reference files list cases in corpus order whatever the check order.
    results.sort_by_key(|r| corpus.cases.iter().position(|c| c.spec.id == r.id));
    let warm: Vec<f64> = timings
        .iter()
        .skip(1)
        .filter_map(|t| t["compile_to_result_ms"].as_f64())
        .collect();
    let latency = serde_json::json!({
        "engine": "session",
        "order": if reverse { "reverse" } else { "corpus" },
        "profile_seed": seed,
        "cold_spawn_to_first_result_ms": timings.first().and_then(|t| t["spawn_to_result_ms"].as_f64()),
        "warm_compile_to_result_ms": summary(&warm),
        "warm_stage_ms": stages
            .iter()
            .map(|(k, v)| (k.clone(), summary(v)))
            .collect::<serde_json::Map<_, _>>(),
        "cases": timings,
    });
    std::fs::write(
        artifacts_dir.join("latency.json"),
        serde_json::to_string_pretty(&latency).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    eprintln!(
        "latency: cold {:.0} ms; warm {}",
        latency["cold_spawn_to_first_result_ms"]
            .as_f64()
            .unwrap_or(f64::NAN),
        latency["warm_compile_to_result_ms"]
    );
    let file = ReferenceFile {
        schema_version: 1,
        evidence: "local-firmware-session".into(),
        description: format!(
            "Original HP Prime G1 firmware 2.4.2 (armfir.elf) Program Editor Check through one \
             long-lived local firmware session ({} order, profile from {seed}). Not a \
             physical-calculator observation.",
            if reverse { "reverse" } else { "corpus" }
        ),
        firmware_sha256: Some(firmware_sha256),
        captured_at: Some(ppl_compat::utc_now()),
        capture_command: Some(format!(
            "ppl diag-parity capture --engine session{} --corpus {} --artifacts-dir {}",
            if reverse { " --reverse" } else { "" },
            corpus_dir.display(),
            artifacts_dir.display()
        )),
        results,
    };
    let mut out = std::fs::File::create(output).map_err(|e| e.to_string())?;
    write!(
        out,
        "# Generated by `ppl diag-parity capture --engine session` from authentic firmware.\n\
         # Do not edit by hand: re-capture instead.\n\n{}",
        file.to_toml()?
    )
    .map_err(|e| e.to_string())?;
    eprintln!("wrote {}", output.display());
    let failed = file
        .results
        .iter()
        .filter(|r| r.verdict == Verdict::InfrastructureError)
        .count();
    Ok(if failed == 0 { 0 } else { 1 })
}

/// Converts a session result. Only an observed firmware verdict on this
/// exact source counts; anything else is recorded as no verdict.
fn session_result(
    case: &diag_parity::CorpusCase,
    check: Option<&CheckVerdict>,
    message: &str,
    artifact: &Path,
) -> ReferenceResult {
    let verdict = match check {
        Some(c) if c.source_sha256 != case.source_sha256 => None,
        Some(c) => Some(c),
        None => None,
    };
    let d = verdict
        .filter(|c| !c.accepted)
        .and_then(|c| c.diagnostics.iter().find(|d| d.source == "guest-firmware"));
    ReferenceResult {
        id: case.spec.id.clone(),
        source_sha256: case.source_sha256.clone(),
        verdict: match verdict {
            Some(c) if c.accepted => Verdict::Accepted,
            Some(_) => Verdict::Rejected,
            None => Verdict::InfrastructureError,
        },
        status: match verdict {
            Some(c) if c.accepted => "success",
            Some(_) => "invalid_source",
            None => "no_verdict",
        }
        .into(),
        message: d
            .and_then(|d| d.message.clone())
            .or_else(|| verdict.is_none().then(|| message.to_owned())),
        line: d.and_then(|d| d.line),
        column: d.and_then(|d| d.column),
        utf16_offset: d
            .and_then(|d| d.raw_position.as_ref())
            .and_then(|p| p["offset_utf16"].as_u64())
            .map(|v| v as u32),
        artifact: Some(artifact.display().to_string()),
    }
}

fn summary(values: &[f64]) -> serde_json::Value {
    if values.is_empty() {
        return serde_json::Value::Null;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let pick = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    serde_json::json!({
        "count": v.len(),
        "min_ms": v[0],
        "median_ms": pick(0.5),
        "p95_ms": pick(0.95),
        "max_ms": v[v.len() - 1],
    })
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let to = target.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
