#![cfg(target_os = "linux")]
//! Direct original-firmware Check (M9) against stored firmware results from
//! tests/diagnostics/reference. Needs PRIME_TEST_FIRMWARE, PRIME_TEST_RUNNER
//! and PRIME_TEST_INIT_DATA; skips when the firmware is unset.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ppl_engine::{Cancellation, direct_check::DirectChecker, firmware};

fn options() -> Option<firmware::Options> {
    let Some(fw) = std::env::var_os("PRIME_TEST_FIRMWARE").filter(|s| !s.is_empty()) else {
        eprintln!("SKIP M9: PRIME_TEST_FIRMWARE unset");
        return None;
    };
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    Some(firmware::Options {
        firmware: Some(PathBuf::from(fw)),
        runner: Some(var("PRIME_TEST_RUNNER").expect("M9 requires PRIME_TEST_RUNNER")),
        init_data: Some(var("PRIME_TEST_INIT_DATA").expect("M9 requires PRIME_TEST_INIT_DATA")),
        os_rom: var("PRIME_TEST_OS_ROM"),
        factory_a: var("PRIME_TEST_FACTORY_A"),
        ..Default::default()
    })
}

/// (verdict accepted, line, column, UTF-16 offset) of a direct Check.
fn check(c: &DirectChecker, source: &str) -> (bool, Option<u32>, Option<u32>, Option<u64>) {
    let v = c
        .check(source, Duration::from_secs(30), &Cancellation::default())
        .unwrap();
    let d = v.diagnostics.first();
    if !v.accepted {
        assert_eq!(
            d.and_then(|d| d.message.as_deref()),
            Some("Error: Syntax Error")
        );
    }
    (
        v.accepted,
        d.and_then(|d| d.line),
        d.and_then(|d| d.column),
        d.and_then(|d| d.raw_position.as_ref())
            .and_then(|p| p["offset_utf16"].as_u64()),
    )
}

#[test]
fn direct_checks_reproduce_stored_firmware_results_repeatedly() {
    let Some(o) = options() else {
        return;
    };
    // `start` already requires the canary (v01, e12) to match exactly.
    let c = DirectChecker::start(&o).unwrap();
    let unicode =
        "export DPK24()\nbegin\n local θ:=π; if θ≥3 then return θ; end;\n return 0;\nend;\n";
    let implicit = "EXPORT DPIMPL()\nBEGIN\n LOCAL x:=3;\n RETURN 2x;\nEND;\n";
    let string = "EXPORT DPSTR()\nBEGIN\n RETURN \"abc;\nEND;\n";
    let mut warm = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        assert_eq!(check(&c, implicit), (false, Some(4), Some(10), Some(44)));
        warm.push(started.elapsed());
        assert_eq!(check(&c, unicode), (true, None, None, None));
        assert_eq!(check(&c, string), (false, Some(5), Some(1), Some(40)));
    }
    warm.sort();
    eprintln!("M9 cold {:?}, warm median {:?}", c.cold_start, warm[1]);
    assert!(
        warm[1] < Duration::from_secs(1),
        "warm direct Check {:?}",
        warm[1]
    );
}
