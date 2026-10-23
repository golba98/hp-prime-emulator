//! Bounded fixed-mailbox/FC proof of concept, not an execution backend.
use super::device::DeviceOpts;
use super::remote::{Failure, Result, control, evidence, input, refuse_pending};
use super::remote_state::Store;
use ppl_device::{FileType, Prime, ReadBack};
use ppl_graphics::Grob;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn source(program: &str, note: &str, prefix: &str) -> Result<String> {
    ppl_device::validate_test_name(program)?;
    ppl_device::validate_test_name(note)?;
    let (stop_name, stop_body) =
        ppl_device::listener::stop_helper(program.trim_start_matches("ZZT_"))?;
    Ok(format!(
        r#"{stop_body}
EXPORT {program}()
BEGIN
  LOCAL zzmsg,zzlast,zzdata,zzvalid,zzj,zzb;
  LOCAL zzv,zzbit,zzidx,zzx,zzy,zzcolor;
  REPEAT zzv:=GETKEY; UNTIL zzv==-1;
  zzlast:="";
  RECT_P(0,0,320,240,16777215);
  TEXTOUT_P("Fixed mailbox test",12,16,3,0);
  TEXTOUT_P("Foreground mode. Esc stops.",12,48,1,0);
  REPEAT
    zzmsg:=Notes("{note}");
    IF DIM(zzmsg)=={total} AND LEFT(zzmsg,{prefix_len})=="{prefix}" THEN
      zzdata:=MID(zzmsg,{start},32); zzvalid:=1;
      IF zzdata<>zzlast THEN
        FOR zzj FROM 1 TO 32 DO
          IF INSTRING("0123456789ABCDEF",MID(zzdata,zzj,1))==0 THEN zzvalid:=0; END;
        END;
        IF zzvalid THEN
          zzlast:=zzdata; zzdata:="C3A5"+zzdata;
          FOR zzj FROM 1 TO 36 DO
            zzv:=INSTRING("0123456789ABCDEF",MID(zzdata,zzj,1))-1;
            FOR zzb FROM 0 TO 3 DO
              zzidx:=(zzj-1)*4+zzb;
              zzx:=16+12*(zzidx MOD 16); zzy:=88+12*IP(zzidx/16);
              zzbit:=IP(zzv/(2^zzb)) MOD 2;
              zzcolor:=16777215; IF zzbit THEN zzcolor:=0; END;
              RECT_P(zzx,zzy,zzx+9,zzy+9,zzcolor);
            END;
          END;
        END;
      END;
    END;
    zzmsg:=""; zzdata:="";
  UNTIL {stop_name}();
  RETURN 0;
END;
"#,
        total = prefix.len() + 32,
        prefix_len = prefix.len(),
        start = prefix.len() + 1
    ))
}

/// Exact black/white cells, with a fixed magic and a fresh 128-bit nonce.
/// This is screenshot transport; no OCR or simulated result is accepted.
fn decode(grob: &Grob) -> Option<String> {
    if grob.width() != 320 || grob.height() != 240 {
        return None;
    }
    let mut value = String::new();
    for nibble in 0..36 {
        let mut digit = 0;
        for bit in 0..4 {
            let i = nibble * 4 + bit;
            let color = grob.get(16 + 12 * (i % 16) + 4, 88 + 12 * (i / 16) + 4)?;
            let (r, g, b) = ppl_graphics::channels(color);
            if r < 32 && g < 32 && b < 32 {
                digit |= 1 << bit;
            } else if r < 224 || g < 224 || b < 224 {
                return None;
            }
        }
        value.push(char::from_digit(digit, 16)?.to_ascii_uppercase());
    }
    value.strip_prefix("C3A5").map(str::to_string)
}

fn wait_frame(
    prime: &mut Prime,
    store: &Store,
    expected: &str,
    seconds: u64,
    label: &str,
    out: &mut Value,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        let png = prime.screenshot(8)?;
        let path = store.dir.join(format!("fixed-{label}-last.raw.png"));
        std::fs::write(&path, &png)?;
        let image = Grob::from_prime_screenshot(&png).map_err(|message| Failure {
            code: 9,
            kind: "invalid_evidence",
            message,
        })?;
        if decode(&image).as_deref() == Some(expected) {
            evidence(out, path);
            out["last_screen_nonce"] = json!(expected);
            return Ok(());
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(5)),
        );
    }
    Err(Failure {
        code: 7,
        kind: "timeout",
        message: format!(
            "no matching {label} frame; an owned-note write alone is not confirmation"
        ),
    })
}

pub(super) fn run(
    prime: &mut Prime,
    store: &Store,
    opts: &DeviceOpts,
    out: &mut Value,
    resume: Option<&str>,
) -> Result<()> {
    refuse_pending(store)?;
    if let Some(program) = resume {
        return run_resumed(prime, store, opts, out, program);
    }
    let program = ppl_device::fresh_test_name();
    let session = program.trim_start_matches("ZZT_");
    let note = format!("{program}M");
    let token = ppl_device::remote::Request::new(session, "", true)?.nonce;
    let prefix = format!("@@PPLMAIL|{token}|");
    let initial = ppl_device::remote::Request::new(session, "", true)?.nonce;
    let source = source(&program, &note, &prefix)?;
    super::remote_input::declarations(&source).map_err(input)?;
    std::fs::create_dir_all(&store.dir)?;
    let path = store.dir.join(format!("fixed-{program}.hpprgm"));
    std::fs::write(&path, &source)?;
    evidence(out, path);
    out["program"] = json!(program);
    out["mailbox_note"] = json!(note);
    let manifest = store.dir.join(format!("fixed-{program}.json"));
    store.save(&manifest,&json!({"schema_version":1,"program":program,"mailbox_note":note,"source_sha256":ppl_compat::sha256_hex(source.as_bytes()),"status":"prepared","execution_confirmed":false}))?;
    evidence(out, manifest.clone());
    out["source_sha256"] = json!(ppl_compat::sha256_hex(source.as_bytes()));
    let result = (|| {
        eprintln!("preflighting fresh fixed-mailbox program {program}");
        let guard = prime.preflight_new_names(&[(FileType::Program, &program)], &control())?;
        let upload = prime.send_new_program(guard, &source)?;
        if upload.readback != ReadBack::Matches {
            return Err(Failure {
                code: 9,
                kind: "upload_unverified",
                message: "fixed receiver upload unverified; nothing will be run automatically"
                    .into(),
            });
        }
        out["uploaded"] = json!(true);
        out["upload_state"] = json!("verified");
        let guard = prime.preflight_new_names(&[(FileType::Note, &note)], &control())?;
        let owned = prime.create_owned_note(guard, &format!("{prefix}{initial}"))?;
        out["probe_state"] = json!("waiting_for_start");
        store.save(&manifest, out)?;
        out["initial_nonce"] = json!(initial);
        store.save(&manifest, out)?;
        exchange(prime, store, opts, out, &initial, &owned, &prefix)?;
        out["execution_state"] = json!("fixed_mailbox_echo_verified");
        // An echo is not PPL expression/program invocation. It does not enable
        // any production execution capability or populate execution_confirmed.
        eprintln!("Two live screen echoes verified. Stop the fixed receiver with Esc.");
        Ok(())
    })();
    if let Err(error) = &result {
        out["probe_error"] = json!({"kind":error.kind,"message":error.message});
    }
    store.save(&manifest, out)?;
    result
}

fn exchange(
    prime: &mut Prime,
    store: &Store,
    opts: &DeviceOpts,
    out: &mut Value,
    initial: &str,
    owned: &ppl_device::ownership::OwnedNote,
    prefix: &str,
) -> Result<()> {
    let program = out["program"]
        .as_str()
        .ok_or_else(|| input("missing fixed receiver name"))?
        .to_string();
    let session = program.trim_start_matches("ZZT_");
    let attempt = ppl_device::fresh_test_name();
    let manifest = store.dir.join(format!("fixed-{program}.json"));
    eprintln!(
        "READY FOR MANUAL START: Run {program} once; it will show Fixed mailbox test. Host waits up to 600 seconds."
    );
    wait_frame(
        prime,
        store,
        initial,
        600,
        &format!("{session}-{attempt}-ready"),
        out,
    )?;
    for number in 1..=2 {
        let nonce = ppl_device::remote::Request::new(session, "", true)?.nonce;
        out["probe_state"] = json!(format!("sending_echo_{number}"));
        out["expected_screen_nonce"] = json!(nonce);
        out["note_update_sent"] = json!(false);
        out[format!("echo_{number}_confirmed")] = json!(false);
        store.save(&manifest, out)?;
        prime.update_owned_note(owned, &format!("{prefix}{nonce}"))?;
        out["note_update_sent"] = json!(true);
        store.save(&manifest, out)?;
        wait_frame(
            prime,
            store,
            &nonce,
            opts.remote_timeout_secs,
            &format!("{session}-{attempt}-echo-{number}"),
            out,
        )?;
        out[format!("echo_{number}_confirmed")] = json!(true);
        store.save(&manifest, out)?;
    }
    Ok(())
}

fn run_resumed(
    prime: &mut Prime,
    store: &Store,
    opts: &DeviceOpts,
    out: &mut Value,
    program: &str,
) -> Result<()> {
    ppl_device::validate_test_name(program)?;
    let manifest = store.dir.join(format!("fixed-{program}.json"));
    let previous: Value = store.read(&manifest)?;
    if previous["device"] != out["device"]
        || previous["uploaded"] != true
        || previous["program"] != program
    {
        return Err(input(
            "fixed receiver creation record does not match this device",
        ));
    }
    let local = std::fs::read_to_string(store.dir.join(format!("fixed-{program}.hpprgm")))?;
    if previous["source_sha256"] != ppl_compat::sha256_hex(local.as_bytes()) {
        return Err(input("fixed receiver source hash mismatch"));
    }
    let prefix = local
        .split_once("==\"@@PPLMAIL|")
        .and_then(|(_, tail)| tail.split_once('"'))
        .map(|(tag, _)| format!("@@PPLMAIL|{tag}"))
        .ok_or_else(|| input("fixed receiver mailbox tag missing"))?;
    let note = format!("{program}M");
    if prefix.len() != 43
        || !prefix.ends_with('|')
        || !prefix.as_bytes()[10..42].iter().all(u8::is_ascii_hexdigit)
        || source(program, &note, &prefix)? != local
    {
        return Err(input(
            "creation record is not the supported fixed receiver template",
        ));
    }
    match prime.request_file(FileType::Program, program)? {
        ppl_device::Probe::Present(file)
            if file.name == program
                && file.ty == FileType::Program
                && file.crc_ok
                && ppl_device::normalize_program_text(&file.text())
                    == ppl_device::normalize_program_text(&local) => {}
        _ => return Err(Failure {
            code: 8,
            kind: "receiver_busy_or_changed",
            message:
                "stop the fixed receiver with Esc before resuming; its source could not be verified"
                    .into(),
        }),
    }
    let expected = if previous["note_update_sent"] == true {
        previous["expected_screen_nonce"].as_str()
    } else {
        previous["initial_nonce"].as_str()
    }
    .unwrap_or("00000000000000000000000000000000");
    if expected.len() != 32 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(input("invalid recorded mailbox nonce"));
    }
    let owned = prime.reopen_owned_note(&note, &format!("{prefix}{expected}"))?;
    store.save(&manifest.with_extension("previous.json"), &previous)?;
    *out = previous;
    out["error"] = Value::Null;
    out["probe_error"] = Value::Null;
    out["resumed"] = json!(true);
    let initial =
        ppl_device::remote::Request::new(program.trim_start_matches("ZZT_"), "", true)?.nonce;
    out["initial_nonce"] = json!(initial);
    out["note_update_sent"] = json!(false);
    out["probe_state"] = json!("refreshing_ready_nonce");
    store.save(&manifest, out)?;
    let result = (|| {
        prime.update_owned_note(&owned, &format!("{prefix}{initial}"))?;
        match prime.request_file(FileType::Note, &note)? {
            ppl_device::Probe::Present(file)
                if file.name == note
                    && file.ty == FileType::Note
                    && file.crc_ok
                    && file.text() == format!("{prefix}{initial}") => {}
            _ => {
                return Err(Failure {
                    code: 9,
                    kind: "upload_unverified",
                    message: "ready nonce update unverified; do not start the receiver".into(),
                });
            }
        }
        out["probe_state"] = json!("waiting_for_start");
        store.save(&manifest, out)?;
        exchange(prime, store, opts, out, &initial, &owned, &prefix)?;
        out["execution_state"] = json!("fixed_mailbox_echo_verified");
        eprintln!("Two live screen echoes verified. Stop the fixed receiver with Esc.");
        Ok(())
    })();
    if let Err(error) = &result {
        out["probe_error"] = json!({"kind":error.kind,"message":error.message});
    }
    store.save(&manifest, out)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_frame_requires_magic_and_all_nonce_bits() {
        let nonce = "0123456789ABCDEFFEDCBA9876543210";
        let mut g = Grob::screen();
        for (nibble, c) in format!("C3A5{nonce}").chars().enumerate() {
            for bit in 0..4 {
                let i = (nibble * 4 + bit) as i64;
                let color = if c.to_digit(16).unwrap() & (1 << bit) != 0 {
                    0
                } else {
                    0xffffff
                };
                g.fill_rect(
                    16 + 12 * (i % 16),
                    88 + 12 * (i / 16),
                    25 + 12 * (i % 16),
                    97 + 12 * (i / 16),
                    color,
                );
            }
        }
        assert_eq!(decode(&g).as_deref(), Some(nonce));
        g.fill_rect(16, 88, 25, 97, 0);
        assert!(decode(&g).is_none());
        assert!(decode(&Grob::new(1, 1, 0)).is_none());
    }
    #[test]
    fn receiver_is_small_valid_ppl_with_no_catalog_scan_or_note_writes() {
        let s = source("ZZT_TEST", "ZZT_TESTM", "@@PPLMAIL|ABC|").unwrap();
        super::super::remote_input::declarations(&s).unwrap();
        assert!(!s.contains(":=Notes;"));
        assert!(!s.contains("Notes(\"ZZT_TESTM\"):="));
        assert!(s.contains("zzmsg:=\"\""));
    }
}
