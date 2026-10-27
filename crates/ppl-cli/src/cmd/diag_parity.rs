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
        /// `direct`: one session, calling the firmware compiler directly.
        #[arg(long, value_enum, default_value_t = Engine::Fresh)]
        engine: Engine,
        #[command(flatten)]
        order: Order,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Engine {
    Fresh,
    Session,
    Direct,
}

/// Check order within one session (`session` and `direct` engines).
#[derive(Clone, Copy, Debug, Default, Args)]
pub struct Order {
    /// Check the cases in reverse order.
    #[arg(long)]
    reverse: bool,
    /// Check the cases in a pseudo-random order from this seed (a new
    /// order for each repeat).
    #[arg(long, conflicts_with = "reverse")]
    shuffle: Option<u64>,
    /// Check every case this many times in the same session; each
    /// result must equal the first.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=20))]
    repeat: u32,
}

impl Order {
    fn label(&self) -> String {
        let order = match (self.reverse, self.shuffle) {
            (true, _) => "reverse".to_string(),
            (_, Some(seed)) => format!("shuffle:{seed}"),
            _ => "corpus".to_string(),
        };
        if self.repeat > 1 {
            format!("{order} x{}", self.repeat)
        } else {
            order
        }
    }
    fn flags(&self) -> String {
        let mut f = String::new();
        if self.reverse {
            f.push_str(" --reverse");
        }
        if let Some(seed) = self.shuffle {
            f.push_str(&format!(" --shuffle {seed}"));
        }
        if self.repeat > 1 {
            f.push_str(&format!(" --repeat {}", self.repeat));
        }
        f
    }
    /// Corpus indices in check order, all repeats concatenated.
    fn sequence(&self, n: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for r in 0..self.repeat {
            let mut round: Vec<usize> = (0..n).collect();
            if self.reverse {
                round.reverse();
            }
            if let Some(seed) = self.shuffle {
                // xorshift64, Fisher-Yates: reproducible from the seed.
                let mut x = seed
                    .wrapping_add(u64::from(r).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                    .max(1);
                for i in (1..round.len()).rev() {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    round.swap(i, (x % (i as u64 + 1)) as usize);
                }
            }
            out.extend(round);
        }
        out
    }
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
            order,
        } => capture(&corpus, output, &artifacts_dir, overwrite, engine, order),
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
    order: Order,
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
    if engine != Engine::Fresh {
        return capture_session(
            &corpus,
            corpus_dir,
            &output,
            &base,
            artifacts_dir,
            engine,
            order,
        );
    }
    if order.reverse || order.shuffle.is_some() || order.repeat > 1 {
        return Err("--reverse, --shuffle and --repeat apply to the session engines".into());
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
    engine: Engine,
    order: Order,
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
    let cases: Vec<_> = order
        .sequence(corpus.cases.len())
        .into_iter()
        .map(|i| &corpus.cases[i])
        .collect();
    let outcome = (|| -> Result<(Vec<ReferenceResult>, Vec<serde_json::Value>), String> {
        let mut results = Vec::new();
        let mut timings = Vec::new();
        for (i, case) in cases.iter().enumerate() {
            let request = i as u64 + 1;
            eprintln!("checking {} …", case.spec.id);
            let sent = Instant::now();
            let source = case.text.clone();
            let command = if engine == Engine::Direct {
                wire::Command::DirectCheck { request, source }
            } else {
                wire::Command::Compile { request, source }
            };
            wire::write_command(&mut stdin, &command).map_err(|e| format!("session write: {e}"))?;
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
            let artifact = dir.join(format!("session-result-{request}.json"));
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
    let (all_results, mut timings) = outcome?;
    // Repeated checks of a case must reproduce its first result exactly.
    let mut results: Vec<ReferenceResult> = Vec::new();
    let mut repeat_differences = Vec::new();
    for r in all_results {
        match results.iter().find(|x| x.id == r.id) {
            None => results.push(r),
            Some(first) => {
                let one = |x: &ReferenceResult| ReferenceFile {
                    schema_version: 1,
                    evidence: String::new(),
                    description: String::new(),
                    firmware_sha256: None,
                    captured_at: None,
                    capture_command: None,
                    results: vec![x.clone()],
                };
                repeat_differences.extend(
                    diag_parity::reference_differences(&one(first), &one(&r))
                        .into_iter()
                        .map(|d| format!("repeat: {d}")),
                );
            }
        }
    }
    for d in &repeat_differences {
        eprintln!("{d}");
    }
    // The session's own per-request report breaks each Check into its UI
    // stages; keep it with the case and fold its stage durations in.
    let mut stages: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for (i, t) in timings.iter_mut().enumerate() {
        let report = profile.join(format!("ide-reports/request-{}.json", i + 1));
        let Ok(raw) = std::fs::read_to_string(&report) else {
            continue;
        };
        if let Some(id) = t["id"].as_str() {
            let name = format!("session-report-{}.json", i + 1);
            let _ = std::fs::write(artifacts_dir.join(id).join(name), &raw);
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
            // A direct check's individual firmware calls (host-measured).
            for call in step["outcome"]["calls"].as_array().into_iter().flatten() {
                if let (Some(n), Some(ms)) = (call[0].as_str(), call[1].as_f64()) {
                    let key = format!("call:{n}");
                    breakdown.insert(key.clone(), ms.into());
                    if i > 0 {
                        stages.entry(key).or_default().push(ms);
                    }
                }
            }
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
        "engine": if engine == Engine::Direct { "direct" } else { "session" },
        "order": order.label(),
        "checks": timings.len(),
        "repeat_differences": repeat_differences,
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
    let (engine_name, evidence, how) = if engine == Engine::Direct {
        (
            "direct",
            "local-firmware-direct",
            "compiler called directly (Programs():= set-and-compile sequence, no UI)",
        )
    } else {
        ("session", "local-firmware-session", "Program Editor Check")
    };
    let file = ReferenceFile {
        schema_version: 1,
        evidence: evidence.into(),
        description: format!(
            "Original HP Prime G1 firmware 2.4.2 (armfir.elf), {how}, through one long-lived \
             local firmware session ({} order, profile from {seed}). Not a \
             physical-calculator observation.",
            order.label()
        ),
        firmware_sha256: Some(firmware_sha256),
        captured_at: Some(ppl_compat::utc_now()),
        capture_command: Some(format!(
            "ppl diag-parity capture --engine {engine_name}{} --corpus {} --artifacts-dir {}",
            order.flags(),
            corpus_dir.display(),
            artifacts_dir.display()
        )),
        results,
    };
    let mut out = std::fs::File::create(output).map_err(|e| e.to_string())?;
    write!(
        out,
        "# Generated by `ppl diag-parity capture --engine {engine_name}` from authentic firmware.\n\
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
    Ok(if failed == 0 && repeat_differences.is_empty() {
        0
    } else {
        1
    })
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
