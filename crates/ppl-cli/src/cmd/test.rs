//! `ppl test …`: corpus generation, campaign management and real-world
//! import (see the subcommands).
//!
//! The campaign walks one corpus suite group by group through the guarded
//! hardware stages. Because `EC` remote key injection is **not effective**
//! on G1 firmware 15157 (see `docs/reverse-engineering/usb-protocol.md`
//! §3.7), running a program on the calculator stays a manual step; the
//! orchestrator automates everything else and is fully resumable.

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use ppl_compat::{
    CampaignState, Channel, DEFAULT_CAMPAIGNS_DIR, DEFAULT_EVIDENCE_DIR, DEFAULT_RUNS_DIR, Outcome,
    RunManifest, RunStatus, Stage, compare_v2, generate_corpus, ledger_append, ledger_contains,
    load_manifest, load_suite, merge_evidence, run_emulator, save_evidence_v2, save_manifest,
    select_cases, serial_fingerprint, sha256_hex, utc_now,
};
use ppl_device::{
    ControlObject, DEFAULT_CONTROL, FileType, Prime, Probe, ReadBack, result_note_name,
};

use super::common::exit;

#[derive(Args, Debug)]
pub struct TestOpts {
    #[command(subcommand)]
    pub action: TestAction,
}

#[derive(Subcommand, Debug)]
pub enum TestAction {
    /// Generate a deterministic PPL compatibility corpus.
    Generate {
        /// Number of cases to generate.
        #[arg(long, default_value_t = 1000)]
        count: usize,
        /// Deterministic seed (same seed → same corpus).
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Restrict to these categories (repeatable); default: all.
        #[arg(long = "category", value_name = "CAT")]
        categories: Vec<String>,
        /// Output TOML path (default: tests/compat/generated/gen_<seed>.toml).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Create or inspect a resumable campaign over a corpus.
    Campaign {
        #[command(subcommand)]
        action: CampaignAction,
    },
    /// Import and classify a real-world archive (hpcalc.org etc.).
    Import {
        /// Archive or program file to import (.hpprgm, .ppl, .zip …).
        archive: PathBuf,
        /// Original download URL (provenance, recorded in the manifest).
        #[arg(long)]
        url: Option<String>,
        /// Author, if known.
        #[arg(long)]
        author: Option<String>,
        /// Import directory (default: artifacts/real_world).
        #[arg(long, default_value = "artifacts/real_world")]
        out_dir: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum CampaignAction {
    /// Plan a campaign over a corpus (no USB traffic).
    Create {
        /// Corpus suite (TOML).
        corpus: PathBuf,
        /// Campaign id (letters/digits/dash).
        id: String,
        /// Maximum program uploads to the calculator in this campaign.
        #[arg(long, default_value_t = 20)]
        max_uploads: usize,
        /// Campaign ledger directory.
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
        /// Run-manifest and ledger directory.
        #[arg(long, default_value = DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
    },
    /// Show campaign progress.
    Status {
        id: String,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
    },
    /// Offline pass: run every case in the emulator and record results.
    Offline {
        id: String,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
    },
    /// Prepare pending groups: generate device programs + manifests (no USB).
    Prepare {
        id: String,
        /// Prepare only this group.
        #[arg(long)]
        group: Option<String>,
        /// Upper bound on groups prepared in this call.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
    },
    /// Guarded upload of the first prepared group (asks for confirmation).
    Upload {
        id: String,
        /// Upload only this group.
        #[arg(long)]
        group: Option<String>,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        /// Positive control object as `type:name`.
        #[arg(long, default_value = DEFAULT_CONTROL)]
        control: String,
    },
    /// Fetch the result note of the first uploaded group and compare (USB,
    /// read-only F8). The user must have run the program on the calculator.
    Capture {
        id: String,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_EVIDENCE_DIR)]
        evidence_dir: PathBuf,
    },
    /// Move a group back to pending (after an aborted or blocked attempt).
    Reset {
        id: String,
        /// Group to reset.
        #[arg(long)]
        group: String,
        /// Why (recorded in the ledger).
        #[arg(long)]
        reason: String,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
    },
    /// Aggregate report of a campaign.
    Report {
        id: String,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
    },
    /// List mismatching cases of a campaign.
    Failures {
        id: String,
        #[arg(long, default_value = DEFAULT_CAMPAIGNS_DIR)]
        campaigns_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_EVIDENCE_DIR)]
        evidence_dir: PathBuf,
    },
}

pub fn run(o: TestOpts) -> u8 {
    match o.action {
        TestAction::Generate { .. } => generate(o),
        TestAction::Campaign { action } => campaign(action),
        TestAction::Import { .. } => import(o),
    }
}

// ---------------------------------------------------------------- generate

fn generate(o: TestOpts) -> u8 {
    let TestAction::Generate {
        count,
        seed,
        categories,
        out,
    } = o.action
    else {
        unreachable!()
    };
    let corpus = match generate_corpus(count, seed, &categories) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::USAGE;
        }
    };
    let out =
        out.unwrap_or_else(|| PathBuf::from(format!("tests/compat/generated/gen_{seed:08x}.toml")));
    let toml = corpus.toml();
    if let Some(dir) = out.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("error: cannot create {}: {e}", dir.display());
            return exit::IO;
        }
    }
    if out.exists() {
        eprintln!(
            "error: {} already exists; generation never overwrites (pass --out)",
            out.display()
        );
        return exit::USAGE;
    }
    if let Err(e) = std::fs::write(&out, &toml) {
        eprintln!("error: {}: {e}", out.display());
        return exit::IO;
    }
    println!(
        "Generated {} cases (seed {seed}, {} rejected)",
        corpus.count, corpus.rejected
    );
    for (cat, n) in &corpus.per_category {
        println!("  {cat:<14} {n}");
    }
    println!(
        "groups (device programs): {}  |  cases/group <= 12",
        corpus.suite.groups.len()
    );
    println!("suite:   {}", corpus.suite.suite);
    println!("sha256:  {}", corpus.sha256);
    println!("written: {}", out.display());
    println!(
        "next:    ppl test campaign create --corpus {} --id <ID>",
        out.display()
    );
    exit::OK
}

// ---------------------------------------------------------------- campaign

fn load_campaign_state(dir: &Path, id: &str) -> Result<CampaignState, u8> {
    CampaignState::load(dir, id).map_err(|e| {
        eprintln!("error: {e}");
        exit::IO
    })
}

fn campaign(action: CampaignAction) -> u8 {
    match action {
        CampaignAction::Create {
            corpus,
            id,
            max_uploads,
            campaigns_dir,
            runs_dir,
        } => {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                eprintln!("error: campaign id must be letters/digits/dashes");
                return exit::USAGE;
            }
            let path = campaigns_dir.join(format!("{id}.json"));
            if path.exists() {
                eprintln!("error: campaign {id} already exists ({})", path.display());
                return exit::USAGE;
            }
            let suite = match load_suite(&corpus) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            let sha = sha256_hex(&std::fs::read(&corpus).unwrap_or_default());
            // sanity: every group must be able to become one device program
            for g in &suite.groups {
                let cases = match select_cases(&suite, Some(&g.id), None) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error: group {}: {e}", g.id);
                        return exit::USAGE;
                    }
                };
                let name = "ZZT_CHECK";
                if let Err(errs) = ppl_compat::generate_device_program(
                    &suite,
                    &cases,
                    name,
                    &Channel::Note {
                        note: result_note_name(name),
                    },
                ) {
                    eprintln!(
                        "error: group {} is not a valid device program: {}",
                        g.id,
                        errs.join("; ")
                    );
                    return exit::USAGE;
                }
            }
            let _ = &runs_dir;
            let c = CampaignState::new(&id, &suite, &corpus, &sha, max_uploads);
            if let Err(e) = c.save(&campaigns_dir) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!(
                "Campaign {id} created over {} ({} groups, {} cases)",
                corpus.display(),
                c.groups.len(),
                c.groups.iter().map(|g| g.case_ids.len()).sum::<usize>()
            );
            println!("Upload budget: {max_uploads} programs (never overwritten, ZZT_ names only)");
            println!("next: ppl test campaign offline {id}");
            println!("      ppl test campaign prepare {id}");
        }
        CampaignAction::Status { id, campaigns_dir } => {
            let c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            println!("{}", c.summary());
            for g in &c.groups {
                println!(
                    "  {:<18} {:<9} {} cases {}{}",
                    g.group,
                    g.stage.as_str(),
                    g.case_ids.len(),
                    g.program
                        .as_deref()
                        .map(|p| format!(" [{p}]"))
                        .unwrap_or_default(),
                    g.note
                        .as_deref()
                        .map(|n| format!(" ({n})"))
                        .unwrap_or_default(),
                );
            }
        }
        CampaignAction::Offline { id, campaigns_dir } => {
            let mut c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let suite = match load_suite(Path::new(&c.corpus_path)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            println!("Running {} cases in the emulator...", suite.cases.len());
            let emu = run_emulator(&suite);
            for (cid, outcome) in &emu {
                c.record_offline(cid, outcome.summary());
            }
            let errors = emu
                .values()
                .filter(|o| matches!(o, Outcome::Error { .. }))
                .count();
            let compiles = emu
                .values()
                .filter(|o| !matches!(o, Outcome::CompileError { .. }))
                .count();
            if let Err(e) = c.save(&campaigns_dir) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!(
                "Offline pass complete: {compiles}/{} cases compile and run, {errors} runtime errors (legitimate differential results)",
                emu.len()
            );
            println!("Campaign ledger saved.");
        }
        CampaignAction::Prepare {
            id,
            group,
            limit,
            campaigns_dir,
            runs_dir,
        } => {
            let mut c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let suite = match load_suite(Path::new(&c.corpus_path)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            let pending: Vec<String> = c
                .groups
                .iter()
                .filter(|g| g.stage == Stage::Pending)
                .map(|g| g.group.clone())
                .collect();
            let todo: Vec<String> = match &group {
                Some(g) => {
                    if !pending.contains(g) {
                        eprintln!("error: group {g} is not pending in campaign {id}");
                        return exit::USAGE;
                    }
                    vec![g.clone()]
                }
                None => pending.into_iter().take(limit).collect(),
            };
            let mut prepared = 0usize;
            for gid in &todo {
                let cases = match select_cases(&suite, Some(gid), None) {
                    Ok(x) => x,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return exit::USAGE;
                    }
                };
                let name = match fresh_campaign_name(&runs_dir) {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return exit::IO;
                    }
                };
                let channel = Channel::Note {
                    note: result_note_name(&name),
                };
                let g2 = match ppl_compat::generate_device_program(&suite, &cases, &name, &channel)
                {
                    Ok(g) => g,
                    Err(errs) => {
                        c.advance(gid, Stage::Blocked, Some(errs.join("; ")));
                        eprintln!("blocked: group {gid}: {}", errs.join("; "));
                        continue;
                    }
                };
                let m = RunManifest::prepared(&suite.suite, Some(gid), &g2, &utc_now());
                if let Err(e) = save_manifest(&runs_dir, &m) {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
                c.group_mut(gid).unwrap().program = Some(name.clone());
                c.advance(gid, Stage::Prepared, None);
                println!("prepared {gid} -> {name} ({} cases)", g2.short_ids.len());
                prepared += 1;
            }
            if let Err(e) = c.save(&campaigns_dir) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!("{prepared} group(s) prepared. Nothing was sent to the calculator.");
            if prepared > 0 {
                println!("next: ppl test campaign upload {id}");
            }
        }
        CampaignAction::Upload {
            id,
            group,
            campaigns_dir,
            runs_dir,
            control,
        } => {
            let mut c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            if c.uploads_left() == 0 {
                eprintln!(
                    "error: upload budget exhausted ({}/{}); raise --max-uploads in a new campaign",
                    c.uploads_used(),
                    c.max_uploads
                );
                return exit::USAGE;
            }
            let g = match &group {
                Some(g) => c.group(g).cloned().filter(|g| g.stage == Stage::Prepared),
                None => c.first_at(Stage::Prepared).cloned(),
            };
            let Some(g) = g else {
                eprintln!("error: no prepared group in campaign {id}; run `prepare` first");
                return exit::USAGE;
            };
            let program = g.program.clone().unwrap_or_default();
            let m = match load_manifest(&runs_dir, &program) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            // budget check again (single group per invocation)
            let control = match ControlObject::parse(&control) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            let note = m.note.clone();
            println!(
                "Uploading group {} as `{program}` (note `{}`).",
                g.group,
                note.clone().unwrap_or_default()
            );
            println!(
                "On the calculator, check in the catalogs that `{program}` (and the note) are NOT listed."
            );
            print!("Type the program name to confirm it is absent: ");
            let _ = io::stdout().flush();
            let mut line = String::new();
            if io::stdin().lock().read_line(&mut line).is_err() || line.trim() != program {
                eprintln!("refused: confirmation did not match; nothing was sent");
                return exit::USAGE;
            }
            let mut p = match Prime::open_first() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            let mut m = m;
            let targets = [
                (FileType::Program, program.clone()),
                (
                    FileType::Note,
                    note.clone().unwrap_or_else(|| result_note_name(&program)),
                ),
            ];
            let refs: Vec<(FileType, &str)> =
                targets.iter().map(|(t, n)| (*t, n.as_str())).collect();
            let guard = match p.preflight_new_names(&refs, Some(&control)) {
                Ok(g) => g,
                Err(e) => {
                    let msg = e.to_string();
                    c.advance(&g.group, Stage::Blocked, Some(msg.clone()));
                    let _ = c.save(&campaigns_dir);
                    eprintln!("error: preflight refused: {msg}");
                    return exit::IO;
                }
            };
            for l in guard.log() {
                println!("  {l}");
            }
            let names: Vec<String> = targets.iter().map(|(_, n)| n.clone()).collect();
            if let Err(e) = ledger_append(&runs_dir, &names, &program, &utc_now()) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            let report = match p.send_new_program(guard, &m.source) {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("upload failed (state on calculator unknown): {e}");
                    c.advance(&g.group, Stage::Blocked, Some(msg.clone()));
                    let _ = c.save(&campaigns_dir);
                    eprintln!("error: {msg}");
                    return exit::IO;
                }
            };
            if !matches!(report.readback, ReadBack::Matches) {
                let msg = format!("read-back did not confirm: {:?}", report.readback);
                c.advance(&g.group, Stage::Blocked, Some(msg.clone()));
                let _ = c.save(&campaigns_dir);
                eprintln!("error: {msg} (do not resend; inspect the catalog)");
                return exit::IO;
            }
            m.status = RunStatus::Uploaded;
            m.uploaded_at_utc = Some(utc_now());
            if let Err(e) = save_manifest(&runs_dir, &m) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            c.record_upload();
            c.advance(&g.group, Stage::Uploaded, None);
            if let Err(e) = c.save(&campaigns_dir) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!(
                "Uploaded `{program}`; read-back matches. Upload budget used: {}/{}",
                c.uploads_used(),
                c.max_uploads
            );
            println!("\n=== MANUAL STEP ===");
            println!("1. On the calculator open the Program catalog and select `{program}`.");
            println!(
                "2. Run it. It writes its results into note `{}`.",
                note.clone().unwrap_or_default()
            );
            println!("3. When it prints `{program} done`, run: ppl test campaign capture {id}");
        }
        CampaignAction::Capture {
            id,
            group,
            campaigns_dir,
            runs_dir,
            evidence_dir,
        } => {
            let mut c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let g = match &group {
                Some(g) => c.group(g).cloned().filter(|g| g.stage == Stage::Uploaded),
                None => c.first_at(Stage::Uploaded).cloned(),
            };
            let Some(g) = g else {
                eprintln!("error: no uploaded-and-pending group; run `upload` first");
                return exit::USAGE;
            };
            let program = g.program.clone().unwrap_or_default();
            let m = match load_manifest(&runs_dir, &program) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            let note = m.note.clone().unwrap_or_else(|| result_note_name(&program));
            let mut p = match Prime::open_first() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            let file = match p.probe_file(FileType::Note, &note, std::time::Duration::from_secs(8))
            {
                Ok(Probe::Present(f)) => f,
                Ok(Probe::NoReply { waited }) => {
                    eprintln!(
                        "note `{note}` did not answer within {:.0}s (this does not prove absence); did the program finish?",
                        waited.as_secs_f64()
                    );
                    return exit::IO;
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            let text = file.text();
            let dir = evidence_dir.join(&m.suite);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("error: {}: {e}", dir.display());
                return exit::IO;
            }
            let raw_path = dir.join(format!("{program}.note.raw"));
            if raw_path.exists() {
                eprintln!(
                    "error: {} exists; evidence is never overwritten",
                    raw_path.display()
                );
                return exit::IO;
            }
            if let Err(e) = std::fs::write(&raw_path, &file.data) {
                eprintln!("error: {}: {e}", raw_path.display());
                return exit::IO;
            }
            let device = ppl_compat::DeviceRecord {
                model: p.detected.model.to_string(),
                usb_pid: format!("{:04x}:{:04x}", p.detected.vendor_id, p.detected.product_id),
                firmware_build: p.info().ok().and_then(|i| i.build),
                boot_version: p.info().ok().and_then(|i| i.version),
                serial_id: p
                    .info()
                    .ok()
                    .and_then(|i| i.serial)
                    .map(|s| serial_fingerprint(&s)),
            };
            let suite2 = match load_suite(Path::new(&c.corpus_path)) {
                Ok(s2) => s2,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            // Device-free pipeline: parse → evidence → emulator comparison
            // → per-case classification → group tally.
            let cap = match ppl_compat::capture_note_text(&m, &text, &suite2, device, &utc_now()) {
                Ok(cap) => cap,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            let mut ev = cap.evidence;
            ev.raw_artifact = Some(raw_path.display().to_string());
            ev.raw_artifact_sha256 = Some(sha256_hex(&file.data));
            let o = cap.tally;
            let path = match save_evidence_v2(&evidence_dir, &ev) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            c.advance(
                &g.group,
                Stage::Done,
                Some(format!(
                    "{} match, {} display-only, {} mismatch, {} unresolved (evidence {})",
                    o.matches,
                    o.display_equal,
                    o.mismatches,
                    o.missing,
                    path.display()
                )),
            );
            if let Err(e) = c.save(&campaigns_dir) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!("Captured group {} (`{program}`):", g.group);
            println!("  matches:        {}", o.matches);
            println!("  display-only:   {}", o.display_equal);
            println!("  mismatches:     {}", o.mismatches);
            println!("  unresolved:     {}", o.missing);
            println!("  evidence:   {}", path.display());
            for cid in &o.mismatch_ids {
                let dev = ev.results.get(cid).map(|x| x.raw.as_str()).unwrap_or("");
                let em = cap
                    .emulator
                    .get(cid)
                    .map(|x| x.summary())
                    .unwrap_or_default();
                println!("  MISMATCH {cid}: emulator [{em}] vs G1 [{dev}]");
            }
            // Evidence is stored locally now: the program and its result
            // note are cleanup-eligible, scoped to this campaign.
            println!(
                "\nnext: `ppl device cleanup dry-run --campaign {id}` — evidence is stored, \
                 the G1 copies can be deleted (`cleanup execute --campaign {id}`, \
                 then run the helper and `cleanup verify`)"
            );
        }
        CampaignAction::Reset {
            id,
            group,
            reason,
            campaigns_dir,
        } => {
            let mut c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let g = match c.group(&group) {
                Some(g) => g,
                None => {
                    eprintln!("error: campaign {id} has no group {group}");
                    return exit::USAGE;
                }
            };
            if matches!(g.stage, Stage::Done | Stage::Captured) {
                eprintln!("error: group {group} already captured evidence; reset refused");
                return exit::USAGE;
            }
            c.advance(&group, Stage::Pending, Some(format!("reset: {reason}")));
            if let Err(e) = c.save(&campaigns_dir) {
                eprintln!("error: {e}");
                return exit::IO;
            }
            println!(
                "group {group} reset to pending ({reason}); run `prepare` to generate a fresh program"
            );
        }
        CampaignAction::Report { id, campaigns_dir } => {
            let c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            println!("{}", c.summary());
            let done: Vec<&ppl_compat::GroupState> =
                c.groups.iter().filter(|g| g.stage == Stage::Done).collect();
            let cases_done: usize = done.iter().map(|g| g.case_ids.len()).sum();
            println!(
                "hardware-complete groups: {}/{} ({} cases)",
                done.len(),
                c.groups.len(),
                cases_done
            );
            let offline_values = c.offline.values().filter(|v| !v.contains("error")).count();
            println!(
                "offline: {} cases ran, {} without runtime errors",
                c.offline.len(),
                offline_values
            );
            println!("note: pass rate is per-corpus, not overall HP Prime compatibility");
        }
        CampaignAction::Failures {
            id,
            campaigns_dir,
            evidence_dir,
        } => {
            let c = match load_campaign_state(&campaigns_dir, &id) {
                Ok(c) => c,
                Err(code) => return code,
            };
            let suite = match load_suite(Path::new(&c.corpus_path)) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::USAGE;
                }
            };
            let emu = run_emulator(&suite);
            let evs = match ppl_compat::load_suite_evidence(&evidence_dir, &suite.suite) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            };
            let merged = merge_evidence(&evs);
            let report = compare_v2(&suite, &emu, &merged);
            let mut shown = 0;
            for case in &report.cases {
                if matches!(
                    case.status,
                    ppl_compat::Status::Failed | ppl_compat::Status::Partial
                ) {
                    shown += 1;
                    println!(
                        "{}: case `{}` | emulator: {} | G1: {}",
                        case.status.as_str(),
                        case.id,
                        case.emulator.summary(),
                        case.device
                            .as_ref()
                            .map(|d| d.summary())
                            .unwrap_or_else(|| "-".into())
                    );
                }
            }
            if shown == 0 {
                println!("no mismatches among hardware-verified cases of {id}");
            }
        }
    }
    exit::OK
}

/// A `ZZT_xxxxxx` name unused by the ledger and the manifests.
/// A readable, ordered campaign name (`ZZT_T01`, `ZZT_T02`, …): the program
/// to test is always the `ZZT_T…` entry with the highest number, which
/// keeps the catalog unambiguous for the human running it (the previous
/// one is safe to delete once its results are captured).
fn fresh_campaign_name(runs_dir: &Path) -> Result<String, String> {
    for n in 1..=99u32 {
        let name = format!("ZZT_T{n:02}");
        let note = result_note_name(&name);
        let used = ledger_contains(runs_dir, &name)?
            || ledger_contains(runs_dir, &note)?
            || runs_dir.join(format!("{name}.json")).exists();
        if !used {
            return Ok(name);
        }
    }
    Err("no unused ZZT_Tnn name found".into())
}

// ------------------------------------------------------------------ import

fn import(o: TestOpts) -> u8 {
    let TestAction::Import {
        archive,
        url,
        author,
        out_dir,
    } = o.action
    else {
        unreachable!()
    };
    match ppl_compat::import_archive(&archive, url.as_deref(), author.as_deref(), &out_dir) {
        Ok(report) => {
            println!("{report}");
            exit::OK
        }
        Err(e) => {
            eprintln!("error: {e}");
            exit::IO
        }
    }
}
