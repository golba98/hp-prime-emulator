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

/// Maps an egui key to an HP Prime keycode (0–50 grid).
pub fn egui_key_to_hp(key: egui::Key) -> Option<i64> {
    use egui::Key::*;
    Some(match key {
        // Controls & Navigation
        Enter => KEY_ENTER,
        Escape => KEY_ESC,
        Backspace | Delete => KEY_DEL,
        Space => KEY_ANS,
        Tab => KEY_TEMPLATE,
        ArrowUp => KEY_UP,
        ArrowLeft => KEY_LEFT,
        ArrowRight => KEY_RIGHT,
        ArrowDown => KEY_DOWN,
        Home => KEY_HOME,
        End => KEY_MENU,
        PageUp => KEY_PLOT,
        PageDown => KEY_NUM,

        // Digits
        Num0 => KEY_0,
        Num1 => KEY_1,
        Num2 => KEY_2,
        Num3 => KEY_3,
        Num4 => KEY_4,
        Num5 => KEY_5,
        Num6 => KEY_6,
        Num7 => KEY_7,
        Num8 => KEY_8,
        Num9 => KEY_9,

        // Operators & Punctuation
        Plus => KEY_PLUS,
        Minus => KEY_MINUS,
        Slash => KEY_DIV,
        Period => KEY_DOT,
        Comma => KEY_COMMA,
        Equals => KEY_ENTER,

        // Letters & Mathematical Shortcuts
        X => KEY_TIMES,
        S => KEY_SIN,
        C => KEY_COS,
        T => KEY_TAN,
        L => KEY_LN,
        E => KEY_EEX,

        // Function Keys (matching physical layout)
        F1 => KEY_APPS,
        F2 => KEY_SYMB,
        F3 => KEY_PLOT,
        F4 => KEY_NUM,
        F5 => KEY_CAS,
        F6 => KEY_TOOLBOX,
        F7 => KEY_MENU,
        F8 => KEY_VARS,
        F9 => KEY_HELP,
        F10 => KEY_VIEW,
        F11 => KEY_POW,
        F12 => KEY_ON,

        _ => return None,
    })
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
        assert_eq!(egui_key_to_hp(Key::Tab), Some(KEY_TEMPLATE));
        assert_eq!(egui_key_to_hp(Key::Enter), Some(KEY_ENTER));
        assert_eq!(egui_key_to_hp(Key::Escape), Some(KEY_ESC));
        assert_eq!(egui_key_to_hp(Key::X), Some(KEY_TIMES));
        assert_eq!(egui_key_to_hp(Key::S), Some(KEY_SIN));
        assert_eq!(egui_key_to_hp(Key::F12), Some(KEY_ON));
    }
}
