//! M4-C: differential calculation verification through the genuine firmware.
//!
//! Three independent firmware runs boot to Home through the first-run
//! wizard and evaluate different expressions:
//!
//! - A: `2+2`  → expected result `4`
//! - B: `4`    → expected result `4`
//! - C: `2+3`  → expected result `5`
//!
//! The expected values are assertions about the firmware, never injected
//! into it. Verification compares the rendered *result glyph* — the
//! rightmost changed pixel cluster inside the final ENTER's change band —
//! across runs: A must match B (both display `4`) and differ from C
//! (which displays `5`). The whole screen is not compared: the history
//! expression legitimately differs between runs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::display::Frame;
use crate::keys::parse_keys;
use crate::runner::{self, Outcome, RunConfig};
use crate::script::ScriptConfig;
use crate::services::UnsupportedPolicy;
use crate::trace::TraceConfig;

/// First-run wizard navigation verified against the firmware (M4).
pub const DEFAULT_SETUP_KEYS: &str = "SWIPE:160:120:310:120:12,ENTER,ENTER,TAP:293:229,TAP:293:229";

#[derive(Debug, Clone)]
pub struct CalcVerifyConfig {
    pub firmware_path: PathBuf,
    pub os_rom_path: Option<PathBuf>,
    /// Factory `A:` tree copied into every run's fresh data directory
    /// (the device's flash contents: `armfir.elf`, `armfir.dat`, the
    /// system DLLs). An empty data directory boots into a resource-less
    /// UI — the wizard pages render without text.
    pub factory_a: PathBuf,
    /// Base output directory; `A/`, `B/`, `C/` subdirectories are created.
    pub out_dir: PathBuf,
    pub setup_keys: String,
    pub settle_ms: u64,
    pub boot_timeout: Duration,
    pub key_timeout: Duration,
    pub max_instructions: u64,
    pub max_seconds: f64,
    /// `--lcd-watch` on run A (guest framebuffer-writer evidence).
    pub lcd_watch: bool,
    pub verbosity: u8,
}

#[derive(Debug, Serialize)]
pub struct RunEvidence {
    pub label: &'static str,
    pub expression: &'static str,
    pub expected: &'static str,
    pub keys: String,
    pub outcome: String,
    pub instructions: u64,
    pub elapsed_secs: f64,
    pub final_sha256: String,
    pub enter_change: u32,
    pub enter_change_bbox: Option<[u32; 4]>,
    /// Result-glyph crop bounds in the final frame `[x0,y0,x1,y1]`.
    pub cluster_bbox: Option<[u32; 4]>,
    /// SHA-256 of the cropped result glyph (RGBA rendering).
    pub cluster_sha256: Option<String>,
    pub cluster_png: Option<String>,
    pub fault: Option<String>,
    /// Guest PCs that stored to the framebuffer during ENTER (`--lcd-watch`).
    pub enter_fb_writer_pcs: Vec<(u32, u64)>,
    pub report_path: String,
}

#[derive(Debug, Serialize)]
pub struct DifferentialReport {
    pub firmware: String,
    pub firmware_sha256: Option<String>,
    pub factory_a: String,
    pub setup_keys: String,
    pub runs: Vec<RunEvidence>,
    /// A's result glyph equals B's (both `4`).
    pub a_matches_b: bool,
    /// A's result glyph differs from C's (`4` vs `5`).
    pub a_differs_from_c: bool,
    /// All three runs completed and rendered a result glyph.
    /// A's typed expression (`2+2`) is not B's (`4`): the key codes were
    /// interpreted as different input.
    pub expressions_differ: bool,
    pub all_runs_complete: bool,
    pub passed: bool,
    pub evidence: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CalcVerifyOutcome {
    Passed,
    Failed(String),
    ConfigError(String),
}

/// One run's material for result extraction.
struct RunArtifacts {
    report: runner::RunReport,
    /// Frame captured after the whole script (settled).
    final_frame: Frame,
    /// Frame captured after the second-to-last key (input line full,
    /// ENTER not yet pressed).
    before_enter: Frame,
}

fn seed_data_dir(factory_a: &Path, data_dir: &Path) -> Result<(), String> {
    let dst = data_dir.join("A");
    copy_tree(factory_a, &dst)
        .map_err(|e| format!("seeding A: from {}: {e}", factory_a.display()))?;
    // C: (user settings) deliberately absent: the first-run wizard must run.
    Ok(())
}

pub fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

fn run_once(cfg: &CalcVerifyConfig, label: &str, expression: &str) -> Result<RunArtifacts, String> {
    let dir = cfg.out_dir.join(label);
    let _ = std::fs::create_dir_all(&dir);
    let data_dir = dir.join("data");
    let _ = std::fs::remove_dir_all(&data_dir);
    seed_data_dir(&cfg.factory_a, &data_dir)?;
    let keys_script = format!("{},{}", cfg.setup_keys, expression);
    let keys = parse_keys(&keys_script).map_err(|e| format!("{label}: bad key script: {e}"))?;
    let run_cfg = RunConfig {
        firmware_path: cfg.firmware_path.clone(),
        os_rom_path: cfg.os_rom_path.clone(),
        data_dir: data_dir.clone(),
        firmware_guest_path: "A:\\programs\\misc\\armfir.elf".to_owned(),
        slice_instructions: 200_000,
        max_instructions: cfg.max_instructions,
        max_seconds: cfg.max_seconds,
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        heartbeat_min_events: 3,
        inject_key: None,
        irq_tick_ms: 10,
        watches: Vec::new(),
        trace: TraceConfig {
            verbosity: cfg.verbosity,
            ..TraceConfig::default()
        },
        script: Some(ScriptConfig {
            keys,
            settle_ms: cfg.settle_ms,
            idle_observe_ms: 1500,
            boot_timeout: cfg.boot_timeout,
            key_timeout: cfg.key_timeout,
            screenshot_dir: Some(dir.join("shots")),
        }),
        screenshot: Some(dir.join("result.png")),
        lcd_watch: cfg.lcd_watch,
        storage_diff: false,
        ui_timers: true,
        program: None,
    };
    let summary = runner::run(&run_cfg).map_err(|e| format!("{label}: {e}"))?;
    let report_path = dir.join("report.json");
    let _ = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&summary.report).unwrap(),
    );
    if summary.outcome != Outcome::ScriptComplete {
        return Err(format!(
            "{label} ({expression}): outcome {} — {}",
            summary.outcome.as_str(),
            summary.report.evidence
        ));
    }
    let script = summary
        .report
        .display
        .script
        .as_ref()
        .ok_or(format!("{label}: no script report"))?;
    let captures = &script.captures;
    if captures.len() < 2 {
        return Err(format!("{label}: not enough captures"));
    }
    let before_enter = captures[captures.len() - 2]
        .png
        .as_ref()
        .ok_or(format!("{label}: before-ENTER capture PNG missing"))?;
    let final_png = captures[captures.len() - 1]
        .png
        .as_ref()
        .ok_or(format!("{label}: final capture PNG missing"))?;
    let load =
        |p: &str| crate::verify::load_png(Path::new(p)).map_err(|e| format!("{label}: {p}: {e}"));
    let final_frame = load(final_png)?;
    let before_enter = load(before_enter)?;
    Ok(RunArtifacts {
        report: summary.report,
        final_frame,
        before_enter,
    })
}

pub fn verify(cfg: &CalcVerifyConfig) -> (DifferentialReport, CalcVerifyOutcome) {
    let cases: [(&'static str, &'static str, &'static str); 3] = [
        ("A", "2,+,2,ENTER", "4"),
        ("B", "4,ENTER", "4"),
        ("C", "2,+,3,ENTER", "5"),
    ];
    let mut runs = Vec::new();
    let mut artifacts = Vec::new();
    let mut failed: Option<String> = None;
    let mut fw_sha: Option<String> = None;
    for (label, expr, expected) in cases {
        // lcd_watch on A only: guest writer evidence without slowing B/C.
        let lcd_watch = cfg.lcd_watch && label == "A";
        let mut c = cfg.clone();
        c.lcd_watch = lcd_watch;
        let _ = expected;
        match run_once(&c, label, expr) {
            Ok(a) => {
                fw_sha = fw_sha.or_else(|| a.report.firmware_sha256.clone());
                let enter = a
                    .report
                    .display
                    .script
                    .as_ref()
                    .and_then(|s| s.keys.last())
                    .expect("script-complete implies at least one key");
                let line = crate::verify::history_line(&a.before_enter, &a.final_frame);
                let dir = cfg.out_dir.join(label);
                let (cluster_sha, cluster_png, cluster_bbox) = match &line {
                    Some((_, result)) => {
                        let frame = Frame {
                            width: result.width(),
                            height: result.height(),
                            pixels: result.pixels.clone(),
                        };
                        let png = dir.join("result_glyph.png");
                        let saved = frame.save_png(&png).is_ok();
                        (
                            Some(frame.sha256()),
                            saved.then(|| png.display().to_string()),
                            Some(result.bbox),
                        )
                    }
                    None => (None, None, None),
                };
                runs.push(RunEvidence {
                    label,
                    expression: expr,
                    expected,
                    keys: format!("{},{}", cfg.setup_keys, expr),
                    outcome: a.report.outcome.clone(),
                    instructions: a.report.instructions,
                    elapsed_secs: a.report.elapsed_secs,
                    final_sha256: a
                        .report
                        .display
                        .final_capture
                        .as_ref()
                        .map(|c| c.sha256.clone())
                        .unwrap_or_default(),
                    enter_change: enter.change.changed,
                    enter_change_bbox: enter.change.bbox,
                    cluster_bbox,
                    cluster_sha256: cluster_sha.clone(),
                    cluster_png,
                    fault: a.report.fault.as_ref().map(|f| f.kind.clone()),
                    enter_fb_writer_pcs: enter.fb_writer_pcs.clone(),
                    report_path: dir.join("report.json").display().to_string(),
                });
                artifacts.push((a.before_enter, a.final_frame));
            }
            Err(e) => {
                failed = Some(e.clone());
                runs.push(RunEvidence {
                    label,
                    expression: expr,
                    expected,
                    keys: format!("{},{}", cfg.setup_keys, expr),
                    outcome: "failed".into(),
                    instructions: 0,
                    elapsed_secs: 0.0,
                    final_sha256: String::new(),
                    enter_change: 0,
                    enter_change_bbox: None,
                    cluster_bbox: None,
                    cluster_sha256: None,
                    cluster_png: None,
                    fault: Some(e),
                    enter_fb_writer_pcs: Vec::new(),
                    report_path: String::new(),
                });
            }
        }
    }
    let all_complete = artifacts.len() == 3;
    // A missing cluster in any run is a failed comparison, never a pass.
    let verdict = all_complete.then(|| {
        crate::verify::verify_calculation(
            (&artifacts[0].0, &artifacts[0].1),
            (&artifacts[1].0, &artifacts[1].1),
            (&artifacts[2].0, &artifacts[2].1),
        )
    });
    let a_matches_b = verdict.as_ref().is_some_and(|v| v.result_matches_reference);
    let a_differs_from_c = verdict
        .as_ref()
        .is_some_and(|v| v.result_differs_from_control);
    let expressions_differ = verdict.as_ref().is_some_and(|v| v.expressions_differ);
    let passed = all_complete && verdict.as_ref().is_some_and(|v| v.verified) && failed.is_none();
    let evidence = if passed {
        "A(2+2) result glyph == B(4) result glyph and != C(2+3) result glyph: \
         the firmware evaluated and rendered 2+2 -> 4"
            .to_string()
    } else {
        failed.unwrap_or_else(|| {
            format!(
                "differential mismatch: a_matches_b={a_matches_b} a_differs_from_c={a_differs_from_c} \
                 expressions_differ={expressions_differ}"
            )
        })
    };
    let report = DifferentialReport {
        firmware: cfg.firmware_path.display().to_string(),
        firmware_sha256: fw_sha,
        factory_a: cfg.factory_a.display().to_string(),
        setup_keys: cfg.setup_keys.clone(),
        runs,
        a_matches_b,
        a_differs_from_c,
        expressions_differ,
        all_runs_complete: all_complete,
        passed,
        evidence: evidence.clone(),
    };
    let outcome = if passed {
        CalcVerifyOutcome::Passed
    } else {
        CalcVerifyOutcome::Failed(evidence.clone())
    };
    (report, outcome)
}
