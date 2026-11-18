//! Plot zoom changes the typed XYZ window while keeping the trace selected.
use super::{Calculation, NAME, NativeTools, Page, Request, SessionSnapshot, SurfacePoint};
use egui::{Frame, Popup, emath::RectAlign};
use ppl_math::Real;
use ppl_vm::{AppPlotSettings, Value};

const BOUNDS: [[&str; 2]; 3] = [["Xmin", "Xmax"], ["Ymin", "Ymax"], ["Zmin", "Zmax"]];

#[derive(Clone, Copy)]
enum Zoom {
    Center,
    In(Option<usize>),
    Out(Option<usize>),
    Square,
}

const OPTIONS: [(&str, Zoom); 10] = [
    ("Center on cursor", Zoom::Center),
    ("In 2×2×2", Zoom::In(None)),
    ("Out 2×2×2", Zoom::Out(None)),
    ("X in ×2", Zoom::In(Some(0))),
    ("X out ×2", Zoom::Out(Some(0))),
    ("Y in ×2", Zoom::In(Some(1))),
    ("Y out ×2", Zoom::Out(Some(1))),
    ("Z in ×2", Zoom::In(Some(2))),
    ("Z out ×2", Zoom::Out(Some(2))),
    ("Square XY", Zoom::Square),
];

fn window(source: &AppPlotSettings) -> Result<[[Real; 2]; 3], String> {
    let mut ranges = [[Real::ZERO; 2]; 3];
    for (range, names) in ranges.iter_mut().zip(BOUNDS) {
        for (bound, name) in range.iter_mut().zip(names) {
            *bound = source
                .get(name)
                .and_then(|value| value.as_real())
                .ok_or("Graph 3D requires finite plot bounds")?;
        }
        if range[0] >= range[1] {
            return Err("Each plot minimum must be less than its maximum".into());
        }
    }
    Ok(ranges)
}

fn resized(
    source: &AppPlotSettings,
    point: SurfacePoint,
    zoom: Zoom,
) -> Result<AppPlotSettings, String> {
    let [x, y] = point.coordinates;
    let two = Real::from_i64(2);
    let mut halves = [Real::ZERO; 3];
    let ranges = window(source)?;
    let [zmin, zmax] = ranges[2];
    let z = point
        .height
        .map_or_else(
            || {
                zmax.sub(zmin)
                    .and_then(|span| span.div(two))
                    .and_then(|half| zmin.add(half))
            },
            Ok,
        )
        .map_err(|error| error.to_string())?;
    for (axis, [lo, hi]) in ranges.into_iter().enumerate() {
        let half = hi.sub(lo).and_then(|span| span.div(two));
        halves[axis] = half
            .and_then(|half| match zoom {
                Zoom::In(target) if target.is_none_or(|target| target == axis) => half.div(two),
                Zoom::Out(target) if target.is_none_or(|target| target == axis) => half.mul(two),
                _ => Ok(half),
            })
            .map_err(|error| error.to_string())?;
    }
    if matches!(zoom, Zoom::Square) {
        halves[1] = halves[0];
    }
    let mut edited = source.clone();
    for ((names, center), half) in BOUNDS.into_iter().zip([x, y, z]).zip(halves) {
        let [lo, hi] = [center.sub(half), center.add(half)];
        for (field, bound) in names.into_iter().zip([lo, hi]) {
            let bound = bound.map_err(|error| error.to_string())?;
            edited
                .set(field, &Value::Real(bound))
                .map_err(|error| error.to_string())?;
        }
    }
    window(&edited)?;
    Ok(edited)
}

impl NativeTools {
    pub(super) fn graph3d_plot_zoom_menu(&mut self, ui: &mut egui::Ui, snapshot: &SessionSnapshot) {
        let Some(point) = self.graph3d_panel.trace.point else {
            return;
        };
        let button = ui.add_enabled(
            !self.graph3d_panel.calculating(),
            egui::Button::new("Zoom").small(),
        );
        let style = ui.style().clone();
        let mut choice = None;
        Popup::menu(&button)
            .frame(Frame::popup(&style))
            .align(RectAlign::BOTTOM_END)
            .show(|ui| {
                ui.set_style(style);
                ui.spacing_mut().item_spacing.y *= 0.5;
                for (name, zoom) in OPTIONS {
                    if ui.small_button(name).clicked() {
                        choice = Some(zoom);
                    }
                }
            });
        let Some(zoom) = choice else {
            return;
        };
        let settings = snapshot
            .app_plot_settings
            .get(NAME)
            .cloned()
            .map_or_else(|| self.memory.graph3d.plot_settings(), Ok);
        let result = settings.and_then(|source| {
            let edited = resized(&source, point, zoom)?;
            let mut book = self.memory.graph3d.clone();
            book.apply_plot_settings(&edited)?;
            book.validate_viewport()?;
            self.pending_actions.push(Request::Graph3dPlotZoom {
                source: Box::new(source),
                edited: Box::new(edited),
            });
            Ok(())
        });
        self.record_calculation_error("surface", result.err());
    }

    pub fn graph3d_refresh_zoom(&mut self, snapshot: SessionSnapshot) {
        if let Err(error) = self.graph3d_apply_plot_context(&snapshot) {
            self.record_calculation_error("surface", Some(error));
            return;
        }
        let selection = self.graph3d_panel.trace.point;
        self.graph3d_panel.mesh = None;
        let generation = snapshot.generation;
        self.spawn_graph3d_calculation(Page::Plot, generation, move |book, cancel| {
            book.mesh(&snapshot, cancel, selection)
                .map(|(mesh, point)| Calculation::Mesh(mesh, point))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_axis_zoom_recenters_all_axes_on_the_live_trace() {
        let source = AppPlotSettings::for_app(NAME).unwrap();
        let point = SurfacePoint {
            index: 0,
            coordinates: [Real::from_i64(2), Real::from_i64(3)],
            height: Some(Real::from_i64(2)),
        };
        for (zoom, expected) in [
            (Zoom::Center, [[-2, 6], [-1, 7], [-2, 6]]),
            (Zoom::In(Some(0)), [[0, 4], [-1, 7], [-2, 6]]),
            (Zoom::In(Some(2)), [[-2, 6], [-1, 7], [0, 4]]),
        ] {
            let result = resized(&source, point, zoom).unwrap();
            assert_eq!(
                window(&result).unwrap(),
                expected.map(|pair| pair.map(Real::from_i64))
            );
            for tick in ["Xtick", "Ytick", "Ztick"] {
                let value =
                    |settings: &AppPlotSettings| settings.get(tick).and_then(|v| v.as_real());
                assert_eq!(value(&result), value(&source));
            }
        }
    }

    #[test]
    fn graph3d_square_sets_y_to_the_x_span_and_recenters_depth() {
        let mut source = AppPlotSettings::for_app(NAME).unwrap();
        for (input, point, expected) in [
            (
                [[-3, 9], [-2, 2], [-6, 2]],
                [3, 0],
                [[-3, 9], [-6, 6], [-4, 4]],
            ),
            (
                [[-2, 2], [-3, 9], [-6, 2]],
                [0, 3],
                [[-2, 2], [1, 5], [-4, 4]],
            ),
        ] {
            for (names, pair) in BOUNDS.into_iter().zip(input) {
                for (name, value) in names.into_iter().zip(pair) {
                    source
                        .set(name, &Value::Real(Real::from_i64(value)))
                        .unwrap();
                }
            }
            let point = SurfacePoint {
                index: 0,
                coordinates: point.map(Real::from_i64),
                height: Some(Real::ZERO),
            };
            let result = resized(&source, point, Zoom::Square).unwrap();
            assert_eq!(
                window(&result).unwrap(),
                expected.map(|pair| pair.map(Real::from_i64))
            );
        }
    }

    #[test]
    fn undefined_trace_height_uses_the_current_depth_midpoint() {
        let mut source = AppPlotSettings::for_app(NAME).unwrap();
        source.set("Zmin", &Value::Real(Real::from_i64(2))).unwrap();
        source.set("Zmax", &Value::Real(Real::from_i64(6))).unwrap();
        let point = SurfacePoint {
            index: 0,
            coordinates: [Real::ZERO; 2],
            height: None,
        };
        let result = resized(&source, point, Zoom::Out(None)).unwrap();
        assert_eq!(
            window(&result).unwrap(),
            [[-8, 8], [-8, 8], [0, 8]].map(|pair| pair.map(Real::from_i64))
        );
    }
}
