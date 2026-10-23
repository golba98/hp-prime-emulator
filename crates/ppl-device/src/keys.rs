//! Named HP Prime key codes (0–50) for the `EC` remote-control command.
//!
//! These are the GETKEY/emulator GUI codes (`ppl-gui/src/keymap.rs`).
//! Their interpretation by EC is unverified: hplp supplies no key-code table,
//! and this encoding had no visible effect on the reference G1 build 15157.
//! Keep the bounded diagnostic interface; do not infer remote execution support.
//!
//! **Safety.** Only names in this table can ever be turned into a key
//! press; [`parse_key_sequence`] rejects everything else, and codes are
//! additionally capped at 50 by [`Prime::press_keys`]. A caller should also
//! restrict *which* names it is willing to press through a session
//! (see [`crate::session`]).

/// One entry of the key table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyDef {
    pub code: u8,
    /// Canonical lowercase name used by the CLI.
    pub name: &'static str,
    /// Extra accepted lowercase spellings.
    pub aliases: &'static [&'static str],
    pub description: &'static str,
}

/// The full 0–50 key grid, in code order.
pub const KEYS: &[KeyDef] = &[
    KeyDef {
        code: 0,
        name: "apps",
        aliases: &["app", "f1"],
        description: "Apps / Application Library",
    },
    KeyDef {
        code: 1,
        name: "symb",
        aliases: &["symbolic", "f2"],
        description: "Symb",
    },
    KeyDef {
        code: 2,
        name: "up",
        aliases: &["arrowup"],
        description: "Cursor up",
    },
    KeyDef {
        code: 3,
        name: "help",
        aliases: &["f9"],
        description: "Help",
    },
    KeyDef {
        code: 4,
        name: "esc",
        aliases: &["escape"],
        description: "Esc / Cancel (back one level)",
    },
    KeyDef {
        code: 5,
        name: "home",
        aliases: &[],
        description: "Home (returns to the Home screen)",
    },
    KeyDef {
        code: 6,
        name: "plot",
        aliases: &["f3"],
        description: "Plot",
    },
    KeyDef {
        code: 7,
        name: "left",
        aliases: &["arrowleft"],
        description: "Cursor left",
    },
    KeyDef {
        code: 8,
        name: "right",
        aliases: &["arrowright"],
        description: "Cursor right",
    },
    KeyDef {
        code: 9,
        name: "view",
        aliases: &["f10"],
        description: "View",
    },
    KeyDef {
        code: 10,
        name: "cas",
        aliases: &["f5"],
        description: "CAS",
    },
    KeyDef {
        code: 11,
        name: "num",
        aliases: &["f4"],
        description: "Num",
    },
    KeyDef {
        code: 12,
        name: "down",
        aliases: &["arrowdown"],
        description: "Cursor down",
    },
    KeyDef {
        code: 13,
        name: "menu",
        aliases: &["f7"],
        description: "Menu",
    },
    KeyDef {
        code: 14,
        name: "vars",
        aliases: &["f8"],
        description: "Vars",
    },
    KeyDef {
        code: 15,
        name: "toolbox",
        aliases: &["f6"],
        description: "Toolbox",
    },
    KeyDef {
        code: 16,
        name: "template",
        aliases: &["tmpl"],
        description: "Template (Chars on Alpha)",
    },
    KeyDef {
        code: 17,
        name: "xtn",
        aliases: &["xthn", "x"],
        description: "X/T/theta/N",
    },
    KeyDef {
        code: 18,
        name: "abc",
        aliases: &["alpha-fraction"],
        description: "a b/c",
    },
    KeyDef {
        code: 19,
        name: "del",
        aliases: &["delete", "backspace"],
        description: "Del",
    },
    KeyDef {
        code: 20,
        name: "pow",
        aliases: &["power"],
        description: "x^y",
    },
    KeyDef {
        code: 21,
        name: "sin",
        aliases: &[],
        description: "sin",
    },
    KeyDef {
        code: 22,
        name: "cos",
        aliases: &[],
        description: "cos",
    },
    KeyDef {
        code: 23,
        name: "tan",
        aliases: &[],
        description: "tan",
    },
    KeyDef {
        code: 24,
        name: "ln",
        aliases: &[],
        description: "ln",
    },
    KeyDef {
        code: 25,
        name: "log",
        aliases: &[],
        description: "log",
    },
    KeyDef {
        code: 26,
        name: "sqr",
        aliases: &["sq", "sqrt"],
        description: "x^2 / sqrt",
    },
    KeyDef {
        code: 27,
        name: "neg",
        aliases: &["plusminus", "chs"],
        description: "+/-",
    },
    KeyDef {
        code: 28,
        name: "paren",
        aliases: &["parens"],
        description: "( )",
    },
    KeyDef {
        code: 29,
        name: "comma",
        aliases: &[],
        description: ",",
    },
    KeyDef {
        code: 30,
        name: "enter",
        aliases: &["return"],
        description: "Enter",
    },
    KeyDef {
        code: 31,
        name: "eex",
        aliases: &[],
        description: "EEX",
    },
    KeyDef {
        code: 32,
        name: "7",
        aliases: &["seven"],
        description: "7",
    },
    KeyDef {
        code: 33,
        name: "8",
        aliases: &["eight"],
        description: "8",
    },
    KeyDef {
        code: 34,
        name: "9",
        aliases: &["nine"],
        description: "9",
    },
    KeyDef {
        code: 35,
        name: "div",
        aliases: &["divide"],
        description: "/",
    },
    KeyDef {
        code: 36,
        name: "alpha",
        aliases: &[],
        description: "Alpha",
    },
    KeyDef {
        code: 37,
        name: "4",
        aliases: &["four"],
        description: "4",
    },
    KeyDef {
        code: 38,
        name: "5",
        aliases: &["five"],
        description: "5",
    },
    KeyDef {
        code: 39,
        name: "6",
        aliases: &["six"],
        description: "6",
    },
    KeyDef {
        code: 40,
        name: "times",
        aliases: &["multiply"],
        description: "*",
    },
    KeyDef {
        code: 41,
        name: "shift",
        aliases: &[],
        description: "Shift",
    },
    KeyDef {
        code: 42,
        name: "1",
        aliases: &["one"],
        description: "1",
    },
    KeyDef {
        code: 43,
        name: "2",
        aliases: &["two"],
        description: "2",
    },
    KeyDef {
        code: 44,
        name: "3",
        aliases: &["three"],
        description: "3",
    },
    KeyDef {
        code: 45,
        name: "minus",
        aliases: &["subtract"],
        description: "-",
    },
    KeyDef {
        code: 46,
        name: "on",
        aliases: &[],
        description: "On (also breaks a running program)",
    },
    KeyDef {
        code: 47,
        name: "0",
        aliases: &["zero"],
        description: "0",
    },
    KeyDef {
        code: 48,
        name: "dot",
        aliases: &["period", "point"],
        description: ".",
    },
    KeyDef {
        code: 49,
        name: "ans",
        aliases: &["space"],
        description: "Ans / Space",
    },
    KeyDef {
        code: 50,
        name: "plus",
        aliases: &["add"],
        description: "+",
    },
];

/// Key codes of the low-risk navigation keys: these only move between
/// screens or cancel an open dialog. Everything else (digits, letters,
/// operators, `del`, `on`, `shift`, `alpha`) can edit data and is *not*
/// part of the default session allowlist.
pub const NAVIGATION_KEYS: &[&str] = &[
    "up", "down", "left", "right", "esc", "home", "enter", "menu", "vars", "help", "view",
];

/// Looks up one key by (case-insensitive) name or alias.
pub fn key_by_name(name: &str) -> Option<&'static KeyDef> {
    let n = name.trim().to_ascii_lowercase();
    KEYS.iter().find(|k| {
        k.name == n || k.aliases.iter().any(|a| *a == n) || k.description.eq_ignore_ascii_case(&n)
    })
}

/// Code for one named key, or `None` for any name outside the 0–50 grid.
pub fn key_code(name: &str) -> Option<u8> {
    key_by_name(name).map(|k| k.code)
}

/// Canonical name for one code, or `None` outside the grid.
pub fn key_name(code: u8) -> Option<&'static str> {
    KEYS.iter().find(|k| k.code == code).map(|k| k.name)
}

/// Parses a space-separated key sequence (`"shift 1 enter"`), rejecting
/// unknown names. Used by the CLI so that no free-form code can be typed.
pub fn parse_key_sequence(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for word in s.split_whitespace() {
        match key_code(word) {
            Some(c) => out.push(c),
            None => {
                return Err(format!(
                    "unknown key name `{word}` (see `ppl device key --list`)"
                ));
            }
        }
    }
    if out.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(out)
}

/// Renders codes back to names (`"shift+1+enter"`); unknown codes keep
/// their number. Used for logs and error messages.
pub fn describe_codes(codes: &[u8]) -> String {
    codes
        .iter()
        .map(|&c| key_name(c).unwrap_or("?").to_string())
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_exactly_0_to_50() {
        for c in 0..=50u8 {
            assert!(key_name(c).is_some(), "code {c} missing");
        }
        assert!(key_name(51).is_none());
        let mut seen = std::collections::BTreeSet::new();
        for k in KEYS {
            assert!(seen.insert(k.code), "duplicate code {}", k.code);
        }
        assert_eq!(seen.len(), 51);
    }

    #[test]
    fn names_and_aliases_resolve() {
        assert_eq!(key_code("home"), Some(5));
        assert_eq!(key_code("HOME"), Some(5));
        assert_eq!(key_code("escape"), Some(4));
        assert_eq!(key_code("return"), Some(30));
        assert_eq!(key_code("0"), Some(47));
        assert_eq!(key_code("space"), Some(49));
        assert_eq!(key_code("no-such-key"), None);
        assert_eq!(key_code(""), None);
    }

    #[test]
    fn sequences_parse_or_fail() {
        assert_eq!(parse_key_sequence("shift 1").unwrap(), vec![41, 42]);
        assert_eq!(parse_key_sequence("Home").unwrap(), vec![5]);
        let e = parse_key_sequence("home nonsense").unwrap_err();
        assert!(e.contains("nonsense"));
        assert!(parse_key_sequence("   ").is_err());
    }

    #[test]
    fn navigation_allowlist_has_no_editing_keys() {
        for name in NAVIGATION_KEYS {
            let code = key_code(name).expect("allowlist names must exist");
            // digits, operators, modifiers and del/on edit or break things
            assert!(!matches!(code, 19 | 20 | 27 | 36 | 41 | 46 | 50), "{name}");
        }
        assert!(NAVIGATION_KEYS.contains(&"esc"));
    }

    #[test]
    fn describe_roundtrip() {
        let codes = parse_key_sequence("shift 1 enter").unwrap();
        assert_eq!(describe_codes(&codes), "shift+1+enter");
    }
}
