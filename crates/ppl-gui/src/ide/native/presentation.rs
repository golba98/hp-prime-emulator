//! Entry is independent of history presentation (HP User Guide pp.20–24).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryMode {
    #[default]
    Textbook,
    Algebraic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresentationSettings {
    #[serde(default, skip_serializing_if = "FontSize::is_medium")]
    pub font_size: FontSize,
    pub entry: EntryMode,
    pub textbook_display: bool,
}
impl Default for PresentationSettings {
    fn default() -> Self {
        Self {
            font_size: FontSize::Medium,
            entry: EntryMode::Textbook,
            textbook_display: true,
        }
    }
}
/// Main Home font sizes measured against G1 glyph rasters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontSize {
    Small,
    #[default]
    Medium,
    Large,
}
impl FontSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];
    pub fn is_medium(&self) -> bool {
        *self == Self::Medium
    }
    pub fn pixels(self) -> f32 {
        match self {
            Self::Small => 12.,
            Self::Medium => 14.,
            Self::Large => 16.,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Small Font",
            Self::Medium => "Medium Font",
            Self::Large => "Large Font",
        }
    }
}
impl EntryMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Textbook => "Textbook",
            Self::Algebraic => "Algebraic",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native::{NativeUiState, expression::ExpressionEditor};
    #[test]
    fn entry_layout_and_history_layout_are_independent_and_do_not_edit_values() {
        let mut state = NativeUiState::default();
        state.edit(|e| e.set_text("(1)/(2)"));
        let editor = ExpressionEditor::from_text(&state.home_input);
        let textbook_height = state.entry_layout(&editor).height;
        state.presentation.entry = EntryMode::Algebraic;
        assert!(state.entry_layout(&editor).height < textbook_height);
        state.add_history("(1)/(2)".into(), "0.5".into(), false, None);
        let textbook_history = state.history_rows()[0].height;
        state.presentation.textbook_display = false;
        assert!(state.history_rows()[0].height < textbook_history);
        assert_eq!(state.home_input, "(1)/(2)");
        assert_eq!(state.history[0].result, "0.5");
        let old: PresentationSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old, PresentationSettings::default());
    }
    #[test]
    fn algebraic_deletion_preserves_empty_calls_but_edits_nonempty_source() {
        let mut algebraic = ExpressionEditor::from_text("SIN()");
        algebraic.set_cursor(4, false);
        algebraic.apply_linear(super::super::expression::EditCommand::Backspace);
        assert_eq!(algebraic.text(), "");
        algebraic.apply_linear(super::super::expression::EditCommand::Undo);
        assert_eq!(algebraic.text(), "SIN()");
        for source in ["SIN()", "sin("] {
            let mut recalled = ExpressionEditor::from_text(source);
            recalled.set_cursor(
                if source.ends_with(')') {
                    4
                } else {
                    source.len()
                },
                false,
            );
            recalled.apply_linear(super::super::expression::EditCommand::Backspace);
            assert_eq!(recalled.text(), "");
        }
        let mut nonempty = ExpressionEditor::from_text("SIN(3)");
        nonempty.set_cursor(4, false);
        nonempty.apply_linear(super::super::expression::EditCommand::Backspace);
        assert_eq!(nonempty.text(), "SIN3)");
        let mut textbook = algebraic.clone();
        textbook.apply(super::super::expression::EditCommand::Backspace);
        assert_eq!(textbook.text(), "");
    }
}
