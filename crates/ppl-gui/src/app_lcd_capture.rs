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
    pub(super) fn composed_tool_identity(&self) -> Option<crate::ide::compare::ScreenIdentity> {
        if self.state.firmware_backend || self.native_tools.page.is_none() {
            return None;
        }
        let stamp = self.tool_capture_stamp.as_ref()?;
        if stamp.tool_fingerprint != self.tool_fingerprint()
            || stamp.frame_generation != self.native_display_generation()
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
            frame_generation: self.native_display_generation(),
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
                (CaptureAction::Save(path), Ok(captured)) => {
                    self.state.status_message = match captured.grob.save_png(&path) {
                        Ok(()) => format!("Screenshot saved to {}", path.display()),
                        Err(e) => format!("Screenshot failed: {e}"),
                    };
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
                (CaptureAction::Save(_), Err(error)) => {
                    self.state.status_message = format!("Screenshot failed: {error}")
                }
                (CaptureAction::Compare, Err(error)) => {
                    self.compare.capture_unavailable(error.to_string())
                }
            }
        }
        if !self.lcd_capture_actions.is_empty() && !ctx.input(|i| i.pointer.any_down()) {
            for action in std::mem::take(&mut self.lcd_capture_actions) {
                if let Some(stamp) = &current {
                    if let Err(error) =
                        self.native_lcd_capture
                            .request(ctx, stamp.clone(), action.clone())
                    {
                        match action {
                            CaptureAction::Save(_) => {
                                self.state.status_message = format!("Screenshot failed: {error}")
                            }
                            CaptureAction::Compare => {
                                self.compare.capture_unavailable(error.to_string())
                            }
                        }
                    }
                } else {
                    self.state.status_message =
                        "The calculator LCD must be visible to capture its tools".into();
                }
            }
        }
    }
}
