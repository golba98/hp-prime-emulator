//! HP Prime keyboard mapping (0–50 grid) to PC keyboard events.
//!
//! Maps `winit` keyboard events (`PhysicalKey`, `Key`, and textual input)
//! to HP Prime key codes according to the physical and functional layout
//! of the HP Prime calculator.

use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

// HP Prime key code constants (0–50)
pub const KEY_APPS: i64 = 0;
pub const KEY_SYMB: i64 = 1;
pub const KEY_UP: i64 = 2;
pub const KEY_HELP: i64 = 3;
pub const KEY_ESC: i64 = 4;
pub const KEY_HOME: i64 = 5;
pub const KEY_PLOT: i64 = 6;
pub const KEY_LEFT: i64 = 7;
pub const KEY_RIGHT: i64 = 8;
pub const KEY_VIEW: i64 = 9;
pub const KEY_CAS: i64 = 10;
pub const KEY_NUM: i64 = 11;
pub const KEY_DOWN: i64 = 12;
pub const KEY_MENU: i64 = 13;
pub const KEY_VARS: i64 = 14;
pub const KEY_TOOLBOX: i64 = 15;
pub const KEY_TEMPLATE: i64 = 16;
pub const KEY_XTN: i64 = 17;
pub const KEY_ABC: i64 = 18;
pub const KEY_DEL: i64 = 19;
pub const KEY_POW: i64 = 20;
pub const KEY_SIN: i64 = 21;
pub const KEY_COS: i64 = 22;
pub const KEY_TAN: i64 = 23;
pub const KEY_LN: i64 = 24;
pub const KEY_LOG: i64 = 25;
pub const KEY_SQR: i64 = 26;
pub const KEY_NEG: i64 = 27;
pub const KEY_PAREN: i64 = 28;
pub const KEY_COMMA: i64 = 29;
pub const KEY_ENTER: i64 = 30;
pub const KEY_EEX: i64 = 31;
pub const KEY_7: i64 = 32;
pub const KEY_8: i64 = 33;
pub const KEY_9: i64 = 34;
pub const KEY_DIV: i64 = 35;
pub const KEY_ALPHA: i64 = 36;
pub const KEY_4: i64 = 37;
pub const KEY_5: i64 = 38;
pub const KEY_6: i64 = 39;
pub const KEY_TIMES: i64 = 40;
pub const KEY_SHIFT: i64 = 41;
pub const KEY_1: i64 = 42;
pub const KEY_2: i64 = 43;
pub const KEY_3: i64 = 44;
pub const KEY_MINUS: i64 = 45;
pub const KEY_ON: i64 = 46;
pub const KEY_0: i64 = 47;
pub const KEY_DOT: i64 = 48;
pub const KEY_ANS: i64 = 49;
pub const KEY_PLUS: i64 = 50;

/// Human-readable name for an HP Prime key code.
pub fn hp_key_name(code: i64) -> Option<&'static str> {
    match code {
        KEY_APPS => Some("Apps"),
        KEY_SYMB => Some("Symb"),
        KEY_UP => Some("Up"),
        KEY_HELP => Some("Help"),
        KEY_ESC => Some("Esc"),
        KEY_HOME => Some("Home"),
        KEY_PLOT => Some("Plot"),
        KEY_LEFT => Some("Left"),
        KEY_RIGHT => Some("Right"),
        KEY_VIEW => Some("View"),
        KEY_CAS => Some("CAS"),
        KEY_NUM => Some("Num"),
        KEY_DOWN => Some("Down"),
        KEY_MENU => Some("Menu"),
        KEY_VARS => Some("Vars"),
        KEY_TOOLBOX => Some("Toolbox"),
        KEY_TEMPLATE => Some("Template"),
        KEY_XTN => Some("X,T,θ,N"),
        KEY_ABC => Some("a b/c"),
        KEY_DEL => Some("Del/Backspace"),
        KEY_POW => Some("x^y"),
        KEY_SIN => Some("SIN"),
        KEY_COS => Some("COS"),
        KEY_TAN => Some("TAN"),
        KEY_LN => Some("LN"),
        KEY_LOG => Some("LOG"),
        KEY_SQR => Some("x²"),
        KEY_NEG => Some("+/-"),
        KEY_PAREN => Some("( )"),
        KEY_COMMA => Some(","),
        KEY_ENTER => Some("Enter"),
        KEY_EEX => Some("EEX"),
        KEY_7 => Some("7"),
        KEY_8 => Some("8"),
        KEY_9 => Some("9"),
        KEY_DIV => Some("÷"),
        KEY_ALPHA => Some("Alpha"),
        KEY_4 => Some("4"),
        KEY_5 => Some("5"),
        KEY_6 => Some("6"),
        KEY_TIMES => Some("×"),
        KEY_SHIFT => Some("Shift"),
        KEY_1 => Some("1"),
        KEY_2 => Some("2"),
        KEY_3 => Some("3"),
        KEY_MINUS => Some("-"),
        KEY_ON => Some("On"),
        KEY_0 => Some("0"),
        KEY_DOT => Some("."),
        KEY_ANS => Some("Ans/Space"),
        KEY_PLUS => Some("+"),
        _ => None,
    }
}

/// Maps a winit keyboard event to an HP Prime key code (0–50).
///
/// Priority:
/// 1. Logical `NamedKey` (Enter, Escape, arrows, Backspace, F1–F10, …)
/// 2. The logical (layout-aware) character, when it is an HP key: so
///    Shift+8 on a US layout (`*`) gives × and Shift+= (`+`) gives +.
/// 3. The physical key position (number row, numpad, letter shortcuts), so
///    digits still work on layouts whose unshifted number row types symbols.
pub fn map_winit_key(logical: &Key, physical: PhysicalKey, text: Option<&str>) -> Option<i64> {
    // 1. Check logical NamedKey
    if let Key::Named(named) = logical {
        let code = match named {
            NamedKey::Enter => Some(KEY_ENTER),
            NamedKey::Escape => Some(KEY_ESC),
            NamedKey::Backspace | NamedKey::Delete => Some(KEY_DEL),
            NamedKey::ArrowUp => Some(KEY_UP),
            NamedKey::ArrowDown => Some(KEY_DOWN),
            NamedKey::ArrowLeft => Some(KEY_LEFT),
            NamedKey::ArrowRight => Some(KEY_RIGHT),
            NamedKey::Home => Some(KEY_HOME),
            NamedKey::Space => Some(KEY_ANS),
            NamedKey::Tab => Some(KEY_TEMPLATE),
            NamedKey::Shift => Some(KEY_SHIFT),
            NamedKey::Control | NamedKey::Alt => Some(KEY_ALPHA),
            NamedKey::F1 => Some(KEY_APPS),
            NamedKey::F2 => Some(KEY_SYMB),
            NamedKey::F3 => Some(KEY_PLOT),
            NamedKey::F4 => Some(KEY_NUM),
            NamedKey::F5 => Some(KEY_CAS),
            NamedKey::F6 => Some(KEY_TOOLBOX),
            NamedKey::F7 => Some(KEY_MENU),
            NamedKey::F8 => Some(KEY_VARS),
            NamedKey::F9 => Some(KEY_HELP),
            NamedKey::F10 => Some(KEY_VIEW),
            _ => None,
        };
        if code.is_some() {
            return code;
        }
    }

    // 2. The logical character (or the event text) when it is an HP key.
    let typed = match logical {
        Key::Character(smol) => Some(smol.as_str()),
        _ => text,
    };
    if let Some(code) = typed.and_then(map_char) {
        return Some(code);
    }

    // 3. Physical KeyCode
    if let PhysicalKey::Code(key_code) = physical {
        return map_physical(key_code);
    }
    None
}

/// HP key for a typed character.
pub fn map_char(s: &str) -> Option<i64> {
    match s {
        "0" => Some(KEY_0),
        "1" => Some(KEY_1),
        "2" => Some(KEY_2),
        "3" => Some(KEY_3),
        "4" => Some(KEY_4),
        "5" => Some(KEY_5),
        "6" => Some(KEY_6),
        "7" => Some(KEY_7),
        "8" => Some(KEY_8),
        "9" => Some(KEY_9),
        "+" => Some(KEY_PLUS),
        "-" => Some(KEY_MINUS),
        "*" => Some(KEY_TIMES),
        "/" => Some(KEY_DIV),
        "=" => Some(KEY_ENTER),
        "." => Some(KEY_DOT),
        "," => Some(KEY_COMMA),
        "(" | ")" => Some(KEY_PAREN),
        "^" => Some(KEY_POW),
        " " => Some(KEY_ANS),
        "x" | "X" => Some(KEY_TIMES),
        _ => None,
    }
}

/// Unified key translation mapping across physical ID, native PPL code,
/// firmware scan code, labels, and PC keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMapping {
    pub physical_id: u8,
    pub ppl_code: i32,
    pub device_code: u16,
    pub primary_label: &'static str,
    pub secondary_label: &'static str,
    pub alpha_label: &'static str,
    pub pc_key: Option<egui::Key>,
}

pub const KEY_MAPPINGS: [KeyMapping; 51] = [
    KeyMapping {
        physical_id: 0,
        ppl_code: 0,
        device_code: 0x00B1,
        primary_label: "Apps",
        secondary_label: "Info",
        alpha_label: "",
        pc_key: Some(egui::Key::F1),
    },
    KeyMapping {
        physical_id: 1,
        ppl_code: 1,
        device_code: 0x0091,
        primary_label: "Symb",
        secondary_label: "S-Setup",
        alpha_label: "",
        pc_key: Some(egui::Key::F2),
    },
    KeyMapping {
        physical_id: 2,
        ppl_code: 2,
        device_code: 0x0003,
        primary_label: "Up",
        secondary_label: "",
        alpha_label: "",
        pc_key: Some(egui::Key::ArrowUp),
    },
    KeyMapping {
        physical_id: 3,
        ppl_code: 3,
        device_code: 0x0095,
        primary_label: "Help",
        secondary_label: "User",
        alpha_label: "",
        pc_key: Some(egui::Key::F3),
    },
    KeyMapping {
        physical_id: 4,
        ppl_code: 4,
        device_code: 0x0001,
        primary_label: "Esc",
        secondary_label: "Clear",
        alpha_label: "",
        pc_key: Some(egui::Key::Escape),
    },
    KeyMapping {
        physical_id: 5,
        ppl_code: 5,
        device_code: 0xE047,
        primary_label: "Home",
        secondary_label: "Settings",
        alpha_label: "",
        pc_key: Some(egui::Key::Home),
    },
    KeyMapping {
        physical_id: 6,
        ppl_code: 6,
        device_code: 0x00B2,
        primary_label: "Plot",
        secondary_label: "P-Setup",
        alpha_label: "",
        pc_key: Some(egui::Key::PageUp),
    },
    KeyMapping {
        physical_id: 7,
        ppl_code: 7,
        device_code: 0x0002,
        primary_label: "Left",
        secondary_label: "",
        alpha_label: "",
        pc_key: Some(egui::Key::ArrowLeft),
    },
    KeyMapping {
        physical_id: 8,
        ppl_code: 8,
        device_code: 0x0004,
        primary_label: "Right",
        secondary_label: "",
        alpha_label: "",
        pc_key: Some(egui::Key::ArrowRight),
    },
    KeyMapping {
        physical_id: 9,
        ppl_code: 9,
        device_code: 0x00B4,
        primary_label: "View",
        secondary_label: "Copy",
        alpha_label: "",
        pc_key: Some(egui::Key::F6),
    },
    KeyMapping {
        physical_id: 10,
        ppl_code: 10,
        device_code: 0x00B5,
        primary_label: "CAS",
        secondary_label: "Settings",
        alpha_label: "",
        pc_key: Some(egui::Key::F7),
    },
    KeyMapping {
        physical_id: 11,
        ppl_code: 11,
        device_code: 0x00B3,
        primary_label: "Num",
        secondary_label: "N-Setup",
        alpha_label: "",
        pc_key: Some(egui::Key::PageDown),
    },
    KeyMapping {
        physical_id: 12,
        ppl_code: 12,
        device_code: 0x0005,
        primary_label: "Down",
        secondary_label: "",
        alpha_label: "",
        pc_key: Some(egui::Key::ArrowDown),
    },
    KeyMapping {
        physical_id: 13,
        ppl_code: 13,
        device_code: 0x0093,
        primary_label: "Menu",
        secondary_label: "Paste",
        alpha_label: "",
        pc_key: Some(egui::Key::F9),
    },
    KeyMapping {
        physical_id: 14,
        ppl_code: 14,
        device_code: 0x0041,
        primary_label: "Vars",
        secondary_label: "Chars",
        alpha_label: "A",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 15,
        ppl_code: 15,
        device_code: 0x0042,
        primary_label: "Toolbox",
        secondary_label: "Mem",
        alpha_label: "B",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 16,
        ppl_code: 16,
        device_code: 0x0043,
        primary_label: "Templates",
        secondary_label: "Units",
        alpha_label: "C",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 17,
        ppl_code: 17,
        device_code: 0x0044,
        primary_label: "x t θ n",
        secondary_label: "Define",
        alpha_label: "D",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 18,
        ppl_code: 18,
        device_code: 0x0045,
        primary_label: "a b/c",
        secondary_label: "° ′ ″",
        alpha_label: "E",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 19,
        ppl_code: 19,
        device_code: 0x000C,
        primary_label: "⌫",
        secondary_label: "Del",
        alpha_label: "",
        pc_key: Some(egui::Key::Backspace),
    },
    KeyMapping {
        physical_id: 20,
        ppl_code: 20,
        device_code: 0x0046,
        primary_label: "xʸ",
        secondary_label: "ʸ√",
        alpha_label: "F",
        pc_key: Some(egui::Key::F11),
    },
    KeyMapping {
        physical_id: 21,
        ppl_code: 21,
        device_code: 0x0047,
        primary_label: "SIN",
        secondary_label: "ASIN",
        alpha_label: "G",
        pc_key: Some(egui::Key::S),
    },
    KeyMapping {
        physical_id: 22,
        ppl_code: 22,
        device_code: 0x0048,
        primary_label: "COS",
        secondary_label: "ACOS",
        alpha_label: "H",
        pc_key: Some(egui::Key::C),
    },
    KeyMapping {
        physical_id: 23,
        ppl_code: 23,
        device_code: 0x0049,
        primary_label: "TAN",
        secondary_label: "ATAN",
        alpha_label: "I",
        pc_key: Some(egui::Key::T),
    },
    KeyMapping {
        physical_id: 24,
        ppl_code: 24,
        device_code: 0x004A,
        primary_label: "LN",
        secondary_label: "eˣ",
        alpha_label: "J",
        pc_key: Some(egui::Key::L),
    },
    KeyMapping {
        physical_id: 25,
        ppl_code: 25,
        device_code: 0x004B,
        primary_label: "LOG",
        secondary_label: "10ˣ",
        alpha_label: "K",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 26,
        ppl_code: 26,
        device_code: 0x004C,
        primary_label: "x²",
        secondary_label: "√",
        alpha_label: "L",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 27,
        ppl_code: 27,
        device_code: 0x004D,
        primary_label: "+/−",
        secondary_label: "|x|",
        alpha_label: "M",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 28,
        ppl_code: 28,
        device_code: 0x004E,
        primary_label: "( )",
        secondary_label: "!",
        alpha_label: "N",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 29,
        ppl_code: 29,
        device_code: 0x004F,
        primary_label: ",",
        secondary_label: "Eval",
        alpha_label: "O",
        pc_key: Some(egui::Key::Comma),
    },
    KeyMapping {
        physical_id: 30,
        ppl_code: 30,
        device_code: 0x000D,
        primary_label: "Enter",
        secondary_label: "≈",
        alpha_label: "",
        pc_key: Some(egui::Key::Enter),
    },
    KeyMapping {
        physical_id: 31,
        ppl_code: 31,
        device_code: 0x0050,
        primary_label: "EEX",
        secondary_label: "Sto ▸",
        alpha_label: "P",
        pc_key: Some(egui::Key::E),
    },
    KeyMapping {
        physical_id: 32,
        ppl_code: 32,
        device_code: 0x0051,
        primary_label: "7",
        secondary_label: "List",
        alpha_label: "Q",
        pc_key: Some(egui::Key::Num7),
    },
    KeyMapping {
        physical_id: 33,
        ppl_code: 33,
        device_code: 0x0052,
        primary_label: "8",
        secondary_label: "{ }",
        alpha_label: "R",
        pc_key: Some(egui::Key::Num8),
    },
    KeyMapping {
        physical_id: 34,
        ppl_code: 34,
        device_code: 0x0053,
        primary_label: "9",
        secondary_label: "! , ∞ , →",
        alpha_label: "S",
        pc_key: Some(egui::Key::Num9),
    },
    KeyMapping {
        physical_id: 35,
        ppl_code: 35,
        device_code: 0x0054,
        primary_label: "÷",
        secondary_label: "x⁻¹",
        alpha_label: "T",
        pc_key: Some(egui::Key::Slash),
    },
    KeyMapping {
        physical_id: 36,
        ppl_code: 36,
        device_code: 0x00B6,
        primary_label: "ALPHA",
        secondary_label: "alpha",
        alpha_label: "",
        pc_key: Some(egui::Key::Tab),
    },
    KeyMapping {
        physical_id: 37,
        ppl_code: 37,
        device_code: 0x0055,
        primary_label: "4",
        secondary_label: "Matrix",
        alpha_label: "U",
        pc_key: Some(egui::Key::Num4),
    },
    KeyMapping {
        physical_id: 38,
        ppl_code: 38,
        device_code: 0x0056,
        primary_label: "5",
        secondary_label: "[ ]",
        alpha_label: "V",
        pc_key: Some(egui::Key::Num5),
    },
    KeyMapping {
        physical_id: 39,
        ppl_code: 39,
        device_code: 0x0057,
        primary_label: "6",
        secondary_label: "≤, ≥, ≠",
        alpha_label: "W",
        pc_key: Some(egui::Key::Num6),
    },
    KeyMapping {
        physical_id: 40,
        ppl_code: 40,
        device_code: 0x0058,
        primary_label: "×",
        secondary_label: "∠",
        alpha_label: "X",
        pc_key: Some(egui::Key::X),
    },
    KeyMapping {
        physical_id: 41,
        ppl_code: 41,
        device_code: 0x008B,
        primary_label: "Shift",
        secondary_label: "",
        alpha_label: "",
        pc_key: None,
    },
    KeyMapping {
        physical_id: 42,
        ppl_code: 42,
        device_code: 0x0059,
        primary_label: "1",
        secondary_label: "Program",
        alpha_label: "Y",
        pc_key: Some(egui::Key::Num1),
    },
    KeyMapping {
        physical_id: 43,
        ppl_code: 43,
        device_code: 0x005A,
        primary_label: "2",
        secondary_label: "i",
        alpha_label: "Z",
        pc_key: Some(egui::Key::Num2),
    },
    KeyMapping {
        physical_id: 44,
        ppl_code: 44,
        device_code: 0x0033,
        primary_label: "3",
        secondary_label: "π",
        alpha_label: "#",
        pc_key: Some(egui::Key::Num3),
    },
    KeyMapping {
        physical_id: 45,
        ppl_code: 45,
        device_code: 0x00B7,
        primary_label: "−",
        secondary_label: "Base",
        alpha_label: ":",
        pc_key: Some(egui::Key::Minus),
    },
    KeyMapping {
        physical_id: 46,
        ppl_code: 46,
        device_code: 0x0083,
        primary_label: "On",
        secondary_label: "Off",
        alpha_label: "",
        pc_key: Some(egui::Key::F12),
    },
    KeyMapping {
        physical_id: 47,
        ppl_code: 47,
        device_code: 0x0030,
        primary_label: "0",
        secondary_label: "Notes",
        alpha_label: "\" ′′",
        pc_key: Some(egui::Key::Num0),
    },
    KeyMapping {
        physical_id: 48,
        ppl_code: 48,
        device_code: 0x00B8,
        primary_label: "•",
        secondary_label: "=",
        alpha_label: "",
        pc_key: Some(egui::Key::Period),
    },
    KeyMapping {
        physical_id: 49,
        ppl_code: 49,
        device_code: 0x0020,
        primary_label: "␣",
        secondary_label: "_",
        alpha_label: "",
        pc_key: Some(egui::Key::Space),
    },
    KeyMapping {
        physical_id: 50,
        ppl_code: 50,
        device_code: 0x00B9,
        primary_label: "+",
        secondary_label: "Ans",
        alpha_label: ";",
        pc_key: Some(egui::Key::Plus),
    },
];

/// Looks up key mapping by physical ID (0..=50).
#[inline]
pub fn lookup_by_physical(id: u8) -> Option<&'static KeyMapping> {
    KEY_MAPPINGS.get(id as usize)
}

/// Looks up key mapping by native PPL keycode (0..=50).
#[inline]
pub fn lookup_by_ppl_code(code: i32) -> Option<&'static KeyMapping> {
    if (0..=50).contains(&code) {
        KEY_MAPPINGS.get(code as usize)
    } else {
        None
    }
}

/// Looks up key mapping by ARM firmware device scan code.
pub fn lookup_by_device_code(code: u16) -> Option<&'static KeyMapping> {
    KEY_MAPPINGS.iter().find(|m| m.device_code == code)
}

/// Looks up key mapping by host PC key (egui Key), including navigation and operator aliases.
pub fn lookup_by_pc_key(key: egui::Key) -> Option<&'static KeyMapping> {
    let id: u8 = match key {
        egui::Key::F1 => 0,
        egui::Key::F2 => 1,
        egui::Key::ArrowUp => 2,
        egui::Key::F3 => 3,
        egui::Key::Escape => 4,
        egui::Key::Home | egui::Key::F4 => 5,
        egui::Key::PageUp | egui::Key::F5 => 6,
        egui::Key::ArrowLeft => 7,
        egui::Key::ArrowRight => 8,
        egui::Key::F6 => 9,
        egui::Key::F7 => 10,
        egui::Key::PageDown | egui::Key::F8 => 11,
        egui::Key::ArrowDown => 12,
        egui::Key::End | egui::Key::F9 => 13,
        egui::Key::Backspace | egui::Key::Delete => 19,
        egui::Key::F11 => 20,
        egui::Key::S => 21,
        egui::Key::C => 22,
        egui::Key::T => 23,
        egui::Key::L => 24,
        egui::Key::Comma => 29,
        egui::Key::Enter | egui::Key::Equals => 30,
        egui::Key::E => 31,
        egui::Key::Num7 => 32,
        egui::Key::Num8 => 33,
        egui::Key::Num9 => 34,
        egui::Key::Slash => 35,
        egui::Key::Tab => 36,
        egui::Key::Num4 => 37,
        egui::Key::Num5 => 38,
        egui::Key::Num6 => 39,
        egui::Key::X => 40,
        egui::Key::Num1 => 42,
        egui::Key::Num2 => 43,
        egui::Key::Num3 => 44,
        egui::Key::Minus => 45,
        egui::Key::F12 => 46,
        egui::Key::Num0 => 47,
        egui::Key::Period => 48,
        egui::Key::Space => 49,
        egui::Key::Plus => 50,
        _ => return None,
    };
    lookup_by_physical(id)
}

/// Maps an egui key to an HP Prime keycode (0–50 grid).
#[inline]
pub fn egui_key_to_hp(key: egui::Key) -> Option<i64> {
    lookup_by_pc_key(key).map(|m| m.ppl_code as i64)
}

/// HP key for a physical key position.
fn map_physical(key_code: KeyCode) -> Option<i64> {
    match key_code {
        // Digits
        KeyCode::Digit0 | KeyCode::Numpad0 => Some(KEY_0),
        KeyCode::Digit1 | KeyCode::Numpad1 => Some(KEY_1),
        KeyCode::Digit2 | KeyCode::Numpad2 => Some(KEY_2),
        KeyCode::Digit3 | KeyCode::Numpad3 => Some(KEY_3),
        KeyCode::Digit4 | KeyCode::Numpad4 => Some(KEY_4),
        KeyCode::Digit5 | KeyCode::Numpad5 => Some(KEY_5),
        KeyCode::Digit6 | KeyCode::Numpad6 => Some(KEY_6),
        KeyCode::Digit7 | KeyCode::Numpad7 => Some(KEY_7),
        KeyCode::Digit8 | KeyCode::Numpad8 => Some(KEY_8),
        KeyCode::Digit9 | KeyCode::Numpad9 => Some(KEY_9),

        // Controls
        KeyCode::Enter | KeyCode::NumpadEnter => Some(KEY_ENTER),
        KeyCode::Escape => Some(KEY_ESC),
        KeyCode::Backspace | KeyCode::Delete => Some(KEY_DEL),
        KeyCode::Space => Some(KEY_ANS),
        KeyCode::Tab => Some(KEY_TEMPLATE),
        KeyCode::Home => Some(KEY_HOME),

        // Arrows
        KeyCode::ArrowUp => Some(KEY_UP),
        KeyCode::ArrowDown => Some(KEY_DOWN),
        KeyCode::ArrowLeft => Some(KEY_LEFT),
        KeyCode::ArrowRight => Some(KEY_RIGHT),

        // Operators & Punctuation
        KeyCode::NumpadAdd | KeyCode::Equal => Some(KEY_PLUS),
        KeyCode::NumpadSubtract | KeyCode::Minus => Some(KEY_MINUS),
        KeyCode::NumpadMultiply => Some(KEY_TIMES),
        KeyCode::NumpadDivide | KeyCode::Slash => Some(KEY_DIV),
        KeyCode::Period | KeyCode::NumpadDecimal => Some(KEY_DOT),
        KeyCode::Comma => Some(KEY_COMMA),

        // HP functions / letters
        KeyCode::KeyX => Some(KEY_TIMES),
        KeyCode::KeyS => Some(KEY_SIN),
        KeyCode::KeyC => Some(KEY_COS),
        KeyCode::KeyT => Some(KEY_TAN),
        KeyCode::KeyL => Some(KEY_LN),

        // Function keys
        KeyCode::F1 => Some(KEY_APPS),
        KeyCode::F2 => Some(KEY_SYMB),
        KeyCode::F3 => Some(KEY_PLOT),
        KeyCode::F4 => Some(KEY_NUM),
        KeyCode::F5 => Some(KEY_CAS),
        KeyCode::F6 => Some(KEY_TOOLBOX),
        KeyCode::F7 => Some(KEY_MENU),
        KeyCode::F8 => Some(KEY_VARS),
        KeyCode::F9 => Some(KEY_HELP),
        KeyCode::F10 => Some(KEY_VIEW),
        KeyCode::F11 => Some(KEY_POW),
        KeyCode::F12 => Some(KEY_ON),

        _ => None,
    }
}

/// Whether the key is the emulator's break key, i.e. the calculator's
/// **On** key used to stop a running program: F12 (mapped to On), the
/// Pause/Break key, or Ctrl+C. A break interrupts the program instead of
/// being queued for `GETKEY`. Esc is *not* a break key: programs read it as
/// HP key 4 (`GETKEY == 4`) and dialogs use it to cancel.
pub fn is_break_key(logical: &Key, physical: PhysicalKey, ctrl: bool) -> bool {
    if matches!(logical, Key::Named(NamedKey::Pause | NamedKey::F12)) {
        return true;
    }
    match physical {
        PhysicalKey::Code(KeyCode::Pause | KeyCode::F12) => true,
        PhysicalKey::Code(KeyCode::KeyC) => ctrl,
        _ => ctrl && matches!(logical, Key::Character(c) if c.eq_ignore_ascii_case("c")),
    }
}

/// The printable character a key event typed (for INPUT edit fields).
pub fn typed_char(logical: &Key, text: Option<&str>) -> Option<char> {
    let s = match logical {
        Key::Named(NamedKey::Space) => " ",
        Key::Character(c) => c.as_str(),
        _ => text?,
    };
    let mut chars = s.chars();
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control()).then_some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_named_navigation_keys() {
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::ArrowUp),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_UP)
        );
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::ArrowDown),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_DOWN)
        );
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::ArrowLeft),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_LEFT)
        );
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::ArrowRight),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_RIGHT)
        );
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::Enter),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_ENTER)
        );
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::Escape),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_ESC)
        );
        assert_eq!(
            map_winit_key(
                &Key::Named(NamedKey::Backspace),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_DEL)
        );
    }

    #[test]
    fn test_numeric_keys() {
        // Test all numbers 0-9 via KeyCode
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit0),
                None
            ),
            Some(47)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit1),
                None
            ),
            Some(42)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit2),
                None
            ),
            Some(43)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit3),
                None
            ),
            Some(44)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit4),
                None
            ),
            Some(37)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit5),
                None
            ),
            Some(38)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit6),
                None
            ),
            Some(39)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit7),
                None
            ),
            Some(32)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit8),
                None
            ),
            Some(33)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Digit9),
                None
            ),
            Some(34)
        );

        // Test Numpad digits
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Numpad0),
                None
            ),
            Some(47)
        );
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                PhysicalKey::Code(KeyCode::Numpad7),
                None
            ),
            Some(32)
        );
    }

    #[test]
    fn test_character_fallback() {
        assert_eq!(
            map_winit_key(
                &Key::Character("+".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_PLUS)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character("-".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_MINUS)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character("*".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_TIMES)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character("/".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_DIV)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character(".".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_DOT)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character(",".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_COMMA)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character("(".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_PAREN)
        );
        assert_eq!(
            map_winit_key(
                &Key::Character("^".into()),
                PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
                None
            ),
            Some(KEY_POW)
        );
    }

    fn unid() -> PhysicalKey {
        PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified)
    }

    #[test]
    fn test_logical_character_beats_physical_key() {
        // US layout Shift+8 types `*`: that is HP ×, not HP 8.
        assert_eq!(
            map_winit_key(
                &Key::Character("*".into()),
                PhysicalKey::Code(KeyCode::Digit8),
                Some("*")
            ),
            Some(KEY_TIMES)
        );
        // Shift+= types `+`.
        assert_eq!(
            map_winit_key(
                &Key::Character("+".into()),
                PhysicalKey::Code(KeyCode::Equal),
                Some("+")
            ),
            Some(KEY_PLUS)
        );
        // Shift+9 types `(`.
        assert_eq!(
            map_winit_key(
                &Key::Character("(".into()),
                PhysicalKey::Code(KeyCode::Digit9),
                Some("(")
            ),
            Some(KEY_PAREN)
        );
        // Shift+6 types `^`.
        assert_eq!(
            map_winit_key(
                &Key::Character("^".into()),
                PhysicalKey::Code(KeyCode::Digit6),
                Some("^")
            ),
            Some(KEY_POW)
        );
    }

    #[test]
    fn test_physical_fallback_when_character_is_not_an_hp_key() {
        // AZERTY unshifted digit row types `&` on Digit1: still HP 1.
        assert_eq!(
            map_winit_key(
                &Key::Character("&".into()),
                PhysicalKey::Code(KeyCode::Digit1),
                Some("&")
            ),
            Some(KEY_1)
        );
        // Letter shortcuts come from the physical key.
        assert_eq!(
            map_winit_key(
                &Key::Character("s".into()),
                PhysicalKey::Code(KeyCode::KeyS),
                Some("s")
            ),
            Some(KEY_SIN)
        );
        // Unmapped letter: no HP key.
        assert_eq!(
            map_winit_key(
                &Key::Character("q".into()),
                PhysicalKey::Code(KeyCode::KeyQ),
                Some("q")
            ),
            None
        );
        // Event text is used when the logical key is unidentified.
        assert_eq!(
            map_winit_key(
                &Key::Unidentified(winit::keyboard::NativeKey::Unidentified),
                unid(),
                Some("7")
            ),
            Some(KEY_7)
        );
    }

    #[test]
    fn test_break_keys() {
        assert!(is_break_key(
            &Key::Named(NamedKey::F12),
            PhysicalKey::Code(KeyCode::F12),
            false
        ));
        assert!(is_break_key(
            &Key::Named(NamedKey::Pause),
            PhysicalKey::Code(KeyCode::Pause),
            false
        ));
        assert!(is_break_key(
            &Key::Character("c".into()),
            PhysicalKey::Code(KeyCode::KeyC),
            true
        ));
        assert!(is_break_key(&Key::Character("c".into()), unid(), true));
        // Plain `c` (COS) and Esc (HP key 4) are not break keys.
        assert!(!is_break_key(
            &Key::Character("c".into()),
            PhysicalKey::Code(KeyCode::KeyC),
            false
        ));
        assert!(!is_break_key(
            &Key::Named(NamedKey::Escape),
            PhysicalKey::Code(KeyCode::Escape),
            false
        ));
    }

    #[test]
    fn test_typed_char() {
        assert_eq!(
            typed_char(&Key::Character("a".into()), Some("a")),
            Some('a')
        );
        assert_eq!(typed_char(&Key::Named(NamedKey::Space), None), Some(' '));
        assert_eq!(
            typed_char(&Key::Named(NamedKey::Backspace), Some("\u{8}")),
            None
        );
        assert_eq!(typed_char(&Key::Named(NamedKey::Enter), Some("\r")), None);
        assert_eq!(typed_char(&Key::Named(NamedKey::ArrowUp), None), None);
    }

    #[test]
    fn test_key_names() {
        assert_eq!(hp_key_name(KEY_ENTER), Some("Enter"));
        assert_eq!(hp_key_name(KEY_ESC), Some("Esc"));
        assert_eq!(hp_key_name(KEY_UP), Some("Up"));
        assert_eq!(hp_key_name(KEY_0), Some("0"));
        assert_eq!(hp_key_name(KEY_7), Some("7"));
        assert_eq!(hp_key_name(KEY_PLUS), Some("+"));
        assert_eq!(hp_key_name(51), None);
        assert_eq!(hp_key_name(-1), None);
    }

    #[test]
    fn test_egui_key_to_hp() {
        use egui::Key;
        assert_eq!(egui_key_to_hp(Key::Num0), Some(KEY_0));
        assert_eq!(egui_key_to_hp(Key::Num9), Some(KEY_9));
        assert_eq!(egui_key_to_hp(Key::Plus), Some(KEY_PLUS));
        assert_eq!(egui_key_to_hp(Key::Minus), Some(KEY_MINUS));
        assert_eq!(egui_key_to_hp(Key::Slash), Some(KEY_DIV));
        assert_eq!(egui_key_to_hp(Key::Backspace), Some(KEY_DEL));
        assert_eq!(egui_key_to_hp(Key::Delete), Some(KEY_DEL));
        assert_eq!(egui_key_to_hp(Key::Space), Some(KEY_ANS));
        assert_eq!(egui_key_to_hp(Key::Tab), Some(KEY_ALPHA));
        assert_eq!(egui_key_to_hp(Key::Enter), Some(KEY_ENTER));
        assert_eq!(egui_key_to_hp(Key::Escape), Some(KEY_ESC));
        assert_eq!(egui_key_to_hp(Key::X), Some(KEY_TIMES));
        assert_eq!(egui_key_to_hp(Key::S), Some(KEY_SIN));
        assert_eq!(egui_key_to_hp(Key::F12), Some(KEY_ON));
    }

    #[test]
    fn test_unified_key_mapping_matrix() {
        use std::collections::HashSet;

        assert_eq!(KEY_MAPPINGS.len(), 51);
        let mut device_codes = HashSet::new();

        for (idx, mapping) in KEY_MAPPINGS.iter().enumerate() {
            assert_eq!(mapping.physical_id as usize, idx);
            assert_eq!(mapping.ppl_code, idx as i32);
            assert!(!mapping.primary_label.is_empty());
            assert!(
                device_codes.insert(mapping.device_code),
                "Duplicate device code {:#06X} at key {}",
                mapping.device_code,
                idx
            );

            // Lookup by physical
            let by_phys = lookup_by_physical(idx as u8).expect("lookup_by_physical");
            assert_eq!(by_phys.physical_id, idx as u8);
            assert_eq!(by_phys.ppl_code, idx as i32);

            // Lookup by PPL code
            let by_ppl = lookup_by_ppl_code(idx as i32).expect("lookup_by_ppl_code");
            assert_eq!(by_ppl.physical_id, idx as u8);

            // Lookup by device code
            let by_dev = lookup_by_device_code(mapping.device_code).expect("lookup_by_device_code");
            assert_eq!(by_dev.physical_id, idx as u8);

            // If pc_key is present, lookup_by_pc_key must return this mapping (or canonical alias)
            if let Some(pc_k) = mapping.pc_key {
                let by_pc = lookup_by_pc_key(pc_k).expect("lookup_by_pc_key");
                assert_eq!(by_pc.ppl_code, idx as i32);
            }
        }

        // Out of range lookups
        assert!(lookup_by_physical(51).is_none());
        assert!(lookup_by_physical(255).is_none());
        assert!(lookup_by_ppl_code(-1).is_none());
        assert!(lookup_by_ppl_code(51).is_none());
        assert!(lookup_by_device_code(0xFFFF).is_none());

        // Function keys match official HP skin
        assert_eq!(lookup_by_pc_key(egui::Key::F1).unwrap().ppl_code, 0); // Apps
        assert_eq!(lookup_by_pc_key(egui::Key::F2).unwrap().ppl_code, 1); // Symb
        assert_eq!(lookup_by_pc_key(egui::Key::F3).unwrap().ppl_code, 3); // Help
        assert_eq!(lookup_by_pc_key(egui::Key::F4).unwrap().ppl_code, 5); // Home
        assert_eq!(lookup_by_pc_key(egui::Key::F5).unwrap().ppl_code, 6); // Plot
        assert_eq!(lookup_by_pc_key(egui::Key::F6).unwrap().ppl_code, 9); // View
        assert_eq!(lookup_by_pc_key(egui::Key::F7).unwrap().ppl_code, 10); // CAS
        assert_eq!(lookup_by_pc_key(egui::Key::F8).unwrap().ppl_code, 11); // Num
        assert_eq!(lookup_by_pc_key(egui::Key::F9).unwrap().ppl_code, 13); // Menu
        assert_eq!(lookup_by_pc_key(egui::Key::Tab).unwrap().ppl_code, 36); // ALPHA
    }
}
