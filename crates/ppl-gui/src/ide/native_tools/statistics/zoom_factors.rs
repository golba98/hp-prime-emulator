//! Statistics 1Var Plot Zoom Factors form and its transactional draft.
use super::{MathTarget, NativeTools, Request, app, view::text};
use crate::ide::native::{chrome, keys::KeyAction};
use crate::ide::native_tools::Page;
use ppl_graphics::{Grob, text::measure};
use ppl_math::Real;
use ppl_vm::SessionSnapshot;

pub(super) struct ZoomFactorsDraft {
    source: ([Real; 2], bool),
    values: ([Real; 2], bool),
    selected: usize,
}

pub(super) fn current(snapshot: &SessionSnapshot) -> ([Real; 2], bool) {
    snapshot
        .plot_settings_for(app(false))
        .and_then(|settings| settings.statistics_zoom())
        .unwrap_or(([Real::from_i64(2); 2], true))
}

impl NativeTools {
    pub(super) fn statistics_open_zoom_factors(&mut self, snapshot: &SessionSnapshot) {
        let source = current(snapshot);
        self.statistics_panel.zoom_factors = Some(ZoomFactorsDraft {
            source,
            values: source,
            selected: 0,
        });
        self.error = None;
    }

    pub(crate) fn statistics_zoom_factors_open(&self) -> bool {
        self.page == Some(Page::Plot) && self.statistics_panel.zoom_factors.is_some()
    }

    pub(crate) fn statistics_zoom_factor_source(&self, axis: usize) -> String {
        self.statistics_panel
            .zoom_factors
            .as_ref()
            .map(|draft| draft.values.0[axis].to_string())
            .unwrap_or_default()
    }

    pub(crate) fn statistics_set_zoom_factor(
        &mut self,
        axis: usize,
        source: &str,
    ) -> Result<(), String> {
        let draft = self
            .statistics_panel
            .zoom_factors
            .as_mut()
            .ok_or("Zoom Factors is closed")?;
        let value = Real::parse(source.trim())
            .filter(|number| *number > Real::ZERO)
            .ok_or("Enter a positive zoom factor")?;
        draft.values.0[axis] = value;
        Ok(())
    }

    fn statistics_finish_zoom_factors(&mut self, commit: bool) -> Option<Request> {
        let draft = self.statistics_panel.zoom_factors.take()?;
        if commit && draft.source != draft.values {
            Some(Request::StatisticsZoomFactors {
                source: draft.source,
                edited: draft.values,
            })
        } else {
            None
        }
    }

    pub(crate) fn statistics_zoom_factors_action(
        &mut self,
        action: KeyAction,
    ) -> Option<Vec<Request>> {
        use KeyAction as A;
        if self.page != Some(Page::Plot) {
            return None;
        }
        let draft = self.statistics_panel.zoom_factors.as_mut()?;
        let request = match action {
            A::Home | A::Tool(_) => {
                self.statistics_finish_zoom_factors(false);
                return None;
            }
            A::History(delta) => {
                draft.selected = (draft.selected as i8 + delta).rem_euclid(3) as usize;
                None
            }
            A::Insert("+" | "-") if draft.selected == 2 => {
                draft.values.1 = !draft.values.1;
                None
            }
            A::Insert(source)
                if draft.selected < 2
                    && source.bytes().all(|c| c.is_ascii_digit() || c == b'.') =>
            {
                Some(Request::EditMath(MathTarget::StatisticsZoomFactor {
                    axis: draft.selected,
                    initial: Some(source),
                }))
            }
            A::Evaluate => self.statistics_finish_zoom_factors(true),
            A::Back | A::On => self.statistics_finish_zoom_factors(false),
            _ => None,
        };
        self.dirty();
        Some(request.into_iter().collect())
    }

    pub(super) fn statistics_show_zoom_factors(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        requests: &mut Vec<Request>,
    ) -> bool {
        const FIELDS: [[i64; 4]; 2] = [[92, 48, 314, 71], [92, 75, 314, 96]];
        let Some(draft) = self.statistics_panel.zoom_factors.as_ref() else {
            return false;
        };
        let mut image = Grob::new(320, 240, 0xe8f1f6);
        chrome::header(&mut image, "Zoom Factors");
        for (axis, [left, top, right, bottom]) in FIELDS.into_iter().enumerate() {
            image.rect(
                left,
                top,
                right,
                bottom,
                if draft.selected == axis { 0 } else { 0xe1e1e1 },
                0xffffff,
            );
            if draft.selected == axis {
                image.rect(left + 1, top + 1, right - 1, bottom - 1, 0, 0xffffff);
            }
            let label = ["X Zoom:", "Y Zoom:"][axis];
            text(&mut image, label, 90 - measure(label, 14.), top + 4);
            text(&mut image, &draft.values.0[axis].to_string(), 95, top + 4);
        }
        text(&mut image, "Recenter:", 90 - measure("Recenter:", 14.), 104);
        image.rect(
            93,
            103,
            110,
            121,
            if draft.selected == 2 { 0 } else { 0xe1e1e1 },
            0xffffff,
        );
        if draft.selected == 2 {
            image.rect(94, 104, 109, 120, 0, 0xffffff);
        }
        if draft.values.1 {
            image.line(97, 111, 100, 117, 0);
            image.line(100, 117, 105, 106, 0);
        }
        text(
            &mut image,
            [
                "Enter horizontal zoom factor",
                "Enter vertical zoom factor",
                "Recenter plot at cursor",
            ][draft.selected],
            0,
            203,
        );
        let (edit, check) = if draft.selected == 2 {
            ("", "✓")
        } else {
            ("Edit", "")
        };
        chrome::softkeys(&mut image, &[edit, check, "", "", "Cancel", "OK"]);
        self.update_statistics_texture(ctx, &image);
        egui::Area::new("native_statistics_zoom_factors".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.statistics_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let interaction = ui.interact(lcd, ui.id().with("fields"), egui::Sense::click());
                if let Some(pointer) = interaction.interact_pointer_pos()
                    && interaction.clicked()
                {
                    let point = (pointer - lcd.min) / scale;
                    let draft = self.statistics_panel.zoom_factors.as_mut().unwrap();
                    let request = if point.y >= chrome::SOFTKEY_TOP as f32 {
                        match chrome::softkey_index(point.x as i64) {
                            0 if draft.selected < 2 => {
                                Some(Request::EditMath(MathTarget::StatisticsZoomFactor {
                                    axis: draft.selected,
                                    initial: None,
                                }))
                            }
                            1 if draft.selected == 2 => {
                                draft.values.1 = !draft.values.1;
                                None
                            }
                            4 => self.statistics_finish_zoom_factors(false),
                            5 => self.statistics_finish_zoom_factors(true),
                            _ => None,
                        }
                    } else {
                        match point.y as i64 {
                            48..=71 | 75..=96 => {
                                let axis = usize::from(point.y >= 75.);
                                draft.selected = axis;
                                interaction.double_clicked().then_some(Request::EditMath(
                                    MathTarget::StatisticsZoomFactor {
                                        axis,
                                        initial: None,
                                    },
                                ))
                            }
                            103..=121 => {
                                if draft.selected == 2 {
                                    draft.values.1 = !draft.values.1;
                                }
                                draft.selected = 2;
                                None
                            }
                            _ => None,
                        }
                    };
                    if let Some(request) = request {
                        requests.push(request);
                    }
                    self.dirty();
                    ctx.request_repaint();
                }
            });
        true
    }
}
