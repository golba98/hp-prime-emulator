//! Native User keyboard discovery and mode transitions, without UI or execution.
//!
//! HP User Guide (2016), printed pp.571–576: Shift+Help activates 1U; a
//! second Shift+Help activates ↑U; Shift+Help then disables it. Shift has no
//! assignable name, and Shift+Help/Shift+On are reserved. The guide's Vars_
//! and Ente spellings conflict with earlier HP tables; documented aliases are
//! accepted explicitly and conflicts are reported, never silently prioritized.

use ppl_syntax::{
    ast::Item,
    lexer::{self, Keyword, Tok},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserMode {
    #[default]
    Off,
    Temporary,
    Persistent,
}
impl UserMode {
    pub fn indicator(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Temporary => "1U",
            Self::Persistent => "↑U",
        }
    }
    /// One complete physical Shift+Help chord, not either modifier alone.
    pub fn activate(&mut self) {
        *self = match self {
            Self::Off => Self::Temporary,
            Self::Temporary => Self::Persistent,
            Self::Persistent => Self::Off,
        };
    }
    pub fn consume(&mut self) {
        if *self == Self::Temporary {
            *self = Self::Off;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyStroke {
    pub physical: u8,
    pub shift: bool,
    pub alpha: bool,
}
impl KeyStroke {
    pub fn new(physical: i64, shift: bool, alpha: bool) -> Result<Self, ResolveError> {
        let physical = u8::try_from(physical)
            .ok()
            .filter(|code| *code < 51)
            .ok_or(ResolveError::InvalidPhysicalKey(physical))?;
        Ok(Self {
            physical,
            shift,
            alpha,
        })
    }
    pub fn is_user_chord(self) -> bool {
        self.physical == 3 && self.shift && !self.alpha
    }
    pub fn internal_name(self) -> Option<&'static str> {
        KEY_NAMES
            .get(self.physical as usize)
            .and_then(|names| match (self.shift, self.alpha) {
                (false, false) => names.primary,
                (true, false) => names.shifted,
                (false, true) => names.alpha,
                (true, true) => names.shift_alpha,
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyNames {
    pub primary: Option<&'static str>,
    pub shifted: Option<&'static str>,
    pub alpha: Option<&'static str>,
    pub shift_alpha: Option<&'static str>,
}
macro_rules! key {
    ($suffix:literal) => {
        KeyNames {
            primary: Some(concat!("K_", $suffix)),
            shifted: Some(concat!("KS_", $suffix)),
            alpha: Some(concat!("KA_", $suffix)),
            shift_alpha: Some(concat!("KSA_", $suffix)),
        }
    };
    ($suffix:literal,reserved_shift) => {
        KeyNames {
            shifted: None,
            ..key!($suffix)
        }
    };
}
/// Physical-key order is exactly GETKEY's 0..50 order. An absent name is a
/// documented reserved modifier/control, not a fabricated handler name.
pub const KEY_NAMES: [KeyNames; 51] = [
    key!("Apps"),
    key!("Symb"),
    key!("Up"),
    key!("Help", reserved_shift),
    key!("Esc"),
    key!("Home"),
    key!("Plot"),
    key!("Left"),
    key!("Right"),
    key!("View"),
    key!("Cas"),
    key!("Num"),
    key!("Down"),
    key!("Menu"),
    key!("Vars"),
    key!("Math"),
    key!("Templ"),
    key!("Xttn"),
    key!("Abc"),
    key!("Bksp"),
    key!("Power"),
    key!("Sin"),
    key!("Cos"),
    key!("Tan"),
    key!("Ln"),
    key!("Log"),
    key!("Sq"),
    key!("Neg"),
    key!("Paren"),
    key!("Comma"),
    key!("Enter"),
    key!("Eex"),
    key!("7"),
    key!("8"),
    key!("9"),
    key!("Div"),
    key!("Alpha"),
    key!("4"),
    key!("5"),
    key!("6"),
    key!("Mul"),
    KeyNames {
        primary: None,
        shifted: None,
        alpha: None,
        shift_alpha: None,
    },
    key!("1"),
    key!("2"),
    key!("3"),
    key!("Minus"),
    key!("On", reserved_shift),
    key!("0"),
    key!("Dot"),
    key!("Space"),
    key!("Plus"),
];

/// Exact, case-sensitive names only. These aliases are explicitly printed in
/// HP's guides; normal names that merely start with K_ are not handlers.
pub fn stroke_for_name(name: &str) -> Option<KeyStroke> {
    for physical in 0..51 {
        for (shift, alpha) in [(false, false), (true, false), (false, true), (true, true)] {
            let stroke = KeyStroke {
                physical,
                shift,
                alpha,
            };
            if stroke.internal_name() == Some(name) {
                return Some(stroke);
            }
        }
    }
    match name {
        "K_Vars_" => KeyStroke::new(14, false, false).ok(),
        "KS_Vars_" => KeyStroke::new(14, true, false).ok(),
        "KA_Vars_" => KeyStroke::new(14, false, true).ok(),
        "KSA_Vars_" => KeyStroke::new(14, true, true).ok(),
        "K_Ente" => KeyStroke::new(30, false, false).ok(),
        // The first-edition guide used Xfcn, later editions use Xttn.
        "K_Xfcn" => KeyStroke::new(17, false, false).ok(),
        "KS_Xfcn" => KeyStroke::new(17, true, false).ok(),
        "KA_Xfcn" => KeyStroke::new(17, false, true).ok(),
        "KSA_Xfcn" => KeyStroke::new(17, true, true).ok(),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub name: String,
    pub source: String,
}
impl SourceSnapshot {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandlerDescriptor {
    /// Owning calculator program/module, not an arbitrary IDE active tab.
    pub name: String,
    /// Exact immutable source revision used during discovery.
    pub source: String,
    /// Actual declared KEY function, preserving canonical/alias spelling.
    pub entry: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryIssue {
    UnknownKeyName {
        program: String,
        entry: String,
    },
    SourceErrors {
        program: String,
        messages: Vec<String>,
    },
    Parameters {
        program: String,
        entry: String,
        count: usize,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    InvalidPhysicalKey(i64),
    Ambiguous {
        key: String,
        candidates: Vec<HandlerDescriptor>,
    },
    InvalidSource {
        handler: HandlerDescriptor,
        messages: Vec<String>,
    },
    HandlerParameters {
        handler: HandlerDescriptor,
        count: usize,
    },
}
impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPhysicalKey(code) => write!(f, "Invalid calculator physical key {code}"),
            Self::Ambiguous { key, candidates } => write!(
                f,
                "User key {key} is ambiguous: {}",
                candidates
                    .iter()
                    .map(|handler| format!("{}::{}", handler.name, handler.entry))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::InvalidSource { handler, messages } => write!(
                f,
                "User key {} in {} has invalid source: {}",
                handler.entry,
                handler.name,
                messages.join("; ")
            ),
            Self::HandlerParameters { handler, count } => write!(
                f,
                "User key {} in {} requires {count} parameters; key handlers must accept none",
                handler.entry, handler.name
            ),
        }
    }
}
impl std::error::Error for ResolveError {}
struct Candidate {
    source: Arc<SourceSnapshot>,
    entry: String,
    errors: Vec<String>,
    parameters: usize,
}
impl Candidate {
    fn descriptor(&self) -> HandlerDescriptor {
        HandlerDescriptor {
            name: self.source.name.clone(),
            source: self.source.source.clone(),
            entry: self.entry.clone(),
        }
    }
}
#[derive(Default)]
pub struct HandlerCatalog {
    bindings: BTreeMap<KeyStroke, Vec<Candidate>>,
    pub issues: Vec<DiscoveryIssue>,
}
impl HandlerCatalog {
    /// An identical installed/open snapshot is one source. Different revisions
    /// are not silently preferred: both remain candidates and cause ambiguity.
    pub fn discover(
        installed: &BTreeMap<String, String>,
        open: impl IntoIterator<Item = SourceSnapshot>,
    ) -> Self {
        Self::from_snapshots(
            installed
                .iter()
                .map(|(name, source)| SourceSnapshot::new(name, source))
                .chain(open),
        )
    }
    pub fn from_snapshots(sources: impl IntoIterator<Item = SourceSnapshot>) -> Self {
        let mut catalog = Self::default();
        let mut seen = BTreeSet::new();
        for source in sources {
            if !seen.insert((source.name.clone(), source.source.clone())) {
                continue;
            }
            let source = Arc::new(source);
            let (program, diagnostics) = ppl_syntax::parse(&source.source);
            let errors: Vec<_> = diagnostics
                .errors()
                .map(|error| error.message.clone())
                .collect();
            if !errors.is_empty() {
                catalog.issues.push(DiscoveryIssue::SourceErrors {
                    program: source.name.clone(),
                    messages: errors.clone(),
                });
            }
            let mut declarations = BTreeSet::new();
            for item in program.items {
                let Item::Function(function) = item else {
                    continue;
                };
                if !function.is_key {
                    continue;
                }
                declarations.insert(function.name.span.start);
                catalog.add(&source, &function.name.name, &errors, function.params.len());
            }
            // Recover names of malformed KEY declarations, without regex matching
            // comments/strings. A broken binding must report its source error,
            // not silently fall through to the default calculator key.
            let tokens = lexer::lex(&source.source, &mut Default::default());
            for pair in tokens.windows(2) {
                if pair[0].tok != Tok::Kw(Keyword::Key) {
                    continue;
                }
                let Tok::Ident(entry) = &pair[1].tok else {
                    continue;
                };
                if declarations.contains(&pair[1].span.start) {
                    continue;
                }
                let recovered = if errors.is_empty() {
                    vec!["KEY declaration could not be parsed".into()]
                } else {
                    errors.clone()
                };
                catalog.add(&source, entry, &recovered, 0);
            }
        }
        catalog
    }
    fn add(
        &mut self,
        source: &Arc<SourceSnapshot>,
        entry: &str,
        errors: &[String],
        parameters: usize,
    ) {
        let Some(stroke) = stroke_for_name(entry) else {
            self.issues.push(DiscoveryIssue::UnknownKeyName {
                program: source.name.clone(),
                entry: entry.into(),
            });
            return;
        };
        if parameters != 0 {
            self.issues.push(DiscoveryIssue::Parameters {
                program: source.name.clone(),
                entry: entry.into(),
                count: parameters,
            });
        }
        self.bindings.entry(stroke).or_default().push(Candidate {
            source: source.clone(),
            entry: entry.into(),
            errors: errors.to_vec(),
            parameters,
        });
    }
    pub fn resolve(&self, stroke: KeyStroke) -> Result<Option<HandlerDescriptor>, ResolveError> {
        if stroke.physical >= 51 {
            return Err(ResolveError::InvalidPhysicalKey(stroke.physical.into()));
        }
        let Some(candidates) = self.bindings.get(&stroke) else {
            return Ok(None);
        };
        if candidates.len() != 1 {
            return Err(ResolveError::Ambiguous {
                key: stroke.internal_name().unwrap_or("reserved").into(),
                candidates: candidates.iter().map(Candidate::descriptor).collect(),
            });
        }
        let candidate = &candidates[0];
        let handler = candidate.descriptor();
        if !candidate.errors.is_empty() {
            return Err(ResolveError::InvalidSource {
                handler,
                messages: candidate.errors.clone(),
            });
        }
        if candidate.parameters != 0 {
            return Err(ResolveError::HandlerParameters {
                handler,
                count: candidate.parameters,
            });
        }
        Ok(Some(handler))
    }
}

#[derive(Default, Clone, Debug)]
pub struct UserKeyboard {
    pub mode: UserMode,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserDispatch {
    Default,
    Consumed,
    ModeChanged(UserMode),
    Handler(HandlerDescriptor),
}
impl UserKeyboard {
    pub fn dispatch(
        &mut self,
        stroke: KeyStroke,
        catalog: &HandlerCatalog,
    ) -> Result<UserDispatch, ResolveError> {
        self.dispatch_event(stroke, false, catalog)
    }
    pub fn dispatch_event(
        &mut self,
        stroke: KeyStroke,
        repeat: bool,
        catalog: &HandlerCatalog,
    ) -> Result<UserDispatch, ResolveError> {
        if stroke.physical >= 51 {
            return Err(ResolveError::InvalidPhysicalKey(stroke.physical.into()));
        }
        if stroke.is_user_chord() {
            if repeat {
                return Ok(UserDispatch::Consumed);
            }
            self.mode.activate();
            return Ok(UserDispatch::ModeChanged(self.mode));
        }
        if self.mode == UserMode::Off || stroke.physical == 41 {
            return Ok(UserDispatch::Default);
        }
        let result = catalog.resolve(stroke);
        // Unassigned Alpha remains a modifier, so KA/KSA bindings are reachable
        // in temporary mode. An explicitly assigned Alpha is a real user key.
        if stroke.physical != 36 || !matches!(result, Ok(None)) {
            self.mode.consume();
        }
        result.map(|handler| handler.map_or(UserDispatch::Default, UserDispatch::Handler))
    }
}

/// Strings returned by KEY handlers are edit text, not quoted string literals.
/// Other objects are inserted using the calculator's typed display syntax.
/// Execution, cancellation and session writeback stay on the existing VM worker.
pub fn returned_text(value: &ppl_vm::PortableValue, settings: &ppl_vm::Settings) -> String {
    match value {
        ppl_vm::PortableValue::String(text) => text.clone(),
        value => value.display(settings),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stroke(physical: i64, shift: bool, alpha: bool) -> KeyStroke {
        KeyStroke::new(physical, shift, alpha).unwrap()
    }
    fn source(name: &str, entry: &str, text: &str) -> SourceSnapshot {
        SourceSnapshot::new(name, format!("KEY {entry}() BEGIN RETURN \"{text}\"; END;"))
    }
    fn catalog() -> HandlerCatalog {
        HandlerCatalog::from_snapshots([
            source("Scientific", "K_Sin", "ALOG("),
            source("Scientific", "KS_Sin", "ACOS("),
            source("Letters", "KA_Sin", "letter"),
            source("Letters", "KSA_Sin", "lower"),
        ])
    }
    #[test]
    fn every_physical_key_has_the_documented_internal_names() {
        let suffixes = [
            "Apps", "Symb", "Up", "Help", "Esc", "Home", "Plot", "Left", "Right", "View", "Cas",
            "Num", "Down", "Menu", "Vars", "Math", "Templ", "Xttn", "Abc", "Bksp", "Power", "Sin",
            "Cos", "Tan", "Ln", "Log", "Sq", "Neg", "Paren", "Comma", "Enter", "Eex", "7", "8",
            "9", "Div", "Alpha", "4", "5", "6", "Mul", "", "1", "2", "3", "Minus", "On", "0",
            "Dot", "Space", "Plus",
        ];
        let mut unique = BTreeSet::new();
        for (physical, suffix) in suffixes.into_iter().enumerate() {
            for (shift, alpha, prefix) in [
                (false, false, "K_"),
                (true, false, "KS_"),
                (false, true, "KA_"),
                (true, true, "KSA_"),
            ] {
                let event = stroke(physical as i64, shift, alpha);
                if physical == 41 || shift && !alpha && (physical == 3 || physical == 46) {
                    assert!(event.internal_name().is_none());
                    continue;
                }
                let name = format!("{prefix}{suffix}");
                assert_eq!(event.internal_name(), Some(name.as_str()));
                assert_eq!(stroke_for_name(&name), Some(event));
                assert!(unique.insert(name));
            }
        }
        assert_eq!(unique.len(), 198);
    }
    #[test]
    fn unknown_or_mis_capitalized_names_and_reserved_states_are_not_invented() {
        for name in [
            "K_Shift",
            "KS_Shift",
            "KS_Help",
            "KS_On",
            "K_CAS",
            "K_SIN",
            "K_Toolbox",
            "K_Unknown",
        ] {
            assert_eq!(stroke_for_name(name), None, "{name}");
        }
        assert!(KeyStroke::new(-1, false, false).is_err());
        assert!(KeyStroke::new(51, false, false).is_err());
    }
    #[test]
    fn documented_aliases_resolve_to_the_same_physical_binding() {
        assert_eq!(stroke_for_name("K_Vars_"), stroke_for_name("K_Vars"));
        assert_eq!(stroke_for_name("KS_Vars_"), stroke_for_name("KS_Vars"));
        assert_eq!(stroke_for_name("KA_Vars_"), stroke_for_name("KA_Vars"));
        assert_eq!(stroke_for_name("KSA_Vars_"), stroke_for_name("KSA_Vars"));
        assert_eq!(stroke_for_name("K_Ente"), stroke_for_name("K_Enter"));
        assert_eq!(stroke_for_name("KSA_Xfcn"), stroke_for_name("KSA_Xttn"));
    }
    #[test]
    fn user_chords_cycle_temporary_persistent_and_off_exactly() {
        let mut keyboard = UserKeyboard::default();
        let empty = HandlerCatalog::default();
        let chord = stroke(3, true, false);
        assert_eq!(
            keyboard.dispatch(chord, &empty).unwrap(),
            UserDispatch::ModeChanged(UserMode::Temporary)
        );
        assert_eq!(keyboard.mode.indicator(), "1U");
        assert_eq!(
            keyboard.dispatch(stroke(41, false, false), &empty).unwrap(),
            UserDispatch::Default
        );
        assert_eq!(keyboard.mode, UserMode::Temporary);
        assert_eq!(
            keyboard.dispatch(chord, &empty).unwrap(),
            UserDispatch::ModeChanged(UserMode::Persistent)
        );
        assert_eq!(keyboard.mode.indicator(), "↑U");
        assert_eq!(
            keyboard.dispatch(chord, &empty).unwrap(),
            UserDispatch::ModeChanged(UserMode::Off)
        );
        assert_eq!(keyboard.mode.indicator(), "");
    }
    #[test]
    fn held_activation_chord_does_not_accidentally_toggle_persistent_mode() {
        let mut keyboard = UserKeyboard::default();
        let empty = HandlerCatalog::default();
        let chord = stroke(3, true, false);
        keyboard.dispatch(chord, &empty).unwrap();
        for _ in 0..10 {
            assert_eq!(
                keyboard.dispatch_event(chord, true, &empty).unwrap(),
                UserDispatch::Consumed
            );
            assert_eq!(keyboard.mode, UserMode::Temporary);
        }
    }
    #[test]
    fn temporary_binding_uses_exact_handler_and_then_restores_default() {
        let mut keyboard = UserKeyboard {
            mode: UserMode::Temporary,
        };
        let catalog = catalog();
        let UserDispatch::Handler(handler) = keyboard
            .dispatch(stroke(21, false, false), &catalog)
            .unwrap()
        else {
            panic!("handler")
        };
        assert_eq!(
            (handler.name.as_str(), handler.entry.as_str()),
            ("Scientific", "K_Sin")
        );
        assert!(handler.source.contains("ALOG("));
        assert_eq!(keyboard.mode, UserMode::Off);
        assert_eq!(
            keyboard
                .dispatch(stroke(21, false, false), &catalog)
                .unwrap(),
            UserDispatch::Default
        );
    }
    #[test]
    fn all_modifier_names_dispatch_independently() {
        let catalog = catalog();
        for (shift, alpha, entry) in [
            (false, false, "K_Sin"),
            (true, false, "KS_Sin"),
            (false, true, "KA_Sin"),
            (true, true, "KSA_Sin"),
        ] {
            let mut keyboard = UserKeyboard {
                mode: UserMode::Persistent,
            };
            let UserDispatch::Handler(handler) = keyboard
                .dispatch(stroke(21, shift, alpha), &catalog)
                .unwrap()
            else {
                panic!("handler")
            };
            assert_eq!(handler.entry, entry);
            assert_eq!(keyboard.mode, UserMode::Persistent);
        }
    }
    #[test]
    fn unassigned_keys_fall_through_and_consume_temporary_mode() {
        let mut keyboard = UserKeyboard {
            mode: UserMode::Temporary,
        };
        assert_eq!(
            keyboard
                .dispatch(stroke(42, false, false), &catalog())
                .unwrap(),
            UserDispatch::Default
        );
        assert_eq!(keyboard.mode, UserMode::Off);
        keyboard.mode = UserMode::Persistent;
        assert_eq!(
            keyboard
                .dispatch(stroke(42, false, false), &catalog())
                .unwrap(),
            UserDispatch::Default
        );
        assert_eq!(keyboard.mode, UserMode::Persistent);
    }
    #[test]
    fn unassigned_modifiers_preserve_pending_temporary_binding() {
        let mut keyboard = UserKeyboard {
            mode: UserMode::Temporary,
        };
        let catalog = catalog();
        keyboard
            .dispatch(stroke(36, false, false), &catalog)
            .unwrap();
        assert_eq!(keyboard.mode, UserMode::Temporary);
        keyboard
            .dispatch(stroke(41, false, true), &catalog)
            .unwrap();
        assert_eq!(keyboard.mode, UserMode::Temporary);
        let UserDispatch::Handler(handler) =
            keyboard.dispatch(stroke(21, true, true), &catalog).unwrap()
        else {
            panic!("handler")
        };
        assert_eq!(handler.entry, "KSA_Sin");
        assert_eq!(keyboard.mode, UserMode::Off);
    }
    #[test]
    fn explicitly_assigned_alpha_is_a_real_user_key() {
        let catalog = HandlerCatalog::from_snapshots([source("Alpha", "K_Alpha", "A")]);
        let mut keyboard = UserKeyboard {
            mode: UserMode::Temporary,
        };
        assert!(matches!(
            keyboard
                .dispatch(stroke(36, false, false), &catalog)
                .unwrap(),
            UserDispatch::Handler(_)
        ));
        assert_eq!(keyboard.mode, UserMode::Off);
    }
    #[test]
    fn alpha_shift_help_is_assignable_and_not_the_activation_chord() {
        let catalog = HandlerCatalog::from_snapshots([source("Help", "KSA_Help", "help")]);
        let mut keyboard = UserKeyboard {
            mode: UserMode::Persistent,
        };
        assert!(matches!(
            keyboard.dispatch(stroke(3, true, true), &catalog).unwrap(),
            UserDispatch::Handler(_)
        ));
        assert_eq!(keyboard.mode, UserMode::Persistent);
    }
    #[test]
    fn installed_and_open_sources_are_discovered_and_identical_snapshots_deduplicated() {
        let installed_source = source("Scientific", "K_Sin", "LOG(");
        let installed = BTreeMap::from([(
            installed_source.name.clone(),
            installed_source.source.clone(),
        )]);
        let catalog = HandlerCatalog::discover(
            &installed,
            [installed_source, source("Other", "K_Cos", "COS(")],
        );
        assert!(catalog.resolve(stroke(21, false, false)).unwrap().is_some());
        assert!(catalog.resolve(stroke(22, false, false)).unwrap().is_some());
    }
    #[test]
    fn ambiguous_programs_revisions_and_aliases_produce_explicit_errors() {
        for sources in [
            vec![source("One", "K_Sin", "A"), source("Two", "K_Sin", "B")],
            vec![source("One", "K_Sin", "A"), source("One", "K_Sin", "B")],
            vec![source("One", "K_Vars", "A"), source("One", "K_Vars_", "B")],
        ] {
            let catalog = HandlerCatalog::from_snapshots(sources);
            let physical = if catalog.bindings.contains_key(&stroke(14, false, false)) {
                14
            } else {
                21
            };
            let mut keyboard = UserKeyboard {
                mode: UserMode::Temporary,
            };
            let error = keyboard
                .dispatch(stroke(physical, false, false), &catalog)
                .unwrap_err();
            assert!(matches!(error, ResolveError::Ambiguous { .. }));
            assert_eq!(keyboard.mode, UserMode::Off);
        }
    }
    #[test]
    fn strings_comments_and_ordinary_functions_do_not_register_handlers() {
        let catalog = HandlerCatalog::from_snapshots([SourceSnapshot::new(
            "NotKey",
            "// KEY K_Sin() BEGIN RETURN 1; END;\nEXPORT K_Sin() BEGIN RETURN \"KEY K_Cos() BEGIN END;\"; END;",
        )]);
        assert!(catalog.resolve(stroke(21, false, false)).unwrap().is_none());
        assert!(catalog.resolve(stroke(22, false, false)).unwrap().is_none());
    }
    #[test]
    fn invalid_sources_parameters_and_unknown_bindings_are_reported() {
        let catalog = HandlerCatalog::from_snapshots([
            SourceSnapshot::new("Broken", "KEY K_Sin() BEGIN RETURN 1+; END;"),
            SourceSnapshot::new("Params", "KEY K_Cos(x) BEGIN RETURN x; END;"),
            source("Unknown", "K_NotAKey", "x"),
        ]);
        assert!(matches!(
            catalog.resolve(stroke(21, false, false)),
            Err(ResolveError::InvalidSource { .. })
        ));
        assert!(matches!(
            catalog.resolve(stroke(22, false, false)),
            Err(ResolveError::HandlerParameters { count: 1, .. })
        ));
        assert!(
            catalog
                .issues
                .iter()
                .any(|issue| matches!(issue, DiscoveryIssue::UnknownKeyName { .. }))
        );
    }
    #[test]
    fn malformed_unparsed_key_declaration_does_not_silently_fall_through() {
        let catalog = HandlerCatalog::from_snapshots([SourceSnapshot::new(
            "Broken",
            "KEY K_Sin( BEGIN RETURN 1; END;",
        )]);
        assert!(matches!(
            catalog.resolve(stroke(21, false, false)),
            Err(ResolveError::InvalidSource { .. })
        ));
    }
    #[test]
    fn unrelated_bad_sources_do_not_block_valid_bindings() {
        let catalog = HandlerCatalog::from_snapshots([
            SourceSnapshot::new("Bad", "EXPORT BROKEN() BEGIN RETURN 1+; END;"),
            source("Valid", "K_Sin", "ok"),
        ]);
        assert_eq!(
            catalog
                .resolve(stroke(21, false, false))
                .unwrap()
                .unwrap()
                .name,
            "Valid"
        );
    }
    #[test]
    fn handler_return_text_preserves_strings_and_other_object_syntax() {
        let settings = ppl_vm::Settings::default();
        assert_eq!(
            returned_text(&ppl_vm::PortableValue::String("ALOG(".into()), &settings),
            "ALOG("
        );
        assert_eq!(
            returned_text(
                &ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(7)),
                &settings
            ),
            "7"
        );
        assert_eq!(
            returned_text(
                &ppl_vm::PortableValue::List(vec![ppl_vm::PortableValue::String("x".into())]),
                &settings
            ),
            "{\"x\"}"
        );
    }
}
