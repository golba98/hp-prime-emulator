//! Case-specific evidence validation for self-authored M19 guests.
use super::BuildIdentity;
use serde_json::Value;

pub(super) fn runner(case: &str) -> Option<&'static str> {
    match case {
        "nand" | "nand_ecc" | "boot_chain" => Some("full_system_nand.py"),
        "clock_reset" => Some("full_system_clock.py"),
        "lcd" | "input" => Some("full_system_display.py"),
        _ => None,
    }
}
fn digest(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
fn touch(v: &Value, status: u8, x: u16, y: u16) -> bool {
    let Some(s) = v["touch_hex"].as_str() else {
        return false;
    };
    if s.len() != 40 || !s.is_ascii() {
        return false;
    }
    let Ok(bytes) = (0..40)
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    bytes[0] == status
        && (u16::from(bytes[1]) << 4 | u16::from(bytes[3] >> 4)) == x
        && (u16::from(bytes[2]) << 4 | u16::from(bytes[3] & 15)) == y
        && bytes[12..] == [255; 8]
        && v["version_hex"] == "06f9ffff"
}
pub(super) fn validate(d: &Value, b: &BuildIdentity, case: &str) -> Result<(), String> {
    let scope = match case {
        "nand" | "nand_ecc" | "boot_chain" => "full_system_synthetic_nand",
        "clock_reset" => "full_system_synthetic_clock_reset",
        _ => "full_system_synthetic_hardware",
    };
    if d["schema_version"] != 1
        || d["scope"] != scope
        || d["case"] != case
        || d["machine"] != "hp-prime"
        || d["cpu"] != "arm926"
        || d["reference_revision"] != b.reference_revision
        || d["binary_sha256"] != b.binary_sha256
        || d["image_id"] != b.image_id
        || d["physical_hardware_verified"] != false
        || d["kernel_boot_verified"] != false
    {
        return Err("M19 result identity or verification scope differs".into());
    }
    if d["status"] == "failed" {
        return Ok(());
    }
    if d["status"] != "passed"
        || d["guest_cpu_execution_verified"] != true
        || d["guest"]["executed"] != true
        || d["guest"]["linked"] != true
        || !digest(&d["guest"]["sha256"])
    {
        return Err("M19 successful result lacks executed guest identity".into());
    }
    let sources = if matches!(case, "lcd" | "input") {
        &d["guest"]["sources"]
    } else {
        &d["guest"]["source_sha256"]
    };
    let sources_valid = digest(sources)
        || sources
            .as_object()
            .is_some_and(|map| !map.is_empty() && map.values().all(digest));
    if !sources_valid {
        return Err("M19 guest lacks bounded source identities".into());
    }
    let o = &d["observations"];
    let entry = if case == "boot_chain" {
        0
    } else {
        0x30000000u64
    };
    let halt = o["halt_address"]
        .as_u64()
        .ok_or("missing terminal address")?;
    let range = if case == "boot_chain" {
        0x30010000..0x30020000
    } else {
        0x30000000..0x30010000
    };
    if o["initial_pc"] != entry
        || o["initial_mailbox"] != 0
        || o["stepped_pc"] != entry + 4
        || !range.contains(&halt)
        || halt % 4 != 0
        || o["stopped_pc"] != halt
        || o["cpsr"]
            .as_u64()
            .is_none_or(|v| v > u32::MAX as u64 || v & 0x3f != 0x13)
        || (0..13).any(|i| {
            o["registers"][format!("r{i}")]
                .as_u64()
                .is_none_or(|v| v > u32::MAX as u64)
        })
    {
        return Err("M19 result lacks actual guest entry/step/halt/register observations".into());
    }
    let w = o["mailbox"]
        .as_array()
        .ok_or("missing guest mailbox")?
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|n| *n <= u32::MAX as u64)
                .map(|n| n as u32)
                .ok_or("invalid mailbox word")
        })
        .collect::<Result<Vec<_>, _>>()?;
    if w.len() != 64 || w[0] != 0x600d0019 {
        return Err("M19 guest success mailbox differs".into());
    }
    let uart = o["uart_hex"]
        .as_str()
        .ok_or("missing captured guest UART")?;
    let passed = match case {
        "clock_reset" => {
            (95..=98).contains(&w[1])
                && (47..=50).contains(&w[2])
                && w[3] == 0
                && w[4] == 0xfeed0019
                && w[5..10] == [0x19abcdef, 0xc, 0, 0, 0x20]
                && w[10..12] == [0xc, 0]
                && uart == "435352595a"
        }
        "nand" => {
            let sum: u32 = (0..3)
                .flat_map(|p| (0..2112).map(move |i| (((i + p) * 37 + 11) & 255) as u32))
                .sum();
            w[2..7] == [0xec, 0xda, 0x10, 0x95, 0x44]
                && w[8..11] == [sum, 3, 2112]
                && w[11..14] == [0, 320, 255]
                && uart == "4d3139204e414e440a"
        }
        "nand_ecc" => {
            w[2] == 0x5f99a596
                && w[3] == 0
                && w[4] == 1 | (1 << 4) | (123 << 7)
                && w[5] == 65284
                && w[6..8] == [3, 2]
                && uart == "4d3139204543430a"
        }
        "boot_chain" => {
            d["guest"]["stage2_crc32"] == w[2]
                && d["guest"]["stage2_size"] == w[3]
                && (1..=2048).contains(&w[3])
                && w[4..6] == [0xface0019, 0x5a190002]
                && uart == "4d313920424f4f54320a"
        }
        "lcd" => {
            o["pixels_checked"] == 230400
                && o["frames"].as_object().is_some_and(|frames| {
                    ["frame0.ppm", "frame1.ppm", "reenabled.ppm"]
                        .iter()
                        .all(|n| frames.get(*n).is_some_and(digest))
                })
                && uart == "4449"
        }
        "input" => {
            let s = &o["input_samples"];
            let zero = serde_json::json!([0, 0, 0, 0, 0, 0, 0, 0]);
            ["idle", "all_up", "aliases_up", "reset", "cancelled"]
                .iter()
                .all(|n| s[*n]["matrix"] == zero)
                && s["p_down"]["matrix"] == serde_json::json!([2, 0, 0, 0, 0, 0, 0, 0])
                && s["two_keys"]["matrix"] == serde_json::json!([130, 0, 0, 0, 0, 0, 0, 0])
                && s["p_up"]["matrix"] == serde_json::json!([128, 0, 0, 0, 0, 0, 0, 0])
                && s["alias_still_down"]["matrix"] == serde_json::json!([128, 0, 0, 0, 0, 0, 0, 0])
                && touch(&s["touch_down"], 9, 225, 80)
                && touch(&s["touch_move"], 10, 337, 160)
                && touch(&s["touch_up"], 11, 337, 160)
                && touch(&s["touch_clamped"], 9, 0, 319)
                && uart == "4449"
        }
        _ => false,
    };
    if !passed {
        return Err("M19 guest observations disagree with case contract".into());
    }
    Ok(())
}

pub(super) fn required_overlay(case: &str) -> Option<&'static str> {
    match case {
        "nand" | "nand_ecc" | "boot_chain" => Some("0004-s3c2416-nand.patch"),
        "clock_reset" => Some("0005-s3c2416-clocks-reset.patch"),
        "lcd" | "input" => Some("0006-lcd-input.patch"),
        _ => None,
    }
}

pub(super) fn validate_artifacts(
    d: &Value,
    root: &std::path::Path,
    case: &str,
) -> Result<(), String> {
    use super::regular_digest;
    if d["status"] != "passed" {
        return Ok(());
    }
    let elf = match case {
        "lcd" | "input" => "display.elf",
        "clock_reset" => "hardware.elf",
        _ => "nand.elf",
    };
    if regular_digest(&root.join("guest").join(elf), 1024 * 1024)? != d["guest"]["sha256"] {
        return Err("captured guest executable identity differs".into());
    }
    let nand = matches!(case, "nand" | "nand_ecc" | "boot_chain");
    let uart_path = if nand {
        root.join("guest_run/uart.log")
    } else {
        root.join("uart.log")
    };
    regular_digest(&uart_path, 65536)?;
    let uart = std::fs::read(uart_path).map_err(|e| e.to_string())?;
    if nand {
        let flash = &d["flash"];
        if flash["bytes"] != 276824064
            || flash["disposable"] != true
            || flash["format"] != "interleaved_main_oob"
            || !digest(&flash["base_sha256"])
            || flash["base_sha256"] != flash["base_after_sha256"]
            || regular_digest(&root.join("base.raw"), 276824064)? != flash["base_sha256"]
        {
            return Err("synthetic NAND base identity or isolation differs".into());
        }
    }
    if case == "boot_chain"
        && regular_digest(&root.join("guest/stage2.elf"), 1024 * 1024)?
            != d["guest"]["stage2_sha256"]
    {
        return Err("synthetic second-stage executable identity differs".into());
    }
    let hex: String = uart.iter().map(|b| format!("{b:02x}")).collect();
    if d["observations"]["uart_hex"] != hex {
        return Err("actual UART artifact differs".into());
    }
    if case == "lcd" {
        for (name, second) in [
            ("frame0.ppm", false),
            ("frame1.ppm", true),
            ("reenabled.ppm", true),
        ] {
            if regular_digest(&root.join(name), 256 * 1024)? != d["observations"]["frames"][name] {
                return Err("frame identity differs".into());
            }
            let data = std::fs::read(root.join(name)).map_err(|e| e.to_string())?;
            let header = b"P6\n320 240\n255\n";
            if !data.starts_with(header) || data.len() != header.len() + 320 * 240 * 3 {
                return Err("frame geometry differs".into());
            }
            for (index, rgb) in data[header.len()..].chunks_exact(3).enumerate() {
                let (x, y) = (index % 320, index / 320);
                let (mut r, mut g, mut b) = ((x / 8) % 32, (y / 4) % 64, ((x ^ y) / 8) % 32);
                if second {
                    r = 31 - r;
                    g = 63 - g;
                    b = 31 - b;
                    if (x, y) == (17, 23) {
                        r = 31;
                        g = 0;
                        b = 31;
                    }
                }
                if rgb
                    != [
                        (r * 8 + r / 4) as u8,
                        (g * 4 + g / 16) as u8,
                        (b * 8 + b / 4) as u8,
                    ]
                {
                    return Err(format!("captured frame pixel differs at {x},{y}"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_nand_and_clock_contracts_reject_corrupt_success() {
        let nand: Value = serde_json::from_str(include_str!(
            "../../../../research/full-system/evidence/m19-nand.json"
        ))
        .unwrap();
        let clock: Value = serde_json::from_str(include_str!(
            "../../../../research/full-system/evidence/m19-clocks.json"
        ))
        .unwrap();
        let mut runs = nand["runs"].as_array().unwrap().clone();
        runs.extend(
            clock["runs"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["result"]["case"] == "clock_reset")
                .map(|r| r["result"].clone()),
        );
        assert_eq!(runs.len(), 6);
        for d in runs {
            let case = d["case"].as_str().unwrap();
            let b = BuildIdentity {
                binary_sha256: d["binary_sha256"].as_str().unwrap().into(),
                image_id: d["image_id"].as_str().unwrap().into(),
                image: String::new(),
                reference_revision: super::super::QEMU_REVISION.into(),
            };
            validate(&d, &b, case).unwrap();
            let mut bad = d.clone();
            bad["observations"]["mailbox"][2] = 0.into();
            assert!(
                validate(&bad, &b, case).is_err(),
                "{case} accepted a corrupt observation"
            );
            let mut bad = d.clone();
            bad["observations"]["cpsr"] = 0.into();
            assert!(validate(&bad, &b, case).is_err());
        }
    }
    #[test]
    fn retained_guest_observations_reject_false_execution_and_input() {
        let evidence: Value = serde_json::from_str(include_str!(
            "../../../../research/full-system/evidence/m19-display-input.json"
        ))
        .unwrap();
        let mut count = 0;
        for d in evidence["captures"].as_array().unwrap() {
            if d["observations"]["uart_hex"] != "4449" {
                continue;
            }
            let case = d["case"].as_str().unwrap();
            let b = BuildIdentity {
                binary_sha256: d["binary_sha256"].as_str().unwrap().into(),
                image_id: d["image_id"].as_str().unwrap().into(),
                image: String::new(),
                reference_revision: super::super::QEMU_REVISION.into(),
            };
            validate(d, &b, case).unwrap();
            for field in ["stepped_pc", "initial_pc", "stopped_pc", "cpsr"] {
                let mut bad = d.clone();
                bad["observations"][field] = Value::Null;
                assert!(validate(&bad, &b, case).is_err(), "{case}: {field}");
            }
            let mut bad = d.clone();
            bad["kernel_boot_verified"] = Value::Bool(true);
            assert!(validate(&bad, &b, case).is_err());
            let mut bad = d.clone();
            bad["observations"]["uart_hex"] = "".into();
            assert!(validate(&bad, &b, case).is_err());
            if case == "input" {
                let mut bad = d.clone();
                bad["observations"]["input_samples"]["reset"]["matrix"][0] = 2.into();
                assert!(validate(&bad, &b, case).is_err());
            }
            count += 1;
        }
        assert_eq!(count, 5);
    }
}
