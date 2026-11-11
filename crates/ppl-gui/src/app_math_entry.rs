//! Reuse the calculator's structured expression editor for tool math fields.
use super::*;
use crate::ide::native_tools::{MathTarget, Page};
pub(super) struct MathEntry {
    target: MathTarget,
    editor: crate::ide::native::expression::ExpressionEditor,
    home_input: String,
    lower: bool,
    previous_page: Page,
}
impl IdeDesktopApp {
    pub(super) fn open_math_entry(&mut self, target: MathTarget, ctx: &egui::Context) {
        if self.controller.running() {
            return;
        }
        let text = match &target {
            MathTarget::SequenceInitial { app, index, term } => self
                .native_tools
                .memory
                .graphs
                .get(app)
                .and_then(|app| app.curves.get(*index))
                .and_then(|curve| curve.sequence.as_ref())
                .and_then(|definition| definition.initial.get(*term))
                .cloned()
                .unwrap_or_default(),
            MathTarget::Curve { app, index, second } => self
                .native_tools
                .memory
                .graphs
                .get(app)
                .and_then(|a| a.curves.get(*index))
                .map(|c| {
                    if *second {
                        c.second.clone()
                    } else {
                        c.expression.clone()
                    }
                })
                .unwrap_or_default(),
            MathTarget::Object => self.native_tools.memory.content.clone(),
            MathTarget::FunctionBody => self.native_tools.memory.function_body.clone(),
        };
        self.math_entry = Some(MathEntry {
            target,
            editor: self.state.native.editor.clone(),
            home_input: self.state.native.home_input.clone(),
            lower: self.state.native.alpha_default_lower,
            previous_page: self.native_tools.page.unwrap_or(Page::Symbolic),
        });
        self.native_tools.close();
        self.state.native.edit(|e| e.set_text(&text));
        self.state.native.edit_title = Some("Edit expression · Enter: OK · Esc: cancel".into());
        self.state.native.alpha_default_lower = matches!(
            self.math_entry.as_ref().map(|entry| entry.previous_page),
            Some(Page::CasVariables)
        );
        self.set_native_view(NativeView::Home, ctx);
    }
    pub(super) fn finish_math_entry(&mut self, commit: bool, ctx: &egui::Context) -> bool {
        let Some(entry) = self.math_entry.take() else {
            return false;
        };
        if commit {
            let source = self.state.native.home_input.clone();
            match entry.target {
                MathTarget::SequenceInitial { app, index, term } => {
                    if let Some(value) = self
                        .native_tools
                        .memory
                        .graphs
                        .get_mut(&app)
                        .and_then(|app| app.curves.get_mut(index))
                        .and_then(|curve| curve.sequence.as_mut())
                        .and_then(|definition| definition.initial.get_mut(term))
                    {
                        *value = source;
                    }
                }
                MathTarget::Curve { app, index, second } => {
                    if let Some(curve) = self
                        .native_tools
                        .memory
                        .graphs
                        .get_mut(&app)
                        .and_then(|a| a.curves.get_mut(index))
                    {
                        if second {
                            curve.second = source
                        } else {
                            curve.expression = source
                        }
                    }
                }
                MathTarget::Object => self.native_tools.memory.content = source,
                MathTarget::FunctionBody => self.native_tools.memory.function_body = source,
            }
        }
        self.state.native.editor = entry.editor;
        self.state.native.home_input = entry.home_input;
        self.state.native.alpha_default_lower = entry.lower;
        self.state.native.edit_title = None;
        self.native_tools.open(entry.previous_page);
        self.release_native_input();
        self.state.native.touch_view();
        ctx.request_repaint();
        true
    }
}
