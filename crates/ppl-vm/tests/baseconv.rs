//! End-to-end regression: the user's Baseconv converter (bases 2–16,
//! fractions) through compile, INPUT dialogs, MSGBOX and string commands.
//!
//! Three fixtures under `tests/fixtures/baseconv/`:
//! * `Baseconv_fixed`: the program as it should behave (expected table);
//! * `Baseconv_corrected`: the user's corrected source, kept verbatim — its
//!   one remaining source defect (a spurious ".0") is pinned below;
//! * `Baseconv_original`: the first version, kept verbatim — its source
//!   defects (untyped list INPUT, `temp(1)` on a string) are pinned below.
//!
//! Results come from the emulator's 12-digit decimal arithmetic; the
//! program itself is finite-precision (inputs above 12 significant digits
//! are rounded before conversion). UNVERIFIED on G1 end to end.

mod common;

use common::{Run, text};
use ppl_vm::{ErrorKind, ScriptEvent};

const FIXED: &str = include_str!("../../../tests/fixtures/baseconv/Baseconv_fixed.hpprgm");
const CORRECTED: &str = include_str!("../../../tests/fixtures/baseconv/Baseconv_corrected.hpprgm");
const ORIGINAL: &str = include_str!("../../../tests/fixtures/baseconv/Baseconv_original.hpprgm");

/// Runs the fixed converter with scripted dialog answers.
fn convert(inputs: &[&str]) -> (String, Run) {
    let mut r = Run::new("Baseconv_fixed", FIXED);
    let v = r
        .call("Baseconv", inputs)
        .unwrap_or_else(|e| panic!("{inputs:?}: {e}"));
    assert_eq!(r.unused_inputs(), 0, "{inputs:?} left answers unused");
    (text(&v), r)
}

fn msgboxes(r: &Run) -> Vec<String> {
    r.dialogs()
        .into_iter()
        .filter(|d| d.starts_with("MSGBOX"))
        .collect()
}

#[test]
fn conversion_table() {
    // (number, from, to, precision, result): the user's required cases
    // plus edge cases. Every answer goes through a typed `[2]` field, so
    // hex digits are never evaluated.
    for (n, from, to, prec, want) in [
        ("1010", "2", "10", "10", "10"),
        ("255", "10", "16", "10", "FF"),
        ("FF", "16", "10", "10", "255"),
        ("A3", "16", "2", "10", "10100011"),
        ("7B.8", "16", "2", "10", "01111011.1000"),
        ("10.5", "10", "2", "10", "1010.1"),
        ("0", "10", "2", "10", "0"),
        // Lowercase, sign, leading zeros, missing integer / fraction part.
        ("-ff", "16", "10", "10", "-255"),
        ("0010", "2", "10", "10", "2"),
        ("1.", "10", "2", "10", "1"),
        (".5", "10", "2", "10", "0.1"),
        // Minimum and maximum bases.
        ("1", "2", "2", "10", "1"),
        ("FF", "16", "16", "10", "FF"),
        // Precision is truncated and clamped to 1..30.
        ("0.1", "10", "2", "0", "0.0"),
        ("0.1", "10", "2", "2.7", "0.00"),
        ("0.1", "10", "2", "99", "0.000110011001100110011001100110"),
        // Repeating expansions stop at the precision.
        ("0.1", "10", "3", "30", "0.002200220022002200220022002200"),
        // 1/3 has 12 significant digits in decimal; the expansion ends there.
        ("0.1", "3", "10", "30", "0.333333333333"),
        // Largest exact integer, then values rounded to 12 digits
        // (a limit of the program's real arithmetic, not of the VM).
        ("999999999999", "10", "16", "10", "E8D4A50FFF"),
        ("9999999999999", "10", "16", "10", "9184E72A000"),
        ("FFFFFFFFFFFFFFFF", "16", "10", "10", "18446744073800000000"),
    ] {
        let (got, r) = convert(&[n, from, to, prec]);
        assert_eq!(got, want, "{n} base {from} -> {to} (precision {prec})");
        assert!(msgboxes(&r).is_empty(), "{n}: {:?}", r.dialogs());
        assert!(
            r.output().contains(&format!("Final Result: {want}")),
            "{n}: {:?}",
            r.output()
        );
    }
}

#[test]
fn invalid_numbers_are_rejected_with_a_message() {
    // (answers, message): the program stops after one MSGBOX.
    for (inputs, msg) in [
        (&["19", "8", "10", "10"][..], "Invalid digit: 9"),
        (&["G1", "16", "10", "10"], "Invalid digit: G"),
        (&["Z", "10", "2", "10"], "Invalid digit: Z"),
        (&["1.F", "10", "2", "10"], "Invalid fractional digit: F"),
        (&[""], "Please enter a number."),
        (&["."], "Invalid number."),
        (&["-"], "Invalid number."),
        (&["1.2.3"], "Invalid number: multiple decimal points."),
    ] {
        let (got, r) = convert(inputs);
        assert_eq!(got, "0", "{inputs:?}");
        assert_eq!(msgboxes(&r), vec![format!("MSGBOX: {msg}")], "{inputs:?}");
        assert!(!r.output().iter().any(|l| l.starts_with("Final Result")));
    }
}

#[test]
fn invalid_bases_prompt_again() {
    let (got, r) = convert(&["FF", "1", "17", "2.5", "16", "0", "-3", "10", "10"]);
    assert_eq!(got, "255");
    assert_eq!(msgboxes(&r).len(), 5, "{:?}", r.dialogs());
    // A base that does not parse re-opens the dialog without a MSGBOX.
    let (got, r) = convert(&["FF", "abc", "16", "10", "10"]);
    assert_eq!(got, "255");
    assert!(r.dialogs().iter().any(|d| d.starts_with("INPUT-ERROR")));
}

#[test]
fn cancel_at_each_dialog_ends_cleanly() {
    for answers in [
        &["<ESC>"][..],
        &["FF", "<ESC>"],
        &["FF", "16", "<ESC>"],
        &["FF", "16", "10", "<ESC>"],
    ] {
        let (got, r) = convert(answers);
        assert_eq!(got, "0", "{answers:?}");
        assert!(msgboxes(&r).is_empty());
        assert!(!r.output().iter().any(|l| l.starts_with("Final Result")));
    }
}

#[test]
fn runs_again_without_stale_state() {
    let mut r = Run::new("Baseconv_fixed", FIXED);
    let first = r.call("Baseconv", &["7B.8", "16", "2", "10"]).unwrap();
    assert_eq!(text(&first), "01111011.1000");
    let cancelled = r.call("Baseconv", &["FF", "<ESC>"]).unwrap();
    assert_eq!(text(&cancelled), "0");
    let again = r.call("Baseconv", &["1010", "2", "10", "10"]).unwrap();
    assert_eq!(text(&again), "10");
    // PRINT() at the start clears the terminal, so only this run's lines.
    assert_eq!(
        r.machine.terminal.first().map(String::as_str),
        Some("=== BASE CONVERTER ===")
    );
}

#[test]
fn corrected_source_keeps_its_trailing_zero_defect() {
    // Source defect, not an emulator bug: step 12 runs with no fractional
    // part and appends "0" before breaking.
    for (inputs, want) in [
        (&["1010", "2", "10", "10"][..], "10.0"),
        (&["255", "10", "16", "10"], "FF.0"),
        (&["0", "10", "2", "10"], "0.0"),
        // The hex → binary path and real fractions are unaffected.
        (&["A3", "16", "2", "10"], "10100011"),
        (&["10.5", "10", "2", "10"], "1010.1"),
    ] {
        let mut r = Run::new("Baseconv_corrected", CORRECTED);
        let v = r.call("Baseconv", inputs).unwrap();
        assert_eq!(text(&v), want, "{inputs:?}");
    }
}

#[test]
fn original_source_defects_are_reported_at_run_time() {
    // INPUT(temp) on a list variable is an untyped field: `FF` is an
    // undeclared name (re-prompt), and a typed string makes `temp(1)` the
    // first character's code, which UPPER rejects.
    let mut r = Run::new("Baseconv_original", ORIGINAL);
    let e = r.call("Baseconv", &["FF", "\"FF\""]).unwrap_err();
    assert_eq!(e.kind, ErrorKind::BadArgType, "{e}");
    assert!(r.dialogs().iter().any(|d| d.starts_with("INPUT-ERROR")));

    // Entering the list the field expects works; the program then waits
    // for Esc (ISKEYDOWN(4)) and one more key (WAIT(-1)).
    let mut r = Run::new("Baseconv_original", ORIGINAL);
    r.host
        .borrow_mut()
        .keys
        .extend([ScriptEvent::Key(4), ScriptEvent::Key(4)]);
    r.call("Baseconv", &["{\"FF\"}", "16", "10", "10"]).unwrap();
    assert!(
        r.output()
            .contains(&"Final Result = 255.0 (Base 10)".to_string()),
        "{:?}",
        r.output()
    );
}
