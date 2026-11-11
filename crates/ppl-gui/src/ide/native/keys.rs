//! Physical calculator actions, independent of egui and firmware transport.
//! Scan codes and legends remain in keypad::KEYS; this table supplies behavior.
use super::expression::{EditCommand, TemplateKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Apps,
    Info,
    Symbolic,
    SymbolicSetup,
    Plot,
    PlotSetup,
    Numeric,
    NumericSetup,
    View,
    Menu,
    HomeSettings,
    Cas,
    CasSettings,
    Help,
    User,
    Variables,
    Toolbox,
    Templates,
    Characters,
    Memory,
    Units,
    Define,
    Base,
    Lists,
    Matrices,
    Notes,
    Symbols,
    Relations,
    Sexagesimal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Insert(&'static str),
    Template(TemplateKind),
    Edit(EditCommand),
    Tool(Tool),
    Home,
    AppVariable,
    Fraction,
    Sexagesimal,
    Polar,
    Programs,
    Back,
    History(i8),
    Evaluate,
    Approximate,
    Copy,
    Paste,
    Sign,
    Shift,
    Alpha,
    On,
    Off,
    None,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlphaMode {
    #[default]
    Off,
    Once {
        lower: bool,
    },
    Locked {
        lower: bool,
    },
}
impl AlphaMode {
    /// Alpha presses preserve the selected case until the next character.
    /// Shift+Alpha changes a lock's case; Alpha, Shift, Alpha locks the
    /// opposite case (HP Prime User Guide, "Entering text", p. 59).
    pub fn press(&mut self, shift: bool, default_lower: bool) {
        *self = match *self {
            Self::Off => Self::Once {
                lower: default_lower ^ shift,
            },
            Self::Once { lower } => Self::Locked {
                lower: lower ^ shift,
            },
            Self::Locked { lower } if shift => Self::Locked { lower: !lower },
            Self::Locked { lower } if lower == default_lower => Self::Off,
            Self::Locked { .. } => Self::Once {
                lower: default_lower,
            },
        };
    }
    pub fn toggle(&mut self, lower: bool) {
        self.press(false, lower);
    }
    pub fn consume(&mut self, shift: bool) -> Option<bool> {
        let lower = match *self {
            Self::Off => return None,
            Self::Once { lower } | Self::Locked { lower } => lower ^ shift,
        };
        if matches!(self, Self::Once { .. }) {
            *self = Self::Off;
        }
        Some(lower)
    }
    pub fn indicator(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Once { lower: false } => "A",
            Self::Once { lower: true } => "a",
            Self::Locked { lower: false } => "A🔒",
            Self::Locked { lower: true } => "a🔒",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct KeyActions {
    pub primary: KeyAction,
    pub shifted: KeyAction,
}
use EditCommand as E;
use KeyAction as A;
use TemplateKind as T;
use Tool as U;
pub const ACTIONS: [KeyActions; 51] = [
    KeyActions {
        primary: A::Tool(U::Apps),
        shifted: A::Tool(U::Info),
    },
    KeyActions {
        primary: A::Tool(U::Symbolic),
        shifted: A::Tool(U::SymbolicSetup),
    },
    KeyActions {
        primary: A::History(-1),
        shifted: A::History(-1),
    },
    KeyActions {
        primary: A::Tool(U::Help),
        shifted: A::Tool(U::User),
    },
    KeyActions {
        primary: A::Back,
        shifted: A::Edit(E::Clear),
    },
    KeyActions {
        primary: A::Home,
        shifted: A::Tool(U::HomeSettings),
    },
    KeyActions {
        primary: A::Tool(U::Plot),
        shifted: A::Tool(U::PlotSetup),
    },
    KeyActions {
        primary: A::Edit(E::Left(false)),
        shifted: A::Edit(E::Left(true)),
    },
    KeyActions {
        primary: A::Edit(E::Right(false)),
        shifted: A::Edit(E::Right(true)),
    },
    KeyActions {
        primary: A::Tool(U::View),
        shifted: A::Copy,
    },
    KeyActions {
        primary: A::Tool(U::Cas),
        shifted: A::Tool(U::CasSettings),
    },
    KeyActions {
        primary: A::Tool(U::Numeric),
        shifted: A::Tool(U::NumericSetup),
    },
    KeyActions {
        primary: A::History(1),
        shifted: A::History(1),
    },
    KeyActions {
        primary: A::Tool(U::Menu),
        shifted: A::Paste,
    },
    KeyActions {
        primary: A::Tool(U::Variables),
        shifted: A::Tool(U::Characters),
    },
    KeyActions {
        primary: A::Tool(U::Toolbox),
        shifted: A::Tool(U::Memory),
    },
    KeyActions {
        primary: A::Tool(U::Templates),
        shifted: A::Tool(U::Units),
    },
    KeyActions {
        primary: A::AppVariable,
        shifted: A::Tool(U::Define),
    },
    KeyActions {
        primary: A::Fraction,
        shifted: A::Sexagesimal,
    },
    KeyActions {
        primary: A::Edit(E::Backspace),
        shifted: A::Edit(E::Delete),
    },
    KeyActions {
        primary: A::Template(T::Power),
        shifted: A::Template(T::NthRoot),
    },
    KeyActions {
        primary: A::Template(T::Function("SIN")),
        shifted: A::Template(T::Function("ASIN")),
    },
    KeyActions {
        primary: A::Template(T::Function("COS")),
        shifted: A::Template(T::Function("ACOS")),
    },
    KeyActions {
        primary: A::Template(T::Function("TAN")),
        shifted: A::Template(T::Function("ATAN")),
    },
    KeyActions {
        primary: A::Template(T::Function("LN")),
        shifted: A::Template(T::Function("EXP")),
    },
    KeyActions {
        primary: A::Template(T::Function("LOG")),
        shifted: A::Template(T::Function("ALOG")),
    },
    KeyActions {
        primary: A::Insert("^2"),
        shifted: A::Template(T::Root),
    },
    KeyActions {
        primary: A::Sign,
        shifted: A::Template(T::Function("ABS")),
    },
    KeyActions {
        primary: A::Template(T::Parentheses),
        shifted: A::Insert("!"),
    },
    KeyActions {
        primary: A::Insert(","),
        shifted: A::Evaluate,
    },
    KeyActions {
        primary: A::Evaluate,
        shifted: A::Approximate,
    },
    KeyActions {
        primary: A::Insert("E"),
        shifted: A::Insert("▶"),
    },
    KeyActions {
        primary: A::Insert("7"),
        shifted: A::Tool(U::Lists),
    },
    KeyActions {
        primary: A::Insert("8"),
        shifted: A::Template(T::List),
    },
    KeyActions {
        primary: A::Insert("9"),
        shifted: A::Tool(U::Symbols),
    },
    KeyActions {
        primary: A::Insert("/"),
        shifted: A::Insert("^(-1)"),
    },
    KeyActions {
        primary: A::Alpha,
        shifted: A::Alpha,
    },
    KeyActions {
        primary: A::Insert("4"),
        shifted: A::Tool(U::Matrices),
    },
    KeyActions {
        primary: A::Insert("5"),
        shifted: A::Template(T::Matrix),
    },
    KeyActions {
        primary: A::Insert("6"),
        shifted: A::Tool(U::Relations),
    },
    KeyActions {
        primary: A::Insert("*"),
        shifted: A::Polar,
    },
    KeyActions {
        primary: A::Shift,
        shifted: A::Shift,
    },
    KeyActions {
        primary: A::Insert("1"),
        shifted: A::Programs,
    },
    KeyActions {
        primary: A::Insert("2"),
        shifted: A::Insert("i"),
    },
    KeyActions {
        primary: A::Insert("3"),
        shifted: A::Insert("π"),
    },
    KeyActions {
        primary: A::Insert("-"),
        shifted: A::Tool(U::Base),
    },
    KeyActions {
        primary: A::On,
        shifted: A::Off,
    },
    KeyActions {
        primary: A::Insert("0"),
        shifted: A::Tool(U::Notes),
    },
    KeyActions {
        primary: A::Insert("."),
        shifted: A::Insert("="),
    },
    KeyActions {
        primary: A::Insert(" "),
        shifted: A::Insert("_"),
    },
    KeyActions {
        primary: A::Insert("+"),
        shifted: A::Insert("Ans"),
    },
];
pub fn resolve_key(code: i64, shifted: bool) -> Option<KeyAction> {
    ACTIONS
        .get(usize::try_from(code).ok()?)
        .map(|a| if shifted { a.shifted } else { a.primary })
}
pub fn alpha_character(code: i64, lower: bool) -> Option<String> {
    let key = super::super::keypad::KEYS.get(usize::try_from(code).ok()?)?;
    if key.alpha.is_empty() {
        return None;
    }
    Some(if code == 47 {
        "\"".into()
    } else if lower {
        key.alpha.to_lowercase()
    } else {
        key.alpha.into()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_physical_key_has_explicit_actions() {
        for code in 0..51 {
            assert!(resolve_key(code, false).is_some());
            assert!(resolve_key(code, true).is_some());
        }
        assert!(resolve_key(-1, false).is_none());
        assert!(resolve_key(51, false).is_none());
    }
    #[test]
    fn inverse_functions_and_skin_labels_are_distinct() {
        assert_eq!(
            resolve_key(21, true),
            Some(A::Template(T::Function("ASIN")))
        );
        assert_eq!(resolve_key(49, false), Some(A::Insert(" ")));
        assert_eq!(resolve_key(50, true), Some(A::Insert("Ans")));
        for code in [
            21, 22, 23, 24, 25, 26, 27, 28, 30, 31, 35, 45, 46, 48, 49, 50,
        ] {
            assert_ne!(resolve_key(code, false), resolve_key(code, true));
        }
    }
    #[test]
    fn alpha_case_and_lock() {
        let mut a = AlphaMode::default();
        a.toggle(false);
        assert_eq!(a.consume(false), Some(false));
        assert_eq!(a, AlphaMode::Off);
        a.toggle(false);
        a.toggle(false);
        assert_eq!(a.consume(true), Some(true));
        assert_eq!(a, AlphaMode::Locked { lower: false });
        a.toggle(false);
        assert_eq!(a, AlphaMode::Off);
        assert_eq!(alpha_character(14, false).as_deref(), Some("A"));
        assert_eq!(alpha_character(43, true).as_deref(), Some("z"));
    }
    #[test]
    fn opposite_case_lock_sequences_follow_entering_text_table() {
        for default_lower in [false, true] {
            let mut alpha = AlphaMode::Off;
            alpha.press(false, default_lower);
            alpha.press(true, default_lower);
            assert_eq!(
                alpha,
                AlphaMode::Locked {
                    lower: !default_lower
                }
            );
            assert_eq!(alpha.consume(false), Some(!default_lower));
            assert_eq!(alpha.consume(true), Some(default_lower));
            assert_eq!(
                alpha,
                AlphaMode::Locked {
                    lower: !default_lower
                }
            );
            alpha.press(true, default_lower);
            assert_eq!(
                alpha,
                AlphaMode::Locked {
                    lower: default_lower
                }
            );
            alpha.press(false, default_lower);
            assert_eq!(alpha, AlphaMode::Off);
        }
    }
    #[test]
    fn every_printed_alpha_label_has_both_cases_and_one_shot_consumption() {
        for key in &crate::ide::keypad::KEYS {
            if key.alpha.is_empty() {
                continue;
            }
            for lower in [false, true] {
                let expected = if key.code == 47 {
                    "\"".to_owned()
                } else if lower {
                    key.alpha.to_lowercase()
                } else {
                    key.alpha.to_owned()
                };
                assert_eq!(
                    alpha_character(key.code, lower).as_deref(),
                    Some(expected.as_str())
                );
                let mut alpha = AlphaMode::Once { lower };
                assert_eq!(alpha.consume(false), Some(lower));
                assert_eq!(alpha, AlphaMode::Off);
                let mut alpha = AlphaMode::Locked { lower };
                assert_eq!(alpha.consume(true), Some(!lower));
                assert_eq!(alpha, AlphaMode::Locked { lower });
            }
        }
    }
}
