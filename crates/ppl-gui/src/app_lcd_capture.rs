//! Bind saved/comparison LCD images to the pixels actually composed by egui.
use super::*;
use crate::ide::native_lcd_capture::{CaptureAction, CaptureBackend, CaptureStamp};
use std::hash::{Hash, Hasher};
impl IdeDesktopApp {
    fn tool_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&serde_json::json!({"page":self.native_tools.page,"memory":self.native_tools.memory,"error":self.native_tools.error,"busy":self.native_tools.cas_busy,"generation":self.native_tools.generation})).unwrap().hash(&mut hasher);
        hasher.finish()
    }
    fn composed_lcd_generation(&self) -> u64 {
        // Native tools cover the Home LCD, including its clock. Keep the
        // underlying Home framebuffer fresh without invalidating a tool capture.
        self.native_display_generation()
            .wrapping_sub(self.state.native.clock_view_changes())
    }
    pub(super) fn composed_tool_identity(&self) -> Option<crate::ide::compare::ScreenIdentity> {
        if self.state.firmware_backend || self.native_tools.page.is_none() {
            return None;
        }
        let stamp = self.tool_capture_stamp.as_ref()?;
        if stamp.tool_fingerprint != self.tool_fingerprint()
            || stamp.frame_generation != self.composed_lcd_generation()
        {
            return None;
        }
        self.native_lcd_capture
            .cached(stamp)
            .map(|c| crate::ide::compare::ScreenIdentity {
                firmware: false,
                generation: c.generation,
            })
    }
    pub(crate) fn composed_tool_frame(
        &self,
    ) -> Option<(ppl_graphics::Grob, crate::ide::compare::ScreenIdentity)> {
        let identity = self.composed_tool_identity()?;
        let stamp = self.tool_capture_stamp.as_ref()?;
        Some((
            (*self.native_lcd_capture.cached(stamp)?.grob).clone(),
            identity,
        ))
    }
    pub(crate) fn request_composed_lcd(&mut self, action: CaptureAction, ctx: &egui::Context) {
        self.lcd_capture_actions.push(action);
        ctx.request_repaint();
    }
    pub(crate) fn lcd_capture_pending(&self) -> bool {
        !self.lcd_capture_actions.is_empty() || self.native_lcd_capture.is_pending()
    }
    pub(super) fn poll_composed_lcd(&mut self, ctx: &egui::Context) {
        let visible = self.native_tools_visible();
        let current = visible.then(|| CaptureStamp {
            backend: CaptureBackend::Native,
            tool_fingerprint: self.tool_fingerprint(),
            frame_generation: self.composed_lcd_generation(),
            lcd_rect: self.display.last_display_rect,
            viewport_rect: ctx.screen_rect(),
            pixels_per_point: ctx.pixels_per_point(),
            viewport_id: ctx.viewport_id(),
        });
        // A comparison intentionally hides the live calculator. Keep its source stamp
        // for staleness tracking, but never allow a fresh export of a hidden LCD.
        if let Some(stamp) = &current {
            self.tool_capture_stamp = Some(stamp.clone());
        }
        let compare_hidden =
            !visible && self.state.show_compare && self.composed_tool_identity().is_some();
        let retained = compare_hidden
            .then(|| self.tool_capture_stamp.clone())
            .flatten();
        let completions = self
            .native_lcd_capture
            .poll(ctx, current.as_ref().or(retained.as_ref()));
        for completion in completions {
            match (completion.action, completion.result) {
                (
                    CaptureAction::Save {
                        path,
                        automation_step,
                    },
                    Ok(captured),
                ) => {
                    let result = captured
                        .grob
                        .save_png(&path)
                        .map_err(|error| error.to_string());
                    self.complete_lcd_save(path, automation_step, result);
                }
                (CaptureAction::Compare, Ok(captured)) => {
                    self.state.show_compare = true;
                    self.compare.begin(
                        (*captured.grob).clone(),
                        crate::ide::compare::ScreenIdentity {
                            firmware: false,
                            generation: captured.generation,
                        },
                    );
                    if !self.device.start(
                        crate::ide::device::Purpose::Screenshot,
                        ctx,
                        crate::ide::device::screenshot,
                    ) {
                        self.compare
                            .set_error("The calculator is busy; try again in a moment.".into());
                    }
                }
                (action, Err(error)) => self.fail_lcd_capture(action, error.to_string()),
            }
        }
        if !self.lcd_capture_actions.is_empty() && !ctx.input(|i| i.pointer.any_down()) {
            for action in std::mem::take(&mut self.lcd_capture_actions) {
                if let Some(stamp) = &current {
                    if let Err(error) =
                        self.native_lcd_capture
                            .request(ctx, stamp.clone(), action.clone())
                    {
                        self.fail_lcd_capture(action, error.to_string());
                    }
                } else {
                    let error = "The calculator LCD must be visible to capture its tools".into();
                    if matches!(action, CaptureAction::Compare) {
                        self.state.status_message = error;
                    } else {
                        self.fail_lcd_capture(action, error);
                    }
                }
            }
        }
    }

    fn fail_lcd_capture(&mut self, action: CaptureAction, error: String) {
        match action {
            CaptureAction::Save {
                path,
                automation_step,
            } => self.complete_lcd_save(path, automation_step, Err(error)),
            CaptureAction::Compare => self.compare.capture_unavailable(error),
        }
    }

    fn complete_lcd_save(
        &mut self,
        path: std::path::PathBuf,
        automation_step: Option<usize>,
        result: Result<(), String>,
    ) {
        self.state.status_message = match &result {
            Ok(()) => format!("Screenshot saved to {}", path.display()),
            Err(error) => format!("Screenshot failed: {error}"),
        };
        if let Some(step) = automation_step
            && let Some(automation) = self.automation.as_mut()
        {
            automation.record_screenshot(step, "composed_lcd", path, result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::{layout::WorkspaceLayout, native::NativeView, native_tools::Page};

    #[test]
    fn composed_lcd_survives_hidden_clock_tick_but_rejects_a_source_change() {
        let root = std::env::temp_dir().join(format!(
            "ppl-lcd-clock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 240.));
        for changed in [false, true] {
            let mut app = IdeDesktopApp::default();
            app.state.layout = WorkspaceLayout::Calculator;
            app.native_tools.open(Page::Symbolic);
            app.display.last_display_rect = rect;
            app.state.native.set_view(NativeView::Home);
            app.state.native.set_status_clock("11:59".into());
            let path = root.join(format!("capture-{changed}.png"));
            let ctx = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(rect),
                ..Default::default()
            };
            let request = ctx.run(input.clone(), |ctx| {
                app.request_composed_lcd(
                    CaptureAction::Save {
                        path: path.clone(),
                        automation_step: None,
                    },
                    ctx,
                );
                app.poll_composed_lcd(ctx);
            });
            let user_data = request.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::ViewportCommand::Screenshot(user_data) => Some(user_data.clone()),
                    _ => None,
                })
                .expect("capture was sent to the current viewport");
            assert!(app.lcd_capture_pending());
            let (old_home, generation) = app.native_display_frame();
            app.state.native.set_status_clock("12:00".into());
            let (new_home, new_generation) = app.native_display_frame();
            assert_ne!(
                old_home.pixels(),
                new_home.pixels(),
                "Home clock remains visible"
            );
            assert_eq!(new_generation, generation.wrapping_add(1));
            if changed {
                app.state.native.touch_view();
            }
            let mut response = input;
            response.events.push(egui::Event::Screenshot {
                viewport_id: egui::ViewportId::ROOT,
                user_data,
                image: std::sync::Arc::new(egui::ColorImage::new(
                    [320, 240],
                    vec![egui::Color32::GREEN; 320 * 240],
                )),
            });
            let _ = ctx.run(response, |ctx| app.poll_composed_lcd(ctx));
            assert!(!app.lcd_capture_pending());
            if changed {
                assert!(!path.exists());
                assert!(app.state.status_message.contains("screen changed"));
                assert!(app.composed_tool_frame().is_none());
            } else {
                assert!(path.is_file(), "{}", app.state.status_message);
                let (lcd, _) = app.composed_tool_frame().unwrap();
                assert_eq!(lcd.pixels(), &[0x00ff00; 320 * 240]);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
