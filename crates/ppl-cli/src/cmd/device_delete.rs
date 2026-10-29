//! Guarded unattended USB deletion with explicit manual helper fallback.
use std::io::{BufRead as _, IsTerminal, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Subcommand;
use ppl_device::{
    ControlObject, DeleteOptions, DeleteTarget, DeviceError, FileType, Prime, Probe, TargetStatus,
};
use serde_json::{Value, json};

use super::common::exit;
use super::device_delete_store::{self as records, Operation, Store, Target};

#[path = "device_delete_flow.rs"]
mod flow;
use flow::{cleanup, manual, reconcile, require_device, retire_confirmed};
#[path = "device_delete_user.rs"]
mod user;

pub const DEFAULT_DELETE_DIR: &str = ppl_programs::DEFAULT_STORE_DIR;

/// Delete a program or note from a connected HP Prime G1.
///
/// One command, no calculator interaction: `ppl device delete program
/// FIREWORKS --yes`. The object is read twice, backed up raw to disk,
/// deleted with one native USB delete command and verified absent. Without
/// --yes you must type the exact name. Objects named ZZT_* are disposable
/// test objects and may instead be authorized by a creation receipt.
/// Lists, matrices, real/complex variables and other types are unsupported.
#[derive(Subcommand, Debug)]
pub enum DeleteAction {
    /// Delete one program: backed up, authorized, independently verified.
    ///
    /// `delete program FIREWORKS --yes` deletes exactly that one program with
    /// no further interaction (no Run press, no helper, no ZZT_CTRL, no
    /// upload receipt). Without --yes you must type the exact name; --json
    /// alone never authorizes. A raw backup is saved first, one native USB
    /// delete is sent (never retried), and absence is verified against a
    /// liveness anchor (built-in settings object, or --control). ZZT_* names are disposable test
    /// objects: several may be given and a creation receipt can authorize
    /// them; --manual-run and --prepare-only apply to those only.
    Program {
        names: Vec<String>,
        /// Liveness anchor `type:name` (default: built-in settings object when
        /// evidenced, else ZZT_CTRL if installed).
        #[arg(long)]
        control: Option<String>,
        /// Authorize deleting exactly this one named program, without
        /// prompting. Scoped to this device, type, name and content, one
        /// attempt; --json alone never authorizes.
        #[arg(
            long,
            conflicts_with_all = ["manual_run", "prepare_only"]
        )]
        yes: bool,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_secs: Option<u64>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=60))]
        poll_secs: Option<u64>,
        /// Upload a verified helper only; no deletion or remote keys.
        #[arg(long, conflicts_with = "manual_run")]
        prepare_only: bool,
        /// Use the established helper workflow and wait for one manual Run press.
        #[arg(long)]
        manual_run: bool,
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_DELETE_DIR)]
        delete_dir: PathBuf,
    },
    /// Delete one note: backed up, typed-name confirmed, independently verified.
    ///
    /// An ordinary name deletes exactly that one note after you type its
    /// exact name; a companion note is never deleted implicitly. ZZT_* names
    /// are disposable test objects (see `delete program --help`).
    Note {
        names: Vec<String>,
        #[arg(long)]
        control: Option<String>,
        /// Not supported for notes.
        #[arg(long, hide = true)]
        yes: bool,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_secs: Option<u64>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=60))]
        poll_secs: Option<u64>,
        #[arg(long, conflicts_with = "manual_run")]
        prepare_only: bool,
        #[arg(long)]
        manual_run: bool,
        #[arg(long, default_value = ppl_compat::DEFAULT_RUNS_DIR)]
        runs_dir: PathBuf,
        #[arg(long, default_value = DEFAULT_DELETE_DIR)]
        delete_dir: PathBuf,
    },
    /// Reconcile an operation by fresh read-only queries, or verify legacy targets.
    Verify {
        ty: Option<String>,
        names: Vec<String>,
        #[arg(long, default_value = ppl_device::DEFAULT_CONTROL)]
        control: String,
        #[arg(long, conflicts_with = "operation_id")]
        helper: Option<String>,
        #[arg(long)]
        operation_id: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout_secs: Option<u64>,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=60))]
        poll_secs: Option<u64>,
        #[arg(long, default_value = DEFAULT_DELETE_DIR)]
        delete_dir: PathBuf,
    },
    /// Remove only archived temporary objects of a confirmed deletion operation.
    Cleanup {
        #[arg(long)]
        operation_id: String,
        #[arg(long, default_value = DEFAULT_DELETE_DIR)]
        delete_dir: PathBuf,
    },
}

/// Chooses the naming/authorization class for a deletion request.
/// ZZT_* names are disposable test objects; everything else is an ordinary
/// object that is deleted alone and only after typed confirmation.
fn classify(
    ty: FileType,
    names: &[String],
    control: &ControlObject,
    yes: bool,
) -> Result<(ppl_device::NamePolicy, Vec<DeleteTarget>), DeviceError> {
    use ppl_device::NamePolicy;
    if yes {
        // Explicit authorization names exactly one object, whatever its
        // prefix; it never goes through the receipt/ledger path.
        if names.len() != 1 {
            return Err(refused("--yes deletes exactly one named program"));
        }
        ppl_device::validate_object_name(ty, &names[0])?;
        if names[0].eq_ignore_ascii_case(&control.name) {
            return Err(refused("protected or duplicate deletion target"));
        }
        return Ok((
            NamePolicy::User,
            vec![DeleteTarget {
                ty,
                name: names[0].clone(),
            }],
        ));
    }
    let test = names
        .iter()
        .filter(|n| n.starts_with(ppl_device::SAFE_PREFIX))
        .count();
    if test == names.len() {
        return Ok((NamePolicy::Test, targets(ty, names, control)?));
    }
    if test != 0 {
        return Err(refused(
            "do not mix ZZT_ test objects and ordinary objects in one request",
        ));
    }
    if names.len() != 1 {
        return Err(refused("ordinary objects are deleted one at a time"));
    }
    ppl_device::validate_object_name(ty, &names[0])?;
    if names[0].eq_ignore_ascii_case(&control.name) {
        return Err(refused("protected or duplicate deletion target"));
    }
    Ok((
        NamePolicy::User,
        vec![DeleteTarget {
            ty,
            name: names[0].clone(),
        }],
    ))
}
fn refused(s: impl Into<String>) -> DeviceError {
    DeviceError::Refused(s.into())
}
fn disk(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Hid(format!("deletion persistence: {e}"))
}
fn targets(
    ty: FileType,
    names: &[String],
    control: &ControlObject,
) -> Result<Vec<DeleteTarget>, DeviceError> {
    if !matches!(ty, FileType::Program | FileType::Note)
        || names.is_empty()
        || names.len() > ppl_device::MAX_TARGETS
    {
        return Err(refused("provide 1–32 program or note targets"));
    }
    for (i, n) in names.iter().enumerate() {
        ppl_device::validate_test_name(n)?;
        if n.eq_ignore_ascii_case("ZZT_CTRL")
            || n.eq_ignore_ascii_case(&control.name)
            || names[..i].iter().any(|other| other.eq_ignore_ascii_case(n))
        {
            return Err(refused("protected or duplicate deletion target"));
        }
    }
    Ok(names
        .iter()
        .map(|name| DeleteTarget {
            ty,
            name: name.clone(),
        })
        .collect())
}
pub fn run(
    action: &DeleteAction,
    json_output: bool,
    selected: Option<&str>,
    log: Option<&Path>,
) -> u8 {
    match execute(action, json_output, selected, log) {
        Ok((mut value, code)) => {
            if matches!(action, DeleteAction::Verify { .. }) {
                value["operation"] = json!("verify");
            }
            if matches!(action, DeleteAction::Cleanup { .. }) {
                value["operation"] = json!("cleanup");
            }
            if value["cleanup_state"] == "complete" {
                value["cleanup_mechanism"] = json!("usb_delete");
            }
            if json_output {
                println!("{value}");
            } else {
                if let Some(id) = value["operation_id"].as_str() {
                    println!("operation: {id}");
                }
                if let Some(results) = value["results"].as_array() {
                    for t in results {
                        println!(
                            "{} `{}`: {}",
                            t["type"].as_str().unwrap_or("object"),
                            t["object"].as_str().unwrap_or("?"),
                            t["status"].as_str().unwrap_or("unknown")
                        );
                    }
                }
                if value["object_class"] == "user" && code != exit::OK {
                    if let Some(r) = value["result"].as_str() {
                        println!("\nResult: {r}");
                    }
                }
                if let Some(error) = value["error"].as_str() {
                    eprintln!("{error}");
                }
                if let Some(recovery) = value["recovery"].as_str() {
                    eprintln!("{recovery}");
                }
                if let Some(cleanup) = value["cleanup_state"].as_str() {
                    println!("cleanup: {cleanup}");
                }
            }
            code
        }
        Err(e) => {
            if json_output {
                println!(
                    "{}",
                    json!({"schema_version":1,"operation":"delete","backup_created":false,"deletion_confirmed":false,"execution_confirmed":false,"result":user::failure_result(&e),"error":e.to_string()})
                );
            } else {
                eprintln!("error: {e}");
            }
            exit::IO
        }
    }
}

fn identity(p: &mut Prime) -> Result<(String, ppl_device::CalcInfo), DeviceError> {
    let info = p.info()?;
    let serial = info
        .serial
        .as_deref()
        .ok_or_else(|| refused("device identity is unavailable"))?;
    let id = ppl_compat::serial_fingerprint(serial);
    // Never include raw serial-bearing FA replies in deletion evidence logs.
    p.log.clear();
    Ok((id, info))
}
pub(super) fn direct_profile(p: &Prime, info: &ppl_device::CalcInfo) -> bool {
    ppl_device::delete::direct_delete_supported(
        p.detected.product_id,
        info.build,
        info.version.as_deref(),
    )
}
fn execute(
    action: &DeleteAction,
    json_output: bool,
    selected: Option<&str>,
    log_path: Option<&Path>,
) -> Result<(Value, u8), DeviceError> {
    match action {
        DeleteAction::Program {
            names,
            control,
            yes,
            timeout_secs,
            poll_secs,
            prepare_only,
            manual_run,
            runs_dir,
            delete_dir,
        }
        | DeleteAction::Note {
            names,
            control,
            yes,
            timeout_secs,
            poll_secs,
            prepare_only,
            manual_run,
            runs_dir,
            delete_dir,
        } => {
            let explicit_control = control
                .as_deref()
                .map(ControlObject::parse)
                .transpose()
                .map_err(refused)?;
            let control = match &explicit_control {
                Some(c) => c.clone(),
                None => ControlObject::parse(ppl_device::DEFAULT_CONTROL).map_err(refused)?,
            };
            let ty = if matches!(action, DeleteAction::Program { .. }) {
                FileType::Program
            } else {
                FileType::Note
            };
            if *yes && ty != FileType::Program {
                return Err(refused("--yes is only supported for programs"));
            }
            let (policy, targets) = classify(ty, names, &control, *yes)?;
            if policy == ppl_device::NamePolicy::User {
                if *manual_run || *prepare_only {
                    return Err(refused(
                        "--manual-run and --prepare-only apply to ZZT_ test objects only",
                    ));
                }
                return user::execute_user(
                    targets,
                    explicit_control,
                    if *yes {
                        user::AuthMode::Yes
                    } else {
                        user::AuthMode::Typed
                    },
                    runs_dir,
                    delete_dir,
                    json_output,
                    selected,
                    log_path,
                );
            }
            let mut p = super::remote::open_selected(selected)?;
            let (serial, info) = identity(&mut p)?;
            let store = Store::new(delete_dir);
            let _journal_lock = store.lock()?;
            store.reject_pending(&serial, &targets)?;
            let deletion = ppl_device::prepare_deletion(&mut p, &targets, &control)?;
            let owned = authorize_targets(&deletion.backups, runs_dir, &serial, json_output)?;
            validate_helper(&deletion)?;
            let now = ppl_compat::utc_now();
            let mut op = Operation {
                schema_version: 1,
                id: deletion.helper.clone(),
                device_serial_id: serial,
                usb_pid: p.detected.product_id,
                firmware_build: info.build,
                boot_version: info.version.clone(),
                control: control.to_string(),
                state: "prepared".into(),
                mechanism: if *manual_run || *prepare_only {
                    "ppl_helper"
                } else {
                    "usb_delete"
                }
                .into(),
                created_utc: now.clone(),
                updated_utc: now,
                targets: deletion
                    .backups
                    .iter()
                    .map(|b| Target {
                        name: b.name.clone(),
                        ty: b.ty.label().into(),
                        sha256: ppl_compat::sha256_hex(&b.data),
                        bytes: b.data.len(),
                        status: "not_submitted".into(),
                    })
                    .collect(),
                helper: None,
                helper_note: None,
                helper_source: None,
                helper_uploaded: false,
                completion: None,
                artifacts: Vec::new(),
                cleanup_state: "not_required".into(),
                runs_dir: runs_dir.clone(),
                error: None,
                class: "test".into(),
                authorization: None,
                requested_utc: None,
                submission: Vec::new(),
            };
            store.create(&op, &deletion.backups)?;
            // Keep the exact authorization mode in the auditable operation directory.
            records::atomic_json(
                &store.directory(&op.id)?.join("authorization.json"),
                &json!({"device_serial_id":op.device_serial_id,"targets":owned}),
            )
            .map_err(disk)?;
            let opts = DeleteOptions {
                timeout: Duration::from_secs(timeout_secs.unwrap_or(600)),
                poll: Duration::from_secs(poll_secs.unwrap_or(2)),
                settle: Duration::from_secs(2),
            };
            let result = (|| -> Result<(), DeviceError> {
                if *manual_run || *prepare_only {
                    manual(
                        &mut p,
                        &deletion,
                        &store,
                        &mut op,
                        &opts,
                        *prepare_only,
                        json_output,
                    )
                } else if direct_profile(&p, &info) {
                    op.state = "submitting".into();
                    store.save(&op)?;
                    let result = ppl_programs::submit_deletion(&mut p, &deletion, &store, &mut op);
                    match result {
                        Ok(report) => {
                            let never_sent = report
                                .submission
                                .iter()
                                .all(|s| *s == ppl_device::delete::SubmissionState::NotSent);
                            op.submission = report.submission;
                            apply_statuses(
                                &mut op,
                                &report
                                    .targets
                                    .iter()
                                    .map(|t| t.status.clone())
                                    .collect::<Vec<_>>(),
                            );
                            op.error = report.error;
                            op.state = if never_sent {
                                "not_submitted"
                            } else if op.confirmed() {
                                "completed"
                            } else {
                                "unresolved"
                            }
                            .into();
                            store.save(&op)?;
                            retire_confirmed(&store, &op)
                        }
                        Err(e) => {
                            // The shared submitter returns Err only before any EB attempt.
                            op.state = "not_submitted".into();
                            Err(e)
                        }
                    }
                } else {
                    op.state = "not_submitted".into();
                    Err(refused(
                        "automatic deletion is unsupported on this unverified device profile; use --manual-run or --prepare-only explicitly",
                    ))
                }
            })();
            if let Err(e) = result {
                op.error = Some(e.to_string());
                if op.confirmed() {
                    op.state = "verified".into();
                } else if op.state == "submitting" {
                    op.state = "unresolved".into();
                }
            }
            op.updated_utc = ppl_compat::utc_now();
            if let Err(e) = store.save(&op) {
                op.error = Some(format!(
                    "{}; {e}",
                    op.error
                        .as_deref()
                        .unwrap_or("device outcome recorded in memory")
                ));
            }
            if let Some(path) = log_path {
                if let Err(e) = std::fs::write(path, p.log_text()) {
                    op.error = Some(format!("cannot save USB log: {e}"));
                }
            }
            let code = if (op.confirmed()
                && matches!(op.cleanup_state.as_str(), "complete" | "not_required")
                && op.error.is_none())
                || (*prepare_only && op.state == "awaiting_run" && op.error.is_none())
            {
                exit::OK
            } else {
                exit::IO
            };
            Ok((outcome(&store, &op), code))
        }
        DeleteAction::Verify {
            ty,
            names,
            control,
            helper,
            operation_id,
            timeout_secs,
            poll_secs,
            delete_dir,
        } => {
            if let Some(id) = operation_id {
                if ty.is_some() || !names.is_empty() {
                    return Err(refused(
                        "--operation-id cannot be combined with positional targets",
                    ));
                }
                let store = Store::new(delete_dir);
                let _journal_lock = store.lock()?;
                if store.directory(id)?.join("replacement.json").exists() {
                    return Err(refused(
                        "replacement operations require `device replace verify --operation-id ID`; deletion-only recovery cannot complete a replacement",
                    ));
                }
                let mut op = store.load(id)?;
                let mut p = super::remote::open_selected(selected)?;
                let (serial, _) = identity(&mut p)?;
                require_device(&op, &serial)?;
                if let Err(e) = reconcile(&mut p, &store, &mut op) {
                    op.error = Some(e.to_string());
                    if let Err(save) = store.save(&op) {
                        op.error = Some(format!("{e}; {save}"));
                    }
                }
                let code = if op.confirmed() && op.error.is_none() {
                    exit::OK
                } else {
                    exit::IO
                };
                Ok((outcome(&store, &op), code))
            } else {
                if let Some(h) = helper {
                    ppl_device::validate_test_name(h)?;
                }
                let control = ControlObject::parse(control).map_err(refused)?;
                let ty = ty.as_deref().and_then(FileType::parse).ok_or_else(|| {
                    refused("verify requires program/note targets or --operation-id")
                })?;
                let targets = targets(ty, names, &control)?;
                let mut p = super::remote::open_selected(selected)?;
                let (serial, _) = identity(&mut p)?;
                let mut completion = None;
                if let Some(helper) = helper {
                    let deletion = ppl_device::Deletion {
                        helper: helper.clone(),
                        helper_note: ppl_device::result_note_name(helper),
                        source: String::new(),
                        backups: Vec::new(),
                        log: Vec::new(),
                        policy: ppl_device::NamePolicy::Test,
                        baseline: None,
                    };
                    let opts = DeleteOptions {
                        timeout: Duration::from_secs(timeout_secs.unwrap_or(600)),
                        poll: Duration::from_secs(poll_secs.unwrap_or(2)),
                        ..DeleteOptions::default()
                    };
                    completion = ppl_device::await_deletion_note(&mut p, &deletion, &opts, |_| {})?
                        .map(|w| w.text);
                }
                let statuses = ppl_device::verify_deleted(&mut p, &targets, &control)?;
                let all = statuses
                    .iter()
                    .all(|s| *s == TargetStatus::ConfirmedDeleted);
                let results:Vec<_>=targets.iter().zip(&statuses).map(|(t,s)|json!({"object":t.name,"type":t.ty.label(),"status":status_str(s),"deletion_confirmed":*s==TargetStatus::ConfirmedDeleted})).collect();
                // Retain the legacy verify event format for old helper journals.
                append_legacy_verify(delete_dir, helper.as_deref(), &serial, &results)?;
                Ok((
                    json!({"schema_version":1,"device_serial_id":serial,"operation":"verify","deletion_confirmed":all,"execution_confirmed":completion.as_deref()==Some("done"),"helper_note":completion,"results":results,"error":if all {None} else {Some("deletion not confirmed")}}),
                    if all { exit::OK } else { exit::IO },
                ))
            }
        }
        DeleteAction::Cleanup {
            operation_id,
            delete_dir,
        } => {
            let store = Store::new(delete_dir);
            let _journal_lock = store.lock()?;
            ppl_device::validate_test_name(operation_id)?;
            let mut p = super::remote::open_selected(selected)?;
            let (serial, info) = identity(&mut p)?;
            let mut op = if store
                .directory(operation_id)?
                .join("operation.json")
                .exists()
            {
                store.load(operation_id)?
            } else {
                super::device_delete_legacy::import(&store, operation_id, &mut p, &serial, &info)?
            };
            require_device(&op, &serial)?;
            let result = if direct_profile(&p, &info) {
                cleanup(&mut p, &store, &mut op)
            } else {
                Err(refused(
                    "cleanup requires hardware-verified direct deletion",
                ))
            };
            if let Err(e) = result {
                op.error = Some(e.to_string());
                store.save(&op)?;
            }
            let code = if matches!(op.cleanup_state.as_str(), "complete" | "not_required")
                && op.error.is_none()
            {
                exit::OK
            } else {
                exit::IO
            };
            Ok((outcome(&store, &op), code))
        }
    }
}

fn authorize_targets(
    backups: &[ppl_device::Backup],
    runs_dir: &Path,
    serial: &str,
    json_output: bool,
) -> Result<Vec<Value>, DeviceError> {
    backups.iter().map(|b| {
        if records::owns(runs_dir,serial,b)? {return Ok(json!({"name":b.name,"type":b.ty.label(),"authorization":"creation_receipt"}));}
        if json_output || !std::io::stdin().is_terminal() {return Err(refused(format!("`{}` lacks a matching device/type/content creation receipt; legacy ledger names alone cannot authorize deletion",b.name)));}
        eprintln!("No matching creation receipt for {} `{}`. Type its exact name to authorize deletion of the backed-up object:",b.ty.label(),b.name);
        std::io::stderr().flush().map_err(disk)?;
        let mut line=String::new();
        std::io::stdin().lock().read_line(&mut line).map_err(disk)?;
        if line.trim_end_matches(['\r','\n']) != b.name {return Err(refused("typed authorization did not match"));}
        Ok(json!({"name":b.name,"type":b.ty.label(),"authorization":"exact_typed_confirmation"}))
    }).collect()
}
fn validate_helper(d: &ppl_device::Deletion) -> Result<(), DeviceError> {
    let pairs = ppl_compat::cleanup::extraction_targets(&d.source).map_err(refused)?;
    let expected: Vec<_> = d
        .allowlist()
        .iter()
        .map(|t| {
            (
                if t.ty == FileType::Program {
                    "Programs"
                } else {
                    "Notes"
                }
                .to_string(),
                t.name.clone(),
            )
        })
        .collect();
    if pairs != expected {
        return Err(refused(
            "helper AST does not match the exact deletion allowlist",
        ));
    }
    Ok(())
}
pub(super) fn status_str(s: &TargetStatus) -> &'static str {
    match s {
        TargetStatus::ConfirmedDeleted => "confirmed_deleted",
        TargetStatus::StillPresent => "still_present",
        TargetStatus::Unknown => "unknown",
    }
}
fn apply_statuses(op: &mut Operation, statuses: &[TargetStatus]) {
    for (t, s) in op.targets.iter_mut().zip(statuses) {
        t.status = status_str(s).into();
    }
}
fn outcome(store: &Store, op: &Operation) -> Value {
    let results:Vec<_>=op.targets.iter().map(|t|json!({"object":t.name,"type":t.ty,"status":t.status,"backup_sha256":t.sha256,"deletion_confirmed":t.status=="confirmed_deleted","error":if t.status=="confirmed_deleted" {None} else {Some("deletion not confirmed")}})).collect();
    let recovery = format!(
        "ppl device delete verify --operation-id {} --delete-dir {}",
        op.id,
        store.root.display()
    );
    let mut out = json!({"schema_version":1,"device":"HP Prime G1","device_serial_id":op.device_serial_id,"operation":"delete","operation_id":op.id,"phase":op.state,"mechanism":op.mechanism,"backup_created":true,"backup_directory":store.directory(&op.id).ok(),"deletion_confirmed":op.confirmed(),"execution_confirmed":op.completion.as_deref()==Some("done"),"helper":op.helper,"helper_uploaded":op.helper_uploaded,"cleanup_state":op.cleanup_state,"temporary_objects":op.artifacts,"results":results,"error":op.error,"recovery":if !matches!(op.state.as_str(),"completed"|"not_submitted") {Some(recovery)} else {None}});
    out["submission_evidence"] = json!(op.submission);
    out["result"] = json!(if op.state == "not_submitted"
        && !op.submission.is_empty()
        && op
            .submission
            .iter()
            .all(|s| *s == ppl_device::delete::SubmissionState::NotSent)
    {
        "verified_not_performed"
    } else if op.confirmed() {
        "confirmed_deleted"
    } else if op.targets.iter().any(|t| t.status == "still_present") {
        "still_present"
    } else {
        "indeterminate"
    });
    if op.targets.len() == 1 {
        out["object"] = json!(op.targets[0].name);
        out["type"] = json!(op.targets[0].ty);
    }
    if !op.confirmed() && op.state == "unresolved" && op.error.is_none() {
        out["error"] = json!("deletion not confirmed; inspect per-target results");
    }
    out
}
fn append_legacy_verify(
    root: &Path,
    helper: Option<&str>,
    serial: &str,
    results: &[Value],
) -> Result<(), DeviceError> {
    std::fs::create_dir_all(root).map_err(disk)?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("journal.jsonl"))
        .map_err(disk)?;
    writeln!(f,"{}",json!({"id":helper.unwrap_or("verify"),"phase":"verify","at_utc":ppl_compat::utc_now(),"device_serial_id":serial,"targets":results})).and_then(|_|f.sync_all()).map_err(disk)
}
/// `ppl device list`: enumerate the host-known ZZT_ objects and probe each
/// one's presence. The G1 protocol has no enumeration command, so only
/// objects this project recorded (ledger) can be listed; everything else
/// on the calculator is invisible to this command by design.
pub fn list(runs_dir: &Path, limit: Option<usize>, json: bool) -> u8 {
    let entries = match ppl_compat::ledger_entries(runs_dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let mut names: Vec<String> = Vec::new();
    for e in entries {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&e.name)) {
            names.push(e.name);
        }
    }
    if let Some(l) = limit {
        names.truncate(l);
    }
    if names.is_empty() {
        if json {
            println!(
                "{}",
                json!({"device": "HP Prime G1", "operation": "list", "results": [], "note": "no recorded objects"})
            );
        } else {
            println!("no recorded objects in {}", runs_dir.display());
        }
        return exit::OK;
    }
    let mut p = match Prime::open_first() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return exit::IO;
        }
    };
    let serial_id = p.info().ok().and_then(|i| i.serial_fingerprint());
    if !json {
        println!(
            "probing {} recorded object(s) from {} … (the G1 protocol cannot enumerate \
             the calculator; only objects this host recorded are known)",
            names.len(),
            runs_dir.display()
        );
    }
    let mut results = Vec::new();
    for name in &names {
        // Programs first, then notes; present probes answer immediately,
        // absent ones cost one PROBE_TIMEOUT each.
        let mut found: Option<(FileType, usize)> = None;
        for ty in [FileType::Program, FileType::Note] {
            match p.probe_file(ty, name, ppl_device::PROBE_TIMEOUT) {
                Ok(Probe::Present(f)) if f.ty == ty && f.name.eq_ignore_ascii_case(name) => {
                    found = Some((ty, f.data.len()));
                    break;
                }
                Ok(Probe::Present(_)) | Ok(Probe::NoReply { .. }) => {}
                Err(e) => {
                    eprintln!("error: {e}");
                    return exit::IO;
                }
            }
        }
        let state = match found {
            Some((ty, bytes)) => json!({
                "object": name,
                "type": ty.label(),
                "state": "present",
                "bytes": bytes,
            }),
            None => json!({
                "object": name,
                "state": "silent",
            }),
        };
        if !json {
            match found {
                Some((ty, bytes)) => println!("{} {:16} {} bytes", name, ty.label(), bytes),
                None => println!("{name} —  no reply (not present or type unknown)"),
            }
        }
        results.push(state);
    }
    let present = results
        .iter()
        .filter(|r| r.get("state") == Some(&json!("present")))
        .count();
    if json {
        println!(
            "{}",
            json!({
                "device": "HP Prime G1",
                "device_serial_id": serial_id,
                "operation": "list",
                "results": results,
                "present": present,
                "silent": names.len() - present,
            })
        );
    } else {
        println!("{} present, {} silent", present, names.len() - present);
    }
    exit::OK
}
