//! Device program generator (v2) and transcript parser.
//!
//! [`generate_device_program`] turns at most [`Channel::max_cases`]
//! audited cases ([`MAX_CASES_PRINT`] on the terminal, [`MAX_CASES_NOTE`]
//! in a note) into one calculator program. Each case becomes a
//! non-exported function `ZCn()`; the exported program evaluates them in
//! `IFERR` and emits one line per case, either on the terminal
//! ([`Channel::Print`], read back from a screenshot) or into a note
//! ([`Channel::Note`], fetched over USB).
//!
//! # Record format
//!
//! ```text
//! @@ZZT|<program>|<sha8>|<HAngle>|<HFormat>|<HDigits>   header (settings at start)
//! cN|<TYPE code>|<STRING text>                          value (text may contain '|')
//! cN|E|                                                 runtime error (caught by IFERR)
//! cN|SETCHG|<setting>                                   case left a setting changed
//! @@END                                                 terminator (required)
//! ```
//!
//! `cN` are short ids (`c1`, `c2`, …) in program order, mapped back to case
//! ids through [`GeneratedProgram::short_ids`]. After every case the
//! program compares `HAngle`/`HFormat`/`HDigits` with the values at start,
//! reports any change, and restores it.
//!
//! # Program tag
//!
//! `sha8` is the first 8 hex digits of the SHA-256 of the program source
//! generated with the placeholder [`SHA8_PLACEHOLDER`] in place of the tag
//! (the tag cannot hash itself). [`verify_program_tag`] recomputes it from a
//! source text. [`GeneratedProgram::sha256`] is the SHA-256 of the final
//! source, i.e. of exactly what is uploaded.

use std::collections::BTreeMap;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::audit::{SIDE_EFFECT_PRINT, audit_case};
use crate::sha256::sha256_hex;
use crate::{Case, Outcome, Suite, indent};

/// Upper bound on cases per generated program over all channels (the
/// [`Channel::Note`] limit). Each channel has its own limit, see
/// [`Channel::max_cases`].
pub const MAX_CASES_PER_PROGRAM: usize = MAX_CASES_NOTE;
/// Cases per [`Channel::Print`] program. The G1 terminal shows about 14
/// lines, and the header, every case line and `@@END` must fit on one
/// screenshot (with room for a wrapped line or a `SETCHG` record).
pub const MAX_CASES_PRINT: usize = 8;
/// Cases per [`Channel::Note`] program (the note has no screen limit).
pub const MAX_CASES_NOTE: usize = 12;
/// Placeholder used for the tag while hashing.
pub const SHA8_PLACEHOLDER: &str = "00000000";
/// Settings snapshotted in the header and checked after every case.
pub const CHECKED_SETTINGS: [&str; 3] = ["HAngle", "HFormat", "HDigits"];

/// Where the generated program writes its result lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Channel {
    /// `PRINT` to the terminal (read back by screenshot).
    Print,
    /// Accumulated in a string and stored in a new note. The program refuses
    /// to run if the note already exists (checked before anything else).
    Note { note: String },
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Print => "print",
            Channel::Note { .. } => "note",
        }
    }

    /// Most cases one program may hold on this channel.
    pub fn max_cases(&self) -> usize {
        match self {
            Channel::Print => MAX_CASES_PRINT,
            Channel::Note { .. } => MAX_CASES_NOTE,
        }
    }

    pub fn note(&self) -> Option<&str> {
        match self {
            Channel::Print => None,
            Channel::Note { note } => Some(note),
        }
    }
}

/// A generated calculator program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedProgram {
    pub program: String,
    pub channel: Channel,
    pub source: String,
    /// SHA-256 (hex) of `source`.
    pub sha256: String,
    /// Tag embedded in the header (see module docs).
    pub sha8: String,
    /// `(short id, case id)` in program order.
    pub short_ids: Vec<(String, String)>,
}

/// `^ZZT_[A-Z0-9_]{1,12}$`: the only names the harness may create on the
/// calculator (also used for note names).
pub fn valid_program_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("ZZT_") else {
        return false;
    };
    (1..=12).contains(&rest.len())
        && rest
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Generates the device program for `cases` (taken from `suite`).
///
/// Refuses (returning every reason) when:
/// * `program` (or the note name) does not match `^ZZT_[A-Z0-9_]{1,12}$`;
/// * there are no cases or more than [`Channel::max_cases`] (8 for
///   [`Channel::Print`], 12 for [`Channel::Note`]);
/// * a case does not compile, or has a side effect (see [`crate::audit`])
///   not declared in its `mutates`;
/// * a case declares `mutates` without `isolate = true`, or is isolated
///   (`isolate = true`, which any `mutates` requires) but is not the only
///   case in the program;
/// * a case prints to the terminal under [`Channel::Print`] (it would
///   corrupt the transcript), even if declared;
/// * a case id appears twice, or the generated source does not parse.
pub fn generate_device_program(
    suite: &Suite,
    cases: &[&Case],
    program: &str,
    channel: &Channel,
) -> Result<GeneratedProgram, Vec<String>> {
    let mut errs = Vec::new();
    if !valid_program_name(program) {
        errs.push(format!(
            "program name `{program}` does not match ^ZZT_[A-Z0-9_]{{1,12}}$"
        ));
    }
    if let Channel::Note { note } = channel {
        if !valid_program_name(note) {
            errs.push(format!(
                "note name `{note}` does not match ^ZZT_[A-Z0-9_]{{1,12}}$"
            ));
        }
    }
    if cases.is_empty() {
        errs.push("no cases selected".into());
    }
    if cases.len() > channel.max_cases() {
        errs.push(format!(
            "{} cases selected; at most {} per {} program",
            cases.len(),
            channel.max_cases(),
            channel.as_str()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in cases {
        if !seen.insert(c.id.as_str()) {
            errs.push(format!("case `{}` selected twice", c.id));
        }
        if c.id.contains('|') || c.id.contains("@@") || c.id.contains('\n') {
            errs.push(format!("case id `{}` contains a reserved character", c.id));
        }
        let audit = match audit_case(c) {
            Ok(a) => a,
            Err(e) => {
                errs.push(format!("case `{}` does not compile: {e}", c.id));
                continue;
            }
        };
        let undeclared: Vec<&str> = audit
            .side_effects
            .iter()
            .filter(|e| !c.mutates.iter().any(|m| m == *e))
            .map(String::as_str)
            .collect();
        if !undeclared.is_empty() {
            errs.push(format!(
                "case `{}` has undeclared side effects: {}",
                c.id,
                undeclared.join(", ")
            ));
        }
        if !c.mutates.is_empty() && !c.isolate {
            errs.push(format!(
                "case `{}` declares mutates = {:?} but is not `isolate = true`",
                c.id, c.mutates
            ));
        }
        if c.isolate && cases.len() > 1 {
            errs.push(format!(
                "case `{}` is isolated and must be the only case in its program",
                c.id
            ));
        }
        if *channel == Channel::Print && audit.side_effects.contains(SIDE_EFFECT_PRINT) {
            errs.push(format!(
                "case `{}` prints to the terminal, which would corrupt the PRINT transcript",
                c.id
            ));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    let short_ids: Vec<(String, String)> = cases
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("c{}", i + 1), c.id.clone()))
        .collect();
    let placeholder = render(suite, cases, program, channel, SHA8_PLACEHOLDER);
    let sha8 = sha256_hex(placeholder.as_bytes())[..8].to_string();
    let source = render(suite, cases, program, channel, &sha8);
    let (_, diags) = ppl_syntax::parse(&source);
    if diags.has_errors() {
        return Err(diags
            .errors()
            .map(|d| format!("generated program does not parse: {}", d.message))
            .collect());
    }
    Ok(GeneratedProgram {
        program: program.to_string(),
        channel: channel.clone(),
        sha256: sha256_hex(source.as_bytes()),
        sha8,
        source,
        short_ids,
    })
}

fn render(suite: &Suite, cases: &[&Case], program: &str, channel: &Channel, tag: &str) -> String {
    let mut out = format!(
        "// Generated by ppl compare (suite {}, program {program}). Safe to delete.\n",
        suite.suite.replace('\n', " ")
    );
    for (i, c) in cases.iter().enumerate() {
        out.push_str(&format!(
            "ZC{}()\nBEGIN\n{}\nEND;\n\n",
            i + 1,
            indent(&c.function_body())
        ));
    }
    let emit = |expr: &str| match channel {
        Channel::Print => format!("PRINT({expr});"),
        Channel::Note { .. } => format!("r:=r+CHAR(10)+{expr};"),
    };
    out.push_str(&format!("EXPORT {program}()\nBEGIN\n"));
    let header = format!("\"@@ZZT|{program}|{tag}|\"+STRING(h0)+\"|\"+STRING(f0)+\"|\"+STRING(d0)");
    match channel {
        Channel::Print => {
            out.push_str("  LOCAL v, ok, h0, f0, d0;\n");
            out.push_str("  h0:=HAngle; f0:=HFormat; d0:=HDigits;\n");
            out.push_str("  PRINT();\n");
            out.push_str(&format!("  PRINT({header});\n"));
        }
        Channel::Note { note } => {
            // Hardware finding (2026-10-22, G1 build 15157): `Notes("x")`
            // of a *missing* note does NOT raise an error on the device,
            // so an IFERR existence probe always "succeeds" and the former
            // abort-if-exists check fired on every run, refusing to write
            // anything. Absence is already proven the strong way — the USB
            // preflight showed the note name silent twice before the upload
            // and the ledger never reuses a name — so the program writes
            // directly. The single write is the last statement before the
            // status print, after every case has run.
            out.push_str("  LOCAL v, ok, h0, f0, d0, r;\n");
            out.push_str("  h0:=HAngle; f0:=HFormat; d0:=HDigits;\n");
            out.push_str(&format!("  r:={header};\n"));
            // Crash-resilient evidence: the note is written immediately
            // (and re-written after every case below), so a program that
            // dies mid-run still leaves every completed case on the
            // calculator for the host to fetch. The name is the
            // preflight-proven ZZT_* note owned by this program alone.
            out.push_str(&format!("  Notes(\"{note}\"):=r;\n"));
        }
    }
    let snap = ["h0", "f0", "d0"];
    for (i, _) in cases.iter().enumerate() {
        let s = format!("c{}", i + 1);
        out.push_str("  ok:=1;\n");
        out.push_str(&format!("  IFERR v:=ZC{}(); THEN ok:=0; END;\n", i + 1));
        out.push_str(&format!(
            "  IF ok THEN {} ELSE {} END;\n",
            emit(&format!("\"{s}|\"+STRING(TYPE(v))+\"|\"+STRING(v)")),
            emit(&format!("\"{s}|E|\""))
        ));
        for (name, var) in CHECKED_SETTINGS.iter().zip(snap) {
            out.push_str(&format!(
                "  IF {name}<>{var} THEN {} {name}:={var}; END;\n",
                emit(&format!("\"{s}|SETCHG|{name}\""))
            ));
        }
        if let Channel::Note { note } = channel {
            out.push_str(&format!("  Notes(\"{note}\"):=r;\n"));
        }
    }
    match channel {
        Channel::Print => {
            out.push_str("  PRINT(\"@@END\");\n  RETURN 0;\nEND;\n");
        }
        Channel::Note { note } => {
            out.push_str(&format!(
                "  r:=r+CHAR(10)+\"@@END\";\n  Notes(\"{note}\"):=r;\n  PRINT(\"{program} done\");\n  RETURN 0;\nEND;\n"
            ));
        }
    }
    out
}

/// Locates the header tag in a generated source and checks it.
/// Returns `(program, sha8)` when the tag matches the source.
pub fn verify_program_tag(source: &str) -> Result<(String, String), String> {
    let marker = "\"@@ZZT|";
    let at = source
        .rfind(marker)
        .ok_or("source contains no @@ZZT header")?;
    let rest = &source[at + marker.len()..];
    let mut parts = rest.splitn(3, '|');
    let program = parts.next().unwrap_or_default().to_string();
    let sha8 = parts.next().unwrap_or_default().to_string();
    if parts.next().is_none() || sha8.len() != 8 {
        return Err("malformed @@ZZT header in source".into());
    }
    let tag_start = at + marker.len() + program.len() + 1;
    let mut placeholder = source.to_string();
    placeholder.replace_range(tag_start..tag_start + 8, SHA8_PLACEHOLDER);
    let want = &sha256_hex(placeholder.as_bytes())[..8];
    if want != sha8 {
        return Err(format!(
            "program tag {sha8} does not match source (expected {want})"
        ));
    }
    Ok((program, sha8))
}

/// One case result as observed on the device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseEvidence {
    /// The whole record line exactly as observed (empty when the record was
    /// missing or the evidence was converted from a v1 file).
    pub raw: String,
    pub outcome: Outcome,
}

/// A parsed result transcript (PRINT screenshot text or note text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedRun {
    pub program: String,
    pub sha8: String,
    /// `HAngle`/`HFormat`/`HDigits` as `STRING()` text at program start.
    pub settings: BTreeMap<String, String>,
    /// By case id. A case listed in the program but absent from a complete
    /// transcript is [`Outcome::Missing`] with an empty `raw`.
    pub results: BTreeMap<String, CaseEvidence>,
    /// `(case id, setting)` for every `SETCHG` record.
    pub setting_changes: Vec<(String, String)>,
}

/// Parses result lines produced by a generated program.
///
/// Text before the header line is ignored (e.g. other terminal output);
/// text after `@@END` is ignored. Between them every non-blank line must be
/// a well-formed record for a short id of this program, else `Err`. A
/// transcript without `@@END` is rejected as truncated.
pub fn parse_result_lines(
    text: &str,
    gen_short_ids: &[(String, String)],
) -> Result<ParsedRun, String> {
    let ids: BTreeMap<&str, &str> = gen_short_ids
        .iter()
        .map(|(s, c)| (s.as_str(), c.as_str()))
        .collect();
    let mut lines = text.split('\n').map(|l| {
        l.trim_end_matches(['\r', '\0'])
            .trim_start_matches('\u{feff}')
    });
    let header = loop {
        match lines.next() {
            Some(l) => {
                if let Some(at) = l.find("@@ZZT|") {
                    break l[at + "@@ZZT|".len()..].trim_end().to_string();
                }
            }
            None => return Err("no @@ZZT header line found".into()),
        }
    };
    let h: Vec<&str> = header.split('|').collect();
    if h.len() != 5 {
        return Err(format!("malformed header `@@ZZT|{header}`"));
    }
    let mut run = ParsedRun {
        program: h[0].to_string(),
        sha8: h[1].to_string(),
        settings: CHECKED_SETTINGS
            .iter()
            .zip(&h[2..])
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        results: BTreeMap::new(),
        setting_changes: Vec::new(),
    };
    let mut complete = false;
    for (n, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if line.trim() == "@@END" {
            complete = true;
            break;
        }
        let mut parts = line.splitn(3, '|');
        let short = parts.next().unwrap_or_default().trim();
        let Some(&case_id) = ids.get(short) else {
            return Err(format!(
                "line {} after header: unknown record `{line}`",
                n + 1
            ));
        };
        let kind = parts
            .next()
            .ok_or_else(|| format!("record `{line}` has no type field"))?
            .trim();
        let rest = parts.next();
        if kind == "SETCHG" {
            run.setting_changes.push((
                case_id.to_string(),
                rest.unwrap_or_default().trim().to_string(),
            ));
            continue;
        }
        let outcome = if kind == "E" {
            Outcome::Error {
                detail: String::new(),
            }
        } else {
            let ty = kind
                .parse::<i64>()
                .map_err(|_| format!("bad type field in record `{line}`"))?;
            Outcome::Value {
                ty,
                text: rest
                    .ok_or_else(|| format!("record `{line}` has no text field"))?
                    .to_string(),
            }
        };
        let ev = CaseEvidence {
            raw: line.to_string(),
            outcome,
        };
        if run.results.insert(case_id.to_string(), ev).is_some() {
            return Err(format!("duplicate record for `{short}` ({case_id})"));
        }
    }
    if !complete {
        return Err("transcript is truncated (no @@END terminator)".into());
    }
    for (_, case_id) in gen_short_ids {
        run.results
            .entry(case_id.clone())
            .or_insert_with(|| CaseEvidence {
                raw: String::new(),
                outcome: Outcome::Missing,
            });
    }
    Ok(run)
}

/// Runs a [`Channel::Print`] program in the emulator (headless) and returns
/// the terminal text, one line per `PRINT`. This is a dry run of what the
/// calculator should show; it never touches a device.
pub fn emulate_print_program(g: &GeneratedProgram) -> Result<String, String> {
    if g.channel != Channel::Print {
        return Err(
            "only Print-channel programs can run in the emulator (Notes is not emulated)".into(),
        );
    }
    let file = ppl_syntax::SourceFile::new(format!("<{}>", g.program), g.source.clone());
    let (prog, diags) =
        ppl_compiler::compile_source(&file, &ppl_compiler::CompileOptions::default());
    let prog = prog.ok_or_else(|| {
        diags
            .errors()
            .next()
            .map(|d| d.message.clone())
            .unwrap_or_default()
    })?;
    let mut m = ppl_vm::Machine::new(
        Rc::new(prog),
        Box::new(ppl_vm::HeadlessHost::new()),
        ppl_vm::Settings::default(),
        ppl_vm::Limits {
            max_steps: Some(5_000_000 * (g.short_ids.len() as u64 + 1)),
            ..ppl_vm::Limits::default()
        },
    );
    m.call_function(&g.program, Vec::new())
        .map_err(|e| format!("program failed in the emulator: {e}"))?;
    Ok(m.terminal.join("\n"))
}
