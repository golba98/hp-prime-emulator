//! M5-C/D: verify imported programs in the firmware's own Program Catalog
//! and Program Editor.
//!
//! Sessions, each on its own copy of an initialised data directory:
//! - `init`: first boot of the factory `A:` tree through the setup wizard
//!   (creates `C:\DATA`), unless `init_data` is given;
//! - `control`: no imports; opens the Program Catalog (`SHIFT 1`);
//! - `session1`: the programs imported; opens the catalog, then opens each
//!   program with the catalog's Edit soft key and leaves with ESC;
//! - `session2` (persistence): a second boot of `session1`'s data, the
//!   same again.
//!
//! Per program, independent evidence:
//! - `recognized_by_firmware`: at boot the firmware enumerated the file
//!   (`_wfindfirst/_wfindnext C:\DATA\*.hpprgm`), opened it and read it to
//!   the end (`--trace-fs`).
//! - `visible_in_catalog`: the catalog row the program occupies holds
//!   pixels the control catalog does not, and the rows after the last
//!   program match the control.
//! - `opened_in_editor`: the Edit press redraws the screen including the
//!   title bar, and leaving the editor makes the firmware save exactly
//!   this program's file (attributed to that key by the fs trace's step).
//! - `source_verified`: the file the firmware saved from its editor holds
//!   exactly the imported source; for source imports the whole file equals
//!   the import except the export cache the firmware adds and `name64`
//!   padding.
//!
//! Catalog geometry (measured on the 2.4.2 firmware's screen): the first
//! program row spans y 42..62, rows are 21 px apart, the current app's row
//! sits above; leaving the editor moves that program to the top of the
//! list and selects it (most recently used order, not persisted across
//! boots).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::calcverify::copy_tree;
use crate::display::Frame;
use crate::hpprgm;
use crate::import::{self, ImportReport, ImportRequest};
use crate::keys::parse_keys;
use crate::runner::{self, Outcome, RunConfig, RunReport};
use crate::script::ScriptConfig;
use crate::services::UnsupportedPolicy;
use crate::trace::TraceConfig;

pub const CATALOG_KEYS: &str = "SHIFT,1";
pub const EDIT_SOFTKEY: &str = "TAP:26:229";
pub const FIRST_ROW_Y: u32 = 42;
pub const ROW_HEIGHT: u32 = 21;
/// Program rows visible without scrolling below the app row.
pub const MAX_PROGRAMS: usize = 8;
/// Pixels a populated row must differ from the empty control row.
const ROW_MIN_PIXELS: usize = 40;
/// Title bar band (the editor shows `NAME:Main(PPL)` there).
const TITLE_BAND: [u32; 2] = [0, 20];

#[derive(Debug, Clone)]
pub struct ProgramSpec {
    pub path: PathBuf,
    pub name: Option<String>,
    pub as_source: bool,
}

#[derive(Debug, Clone)]
pub struct ProgramVerifyConfig {
    pub firmware_path: PathBuf,
    pub os_rom_path: Option<PathBuf>,
    pub factory_a: PathBuf,
    pub out_dir: PathBuf,
    pub programs: Vec<ProgramSpec>,
    /// An initialised data directory to start from (copied, never
    /// modified); otherwise the `init` session creates one.
    pub init_data: Option<PathBuf>,
    pub setup_keys: String,
    pub settle_ms: u64,
    pub boot_timeout: Duration,
    pub key_timeout: Duration,
    pub max_instructions: u64,
    pub max_seconds: f64,
    pub persistence: bool,
    pub verbosity: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct State {
    pub ok: bool,
    pub evidence: String,
}

impl State {
    fn new(ok: bool, evidence: impl Into<String>) -> State {
        State {
            ok,
            evidence: evidence.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionProgram {
    pub recognized_by_firmware: State,
    pub visible_in_catalog: State,
    pub opened_in_editor: State,
    pub source_verified: State,
    /// Row in the catalog as first shown (enumeration order).
    pub catalog_row: Option<usize>,
    /// Row the program was opened from (most recently used order).
    pub opened_from_row: Option<usize>,
    pub editor_png: Option<String>,
    pub editor_sha256: Option<String>,
    /// Export cache the firmware wrote when it saved the program.
    pub firmware_exports: Vec<(String, u32)>,
    pub saved_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgramResult {
    pub name: String,
    pub imported: State,
    pub import: Option<ImportReport>,
    pub session1: Option<SessionProgram>,
    pub session2: Option<SessionProgram>,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub label: String,
    pub outcome: String,
    pub keys: String,
    pub catalog_png: Option<String>,
    pub catalog_sha256: Option<String>,
    /// `C:\DATA\*.hpprgm` names in the order the firmware enumerated them.
    pub enumerated: Vec<String>,
    pub elapsed_s: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgramVerifyReport {
    pub status: String,
    pub evidence: String,
    pub sessions: Vec<SessionSummary>,
    pub programs: Vec<ProgramResult>,
}

struct Session {
    summary: SessionSummary,
    report: RunReport,
    data_dir: PathBuf,
}

fn run_session(
    cfg: &ProgramVerifyConfig,
    label: &str,
    data_dir: &Path,
    keys: &str,
) -> Result<Session, String> {
    let dir = cfg.out_dir.join(label);
    let started = std::time::Instant::now();
    let run_cfg = RunConfig {
        firmware_path: cfg.firmware_path.clone(),
        os_rom_path: cfg.os_rom_path.clone(),
        data_dir: data_dir.to_path_buf(),
        on_unsupported: UnsupportedPolicy::ZeroReturn,
        max_instructions: cfg.max_instructions,
        max_seconds: cfg.max_seconds,
        trace: TraceConfig {
            verbosity: cfg.verbosity,
            fs_trace: true,
            ..TraceConfig::default()
        },
        script: Some(ScriptConfig {
            keys: parse_keys(keys).map_err(|e| format!("{label}: {e}"))?,
            settle_ms: cfg.settle_ms,
            boot_timeout: cfg.boot_timeout,
            key_timeout: cfg.key_timeout,
            screenshot_dir: Some(dir.join("shots")),
            start_with_keys: false,
            ..ScriptConfig::default()
        }),
        storage_diff: true,
        ui_timers: true,
        ..RunConfig::default()
    };
    let summary = runner::run(&run_cfg).map_err(|e| format!("{label}: {e}"))?;
    let report = summary.report;
    let _ = std::fs::write(
        dir.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap_or_default(),
    );
    let enumerated = report
        .fs_events
        .iter()
        .filter(|e| e["op"] == "wfindnext")
        .filter_map(|e| e["name"].as_str())
        .filter(|n| n.to_ascii_lowercase().ends_with(".hpprgm"))
        .map(str::to_owned)
        .collect();
    let catalog = report
        .display
        .script
        .as_ref()
        .and_then(|s| s.keys.iter().find(|k| k.name == "1"))
        .map(|k| k.after.clone());
    Ok(Session {
        summary: SessionSummary {
            label: label.to_owned(),
            outcome: format!("{:?}", summary.outcome),
            keys: keys.to_owned(),
            catalog_png: catalog.as_ref().and_then(|c| c.png.clone()),
            catalog_sha256: catalog.map(|c| c.sha256),
            enumerated,
            elapsed_s: started.elapsed().as_secs_f64(),
        },
        report,
        data_dir: data_dir.to_path_buf(),
    })
    .and_then(|s| {
        if summary.outcome == Outcome::ScriptComplete {
            Ok(s)
        } else {
            Err(format!(
                "{label}: key script did not complete ({:?}): {}",
                summary.outcome, s.report.evidence
            ))
        }
    })
}

fn load(path: &Option<String>) -> Option<Frame> {
    crate::verify::load_png(Path::new(path.as_ref()?)).ok()
}

fn row_band(row: usize) -> (u32, u32) {
    let y0 = FIRST_ROW_Y + ROW_HEIGHT * row as u32;
    (y0, y0 + ROW_HEIGHT - 1)
}

fn band_diff(a: &Frame, b: &Frame, (y0, y1): (u32, u32)) -> usize {
    let mut n = 0;
    for y in y0..=y1.min(a.height - 1) {
        for x in 0..a.width {
            if a.pixel(x, y) != b.pixel(x, y) {
                n += 1;
            }
        }
    }
    n
}

/// Enumeration order of our VFS (`Vfs::find_first` sorts host names).
fn enumeration_order(names: &[String]) -> Vec<String> {
    let mut files: Vec<String> = names.iter().map(|n| format!("{n}.hpprgm")).collect();
    files.sort();
    files
        .into_iter()
        .map(|f| f.trim_end_matches(".hpprgm").to_owned())
        .collect()
}

/// Catalog keys that open every program once in `order` (catalog model:
/// enumeration order, edited program moves to the top and stays
/// selected). Returns the keys and, per program, (row, edit step, esc step).
fn open_script(initial: &[String], order: &[String]) -> (String, Vec<(usize, usize, usize)>) {
    let mut model = initial.to_vec();
    let mut keys: Vec<String> = CATALOG_KEYS.split(',').map(str::to_owned).collect();
    let mut steps = Vec::new();
    for name in order {
        let row = model.iter().position(|n| n == name).unwrap_or(0);
        keys.extend(std::iter::repeat_n("DOWN".to_owned(), row));
        let edit = keys.len();
        keys.push(EDIT_SOFTKEY.to_owned());
        keys.push("ESC".to_owned());
        steps.push((row, edit, edit + 1));
        let n = model.remove(row);
        model.insert(0, n);
    }
    (keys.join(","), steps)
}

fn same_name(path: &str, name: &str) -> bool {
    let want = format!("c:\\data\\{}.hpprgm", name.to_ascii_lowercase());
    path.to_ascii_lowercase() == want
}

fn events_at(r: &RunReport, step: Option<usize>) -> impl Iterator<Item = &Value> {
    r.fs_events.iter().filter(move |e| match step {
        None => e["step"].is_null(),
        Some(s) => e["step"].as_u64() == Some(s as u64),
    })
}

fn strip_cache(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut c = hpprgm::parse(bytes).ok()?;
    c.objects
        .retain(|o| o.ty != hpprgm::SECTION_EXPORTS && o.ty != hpprgm::SECTION_VARIABLES);
    hpprgm::canonical_padding(&c.to_bytes()).ok()
}

#[allow(clippy::too_many_arguments)]
fn evaluate(
    session: &Session,
    control_catalog: Option<&Frame>,
    name: &str,
    import: &ImportReport,
    imported_bytes: &[u8],
    source: &str,
    position: Option<(usize, usize, usize)>,
    initial_order: &[String],
) -> SessionProgram {
    let program_count = initial_order.len();
    let r = &session.report;
    let keys = r
        .display
        .script
        .as_ref()
        .map(|s| s.keys.clone())
        .unwrap_or_default();

    // recognized_by_firmware: enumeration + full read at boot.
    let listed = session
        .summary
        .enumerated
        .iter()
        .any(|n| n.eq_ignore_ascii_case(&format!("{name}.hpprgm")));
    let open = events_at(r, None)
        .find(|e| e["op"] == "wfopen" && e["path"].as_str().is_some_and(|p| same_name(p, name)));
    let handle = open.and_then(|e| e["handle"].as_u64());
    let read: u64 = events_at(r, None)
        .filter(|e| e["op"] == "fread" && e["handle"].as_u64() == handle && handle.is_some())
        .filter_map(|e| e["read"].as_u64())
        .sum();
    let file_size_at_boot = session_input_size(session, name).unwrap_or(import.size);
    let recognized = State::new(
        listed && handle.is_some() && read == file_size_at_boot,
        format!(
            "enumerated: {listed}; opened at boot: {} (mode {}); read {read} of {file_size_at_boot} bytes",
            handle.is_some(),
            open.and_then(|e| e["mode"].as_str()).unwrap_or("-"),
        ),
    );

    // visible_in_catalog: the catalog screenshot (taken before any edit,
    // in the firmware's enumeration order) vs the empty control catalog,
    // at this program's row. The row model is confirmed by the firmware's
    // enumeration order here and by which file each Edit/ESC saves.
    let catalog = load(&session.summary.catalog_png);
    let enumerated: Vec<String> = session
        .summary
        .enumerated
        .iter()
        .map(|n| n.trim_end_matches(".hpprgm").to_owned())
        .collect();
    let row = initial_order.iter().position(|n| n == name);
    let visible = match (row, catalog.as_ref(), control_catalog) {
        (Some(row), Some(cat), Some(ctrl)) => {
            let here = band_diff(cat, ctrl, row_band(row));
            let after = band_diff(cat, ctrl, row_band(program_count));
            let order_ok = enumerated == initial_order;
            State::new(
                order_ok && here >= ROW_MIN_PIXELS && after == 0,
                format!(
                    "firmware enumeration order {enumerated:?} (expected {initial_order:?}); \
                     row {row} (y {:?}) differs from the control catalog in {here} px; \
                     row {program_count} after the last program differs in {after} px",
                    row_band(row)
                ),
            )
        }
        _ => State::new(false, "catalog or control screenshot missing"),
    };

    // opened_in_editor: Edit redraws incl. title; ESC saves this file only.
    let (opened, editor) = match position {
        Some((row, edit, esc)) => {
            let ek = keys.get(edit);
            let title_changed = ek.is_some_and(|k| {
                k.stable_change
                    .bbox
                    .is_some_and(|b| b[1] <= TITLE_BAND[1] && b[3] > 200)
            });
            let saves: Vec<String> = events_at(r, Some(esc))
                .filter(|e| {
                    e["op"] == "wfopen"
                        && e["mode"].as_str().is_some_and(|m| m.starts_with('w'))
                        && e["path"]
                            .as_str()
                            .is_some_and(|p| p.to_ascii_lowercase().ends_with(".hpprgm"))
                })
                .filter_map(|e| e["path"].as_str().map(str::to_owned))
                .collect();
            let only_this = saves.len() == 1 && same_name(&saves[0], name);
            (
                State::new(
                    title_changed && only_this,
                    format!(
                        "catalog row {row}: Edit (step {edit}) changed {} px, bbox {:?}; \
                         ESC (step {esc}) saved {saves:?}",
                        ek.map_or(0, |k| k.stable_change.changed),
                        ek.and_then(|k| k.stable_change.bbox),
                    ),
                ),
                ek.map(|k| k.after.clone()),
            )
        }
        None => (State::new(false, "program not opened"), None),
    };

    // source_verified: the firmware's own save of the editor's program.
    let saved =
        std::fs::read(import::data_dir_of(&session.data_dir).join(format!("{name}.hpprgm")))
            .ok()
            .or_else(|| {
                import::find_program(&import::data_dir_of(&session.data_dir), name)
                    .and_then(|p| std::fs::read(p).ok())
            });
    let saved_in_session = r.fs_events.iter().any(|e| {
        !e["step"].is_null()
            && e["op"] == "wfopen"
            && e["mode"].as_str().is_some_and(|m| m.starts_with('w'))
            && e["path"].as_str().is_some_and(|p| same_name(p, name))
    });
    let (source_state, exports, saved_sha) = match &saved {
        None => (
            State::new(false, "program file missing after the session"),
            Vec::new(),
            None,
        ),
        Some(bytes) => match hpprgm::parse(bytes).and_then(|c| Ok((c.source()?, c.exports()))) {
            Err(e) => (
                State::new(false, format!("firmware save does not parse: {e}")),
                Vec::new(),
                None,
            ),
            Ok((src, exports)) => {
                let exact = src.source == source;
                // Source imports: the whole file must equal the import
                // except the firmware's export cache and name64 padding.
                let whole_ok = import.input.kind != "ppl-source"
                    || strip_cache(bytes) == strip_cache(imported_bytes);
                let whole = if import.input.kind == "ppl-source" {
                    format!("; whole file equals the import except the export cache: {whole_ok}")
                } else {
                    String::new()
                };
                (
                    State::new(
                        exact && whole_ok && saved_in_session,
                        format!(
                            "saved by the firmware during the session: {saved_in_session}; \
                             firmware-saved source ({} chars) equals the imported source: {exact}{whole}",
                            src.source.chars().count()
                        ),
                    ),
                    exports,
                    Some(crate::sha256::hex(bytes)),
                )
            }
        },
    };

    SessionProgram {
        recognized_by_firmware: recognized,
        visible_in_catalog: visible,
        opened_in_editor: opened,
        source_verified: source_state,
        catalog_row: row,
        opened_from_row: position.map(|p| p.0),
        editor_png: editor.as_ref().and_then(|c| c.png.clone()),
        editor_sha256: editor.map(|c| c.sha256),
        firmware_exports: exports,
        saved_sha256: saved_sha,
    }
}

/// Size of `name`'s file when the session booted (from the storage diff).
fn session_input_size(session: &Session, name: &str) -> Option<u64> {
    let diff = session.report.storage.as_ref()?;
    let rel = format!("C/DATA/{name}.hpprgm");
    diff.modified
        .iter()
        .chain(diff.deleted.iter())
        .find(|c| c.path.eq_ignore_ascii_case(&rel))
        .and_then(|c| c.size_before)
        .or_else(|| {
            std::fs::metadata(import::data_dir_of(&session.data_dir).join(format!("{name}.hpprgm")))
                .ok()
                .map(|m| m.len())
        })
}

pub fn verify(cfg: &ProgramVerifyConfig) -> ProgramVerifyReport {
    let mut report = ProgramVerifyReport {
        status: "failed".into(),
        evidence: String::new(),
        sessions: Vec::new(),
        programs: Vec::new(),
    };
    let fail = |mut r: ProgramVerifyReport, e: String| {
        r.evidence = e;
        r
    };
    if cfg.programs.is_empty() || cfg.programs.len() > MAX_PROGRAMS {
        return fail(
            report,
            format!("1..={MAX_PROGRAMS} programs required (no catalog scrolling)"),
        );
    }
    let _ = std::fs::create_dir_all(&cfg.out_dir);

    // Initialised data directory.
    let init = match &cfg.init_data {
        Some(p) => p.clone(),
        None => {
            let d = cfg.out_dir.join("init/data");
            let _ = std::fs::remove_dir_all(&d);
            if let Err(e) = copy_tree(&cfg.factory_a, &d.join("A")) {
                return fail(report, format!("seed factory A: {e}"));
            }
            match run_session(cfg, "init", &d, &format!("{},IDLE", cfg.setup_keys)) {
                Ok(s) => report.sessions.push(s.summary),
                Err(e) => return fail(report, e),
            }
            d
        }
    };
    if !import::data_dir_of(&init).is_dir() {
        return fail(
            report,
            format!("{} has no C/DATA after initialisation", init.display()),
        );
    }
    let fresh = |label: &str| -> Result<PathBuf, String> {
        let d = cfg.out_dir.join(label).join("data");
        let _ = std::fs::remove_dir_all(&d);
        copy_tree(&init, &d).map_err(|e| format!("{label}: copy data: {e}"))?;
        Ok(d)
    };

    // Control: the catalog with no imported program.
    let control = match fresh("control").and_then(|d| run_session(cfg, "control", &d, CATALOG_KEYS))
    {
        Ok(s) => s,
        Err(e) => return fail(report, e),
    };
    let control_catalog = load(&control.summary.catalog_png);
    report.sessions.push(control.summary.clone());

    // Imports.
    let data1 = match fresh("session1") {
        Ok(d) => d,
        Err(e) => return fail(report, e),
    };
    let mut imported: Vec<(String, ImportReport, Vec<u8>, String)> = Vec::new();
    for spec in &cfg.programs {
        let req = ImportRequest {
            program: spec.path.clone(),
            data_dir: data1.clone(),
            name: spec.name.clone(),
            replace: false,
            as_source: spec.as_source,
        };
        let name = spec
            .name
            .clone()
            .or_else(|| {
                spec.path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        match import::import(&req) {
            Ok(r) => {
                let bytes = std::fs::read(&r.host_path).unwrap_or_default();
                let source = hpprgm::parse(&bytes)
                    .and_then(|c| c.source())
                    .map(|s| s.source)
                    .unwrap_or_default();
                report.programs.push(ProgramResult {
                    name: r.name.clone(),
                    imported: State::new(
                        true,
                        format!("{} ({} bytes, sha256 {})", r.guest_path, r.size, r.sha256),
                    ),
                    import: Some(r.clone()),
                    session1: None,
                    session2: None,
                    passed: false,
                });
                imported.push((r.name.clone(), r, bytes, source));
            }
            Err(e) => report.programs.push(ProgramResult {
                name,
                imported: State::new(false, e.to_string()),
                import: None,
                session1: None,
                session2: None,
                passed: false,
            }),
        }
    }
    if imported.is_empty() {
        return fail(report, "no program imported".into());
    }
    let names: Vec<String> = imported.iter().map(|i| i.0.clone()).collect();
    let initial = enumeration_order(&names);
    let (keys, steps) = open_script(&initial, &names);

    let mut sessions = vec![("session1", data1.clone())];
    if cfg.persistence {
        let d2 = cfg.out_dir.join("session2/data");
        sessions.push(("session2", d2));
    }
    for (label, data) in sessions {
        if label == "session2" {
            let _ = std::fs::remove_dir_all(&data);
            if let Err(e) = copy_tree(&data1, &data) {
                return fail(report, format!("session2: copy data: {e}"));
            }
        }
        let session = match run_session(cfg, label, &data, &keys) {
            Ok(s) => s,
            Err(e) => {
                report.evidence = e;
                break;
            }
        };
        for (i, (name, imp, bytes, source)) in imported.iter().enumerate() {
            let sp = evaluate(
                &session,
                control_catalog.as_ref(),
                name,
                imp,
                bytes,
                source,
                steps.get(i).copied(),
                &initial,
            );
            if let Some(p) = report.programs.iter_mut().find(|p| &p.name == name) {
                if label == "session1" {
                    p.session1 = Some(sp);
                } else {
                    p.session2 = Some(sp);
                }
            }
        }
        report.sessions.push(session.summary);
    }

    let ok = |s: &Option<SessionProgram>| {
        s.as_ref().is_some_and(|s| {
            s.recognized_by_firmware.ok
                && s.visible_in_catalog.ok
                && s.opened_in_editor.ok
                && s.source_verified.ok
        })
    };
    for p in &mut report.programs {
        p.passed = p.imported.ok && ok(&p.session1) && (!cfg.persistence || ok(&p.session2));
    }
    let passed = report.programs.iter().filter(|p| p.passed).count();
    if passed == report.programs.len() && report.evidence.is_empty() {
        report.status = "passed".into();
        report.evidence = format!(
            "{passed} program(s): imported, enumerated and read by the firmware, listed in its \
             Program Catalog, opened in its Program Editor, saved back by the firmware with the \
             exact imported source{}",
            if cfg.persistence {
                ", and again after a second boot"
            } else {
                ""
            }
        );
    } else if report.evidence.is_empty() {
        report.evidence = format!("{passed} of {} program(s) passed", report.programs.len());
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_script_tracks_most_recently_used_order() {
        let initial: Vec<String> = ["HELLO", "SQUARE", "TEST"].map(String::from).to_vec();
        let (keys, steps) = open_script(&initial, &initial);
        assert_eq!(
            keys,
            "SHIFT,1,TAP:26:229,ESC,DOWN,TAP:26:229,ESC,DOWN,DOWN,TAP:26:229,ESC"
        );
        // HELLO row 0; SQUARE row 1; then order SQUARE, HELLO, TEST: TEST row 2.
        assert_eq!(steps, vec![(0, 2, 3), (1, 5, 6), (2, 9, 10)]);
    }

    #[test]
    fn enumeration_order_sorts_file_names() {
        let names: Vec<String> = ["TEST", "AB", "ABC"].map(String::from).to_vec();
        assert_eq!(enumeration_order(&names), vec!["AB", "ABC", "TEST"]);
    }
}
