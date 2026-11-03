//! Desktop text via consumed physical keys and observed firmware Alpha state.
//!
//! The yellow A..Z/a..z status glyphs (and cyan Shift glyph) were captured
//! from local G1 build 15751 in Home, CAS and the program editor. They are
//! evidence of the firmware's active case, not a guess based on the last
//! Home/CAS key. Unrecognised status bars fail closed. No guest text or
//! modifier memory is written.
use std::collections::VecDeque;

const ALPHA: u16 = 0xb6;
const SHIFT: u16 = 0x8b;
const LIMIT: usize = 2048;
const UPPER: [u32; 8] = [0, 253980, 131106, 65570, 32830, 16418, 8226, 256674];
const LOWER: [u32; 8] = [0, 0, 0, 253980, 65568, 32828, 16418, 256700];
const LOCKED_UPPER: [u32; 8] = [0, 255752, 133276, 67774, 36764, 18588, 10396, 256156];
const SHIFT_GLYPH: [u32; 8] = [0, 1800, 2204, 190, 1820, 2076, 2204, 1820];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modifiers {
    pub lower: Option<bool>,
    pub shift: bool,
}
/// The input pump can park before the main UI processes a delivered key.
/// Character translation must observe a fresh main-loop wait after that press.
#[derive(Default)]
pub struct UiAcknowledgement {
    last_press_wait: Option<u64>,
}
impl UiAcknowledgement {
    pub fn baseline(&self) -> Option<u64> {
        self.last_press_wait
    }
    pub fn pressed(&mut self, main_wait: u64) {
        self.last_press_wait = Some(main_wait);
    }
    pub fn ready(&self, main_waiting: bool, main_wait: u64) -> bool {
        main_waiting
            && self
                .last_press_wait
                .is_none_or(|previous| main_wait > previous)
    }
}
/// Recognise only the exact indicator glyphs in the firmware framebuffer.
/// Unrelated colours/text, absent status bars and partial frames are rejected.
pub fn observe(bytes: &[u8]) -> Option<Modifiers> {
    if bytes.len() < 320 * 8 * 4 {
        return None;
    }
    let mut alpha = [0u32; 8];
    let mut shift = [0u32; 8];
    for y in 0..8 {
        for x in 0..34 {
            let i = (y * 320 + x) * 4;
            let (b, g, r) = (bytes[i], bytes[i + 1], bytes[i + 2]);
            if r > 150 && g > 120 && b < 100 && x >= 14 {
                alpha[y] |= 1 << (x - 14);
            } else if r < 60 && g > 180 && b > 180 && x < 14 {
                shift[y] |= 1 << x;
            } else if !(r < 60 && (65..=120).contains(&g) && (140..=190).contains(&b)) {
                return None;
            }
        }
    }
    let lower = match alpha {
        UPPER | LOCKED_UPPER => Some(false),
        LOWER => Some(true),
        mask if mask == [0; 8] => None,
        _ => return None,
    };
    let shift = match shift {
        SHIFT_GLYPH => true,
        mask if mask == [0; 8] => false,
        _ => return None,
    };
    Some(Modifiers { lower, shift })
}

#[derive(Debug)]
enum Item {
    Key(u16, bool),
    Character(char),
}
#[derive(Default)]
pub struct Typing {
    queue: VecDeque<Item>,
    /// Desktop-generated paired delimiters; physical editing invalidates this.
    pairs: Vec<char>,
    attempts: u8,
}
impl Typing {
    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }
    pub fn observes_modifiers(&self) -> bool {
        matches!(self.queue.front(), Some(Item::Character(_)))
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn clear(&mut self) {
        self.queue.clear();
        self.pairs.clear();
        self.attempts = 0;
    }
    pub fn key(&mut self, code: u16, down: bool) {
        self.pairs.clear();
        self.queue.push_back(Item::Key(code, down));
    }
    /// Validate the complete packet before queuing any part of it. Pasted
    /// newlines are never Enter, and unsupported Unicode is never discarded.
    pub fn paste(&mut self, text: &str) -> Result<(), String> {
        if text.contains('^') {
            return Err("Calculator paste does not translate linear powers (^): use the power key (F11), then Right to leave the exponent. No text from this paste was queued".into());
        }
        self.text(text)
    }
    pub fn text(&mut self, text: &str) -> Result<(), String> {
        if self.queue.len() + text.chars().count() > LIMIT {
            return Err("Calculator typing queue is full; wait for the current text or press Escape to cancel".into());
        }
        let mut pairs = self.pairs.clone();
        // Include not-yet-delivered text when validating a subsequent packet.
        for item in &self.queue {
            match item {
                Item::Character(ch) => delimiter(*ch, &mut pairs)?,
                Item::Key(..) => pairs.clear(),
            }
        }
        for ch in text.chars() {
            if !supported(ch) {
                return Err(format!(
                    "Calculator paste/typing does not support {ch:?}; use the IDE source editor and Import for PPL source. No text from this packet was queued"
                ));
            }
            delimiter(ch, &mut pairs)?;
        }
        self.queue.extend(text.chars().map(Item::Character));
        Ok(())
    }
    /// Called only after previously delivered key transitions were consumed.
    /// Return one tap (or raw key transition), then observe the status again.
    pub fn next(&mut self, modifiers: Option<Modifiers>) -> Result<Vec<(u16, bool)>, String> {
        let Some(item) = self.queue.front() else {
            return Ok(vec![]);
        };
        if let Item::Key(code, down) = item {
            let key = (*code, *down);
            self.queue.pop_front();
            self.pairs.clear();
            self.attempts = 0;
            return Ok(vec![key]);
        }
        let Item::Character(ch) = item else {
            unreachable!()
        };
        let ch = *ch;
        let Some(m) = modifiers else {
            self.clear();
            return Err("Desktop typing stopped: this firmware screen has no recognised Alpha indicator. Use the calculator keypad, or return to Home/CAS/the program editor".into());
        };
        self.attempts += 1;
        if self.attempts > 6 {
            self.clear();
            return Err("Desktop typing stopped: firmware modifier state did not settle; use the calculator keypad to clear Shift/Alpha".into());
        }
        let wants_alpha = ch.is_ascii_alphabetic() || matches!(ch, ':' | ';');
        let modifier = if wants_alpha {
            match m.lower {
                None if m.shift => Some(SHIFT),
                None => Some(ALPHA),
                Some(lower) if ch.is_ascii_alphabetic() && lower != ch.is_ascii_lowercase() => {
                    Some(SHIFT)
                }
                _ => None,
            }
        } else if m.lower.is_some() {
            Some(ALPHA) // one-shot -> lock -> off, observed after each tap
        } else if m.shift {
            Some(SHIFT)
        } else {
            None
        };
        if let Some(code) = modifier {
            return Ok(tap(code));
        }
        self.queue.pop_front();
        self.attempts = 0;
        let keys = match ch {
            c if c.is_ascii_alphabetic() => vec![c.to_ascii_uppercase() as u16],
            '(' => {
                self.pairs.push(')');
                vec![0x4e]
            }
            '[' => {
                self.pairs.push(']');
                vec![SHIFT, 0x56]
            }
            '{' => {
                self.pairs.push('}');
                vec![SHIFT, 0x52]
            }
            ')' | ']' | '}' => {
                self.pairs.pop();
                vec![0x04]
            }
            '=' => vec![SHIFT, 0xb8],
            '_' => vec![SHIFT, 0x20],
            ':' => vec![0xb7],
            ';' => vec![0xb9],
            _ => vec![plain_key(ch).expect("prevalidated character")],
        };
        Ok(keys.into_iter().flat_map(tap).collect())
    }
}
fn tap(code: u16) -> Vec<(u16, bool)> {
    vec![(code, true), (code, false)]
}
fn delimiter(ch: char, pairs: &mut Vec<char>) -> Result<(), String> {
    match ch {
        '(' => pairs.push(')'), '[' => pairs.push(']'), '{' => pairs.push('}'),
        ')' | ']' | '}' if pairs.pop() != Some(ch) => return Err("Closing delimiter has no desktop-created pair; use the calculator Right key to leave an existing template".into()),
        _ => {}
    }
    Ok(())
}
fn supported(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || plain_key(ch).is_some()
        || matches!(
            ch,
            '(' | ')' | '[' | ']' | '{' | '}' | '=' | '_' | ':' | ';'
        )
}
fn plain_key(ch: char) -> Option<u16> {
    Some(match ch {
        '0' => 0x30,
        '1' => 0x59,
        '2' => 0x5a,
        '3' => 0x33,
        '4' => 0x55,
        '5' => 0x56,
        '6' => 0x57,
        '7' => 0x51,
        '8' => 0x52,
        '9' => 0x53,
        '+' => 0xb9,
        '-' => 0xb7,
        '*' => 0x58,
        '/' => 0x54,
        '.' => 0xb8,
        ',' => 0x4f,
        ' ' => 0x20,
        '^' => 0x46,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(lower: Option<bool>, shift: bool) -> Option<Modifiers> {
        Some(Modifiers { lower, shift })
    }
    #[test]
    fn input_pump_consumption_does_not_acknowledge_main_ui_rendering() {
        let mut ack = UiAcknowledgement::default();
        assert!(
            !ack.ready(false, 7),
            "an executing renderer is not an observation point"
        );
        assert!(ack.ready(true, 7));
        ack.pressed(7);
        assert!(
            !ack.ready(true, 7),
            "old main wait can remain flagged while the pump has already parked"
        );
        assert!(
            !ack.ready(false, 8),
            "a new wait followed by resumed rendering is not settled"
        );
        assert!(ack.ready(true, 8));
        ack.pressed(8);
        assert!(!ack.ready(true, 8));
        assert!(ack.ready(true, 9));
    }
    #[test]
    fn raw_key_releases_do_not_need_modifier_observation() {
        let mut typing = Typing::default();
        typing.key(ALPHA, false);
        assert!(!typing.observes_modifiers());
        assert_eq!(typing.next(None).unwrap(), vec![(ALPHA, false)]);
        typing.text("a").unwrap();
        assert!(typing.observes_modifiers());
    }
    #[test]
    fn letter_uses_observed_case_instead_of_home_assumption() {
        for (wanted, lower) in [('a', true), ('A', false)] {
            let mut t = Typing::default();
            t.text(&wanted.to_string()).unwrap();
            assert_eq!(t.next(state(None, false)).unwrap(), tap(ALPHA));
            assert_eq!(t.next(state(Some(!lower), false)).unwrap(), tap(SHIFT));
            assert_eq!(t.next(state(Some(lower), true)).unwrap(), tap(0x41));
            assert!(t.is_empty());
        }
    }
    #[test]
    fn latched_alpha_is_not_toggled_before_a_matching_letter() {
        let mut t = Typing::default();
        t.text("a").unwrap();
        assert_eq!(t.next(state(Some(true), false)).unwrap(), tap(0x41));
    }
    #[test]
    fn rejected_paste_is_atomic_and_newlines_never_execute() {
        for s in ["abc\n", "abcλ", "abc)"] {
            let mut t = Typing::default();
            assert!(t.text(s).is_err());
            assert!(t.is_empty());
        }
    }
    #[test]
    fn closing_parenthesis_leaves_the_pair_instead_of_inserting_another() {
        let mut t = Typing::default();
        t.text("(a)").unwrap();
        assert_eq!(t.next(state(None, false)).unwrap(), tap(0x4e));
        assert_eq!(t.next(state(Some(true), false)).unwrap(), tap(0x41));
        assert_eq!(t.next(state(None, false)).unwrap(), tap(0x04));
    }
    #[test]
    fn queue_limit_and_rejected_packet_preserve_preceding_input() {
        let mut t = Typing::default();
        t.text("a").unwrap();
        assert!(t.text(&"b".repeat(LIMIT)).is_err());
        assert!(t.text("b\n").is_err());
        assert_eq!(t.next(state(Some(true), false)).unwrap(), tap(0x41));
        assert!(t.is_empty());
        t.text("hello").unwrap();
        t.clear();
        assert!(t.next(state(None, false)).unwrap().is_empty());
    }
    #[test]
    fn paste_never_silently_turns_a_linear_power_into_a_template() {
        let mut t = Typing::default();
        assert!(t.paste("2^3+4").is_err());
        assert!(t.is_empty());
        assert!(t.text("^").is_ok()); // explicit desktop power shortcut
    }
    #[test]
    fn physical_edit_invalidates_the_desktop_pair() {
        let mut t = Typing::default();
        t.text("(").unwrap();
        t.next(state(None, false)).unwrap();
        t.key(0x02, true);
        assert!(t.text(")").is_err());
        let mut t = Typing::default();
        t.text("(").unwrap();
        t.key(0x02, true);
        assert!(
            t.text(")").is_err(),
            "queued cursor moves invalidate queued pairs too"
        );
    }
    #[test]
    fn unknown_frame_and_nonsettling_modifiers_fail_closed() {
        let mut t = Typing::default();
        t.text("hello").unwrap();
        assert!(t.next(None).is_err());
        assert!(t.is_empty());
        t.text("a").unwrap();
        for _ in 0..6 {
            t.next(state(None, false)).unwrap();
        }
        assert!(t.next(state(None, false)).is_err());
    }
    #[test]
    fn observed_firmware_crops_identify_actual_home_cas_editor_modifiers() {
        for (name, expected) in [
            ("neutral", state(None, false)),
            ("home_upper", state(Some(false), false)),
            ("cas_lower", state(Some(true), false)),
            ("locked_upper", state(Some(false), false)),
            ("shift", state(None, true)),
            ("editor_shift_lower", state(Some(true), true)),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/fixtures/typing/{name}.png"));
            let decoder =
                png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
            let mut reader = decoder.read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (34, 8));
            assert_eq!(info.color_type, png::ColorType::Rgb);
            let mut bytes = vec![0; 320 * 8 * 4];
            for y in 0..8 {
                for x in 0..34 {
                    let from = (y * 34 + x) * 3;
                    let to = (y * 320 + x) * 4;
                    bytes[to..to + 4].copy_from_slice(&[
                        pixels[from + 2],
                        pixels[from + 1],
                        pixels[from],
                        0,
                    ]);
                }
            }
            assert_eq!(observe(&bytes), expected, "{name}");
        }
    }
    #[test]
    fn exact_status_masks_reject_an_unknown_screen() {
        let mut bytes = vec![0; 320 * 8 * 4];
        assert_eq!(observe(&bytes), None);
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[163, 92, 38, 0]);
        }
        assert_eq!(observe(&bytes), state(None, false));
        for (y, row) in LOWER.iter().enumerate() {
            for x in 0..20 {
                if row & (1 << x) != 0 {
                    let i = (y * 320 + x + 14) * 4;
                    bytes[i..i + 4].copy_from_slice(&[0, 255, 255, 0]);
                }
            }
        }
        assert_eq!(observe(&bytes), state(Some(true), false));
        bytes[0..4].copy_from_slice(&[255, 255, 255, 0]);
        assert_eq!(observe(&bytes), None);
    }
}
