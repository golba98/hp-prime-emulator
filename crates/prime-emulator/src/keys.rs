//! Calculator key names → device key codes, for scripted input.
//!
//! Source: PrimU2, two independent tables that agree on every entry used
//! here — `mcp_server.py` (key-name table of its automation interface) and
//! `PrimU/LCD.cpp` (`vk_to_device_keymap`, the code sent by its keyboard
//! handler). `LCD.cpp`'s *comments* label 0xB7/0xB9 the wrong way round;
//! its code and the MCP table both map `+` to 0xB9 and `-` to 0xB7.
//!
//! Firmware cross-check (M4): every code below has an entry in the
//! firmware's own device-code table (192 signed bytes at `0x30C09B7C`,
//! read by `0x30606FC0`; 0xE047 is special-cased to 5), and each maps to
//! the physical key of the same name in the documented 0–50 numbering
//! (e.g. 0x5A → 43 "2", 0xB9 → 50 "+", 0x0D → 30 Enter, 0xE047 → 5 Home).
//! [`KEYS`] additionally marks the keys whose effect M4 observed on the
//! firmware's own screen (typing `2+2`, `4`, `2+3` and ENTER on Home);
//! docs/emulator/M4_FIRMWARE_DISPLAY_REPORT.md.

/// `(name, device code, effect observed on the firmware screen in M4)`.
pub const KEYS: &[(&str, u16, bool)] = &[
    ("0", 0x30, false),
    ("1", 0x59, false),
    ("2", 0x5A, true),
    ("3", 0x33, true),
    ("4", 0x55, true),
    ("5", 0x56, false),
    ("6", 0x57, false),
    ("7", 0x51, false),
    ("8", 0x52, false),
    ("9", 0x53, false),
    ("+", 0xB9, true),
    ("-", 0xB7, false),
    ("*", 0x58, false),
    ("/", 0x54, false),
    (".", 0xB8, false),
    (",", 0x4F, false),
    ("ENTER", 0x0D, true),
    ("ESC", 0x01, false),
    ("HOME", 0xE047, false),
    ("APPS", 0xB1, false),
    ("ON", 0x83, false),
    ("LEFT", 0x02, false),
    ("UP", 0x03, false),
    ("RIGHT", 0x04, false),
    ("DOWN", 0x05, false),
    ("BACK", 0x0C, false),
    ("SHIFT", 0x8B, false),
    ("ALPHA", 0xB6, false),
    ("SYMB", 0x91, false),
    ("PLOT", 0xB2, false),
    ("NUM", 0xB3, false),
    ("VIEW", 0xB4, false),
    ("CAS", 0xB5, false),
    ("HELP", 0x95, false),
    ("VARS", 0x41, false),
    ("TOOLBOX", 0x42, false),
    ("MATH", 0x43, false),
];

/// One item of a script: a key press, or a touch gesture.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Key {
    /// The item as given.
    pub name: String,
    /// Device key code (0 for touch gestures).
    pub code: u16,
    /// Touch samples `(status, x, y)` for `TAP` / `SWIPE` items; empty for
    /// keys.
    pub touch: TouchSamples,
}

/// Touch samples `(status, x, y)`: begin, moves, end.
pub type TouchSamples = Vec<(u32, u16, u16)>;

const TOUCH_BEGIN: u32 = 1;
const TOUCH_MOVE: u32 = 2;
const TOUCH_END: u32 = 8;
/// Same-spot samples of a `TAP` (~16 ms apart: ~300 ms held).
const TAP_HOLD_SAMPLES: usize = 18;

/// `TAP:x:y` or `SWIPE:x0:y0:x1:y1[:steps]` (screen pixels).
fn parse_touch(item: &str) -> Option<Result<TouchSamples, String>> {
    let mut parts = item.split(':');
    let kind = parts.next()?.to_ascii_uppercase();
    if kind != "TAP" && kind != "SWIPE" {
        return None;
    }
    let nums: Result<Vec<u16>, _> = parts.map(|p| p.trim().parse::<u16>()).collect();
    let Ok(nums) = nums else {
        return Some(Err(format!("bad coordinates in `{item}`")));
    };
    let inside = |x: u16, y: u16| x < 320 && y < 240;
    Some(match (kind.as_str(), nums.as_slice()) {
        // A held press: down, same-spot samples for ~300 ms (one per pump
        // round at ~60 Hz), up. The firmware recognises soft-key presses on
        // its UI timer (SYS_TIMER every ~183 ms): an instant down/up, or a
        // press shorter than a timer period, can fall between two ticks.
        ("TAP", &[x, y]) if inside(x, y) => {
            let mut v = vec![(TOUCH_BEGIN, x, y)];
            v.extend(std::iter::repeat_n((TOUCH_MOVE, x, y), TAP_HOLD_SAMPLES));
            v.push((TOUCH_END, x, y));
            Ok(v)
        }
        ("SWIPE", &[x0, y0, x1, y1, ..]) if inside(x0, y0) && inside(x1, y1) && nums.len() <= 5 => {
            let steps = nums.get(4).copied().unwrap_or(10).max(1);
            let mut v = vec![(TOUCH_BEGIN, x0, y0)];
            for i in 1..=steps {
                let lerp = |a: u16, b: u16| {
                    (f32::from(a) + (f32::from(b) - f32::from(a)) * i as f32 / steps as f32).round()
                        as u16
                };
                v.push((TOUCH_MOVE, lerp(x0, x1), lerp(y0, y1)));
            }
            v.push((TOUCH_END, x1, y1));
            Ok(v)
        }
        _ => Err(format!(
            "`{item}`: use TAP:x:y or SWIPE:x0:y0:x1:y1[:steps] inside 320x240"
        )),
    })
}

/// Looks up a key name (case-insensitive). Aliases: `ADD`/`PLUS` for `+`,
/// `SUB`/`MINUS`, `MUL`/`TIMES`, `DIV`, `RETURN` for `ENTER`.
pub fn code_of(name: &str) -> Option<u16> {
    let upper = name.trim().to_ascii_uppercase();
    let canonical = match upper.as_str() {
        "ADD" | "PLUS" => "+",
        "SUB" | "MINUS" => "-",
        "MUL" | "TIMES" => "*",
        "DIV" => "/",
        "RETURN" => "ENTER",
        other => other,
    };
    KEYS.iter()
        .find(|(n, _, _)| *n == canonical)
        .map(|&(_, c, _)| c)
}

/// Device code of an upper-case letter: its alpha legend's ASCII code.
/// The firmware's device-code table maps 0x41..=0x5A to the keys whose
/// alpha legends are A..Z (A = Vars, key 14 ... Z = "2", key 43); M5
/// typed `HELLO` with ALPHA + these codes and saw it on the firmware's
/// edit line.
pub fn letter_code(c: char) -> Option<u16> {
    c.is_ascii_uppercase().then_some(c as u16)
}

/// `ALPHA:HELLO`: ALPHA, then the letter, for each character (one-shot
/// alpha: the firmware's name field is not alpha-locked).
fn alpha_text(text: &str) -> Result<Vec<Key>, String> {
    if text.is_empty() {
        return Err("`ALPHA:` needs letters".into());
    }
    let mut out = Vec::new();
    for c in text.chars() {
        let code = letter_code(c)
            .ok_or_else(|| format!("`ALPHA:{text}`: only upper-case A-Z (got `{c}`)"))?;
        out.push(Key {
            name: "ALPHA".to_owned(),
            code: code_of("ALPHA").unwrap(),
            touch: Vec::new(),
        });
        out.push(Key {
            name: c.to_string(),
            code,
            touch: Vec::new(),
        });
    }
    Ok(out)
}

/// Parses `"2,+,2,ENTER"`. Items are key names, raw `0xNN` codes,
/// `TAP:x:y`, `SWIPE:x0:y0:x1:y1[:steps]`, `ALPHA:TEXT` (ALPHA + letter per
/// character, A-Z), or `IDLE` (send nothing, measure
/// the screen as for a key: a negative control). A literal comma key is
/// written `COMMA`.
pub fn parse_keys(script: &str) -> Result<Vec<Key>, String> {
    let mut out = Vec::new();
    for item in script.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(format!("empty key in script `{script}`"));
        }
        if let Some(touch) = parse_touch(item) {
            out.push(Key {
                name: item.to_owned(),
                code: 0,
                touch: touch?,
            });
            continue;
        }
        if let Some(text) = item
            .strip_prefix("ALPHA:")
            .or_else(|| item.strip_prefix("alpha:"))
        {
            out.extend(alpha_text(text)?);
            continue;
        }
        if item.eq_ignore_ascii_case("IDLE") {
            // Negative control: no input, measured like a key.
            out.push(Key {
                name: "IDLE".to_owned(),
                code: 0,
                touch: Vec::new(),
            });
            continue;
        }
        let code = if let Some(hex) = item.strip_prefix("0x").or_else(|| item.strip_prefix("0X")) {
            u16::from_str_radix(hex, 16).map_err(|_| format!("bad key code `{item}`"))?
        } else if item.eq_ignore_ascii_case("COMMA") {
            0x4F
        } else {
            code_of(item).ok_or_else(|| {
                format!(
                    "unknown key `{item}` (known: {})",
                    KEYS.iter().map(|k| k.0).collect::<Vec<_>>().join(" ")
                )
            })?
        };
        out.push(Key {
            name: item.to_owned(),
            code,
            touch: Vec::new(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_text_expands_to_alpha_plus_letter() {
        let keys = parse_keys("ALPHA:HEL,ENTER").unwrap();
        let v: Vec<(&str, u16)> = keys.iter().map(|k| (k.name.as_str(), k.code)).collect();
        assert_eq!(
            v,
            vec![
                ("ALPHA", 0xB6),
                ("H", 0x48),
                ("ALPHA", 0xB6),
                ("E", 0x45),
                ("ALPHA", 0xB6),
                ("L", 0x4C),
                ("ENTER", 0x0D)
            ]
        );
        assert!(parse_keys("ALPHA:hi").is_err());
        assert!(parse_keys("ALPHA:").is_err());
    }

    #[test]
    fn parses_names_aliases_and_raw_codes() {
        let keys = parse_keys("2, +,2 ,enter").unwrap();
        let codes: Vec<u16> = keys.iter().map(|k| k.code).collect();
        assert_eq!(codes, vec![0x5A, 0xB9, 0x5A, 0x0D]);
        assert_eq!(parse_keys("plus,RETURN,0x83,comma").unwrap()[0].code, 0xB9);
        let raw = parse_keys("0xE047").unwrap();
        assert_eq!(raw[0].code, 0xE047);
        assert_eq!(parse_keys("COMMA").unwrap()[0].code, 0x4F);
        let idle = &parse_keys("idle").unwrap()[0];
        assert_eq!(
            (idle.name.as_str(), idle.code, idle.touch.len()),
            ("IDLE", 0, 0)
        );
    }

    #[test]
    fn parses_touch_gestures() {
        let k = parse_keys("TAP:160:120,SWIPE:20:120:300:120:4").unwrap();
        assert_eq!(k[0].touch.first(), Some(&(1, 160, 120)));
        assert_eq!(k[0].touch.last(), Some(&(8, 160, 120)));
        assert_eq!(k[0].touch.len(), 20, "a held press, not an instant down/up");
        assert_eq!(
            k[1].touch,
            vec![
                (1, 20, 120),
                (2, 90, 120),
                (2, 160, 120),
                (2, 230, 120),
                (2, 300, 120),
                (8, 300, 120)
            ]
        );
        assert!(parse_keys("TAP:400:10").is_err());
        assert!(parse_keys("SWIPE:1:2").is_err());
        assert!(parse_keys("TAP:a:b").is_err());
    }

    #[test]
    fn rejects_unknown_and_empty_items() {
        assert!(parse_keys("2,,3").is_err());
        assert!(parse_keys("FOO").unwrap_err().contains("unknown key"));
        assert!(parse_keys("0xZZ").is_err());
    }

    #[test]
    fn reference_codes_match_both_primu2_tables() {
        // Spot checks against mcp_server.py and LCD.cpp vk_to_device_keymap.
        for (name, code) in [
            ("0", 0x30),
            ("1", 0x59),
            ("2", 0x5A),
            ("3", 0x33),
            ("4", 0x55),
            ("9", 0x53),
            ("+", 0xB9),
            ("-", 0xB7),
            ("*", 0x58),
            ("/", 0x54),
            ("ENTER", 0x0D),
            ("ESC", 0x01),
            ("HOME", 0xE047),
            ("ON", 0x83),
        ] {
            assert_eq!(code_of(name), Some(code), "{name}");
        }
        // Names are unique.
        let mut names: Vec<&str> = KEYS.iter().map(|k| k.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), KEYS.len());
    }
}
