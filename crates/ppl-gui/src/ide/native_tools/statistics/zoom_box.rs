//! Two-corner Statistics 1Var Plot Zoom, using the calculator's LCD pixel space.
use super::{NativeTools, plot_cursor::PlotCursor, view::text};
use crate::ide::{
    native::{chrome, keys::KeyAction},
    native_apps::PlotWindow,
    native_plot_controls::{X_PIXEL_SPAN, Y_PIXEL_SPAN},
};
use ppl_graphics::Grob;
use ppl_math::Real;
use ppl_vm::SessionSnapshot;

#[derive(Clone, Copy)]
pub(super) struct ZoomBox {
    cursor: PlotCursor,
    anchor: Option<PlotCursor>,
}

fn box_window(
    mut window: PlotWindow,
    [x1, y1]: [i64; 2],
    [x2, y2]: [i64; 2],
) -> Result<PlotWindow, String> {
    window.validate()?;
    let x =
        |pixel: i64| window.xmin + (pixel - 1) as f64 * (window.xmax - window.xmin) / X_PIXEL_SPAN;
    let y = |pixel: i64| window.ymax - pixel as f64 * (window.ymax - window.ymin) / Y_PIXEL_SPAN;
    // The firmware rounds both picked X coordinates to calculator reals
    // before insetting the left edge by one pixel of the new viewport.
    let decimal = |value| Real::from_f64(value).map_err(|error| error.to_string());
    let left = decimal(x(x1.min(x2)))?;
    let right = decimal(x(x1.max(x2)))?;
    let xmin = right
        .sub(left)
        .and_then(|span| span.div(Real::from_i64(319)))
        .and_then(|inset| left.add(inset))
        .map_err(|error| error.to_string())?;
    (window.xmin, window.xmax) = (xmin.to_f64(), right.to_f64());
    (window.ymin, window.ymax) = (y(y1.max(y2)), y(y1.min(y2)));
    window.validate()?;
    Ok(window)
}

impl NativeTools {
    pub(super) fn statistics_open_zoom_box(&mut self) {
        let window = self.trace_window();
        if let Err(error) = window.validate() {
            self.error = Some(error);
            return;
        }
        let cursor = self
            .statistics_panel
            .free_plot_cursor
            .filter(|_| !self.trace.enabled)
            .unwrap_or_else(|| {
                let mut trace = self.trace;
                trace
                    .enable(&self.trace_samples())
                    .map_or(PlotCursor::CENTER, |sample| {
                        PlotCursor::from_point(&window, sample.point)
                    })
            });
        self.trace.enabled = false;
        self.statistics_panel.zoom_box = Some(ZoomBox {
            cursor,
            anchor: None,
        });
        self.error = None;
    }

    pub(super) fn statistics_zoom_box_action(
        &mut self,
        action: KeyAction,
        snapshot: &SessionSnapshot,
    ) -> bool {
        use KeyAction as A;
        let Some(zoom) = self.statistics_panel.zoom_box.as_mut() else {
            return false;
        };
        if let Some(cursor) = zoom.cursor.step(action) {
            zoom.cursor = cursor;
        } else {
            match action {
                A::Evaluate => self.statistics_pick_zoom_corner(snapshot),
                A::Back | A::On => self.statistics_cancel_zoom_box(),
                _ => {}
            }
        }
        self.dirty();
        true
    }

    fn statistics_cancel_zoom_box(&mut self) {
        if let Some(zoom) = self.statistics_panel.zoom_box.take() {
            self.statistics_panel.free_plot_cursor = Some(zoom.cursor);
        }
    }

    fn statistics_pick_zoom_corner(&mut self, snapshot: &SessionSnapshot) {
        let Some(zoom) = self.statistics_panel.zoom_box.as_mut() else {
            return;
        };
        let cursor = zoom.cursor;
        let Some(anchor) = zoom.anchor else {
            zoom.anchor = Some(cursor);
            return;
        };
        if anchor.0[0] == cursor.0[0] || anchor.0[1] == cursor.0[1] {
            return;
        }
        let previous = self.trace_window();
        match box_window(previous.clone(), anchor.0, cursor.0) {
            Ok(next) => {
                self.statistics_panel.zoom_box = None;
                self.statistics_panel.free_plot_cursor = Some(PlotCursor::CENTER);
                self.statistics_apply_discrete_zoom(previous, next, snapshot);
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(super) fn statistics_zoom_box_footer(&self, image: &mut Grob) {
        let Some(zoom) = self.statistics_panel.zoom_box else {
            return;
        };
        let can_pick = zoom
            .anchor
            .is_none_or(|PlotCursor([x, y])| x != zoom.cursor.0[0] && y != zoom.cursor.0[1]);
        chrome::softkeys(
            image,
            &["", "", "", "", "Cancel", if can_pick { "OK" } else { "" }],
        );
        image.fill_rect(0, 220, 210, 239, 0xe8f1f6);
        let label = if zoom.anchor.is_some() {
            "Select opposite corner"
        } else {
            "Select a corner"
        };
        text(image, label, 2, 222);
    }

    pub(super) fn statistics_zoom_box_input(
        &mut self,
        ui: &mut egui::Ui,
        lcd: egui::Rect,
        scale: f32,
    ) {
        let Some(zoom) = self.statistics_panel.zoom_box.as_mut() else {
            return;
        };
        let plot = egui::Rect::from_min_size(lcd.min, egui::vec2(lcd.width(), 219. * scale));
        let response = ui.interact(
            plot,
            ui.id().with("statistics_zoom_box"),
            egui::Sense::click_and_drag(),
        );
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            zoom.cursor = PlotCursor::from_pointer(pointer, plot.min, scale);
            ui.ctx().request_repaint();
        }
        let painter = ui.painter().with_clip_rect(plot);
        if let Some(anchor) = zoom.anchor {
            let bounds = egui::Rect::from_two_pos(
                anchor.screen(plot.min, scale),
                zoom.cursor.screen(plot.min, scale),
            )
            .expand(scale * 0.5);
            painter.rect_filled(
                bounds,
                0.,
                egui::Color32::from_rgba_unmultiplied(176, 176, 92, 100),
            );
        }
        zoom.cursor.paint(&painter, plot.min, scale);
    }

    pub(super) fn statistics_zoom_box_softkey(&mut self, key: usize, snapshot: &SessionSnapshot) {
        match key {
            4 => self.statistics_cancel_zoom_box(),
            5 => self.statistics_pick_zoom_corner(snapshot),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_box_corners_keep_the_measured_left_inset() {
        let window = PlotWindow {
            xmin: 0.,
            xmax: 12.,
            ymin: -4.,
            ymax: 8.,
            ..Default::default()
        };
        let first = [40, 44];
        let opposite = [205, 174];
        let expected = [
            "1.49121665583",
            "7.69811320755",
            "-1.57798165138",
            "5.57798165138",
        ]
        .map(|text| Real::parse(text).unwrap());
        for (a, b) in [(first, opposite), (opposite, first)] {
            let zoom = box_window(window.clone(), a, b).unwrap();
            assert_eq!(
                [zoom.xmin, zoom.xmax, zoom.ymin, zoom.ymax]
                    .map(|bound| Real::from_f64(bound).unwrap()),
                expected
            );
        }
        assert!(box_window(window, first, [40, 174]).is_err());
    }
}
