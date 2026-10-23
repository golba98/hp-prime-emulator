//! Virtual calculator keypad grid supporting 0–50 HP Prime key layout.

use super::{IdeAppState, IdePanel};
use crate::theme::*;
use egui::{Color32, RichText, Stroke, Vec2};

/// Representation of a single button on the virtual keypad.
#[derive(Debug, Clone)]
pub struct KeyDef {
    pub label: &'static str,
    pub code: i64,
    pub bg_color: Color32,
    pub fg_color: Color32,
}

impl KeyDef {
    pub const fn func(label: &'static str, code: i64) -> Self {
        Self {
            label,
            code,
            bg_color: CALC_KEY_FN_BG,
            fg_color: CALC_KEY_FN_FG,
        }
    }

    pub const fn num(label: &'static str, code: i64) -> Self {
        Self {
            label,
            code,
            bg_color: CALC_KEY_NUM_BG,
            fg_color: CALC_KEY_NUM_FG,
        }
    }

    pub const fn special(label: &'static str, code: i64, bg: Color32, fg: Color32) -> Self {
        Self {
            label,
            code,
            bg_color: bg,
            fg_color: fg,
        }
    }
}

/// Keypad container on the right side panel below the screen.
#[derive(Debug, Default)]
pub struct VirtualKeypad {
    pub last_clicked_key: Option<i64>,
}

impl VirtualKeypad {
    pub fn new() -> Self {
        Self::default()
    }
}

// 0–50 HP Prime Grid layout rows
pub const ROW_1: [KeyDef; 5] = [
    KeyDef::func("Apps", 0),
    KeyDef::func("Symb", 1),
    KeyDef::func("▲", 2),
    KeyDef::func("Help", 3),
    KeyDef::func("Esc", 4),
];

pub const ROW_2: [KeyDef; 5] = [
    KeyDef::func("Home", 5),
    KeyDef::func("Plot", 6),
    KeyDef::func("◀", 7),
    KeyDef::func("▶", 8),
    KeyDef::func("View", 9),
];

pub const ROW_3: [KeyDef; 5] = [
    KeyDef::func("CAS", 10),
    KeyDef::func("Num", 11),
    KeyDef::func("▼", 12),
    KeyDef::func("Menu", 13),
    KeyDef::func("Vars", 14),
];

pub const ROW_4: [KeyDef; 5] = [
    KeyDef::func("Toolbox", 15),
    KeyDef::func("Templ", 16),
    KeyDef::func("X,T,θ", 17),
    KeyDef::func("a b/c", 18),
    KeyDef::func("Del", 19),
];

pub const ROW_5: [KeyDef; 5] = [
    KeyDef::func("x^y", 20),
    KeyDef::func("SIN", 21),
    KeyDef::func("COS", 22),
    KeyDef::func("TAN", 23),
    KeyDef::func("LN", 24),
];

pub const ROW_6: [KeyDef; 5] = [
    KeyDef::func("LOG", 25),
    KeyDef::func("x²", 26),
    KeyDef::func("+/-", 27),
    KeyDef::func("( )", 28),
    KeyDef::func(",", 29),
];

pub const ROW_7: [KeyDef; 5] = [
    KeyDef::func("EEX", 31),
    KeyDef::num("7", 32),
    KeyDef::num("8", 33),
    KeyDef::num("9", 34),
    KeyDef::func("÷", 35),
];

pub const ROW_8: [KeyDef; 5] = [
    KeyDef::special("Alpha", 36, CALC_KEY_ALPHA, Color32::WHITE),
    KeyDef::num("4", 37),
    KeyDef::num("5", 38),
    KeyDef::num("6", 39),
    KeyDef::func("×", 40),
];

pub const ROW_9: [KeyDef; 5] = [
    KeyDef::special("Shift", 41, CALC_KEY_SHIFT, Color32::WHITE),
    KeyDef::num("1", 42),
    KeyDef::num("2", 43),
    KeyDef::num("3", 44),
    KeyDef::func("-", 45),
];

pub const ROW_10: [KeyDef; 5] = [
    KeyDef::special("On", 46, CALC_KEY_ON, Color32::WHITE),
    KeyDef::num("0", 47),
    KeyDef::num(".", 48),
    KeyDef::func("Ans", 49),
    KeyDef::func("+", 50),
];

pub const KEY_ENTER_DEF: KeyDef = KeyDef::special("Enter", 30, CALC_KEY_ENTER, Color32::WHITE);

impl IdePanel for VirtualKeypad {
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState) {
        ui.vertical(|ui| {
            ui.label(RichText::new("VIRTUAL KEYPAD (0–50)").strong().size(12.0));
            ui.add(egui::Separator::default().spacing(4.0));

            let key_height = 24.0;
            let cluster_1 = [&ROW_1[..], &ROW_2[..], &ROW_3[..]];
            let cluster_2 = [&ROW_4[..], &ROW_5[..], &ROW_6[..]];
            let cluster_3 = [&ROW_7[..], &ROW_8[..], &ROW_9[..], &ROW_10[..]];

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    let avail_width = ui.available_width() - 8.0;
                    let col_width = (avail_width / 5.0).max(40.0);

                    // Cluster 1: Function Rows 1–3 (Apps, Symb, D-pad, CAS, Num, View, Menu, Vars)
                    for row in cluster_1 {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
                            for key in row {
                                let btn = egui::Button::new(
                                    RichText::new(key.label)
                                        .color(key.fg_color)
                                        .size(11.0)
                                        .strong(),
                                )
                                .fill(key.bg_color)
                                .stroke(Stroke::new(1.0_f32, BORDER_CRISP))
                                .min_size(Vec2::new(col_width - 2.0, key_height));

                                if ui.add(btn).clicked() {
                                    state.calc_input_focus = true;
                                    state.push_key_event(key.code);
                                    self.last_clicked_key = Some(key.code);
                                }
                            }
                        });
                    }

                    // Spacer between Function & Math clusters
                    ui.add_space(4.0);

                    // Cluster 2: Math Rows 4–6 (Toolbox, Trig, Log, Powers, Parens)
                    for row in cluster_2 {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
                            for key in row {
                                let btn = egui::Button::new(
                                    RichText::new(key.label)
                                        .color(key.fg_color)
                                        .size(11.0)
                                        .strong(),
                                )
                                .fill(key.bg_color)
                                .stroke(Stroke::new(1.0_f32, BORDER_CRISP))
                                .min_size(Vec2::new(col_width - 2.0, key_height));

                                if ui.add(btn).clicked() {
                                    state.calc_input_focus = true;
                                    state.push_key_event(key.code);
                                    self.last_clicked_key = Some(key.code);
                                }
                            }
                        });
                    }

                    // Spacer between Math & Number clusters
                    ui.add_space(4.0);

                    // Cluster 3: Number Rows 7–10 (EEX, Digits 0–9, Alpha, Shift, On, + - × ÷)
                    for row in cluster_3 {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
                            for key in row {
                                let btn = egui::Button::new(
                                    RichText::new(key.label)
                                        .color(key.fg_color)
                                        .size(11.0)
                                        .strong(),
                                )
                                .fill(key.bg_color)
                                .stroke(Stroke::new(1.0_f32, BORDER_CRISP))
                                .min_size(Vec2::new(col_width - 2.0, key_height));

                                if ui.add(btn).clicked() {
                                    state.calc_input_focus = true;
                                    state.push_key_event(key.code);
                                    self.last_clicked_key = Some(key.code);
                                }
                            }
                        });
                    }

                    // Spacer before Enter cluster
                    ui.add_space(4.0);

                    // Cluster 4: Enter Key spanning full width
                    let enter_btn = egui::Button::new(
                        RichText::new("Enter")
                            .color(KEY_ENTER_DEF.fg_color)
                            .size(13.0)
                            .strong(),
                    )
                    .fill(KEY_ENTER_DEF.bg_color)
                    .stroke(Stroke::new(1.0_f32, BORDER_CRISP))
                    .min_size(Vec2::new(avail_width, 28.0));

                    if ui.add(enter_btn).clicked() {
                        state.calc_input_focus = true;
                        state.push_key_event(KEY_ENTER_DEF.code);
                        self.last_clicked_key = Some(KEY_ENTER_DEF.code);
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_51_keydefs_valid() {
        let mut codes = std::collections::HashSet::new();
        let rows = [
            &ROW_1[..],
            &ROW_2[..],
            &ROW_3[..],
            &ROW_4[..],
            &ROW_5[..],
            &ROW_6[..],
            &ROW_7[..],
            &ROW_8[..],
            &ROW_9[..],
            &ROW_10[..],
        ];
        for row in rows {
            for key in row {
                assert!((0..=50).contains(&key.code));
                assert!(!codes.contains(&key.code));
                codes.insert(key.code);
            }
        }
        assert_eq!(KEY_ENTER_DEF.code, 30);
        assert!(!codes.contains(&30));
        codes.insert(30);
        assert_eq!(codes.len(), 51);
    }
}
