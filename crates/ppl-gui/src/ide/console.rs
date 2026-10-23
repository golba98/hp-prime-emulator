//! Bottom dock Output console tab capturing PRINT(...) streams.

use std::collections::VecDeque;

use super::{IdeAppState, IdePanel};
use crate::theme::{BG_EXTREME, BORDER_CRISP, TEXT_MUTED, TEXT_NORMAL};
use egui::{FontId, RichText, ScrollArea, Stroke};

/// Default maximum number of console lines retained in memory.
pub const DEFAULT_MAX_CONSOLE_LINES: usize = 5_000;

/// Output message with severity / styling level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleMessage {
    pub text: String,
    pub is_diagnostic: bool,
    pub is_error: bool,
}

/// Output console panel widget with bounded ring buffer history.
#[derive(Debug)]
pub struct ConsolePanel {
    pub messages: VecDeque<ConsoleMessage>,
    pub auto_scroll: bool,
    pub max_lines: usize,
}

impl Default for ConsolePanel {
    fn default() -> Self {
        let mut messages = VecDeque::new();
        messages.push_back(ConsoleMessage {
            text: "HP Prime PPL Emulator output console initialized.".to_string(),
            is_diagnostic: true,
            is_error: false,
        });
        Self {
            messages,
            auto_scroll: true,
            max_lines: DEFAULT_MAX_CONSOLE_LINES,
        }
    }
}

impl ConsolePanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pushes a message into the console buffer, evicting the oldest lines if capacity exceeded.
    pub fn push_message(&mut self, msg: ConsoleMessage) {
        self.messages.push_back(msg);
        while self.messages.len() > self.max_lines {
            self.messages.pop_front();
        }
    }

    /// Appends standard output from PPL `PRINT(...)`.
    pub fn print_output(&mut self, text: impl Into<String>) {
        self.push_message(ConsoleMessage {
            text: text.into(),
            is_diagnostic: false,
            is_error: false,
        });
    }

    /// Appends an informational or diagnostic banner message.
    pub fn print_diagnostic(&mut self, text: impl Into<String>) {
        self.push_message(ConsoleMessage {
            text: text.into(),
            is_diagnostic: true,
            is_error: false,
        });
    }

    /// Appends a runtime error banner message.
    pub fn print_error(&mut self, text: impl Into<String>) {
        self.push_message(ConsoleMessage {
            text: text.into(),
            is_diagnostic: true,
            is_error: true,
        });
    }

    /// Clears all accumulated messages.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Serializes all messages with icon prefixes for formatted export.
    pub fn full_text(&self) -> String {
        self.messages
            .iter()
            .map(|m| {
                if m.is_error {
                    format!("✖ {}", m.text)
                } else if m.is_diagnostic {
                    format!("ℹ {}", m.text)
                } else {
                    m.text.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Serializes all messages with raw text only.
    pub fn plain_text(&self) -> String {
        self.messages
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl IdePanel for ConsolePanel {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.vertical(|ui| {
            // Controls header
            ui.horizontal(|ui| {
                ui.label(RichText::new("Terminal Output").size(11.0).strong());
                ui.label(
                    RichText::new(format!("({} lines)", self.messages.len()))
                        .color(TEXT_MUTED)
                        .size(11.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Clear")
                        .on_hover_text("Clear terminal output")
                        .clicked()
                    {
                        self.clear();
                    }
                    if ui
                        .small_button("Copy")
                        .on_hover_text("Copy console output to clipboard")
                        .clicked()
                    {
                        let text = self.full_text();
                        ui.ctx().copy_text(text);
                        state.status_message = "Console output copied to clipboard".to_string();
                    }
                    ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                });
            });

            ui.add(egui::Separator::default().spacing(2.0));

            // Terminal buffer area
            let frame = egui::Frame::NONE
                .fill(BG_EXTREME)
                .stroke(Stroke::new(1.0_f32, BORDER_CRISP))
                .inner_margin(4.0);

            frame.show(ui, |ui| {
                let scroll = ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .stick_to_bottom(self.auto_scroll);

                scroll.show(ui, |ui| {
                    for msg in &self.messages {
                        let (color, prefix) = if msg.is_error {
                            (crate::theme::ACCENT_RED, "✖ ")
                        } else if msg.is_diagnostic {
                            (TEXT_MUTED, "ℹ ")
                        } else {
                            (TEXT_NORMAL, "")
                        };

                        ui.label(
                            RichText::new(format!("{prefix}{}", msg.text))
                                .color(color)
                                .font(FontId::monospace(12.0)),
                        );
                    }
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_console_default() {
        let panel = ConsolePanel::new();
        assert_eq!(panel.messages.len(), 1);
        assert!(panel.messages[0].is_diagnostic);
        assert!(!panel.messages[0].is_error);
        assert!(panel.auto_scroll);
        assert_eq!(panel.max_lines, DEFAULT_MAX_CONSOLE_LINES);
    }

    #[test]
    fn test_console_push_and_drain() {
        let mut panel = ConsolePanel::new();
        panel.clear();
        assert_eq!(panel.messages.len(), 0);

        panel.print_output("normal output");
        panel.print_diagnostic("diagnostic banner");
        panel.print_error("runtime error");

        assert_eq!(panel.messages.len(), 3);
        assert!(!panel.messages[0].is_diagnostic && !panel.messages[0].is_error);
        assert_eq!(panel.messages[0].text, "normal output");

        assert!(panel.messages[1].is_diagnostic && !panel.messages[1].is_error);
        assert_eq!(panel.messages[1].text, "diagnostic banner");

        assert!(panel.messages[2].is_diagnostic && panel.messages[2].is_error);
        assert_eq!(panel.messages[2].text, "runtime error");
    }

    #[test]
    fn test_console_clear() {
        let mut panel = ConsolePanel::new();
        panel.print_output("line 1");
        panel.print_output("line 2");
        assert!(panel.messages.len() >= 2);
        panel.clear();
        assert_eq!(panel.messages.len(), 0);
        assert_eq!(panel.full_text(), "");
    }

    #[test]
    fn test_console_bounded_ring_buffer() {
        let mut panel = ConsolePanel::new();
        panel.max_lines = 10;
        panel.clear();

        for i in 0..15 {
            panel.print_output(format!("line {i}"));
        }

        assert_eq!(panel.messages.len(), 10);
        assert_eq!(panel.messages.front().unwrap().text, "line 5");
        assert_eq!(panel.messages.back().unwrap().text, "line 14");
    }

    #[test]
    fn test_console_full_text_and_plain_text() {
        let mut panel = ConsolePanel::new();
        panel.clear();
        panel.print_output("stdout line");
        panel.print_diagnostic("info banner");
        panel.print_error("error line");

        assert_eq!(
            panel.full_text(),
            "stdout line\nℹ info banner\n✖ error line"
        );
        assert_eq!(panel.plain_text(), "stdout line\ninfo banner\nerror line");
    }
}
