//! Pointer sequences against the production painter, not a duplicate reducer.
use crate::ide::{
    native_apps::{CurveSamples, GraphApp, GraphKind, PlotWindow},
    native_plot_controls::TraceState,
    native_plot_view::{self, PlotInteraction},
};
use std::sync::{Arc, atomic::AtomicBool};
struct Harness {
    ctx: egui::Context,
    window: PlotWindow,
    curves: Vec<CurveSamples>,
    size: egui::Vec2,
    scale: f32,
    time: f64,
}
impl Harness {
    fn new(scale: f32) -> Self {
        Self {
            ctx: egui::Context::default(),
            window: PlotWindow {
                xmin: -20.,
                xmax: 20.,
                ymin: -10.,
                ymax: 30.,
                ..Default::default()
            },
            curves: vec![],
            size: egui::vec2(318. * scale, 218. * scale),
            scale,
            time: 0.,
        }
    }
    fn frame(
        &mut self,
        events: Vec<egui::Event>,
        trace: Option<[f64; 2]>,
    ) -> (PlotInteraction, egui::Rect, egui::FullOutput) {
        self.time += 0.02;
        let mut result = PlotInteraction::default();
        let mut rect = egui::Rect::NOTHING;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400., 1000.),
                )),
                events,
                time: Some(self.time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = egui::Rect::from_min_size(ui.next_widget_position(), self.size);
                    result = native_plot_view::show(
                        ui,
                        &mut self.window,
                        &self.curves,
                        &[],
                        self.size,
                        self.scale,
                        trace,
                    );
                });
            },
        );
        (result, rect, output)
    }
}
fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}
fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 2e-5, "{actual}!={expected}");
}
fn same_window(actual: &PlotWindow, expected: &PlotWindow) {
    for (a, b) in [
        (actual.xmin, expected.xmin),
        (actual.xmax, expected.xmax),
        (actual.ymin, expected.ymin),
        (actual.ymax, expected.ymax),
    ] {
        close(a, b);
    }
}
#[test]
fn multiframe_drag_pans_by_total_pointer_motion_and_release_fires_once() {
    for scale in [0.75, 1., 2.] {
        let mut view = Harness::new(scale);
        let initial = view.window.clone();
        let (_, rect, _) = view.frame(vec![], None);
        let start = rect.center();
        view.frame(vec![egui::Event::PointerMoved(start)], None);
        view.frame(vec![button(start, true)], None);
        let first = start + egui::vec2(24. * scale, 12. * scale);
        let end = start + egui::vec2(48. * scale, 24. * scale);
        for pos in [first, end] {
            let (event, _, _) = view.frame(vec![egui::Event::PointerMoved(pos)], None);
            assert!(event.clicked.is_none());
            assert!(!event.drag_stopped);
        }
        let expected = PlotWindow {
            xmin: initial.xmin - 48. / 318. * 40.,
            xmax: initial.xmax - 48. / 318. * 40.,
            ymin: initial.ymin + 24. / 218. * 40.,
            ymax: initial.ymax + 24. / 218. * 40.,
            ..initial
        };
        same_window(&view.window, &expected);
        view.frame(vec![], None);
        same_window(&view.window, &expected);
        let (released, _, _) = view.frame(vec![button(end, false)], None);
        assert!(released.drag_stopped);
        assert!(released.clicked.is_none());
        same_window(&view.window, &expected);
        let (idle, _, _) = view.frame(vec![], None);
        assert!(!idle.drag_stopped);
        same_window(&view.window, &expected);
    }
}
#[test]
fn click_projection_is_affine_at_real_scales_without_mutating_window() {
    for scale in [0.75, 1., 2.] {
        let mut view = Harness::new(scale);
        let original = view.window.clone();
        let (_, rect, _) = view.frame(vec![], None);
        for [fx, fy] in [[0.25, 0.75], [0.5, 0.5], [0.75, 0.25]] {
            let point = rect.min + egui::vec2(rect.width() * fx, rect.height() * fy);
            view.frame(vec![egui::Event::PointerMoved(point)], None);
            view.frame(vec![button(point, true)], None);
            let (event, _, _) = view.frame(vec![button(point, false)], None);
            let [x, y] = event
                .clicked
                .expect("pointer click must return mathematical coordinates");
            close(x, -20. + f64::from(fx) * 40.);
            close(y, 30. - f64::from(fy) * 40.);
            assert!(!event.drag_stopped);
            same_window(&view.window, &original);
        }
    }
}
#[test]
fn pointer_trace_returns_a_computed_vm_sample_not_pointer_interpolation() {
    let snapshot = ppl_vm::SessionContext::default().snapshot();
    let mut app = GraphApp::new(GraphKind::Function);
    app.window = PlotWindow {
        xmin: -2.,
        xmax: 2.,
        ymin: -1.,
        ymax: 10.,
        ..Default::default()
    };
    app.curves[0].expression = "X^2+2*X+1".into();
    let samples = app
        .sample(&snapshot, 5, Arc::new(AtomicBool::new(false)))
        .unwrap();
    let mut view = Harness::new(1.);
    view.window = app.window;
    view.curves = samples.clone();
    let (_, rect, _) = view.frame(vec![], None);
    let target = [0.55, 2.7];
    let point = egui::pos2(
        rect.left() + ((target[0] + 2.) / 4.) as f32 * rect.width(),
        rect.bottom() - ((target[1] + 1.) / 11.) as f32 * rect.height(),
    );
    view.frame(vec![egui::Event::PointerMoved(point)], None);
    view.frame(vec![button(point, true)], None);
    let (event, _, _) = view.frame(vec![button(point, false)], None);
    let mathematical = event.clicked.unwrap();
    close(mathematical[0], target[0]);
    close(mathematical[1], target[1]);
    let trace = TraceState::default()
        .nearest(mathematical, &view.window, &samples)
        .unwrap()
        .unwrap();
    assert_eq!(trace.point, [1., 4.]);
    assert_eq!(samples[trace.curve][trace.sample], Some(trace.point));
    assert_ne!(trace.point, target);
    let (_, _, painted) = view.frame(vec![], Some(trace.point));
    assert!(painted.shapes.iter().any(|shape|matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().contains("x=1.000000  y=4.000000"))),"visible coordinates must show the actual sample");
}
#[test]
fn movement_delivered_with_release_is_included_once_in_final_pan() {
    let mut view = Harness::new(1.);
    let initial = view.window.clone();
    let (_, rect, _) = view.frame(vec![], None);
    let start = rect.center();
    view.frame(vec![egui::Event::PointerMoved(start)], None);
    view.frame(vec![button(start, true)], None);
    let first = start + egui::vec2(20., 0.);
    let end = start + egui::vec2(40., 0.);
    view.frame(vec![egui::Event::PointerMoved(first)], None);
    let (event, _, _) = view.frame(
        vec![egui::Event::PointerMoved(end), button(end, false)],
        None,
    );
    assert!(event.drag_stopped);
    close(view.window.xmin, initial.xmin - 40. / 318. * 40.);
    let after = view.window.clone();
    view.frame(vec![], None);
    same_window(&view.window, &after);
}

#[test]
fn native_model_pointer_trace_uses_the_visible_canvas_aspect() {
    use crate::ide::{
        native_app_models::Statistics2,
        native_model_plots,
        native_tools::{NativeTools, Page},
    };
    let model = Statistics2 {
        pairs: vec![[0.1, 0.], [0., 0.18]],
    };
    let mut data = native_model_plots::statistics2(&model, &AtomicBool::new(false)).unwrap();
    // Show the observations without their optional regression curve.
    data.curves.truncate(1);
    data.labels.truncate(1);
    data.styles.truncate(1);
    let mut tools = NativeTools::default();
    tools.memory.selected_app = "Statistics 2Var".into();
    tools.memory.models.statistics2 = model;
    tools.memory.models.results.insert(
        "Statistics 2Var:plot".into(),
        serde_json::to_value(data).unwrap(),
    );
    tools.memory.model_windows.insert(
        "Statistics 2Var".into(),
        PlotWindow {
            xmin: 0.,
            xmax: 1.,
            ymin: 0.,
            ymax: 1.,
            ..Default::default()
        },
    );
    tools.open(Page::Plot);
    let ctx = egui::Context::default();
    let snapshot = ppl_vm::SessionContext::default().snapshot();
    let mut time = 0.;
    let mut frame = |tools: &mut NativeTools, events| {
        time += 0.025;
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ctx| {
                tools.show(
                    ctx,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 240.)),
                    &snapshot,
                );
            },
        )
    };
    frame(&mut tools, vec![]);
    let output = frame(&mut tools, vec![]);
    let rect = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Rect(shape)
                if (shape.rect.width() - 316.).abs() < 0.1
                    && (shape.rect.height() - 145.).abs() < 0.1 =>
            {
                Some(shape.rect)
            }
            _ => None,
        })
        .expect("production plot canvas must be located to drive its actual pointer target");
    let click = egui::pos2(
        rect.left() + rect.width() * 0.01,
        rect.bottom() - rect.height() * 0.01,
    );
    frame(&mut tools, vec![egui::Event::PointerMoved(click)]);
    frame(&mut tools, vec![button(click, true)]);
    frame(&mut tools, vec![button(click, false)]);
    let samples = tools.trace_samples();
    let selected = tools
        .trace
        .current(&samples)
        .expect("click must select a real visible sample");
    assert_eq!(
        selected.point,
        [0., 0.18],
        "visible distances are24.85px toB and28.48px toA; full-screen318x218 metric incorrectlyselectsA"
    );
    let mut trace = TraceState::default();
    let before = trace;
    assert!(
        trace
            .nearest_in_view([0., 0.], &PlotWindow::default(), &samples, [0., 145.])
            .is_err()
    );
    assert_eq!(trace, before);
}
