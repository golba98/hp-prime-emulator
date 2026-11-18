use super::*;
use crate::ide::{native::chrome, native_apps::PlotWindow};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
const PAPER: u32 = 0xe8f1f6;
const TRACE_INK: u32 = 0x177a99;
const COMMAND_ORIGIN: [i64; 2] = [126, 198];
const COMMAND_SIZE: [i64; 2] = [194, 21];
const COMMAND_PADDING: i64 = 4;
const COMMAND_WIDTH: i64 = COMMAND_SIZE[0] - 2 * COMMAND_PADDING;
const COMMAND_GAP: i64 = 18;
const COMMAND_COLOR: u32 = 0xe0edf3;
const COMMAND_FONT: f32 = 14.;

impl CommandText {
    fn overflow(&self) -> i64 {
        self.source.as_deref().map_or(0, |source| {
            (measure(source, COMMAND_FONT) - COMMAND_WIDTH).max(0)
        })
    }
    fn paint(&self, g: &mut Grob, ctx: &egui::Context, now: f64) {
        let Some(source) = self.source.as_deref() else {
            return;
        };
        let width = measure(source, COMMAND_FONT);
        let scrolling = self.automatic && width > COMMAND_WIDTH;
        let offset = if scrolling {
            ctx.request_repaint_after(std::time::Duration::from_millis(45));
            ((now - self.started).max(0.) * 20.).rem_euclid((width + COMMAND_GAP) as f64) as i64
        } else {
            self.offset.round() as i64
        };
        let mut strip = Grob::new(
            COMMAND_WIDTH as usize,
            COMMAND_SIZE[1] as usize,
            COMMAND_COLOR,
        );
        draw_text(
            &mut strip,
            source,
            -offset,
            3,
            COMMAND_FONT,
            0x203344,
            None,
            None,
        );
        if scrolling {
            draw_text(
                &mut strip,
                source,
                width + COMMAND_GAP - offset,
                3,
                COMMAND_FONT,
                0x203344,
                None,
                None,
            );
        }
        let [left, top] = COMMAND_ORIGIN;
        let [width, height] = COMMAND_SIZE;
        g.fill_rect(left, top, left + width - 1, top + height - 1, COMMAND_COLOR);
        g.blit(
            (
                left + COMMAND_PADDING,
                top,
                left + COMMAND_PADDING + COMMAND_WIDTH,
                top + height,
            ),
            &strip,
            (0, 0, COMMAND_WIDTH, height),
            None,
            255,
        );
    }
}
fn text(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 14., 0, None, None);
}
fn clipped(g: &mut Grob, s: &str, x: i64, y: i64, width: usize) {
    let mut c = Grob::new(width, 22, 0xffffff);
    text(&mut c, s, 1, 1);
    g.blit(
        (x, y, x + width as i64, y + 22),
        &c,
        (0, 0, width as i64, 22),
        None,
        255,
    );
}
pub(super) fn project(p: Point2, w: &PlotWindow) -> [f64; 2] {
    [
        319. * (p.x - w.xmin) / (w.xmax - w.xmin),
        219. - 219. * (p.y - w.ymin) / (w.ymax - w.ymin),
    ]
    .map(|pixel| {
        // The decimal camera can land exactly on an integer pixel. Avoid
        // losing a pixel when conversion to f64 falls just below that integer.
        let integer = pixel.round();
        if (pixel - integer).abs() < 1e-10 {
            integer
        } else {
            pixel
        }
    })
}
fn unproject(p: [f64; 2], w: &PlotWindow) -> Point2 {
    Point2 {
        x: w.xmin + p[0] * (w.xmax - w.xmin) / 319.,
        y: w.ymin + (219. - p[1]) * (w.ymax - w.ymin) / 219.,
    }
}
fn segment(g: &mut Grob, a: Point2, b: Point2, extent: geo::Extent, w: &PlotWindow, color: u32) {
    if let Some([a, b]) = geo::plot::clip(a, b, extent, w) {
        let a = project(a, w);
        let b = project(b, w);
        g.line(
            a[0].round() as i64,
            a[1].round() as i64,
            b[0].round() as i64,
            b[1].round() as i64,
            color,
        );
    }
}
fn display(n: f64, s: &ppl_vm::Settings) -> String {
    ppl_math::Real::from_f64(n)
        .map(|v| v.format(s.home_format()))
        .unwrap_or_else(|_| "Out of range".into())
}
fn value_text(shape: &Shape, s: &ppl_vm::Settings) -> String {
    match shape {
        Shape::Objects(objects) => format!(
            "[{}]",
            objects
                .iter()
                .map(|o| value_text(o, s))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Shape::Undefined => "undef".into(),
        Shape::EmptyConic(s) => s.clone(),
        Shape::Number { re, im: 0. } => {
            let mut value = display(*re, s);
            let decimal = s.digit_grouping.decimal();
            if s.hformat == 0
                && re.fract() == 0.
                && !value.contains(['E', 'ᴇ'])
                && !value.contains(decimal)
            {
                value.push(decimal);
            }
            value
        }
        Shape::Number { re, im } => format!("{}+({})*i", display(*re, s), display(*im, s)),
        Shape::Point(p) => format!("point({},{})", display(p.x, s), display(p.y, s)),
        Shape::Line { .. } => "Line".into(),
        Shape::Lines(lines) if lines.is_empty() => "{}".into(),
        Shape::Lines(lines) => format!("{} lines", lines.len()),
        Shape::Circle { .. } => "Circle".into(),
        Shape::Arc(_) => "Arc".into(),
        Shape::Function { .. } => "Function Graph".into(),
        Shape::Parametric(_) => "Parametric".into(),
        Shape::Sequence(_) => "Sequence".into(),
        Shape::Hyperbola(_) => "Hyperbola".into(),
        Shape::Parabola(_) => "Parabola".into(),
        Shape::Ellipse(_) => "Ellipse".into(),
        Shape::Polygon(_) => "Polygon".into(),
        Shape::Points(p) => format!(
            "[{}]",
            p.iter()
                .map(|p| format!("({},{})", display(p.x, s), display(p.y, s)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
fn tick(g: &mut Grob, x: i64, y: i64) {
    g.line(x + 2, y + 7, x + 5, y + 11, 0);
    g.line(x + 5, y + 11, x + 11, y + 1, 0);
}
fn grid_pixels(min: f64, max: f64, step: f64, span: usize) -> Vec<i64> {
    let pixel = (max - min) / span as f64;
    if !pixel.is_finite() || pixel <= 0. || step < 0. || step.is_nan() {
        return vec![];
    }
    if step.is_infinite() {
        return if min <= 0. && max >= 0. {
            vec![(-min / pixel) as i64]
        } else {
            vec![]
        };
    }
    if step / pixel < 1. {
        return (0..=span as i64).collect();
    }
    let first = (min / step).ceil() * step;
    (0..=span)
        .map(|i| first + i as f64 * step)
        .take_while(|v| *v <= max)
        .map(|v| ((v - min) / pixel) as i64)
        .collect()
}
fn point_marker(g: &mut Grob, p: Point2, w: &PlotWindow, color: u32) {
    let [x, y] = project(p, w);
    if (-3. ..=323.).contains(&x) && (-3. ..=222.).contains(&y) {
        g.circle(x as i64, y as i64, 2, color);
        g.set(x as i64, y as i64, color);
    }
}
fn paint_trace(
    g: &mut Grob,
    points: impl Iterator<Item = Point2>,
    connected: bool,
    w: &PlotWindow,
) {
    let mut previous = None;
    for p in points {
        if connected && let Some(previous) = previous {
            segment(g, previous, p, geo::Extent::Segment, w, TRACE_INK);
        } else {
            let [x, y] = project(p, w);
            if (0. ..=319.).contains(&x) && (0. ..=218.).contains(&y) {
                g.set(x.round() as i64, y.round() as i64, TRACE_INK);
            }
        }
        previous = Some(p);
    }
}
fn paint_shape(g: &mut Grob, shape: &Shape, w: &PlotWindow, color: u32) {
    match shape {
        Shape::Objects(objects) => {
            for o in objects {
                paint_shape(g, o, w, color)
            }
        }
        Shape::Point(p) => point_marker(g, *p, w, color),
        Shape::Points(points) => {
            for p in points {
                point_marker(g, *p, w, color);
            }
        }
        Shape::Line { a, b, extent } => segment(g, *a, *b, *extent, w, color),
        Shape::Lines(lines) => {
            for &[a, b] in lines {
                segment(g, a, b, geo::Extent::Line, w, color);
            }
        }
        Shape::Circle { center, radius } => {
            let mut previous = None;
            for i in 0..=192 {
                let a = std::f64::consts::TAU * i as f64 / 192.;
                let p = Point2 {
                    x: center.x + radius * a.cos(),
                    y: center.y + radius * a.sin(),
                };
                if let Some(q) = previous {
                    segment(g, q, p, geo::Extent::Segment, w, color);
                }
                previous = Some(p);
            }
        }
        Shape::Arc(arc) => {
            for [a, b] in arc.edges() {
                segment(g, a, b, geo::Extent::Segment, w, color);
            }
        }
        Shape::Hyperbola(curve) => {
            for [a, b] in curve.edges() {
                segment(g, a, b, geo::Extent::Segment, w, color);
            }
        }
        Shape::Parabola(curve) => {
            for [a, b] in curve.edges() {
                segment(g, a, b, geo::Extent::Segment, w, color);
            }
        }
        Shape::Ellipse(ellipse) => {
            for [a, b] in ellipse.edges() {
                segment(g, a, b, geo::Extent::Segment, w, color);
            }
        }
        Shape::Polygon(points) => {
            for i in 0..points.len() {
                segment(
                    g,
                    points[i],
                    points[(i + 1) % points.len()],
                    geo::Extent::Segment,
                    w,
                    color,
                );
            }
        }
        Shape::Sequence(curve) => {
            segment(
                g,
                Point2 { x: 0., y: 0. },
                Point2 { x: 1., y: 1. },
                geo::Extent::Line,
                w,
                color,
            );
            for pair in curve.paths.iter().flat_map(|p| p.windows(2)) {
                segment(g, pair[0], pair[1], geo::Extent::Segment, w, color);
            }
        }
        Shape::Function { paths, .. }
        | Shape::Parametric(crate::ide::native_geometry::Parametric { paths, .. }) => {
            for pair in paths.iter().flat_map(|path| path.windows(2)) {
                segment(g, pair[0], pair[1], geo::Extent::Segment, w, color);
            }
        }
        Shape::Number { .. } | Shape::EmptyConic(_) | Shape::Undefined => {}
    }
}
fn distance_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length = d[0] * d[0] + d[1] * d[1];
    let t = if length == 0. {
        0.
    } else {
        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length
    }
    .clamp(0., 1.);
    (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
}
fn hit_distance(shape: &Shape, p: [f64; 2], w: &PlotWindow) -> f64 {
    match shape {
        Shape::Objects(objects) => objects
            .iter()
            .map(|o| hit_distance(o, p, w))
            .fold(f64::INFINITY, f64::min),
        Shape::Point(a) => {
            let a = project(*a, w);
            (p[0] - a[0]).hypot(p[1] - a[1])
        }
        Shape::Points(points) => points
            .iter()
            .map(|a| {
                let a = project(*a, w);
                (p[0] - a[0]).hypot(p[1] - a[1])
            })
            .fold(f64::INFINITY, f64::min),
        Shape::Line { a, b, extent } => geo::plot::clip(*a, *b, *extent, w)
            .map_or(f64::INFINITY, |[a, b]| {
                distance_segment(p, project(a, w), project(b, w))
            }),
        Shape::Lines(lines) => lines
            .iter()
            .filter_map(|&[a, b]| geo::plot::clip(a, b, geo::Extent::Line, w))
            .map(|[a, b]| distance_segment(p, project(a, w), project(b, w)))
            .fold(f64::INFINITY, f64::min),
        Shape::Circle { center, radius } => (0..96)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 96.;
                let q = project(
                    Point2 {
                        x: center.x + radius * a.cos(),
                        y: center.y + radius * a.sin(),
                    },
                    w,
                );
                (p[0] - q[0]).hypot(p[1] - q[1])
            })
            .fold(f64::INFINITY, f64::min),
        Shape::Arc(arc) => arc
            .edges()
            .filter_map(|[a, b]| geo::plot::clip(a, b, geo::Extent::Segment, w))
            .map(|[a, b]| distance_segment(p, project(a, w), project(b, w)))
            .fold(f64::INFINITY, f64::min),
        Shape::Hyperbola(curve) => curve
            .edges()
            .filter_map(|[a, b]| geo::plot::clip(a, b, geo::Extent::Segment, w))
            .map(|[a, b]| distance_segment(p, project(a, w), project(b, w)))
            .fold(f64::INFINITY, f64::min),
        Shape::Parabola(curve) => curve
            .edges()
            .filter_map(|[a, b]| geo::plot::clip(a, b, geo::Extent::Segment, w))
            .map(|[a, b]| distance_segment(p, project(a, w), project(b, w)))
            .fold(f64::INFINITY, f64::min),
        Shape::Ellipse(ellipse) => ellipse
            .edges()
            .filter_map(|[a, b]| geo::plot::clip(a, b, geo::Extent::Segment, w))
            .map(|[a, b]| distance_segment(p, project(a, w), project(b, w)))
            .fold(f64::INFINITY, f64::min),
        Shape::Polygon(points) => (0..points.len())
            .map(|i| {
                distance_segment(
                    p,
                    project(points[i], w),
                    project(points[(i + 1) % points.len()], w),
                )
            })
            .fold(f64::INFINITY, f64::min),
        Shape::Sequence(crate::ide::native_geometry::SequencePlot { paths, .. })
        | Shape::Function { paths, .. }
        | Shape::Parametric(crate::ide::native_geometry::Parametric { paths, .. }) => paths
            .iter()
            .flat_map(|path| path.windows(2))
            .filter_map(|p| geo::plot::clip(p[0], p[1], geo::Extent::Segment, w))
            .map(|[a, b]| distance_segment(p, project(a, w), project(b, w)))
            .fold(f64::INFINITY, f64::min),
        Shape::Number { .. } | Shape::EmptyConic(_) | Shape::Undefined => f64::INFINITY,
    }
}
pub(super) fn hit(
    scene: &Scene,
    book: &Book,
    p: [f64; 2],
    w: &PlotWindow,
    filter: Option<Need>,
) -> Option<String> {
    let mut candidates = Vec::new();
    for object in &scene.objects {
        if !book
            .objects
            .iter()
            .any(|o| o.name == object.name && o.visible)
        {
            continue;
        }
        let Ok(shape) = &object.value else { continue };
        let eligible = match filter {
            Some(Need::Curve) => matches!(
                shape,
                Shape::Line { .. }
                    | Shape::Circle { .. }
                    | Shape::Parametric(_)
                    | Shape::Function { .. }
                    | Shape::Arc(_)
                    | Shape::Ellipse(_)
                    | Shape::Parabola(_)
                    | Shape::Hyperbola(_)
            ),
            Some(Need::Point) => matches!(shape, Shape::Point(_)),
            Some(Need::PointOnCurve) => scene.is_point_on_curve(&object.name),
            _ => true,
        };
        if !eligible {
            continue;
        }
        let distance = hit_distance(shape, p, w);
        if distance <= 7. {
            candidates.push((
                !matches!(shape, Shape::Point(_)),
                distance,
                object.name.clone(),
            ));
        }
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    candidates.first().map(|c| c.2.clone())
}
impl NativeTools {
    fn show_geometry_plot_setup(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        requests: &mut Vec<Request>,
    ) {
        const FIELDS: [[i64; 4]; 10] = [
            [8, 50, 103, 69],
            [112, 50, 207, 69],
            [216, 50, 311, 69],
            [8, 109, 150, 128],
            [169, 109, 311, 128],
            [8, 155, 150, 174],
            [169, 155, 311, 174],
            [8, 178, 150, 197],
            [169, 178, 311, 197],
            [8, 201, 311, 219],
        ];
        let settings = self.geometry_panel.settings.clone().unwrap_or_default();
        let mut image = Grob::new(320, 240, 0xf3f6fa);
        image.fill_rect(0, 0, 319, 27, 0x203344);
        draw_text(
            &mut image,
            "Geometry viewport",
            8,
            5,
            16.,
            0xffffff,
            None,
            None,
        );
        for (label, x, y) in [
            ("Left edge", 8, 31),
            ("Bottom edge", 112, 31),
            ("Units per pixel", 216, 31),
            ("Horizontal tick", 8, 90),
            ("Vertical tick", 169, 90),
            ("Display", 8, 134),
        ] {
            text(&mut image, label, x, y);
        }
        for (index, (name, [left, top, right, bottom])) in
            SETUP_FIELDS.iter().zip(FIELDS).enumerate()
        {
            let selected = index == self.geometry_panel.setup_index;
            image.rect(
                left,
                top,
                right,
                bottom,
                if selected { 0x177a99 } else { 0xa2b0bb },
                0xffffff,
            );
            let value = if index < 5 {
                settings
                    .get(name)
                    .unwrap()
                    .string_form(ppl_vm::value::default_format())
            } else {
                let (label, enabled) = match *name {
                    "Axes" => ("Axes", !settings.axes),
                    "Labels" => ("Axis labels", settings.labels),
                    "GridDots" => ("Grid dots", !settings.grid_dots),
                    "GridLines" => ("Grid lines", !settings.grid_lines),
                    "ScrollText" => ("Auto-scroll commands", settings.scroll_text),
                    _ => unreachable!(),
                };
                format!("{label}  {}", if enabled { "On" } else { "Off" })
            };
            draw_text(
                &mut image,
                &value,
                left + 5,
                top + 1,
                14.,
                0x203344,
                Some(right - left - 9),
                None,
            );
        }
        let right = settings
            .xmin
            .add(
                settings
                    .pix_size
                    .mul(ppl_math::Real::from_i64(320))
                    .unwrap(),
            )
            .unwrap();
        let top = settings.derived_bounds()[3];
        text(
            &mut image,
            &format!("Right edge {right}   Top edge {top}"),
            8,
            70,
        );
        if let Some(error) = &self.error {
            draw_text(&mut image, error, 8, 222, 13., 0x983923, Some(303), None);
        } else {
            text(
                &mut image,
                "Enter edits or toggles the selected field",
                8,
                222,
            );
        }
        self.geometry_panel.pixels = image.pixels().to_vec();
        self.geometry_panel.plot_minified = false;
        let pixels = image
            .pixels()
            .iter()
            .map(|&v| egui::Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
            .collect();
        let image = egui::ColorImage::new([320, 240], pixels);
        if let Some(texture) = &mut self.geometry_panel.texture {
            texture.set(image, egui::TextureOptions::NEAREST);
        } else {
            self.geometry_panel.texture =
                Some(ctx.load_texture("native_geometry", image, egui::TextureOptions::NEAREST));
        }
        egui::Area::new("native_geometry_setup".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.geometry_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let response = ui.interact(
                    lcd,
                    ui.id().with("geometry_setup_lcd"),
                    egui::Sense::click(),
                );
                if response.clicked()
                    && !self.geometry_panel.busy
                    && let Some(position) = response.interact_pointer_pos()
                {
                    let p = (position - lcd.min) / scale;
                    if let Some(index) = FIELDS.iter().position(|r| {
                        p.x >= r[0] as f32
                            && p.x <= r[2] as f32
                            && p.y >= r[1] as f32
                            && p.y <= r[3] as f32
                    }) {
                        self.geometry_panel.setup_index = index;
                        requests.extend(self.geometry_setup_activate());
                        self.dirty();
                    }
                }
            });
    }
    fn paint_geometry_plot(
        &self,
        g: &mut Grob,
        book: &Book,
        scene: Option<&Scene>,
        w: &PlotWindow,
        settings: &ppl_vm::Settings,
    ) {
        let configuration = self.geometry_panel.settings.clone().unwrap_or_default();
        let xtick = configuration.xtick.to_f64();
        let ytick = configuration.ytick.to_f64();
        if !configuration.grid_lines {
            for minor in [true, false] {
                let factor = if minor { 0.5 } else { 1. };
                let color = if minor { 0xe4e4e4 } else { 0xbdbdbd };
                for x in grid_pixels(w.xmin, w.xmax, xtick * factor, 319) {
                    g.line(x, 0, x, 218, color);
                }
                for y in grid_pixels(w.ymin, w.ymax, ytick * factor, 219) {
                    g.line(0, 219 - y, 319, 219 - y, color);
                }
            }
        }
        let x_ticks = grid_pixels(w.xmin, w.xmax, xtick, 319);
        let y_ticks = grid_pixels(w.ymin, w.ymax, ytick, 219);
        if !configuration.grid_dots {
            for x in &x_ticks {
                for y in &y_ticks {
                    g.set(*x, 219 - *y, 0xbdbdbd);
                }
            }
        }
        if w.axes {
            if w.xmin <= 0. && w.xmax >= 0. {
                let x = project(Point2 { x: 0., y: 0. }, w)[0].round() as i64;
                g.line(x, 0, x, 218, 0);
                for y in &y_ticks {
                    g.line(x - 2, 219 - *y, x + 2, 219 - *y, 0);
                }
                if configuration.labels {
                    text(g, "Y", x - 11, 0);
                }
            }
            if w.ymin <= 0. && w.ymax >= 0. {
                let y = project(Point2 { x: 0., y: 0. }, w)[1].round() as i64;
                g.line(0, y, 319, y, 0);
                for x in &x_ticks {
                    g.line(*x, y - 2, *x, y + 2, 0);
                }
                if configuration.labels {
                    text(g, "X", 310, y - 16);
                }
            }
        }
        if let Some(scene) = scene {
            for object in &scene.objects {
                let Some(metadata) = book
                    .objects
                    .iter()
                    .find(|o| o.name == object.name && o.visible)
                else {
                    continue;
                };
                let Ok(shape) = &object.value else { continue };
                let drag =
                    metadata.traced && self.geometry_panel.drag.as_deref() == Some(&object.name);
                let pending = self.geometry_panel.drag_trace.iter().filter(|_| drag);
                paint_trace(
                    g,
                    metadata.trace_samples.iter().chain(pending).copied(),
                    metadata.trace_connected,
                    w,
                );
                let preview = self
                    .geometry_panel
                    .drag_trace
                    .back()
                    .filter(|_| drag)
                    .map(|p| Shape::Point(*p));
                let shape = preview.as_ref().unwrap_or(shape);
                let selected = self.geometry_panel.selected.as_deref() == Some(&object.name);
                let hovered = self.geometry_panel.hover.as_deref() == Some(&object.name);
                let color = if selected {
                    0x00bfff
                } else if hovered {
                    0xff8080
                } else {
                    metadata.color
                };
                paint_shape(g, shape, w, color);
                if metadata.label
                    && let Shape::Point(p) = shape
                {
                    let [x, y] = project(*p, w);
                    if (0. ..=319.).contains(&x) && (0. ..=218.).contains(&y) {
                        draw_text(
                            g,
                            &object.name,
                            x.round() as i64 + 3,
                            y.round() as i64,
                            14.,
                            0xff0000,
                            None,
                            None,
                        );
                    }
                }
            }
            for (index, value) in scene.measurements.iter().enumerate() {
                if let Some(m) = book.measurements.get(index)
                    && m.visible
                {
                    let label = if m.label.is_empty() {
                        &m.source
                    } else {
                        &m.label
                    };
                    let value = value
                        .as_ref()
                        .map(|v| value_text(v, settings))
                        .unwrap_or_else(|_| "Error".into());
                    text(g, &format!("{label}: {value}"), 2, 2 + index as i64 * 17);
                }
            }
        }
        let p = Point2 {
            x: self.memory.geometry_cursor.pointer[0],
            y: self.memory.geometry_cursor.pointer[1],
        };
        let xy = project(p, w);
        if self.geometry_panel.tool.is_some()
            && (0. ..=319.).contains(&xy[0])
            && (0. ..=218.).contains(&xy[1])
        {
            g.line(
                xy[0].round() as i64 - 4,
                xy[1].round() as i64,
                xy[0].round() as i64 + 4,
                xy[1].round() as i64,
                0,
            );
            g.line(
                xy[0].round() as i64,
                xy[1].round() as i64 - 4,
                xy[0].round() as i64,
                xy[1].round() as i64 + 4,
                0,
            );
        }
    }
    pub(crate) fn show_geometry(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        snapshot: &ppl_vm::SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        if self.page == Some(Page::PlotSetup) {
            self.show_geometry_plot_setup(ctx, lcd, scale, requests);
            return;
        }
        if let Err(error) = self.memory.models.ensure_geometry_book() {
            self.error = Some(error);
            return;
        }
        let snapshot = self.geometry_context(snapshot);
        if (self.geometry_panel.refresh
            || (self.geometry_scene().is_none() && self.error.is_none()))
            && !self.calculating()
            && !self.geometry_panel.busy
        {
            self.geometry_panel.refresh = false;
            self.plot(snapshot.clone());
        }
        let scene = self.geometry_scene();
        let book = self.memory.models.geometry_book.as_ref().unwrap().clone();
        let mut settings = snapshot.settings.clone();
        if let Some(app) = snapshot.app_settings.get("Geometry") {
            settings = app.effective(&settings);
        }
        self.geometry_panel.angle_turn = settings.angle.full_turn();
        let page = self.page.unwrap();
        let plot = page == Page::Plot;
        let numeric = page == Page::Numeric;
        let mut image = Grob::new(320, 240, if plot { 0xffffff } else { PAPER });
        let window = self.trace_window();
        let preview = if plot && self.geometry_panel.menu.is_none() && self.error.is_none() {
            self.geometry_command_preview(scene.as_ref())
        } else {
            None
        };
        let automatic = self
            .geometry_panel
            .settings
            .as_ref()
            .is_some_and(|s| s.scroll_text);
        let now = ctx.input(|input| input.time);
        self.geometry_panel
            .command_text
            .update(preview, automatic, now);
        if plot {
            let trace = self.trace_selected(scene.as_ref());
            self.paint_geometry_plot(&mut image, &book, scene.as_ref(), &window, &settings);
            image.fill_rect(0, 219, 319, 239, PAPER);
            chrome::softkeys(
                &mut image,
                &[
                    "Cmds",
                    "",
                    "",
                    "",
                    if trace { "Trace" } else { "" },
                    if self.geometry_panel.selected.is_some() {
                        "Opts"
                    } else {
                        ""
                    },
                ],
            );
            image.fill_rect(
                53,
                220,
                if trace {
                    209
                } else if self.geometry_panel.selected.is_some() {
                    264
                } else {
                    319
                },
                239,
                PAPER,
            );
            let label = if let Some(command) = self.geometry_panel.tool {
                match (command.function, self.geometry_panel.args.len()) {
                    ("ellipse", 0) => "Ellipse · First focus".into(),
                    ("polygon", 3..) => "Polygon · Select first vertex to close".into(),
                    ("ellipse", 1) => "Ellipse · Second focus".into(),
                    ("ellipse", _) => "Ellipse · Point on curve".into(),
                    _ => match command.needs.get(self.geometry_panel.args.len()) {
                        Some(Need::PointOnCurve) => "Select a point on a curve".into(),
                        need => format!(
                            "{}: select {:?}",
                            command.label,
                            need.unwrap_or(&Need::Point)
                        ),
                    },
                }
            } else {
                format!(
                    "X:{} Y:{}",
                    display(self.memory.geometry_cursor.pointer[0], &settings),
                    display(self.memory.geometry_cursor.pointer[1], &settings)
                )
            };
            draw_text(
                &mut image,
                &label,
                53,
                222,
                14.,
                0,
                Some(if trace {
                    156
                } else if self.geometry_panel.selected.is_some() {
                    210
                } else {
                    265
                }),
                None,
            );
            self.geometry_panel.command_text.paint(&mut image, ctx, now);
            if let Some(error) = &self.error {
                image.fill_rect(0, 203, 319, 219, 0xffffff);
                text(&mut image, error, 0, 203);
            }
        } else {
            chrome::header(
                &mut image,
                if numeric {
                    "Geometry Numeric View"
                } else {
                    "Geometry Symbolic View"
                },
            );
            let c = &self.memory.geometry_cursor;
            let (index, first, len) = if numeric {
                (c.measurement, c.first_measurement, book.measurements.len())
            } else {
                (c.object, c.first_object, book.objects.len())
            };
            for row in first..=(first + 6).min(len) {
                let top = 22 + (row - first) as i64 * 27;
                if row == index {
                    image.fill_rect(0, top - 1, 319, top + 24, 0xadccff);
                }
                image.rect(
                    1,
                    top + 4,
                    14,
                    top + 17,
                    if row == index && c.visibility {
                        0
                    } else {
                        0xd0d0d0
                    },
                    0xffffff,
                );
                let visible = if numeric {
                    book.measurements.get(row).is_some_and(|m| m.visible)
                } else {
                    book.objects.get(row).is_some_and(|m| m.visible)
                };
                if visible {
                    tick(&mut image, 1, top + 4);
                }
                for y in 0..15 {
                    for x in 0..15 {
                        image.set(
                            20 + x,
                            top + 2 + y,
                            if (x + y) % 2 == 0 { 0xffffff } else { 0x9b9b9b },
                        );
                    }
                }
                if numeric {
                    let source = book.measurements.get(row).map_or("", |m| m.source.as_str());
                    clipped(&mut image, source, 45, top, 103);
                    let result = scene.as_ref().and_then(|s| s.measurements.get(row));
                    let label = result
                        .map(|v| {
                            v.as_ref()
                                .map(|v| value_text(v, &settings))
                                .unwrap_or_else(|_| "Error".into())
                        })
                        .unwrap_or_default();
                    let mut result_grob =
                        Grob::new(164, 22, if row == index { 0xadccff } else { PAPER });
                    text(&mut result_grob, &label, 1, 1);
                    image.blit(
                        (154, top, 318, top + 22),
                        &result_grob,
                        (0, 0, 164, 22),
                        None,
                        255,
                    );
                    if row == index && !c.visibility {
                        image.rect(44, top - 1, 149, top + 22, 0, 0xffffff);
                        clipped(&mut image, source, 46, top, 101);
                    }
                } else {
                    image.rect(46, top, 85, top + 22, 0xd0d0d0, 0xffffff);
                    let name = book
                        .objects
                        .get(row)
                        .map(|o| format!("G{}", o.name))
                        .unwrap_or_default();
                    text(&mut image, &name, 48, top + 1);
                    let source = book.objects.get(row).map_or("", |o| o.source.as_str());
                    if row == index && !c.visibility {
                        image.rect(87, top - 1, 314, top + 22, 0, 0xffffff);
                    }
                    clipped(&mut image, source, 89, top, 224);
                }
            }
            image.fill_rect(0, 203, 319, 219, PAPER);
            let source = if numeric {
                book.measurements
                    .get(index)
                    .map_or("", |m| m.source.as_str())
            } else {
                book.objects.get(index).map_or("", |o| o.source.as_str())
            };
            text(&mut image, self.error.as_deref().unwrap_or(source), 0, 203);
            let labels = if numeric {
                ["Cmds", "Edit", "Label", "", "", ""]
            } else {
                [
                    "Cmds",
                    "Edit",
                    "",
                    if index < len { "Insert" } else { "New" },
                    "",
                    "",
                ]
            };
            chrome::softkeys(&mut image, &labels);
        }
        if let Some(menu) = self.geometry_panel.menu {
            let items = self.geometry_menu_items(menu.category);
            image.rect(0, 22, 180, 219, 0, 0xffffff);
            image.fill_rect(1, 23, 179, 41, 0x76a5d8);
            text(&mut image, "Geometry Commands", 10, 23);
            for (row, (index, label)) in items
                .iter()
                .enumerate()
                .skip(menu.first)
                .take(8)
                .enumerate()
            {
                let top = 43 + row as i64 * 21;
                if index == menu.index {
                    image.fill_rect(1, top, 179, top + 20, 0xadccff);
                }
                text(&mut image, label, 3, top);
                image.line(1, top + 20, 179, top + 20, 0xe0e0e0);
            }
        }
        let plot_minified = plot && (lcd.width() * ctx.pixels_per_point()).round() < 320.;
        if self.geometry_panel.pixels != image.pixels()
            || self.geometry_panel.plot_minified != plot_minified
        {
            self.geometry_panel.pixels = image.pixels().to_vec();
            self.geometry_panel.plot_minified = plot_minified;
            let pixels = image
                .pixels()
                .iter()
                .map(|&v| egui::Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
                .collect();
            let image = egui::ColorImage::new([320, 240], pixels);
            let options = if plot_minified {
                // Sampling a smaller plot with nearest can drop an entire thin stroke.
                egui::TextureOptions {
                    minification: egui::TextureFilter::Linear,
                    ..egui::TextureOptions::NEAREST
                }
            } else {
                egui::TextureOptions::NEAREST
            };
            if let Some(texture) = &mut self.geometry_panel.texture {
                texture.set(image, options);
            } else {
                self.geometry_panel.texture =
                    Some(ctx.load_texture("native_geometry", image, options));
            }
        }
        egui::Area::new("native_geometry".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.geometry_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let response = ui.interact(
                    lcd,
                    ui.id().with("geometry_surface"),
                    egui::Sense::click_and_drag(),
                );
                let command_text = &mut self.geometry_panel.command_text;
                if command_text.source.is_some() {
                    let [left, top] = COMMAND_ORIGIN.map(|p| p as f32);
                    let [width, height] = COMMAND_SIZE.map(|p| p as f32);
                    let rect = egui::Rect::from_min_size(
                        lcd.min + egui::vec2(left, top) * scale,
                        egui::vec2(width, height) * scale,
                    );
                    let command = ui.interact(
                        rect,
                        ui.id().with("geometry_command_text"),
                        egui::Sense::click_and_drag(),
                    );
                    let overflow = command_text.overflow();
                    if !command_text.automatic && overflow > 0 && command.dragged() {
                        command_text.offset = (command_text.offset
                            - command.drag_delta().x / scale)
                            .clamp(0., overflow as f32);
                        ctx.request_repaint();
                    }
                }
                if response.hovered() {
                    let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
                    if wheel.abs() > 1. && self.geometry_panel.menu.is_some() {
                        let mut menu = self.geometry_panel.menu.unwrap();
                        let max = self
                            .geometry_menu_items(menu.category)
                            .len()
                            .saturating_sub(8);
                        menu.first = (menu.first as i64 + if wheel < 0. { 1 } else { -1 })
                            .clamp(0, max as i64) as usize;
                        self.geometry_panel.menu = Some(menu);
                        self.dirty();
                        ctx.request_repaint();
                    } else if wheel.abs() > 1. {
                        let c = &mut self.memory.geometry_cursor;
                        let (first, len) = if numeric {
                            (&mut c.first_measurement, book.measurements.len())
                        } else {
                            (&mut c.first_object, book.objects.len())
                        };
                        if !plot {
                            *first = (*first as i64 + if wheel < 0. { 1 } else { -1 })
                                .clamp(0, len as i64) as usize;
                            self.dirty();
                            ctx.request_repaint();
                        }
                    }
                }
                if response.clicked()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / scale;
                    self.geometry_pointer(
                        [p.x as f64, p.y as f64],
                        &book,
                        scene.as_ref(),
                        &window,
                        requests,
                    );
                    ctx.request_repaint();
                }
                if plot && self.geometry_panel.menu.is_none() && !self.geometry_panel.busy {
                    if response.drag_started() {
                        self.geometry_panel.drag = self
                            .geometry_panel
                            .selected
                            .clone()
                            .filter(|name| book.can_move_point(name));
                        self.geometry_panel.drag_trace.clear();
                        if let Some(name) = self.geometry_panel.drag.as_deref()
                            && book.objects.iter().any(|o| o.name == name && o.traced)
                            && let Some(object) = scene.as_ref().and_then(|scene| {
                                scene.objects.iter().find(|object| object.name == name)
                            })
                            && let Ok(Shape::Point(p)) = object.value
                        {
                            geo::push_trace(&mut self.geometry_panel.drag_trace, p);
                        }
                    }
                    if response.dragged()
                        && let Some(p) = response.interact_pointer_pos()
                    {
                        let p = (p - lcd.min) / scale;
                        let p = unproject([p.x.floor() as f64, p.y.floor() as f64], &window);
                        self.memory.geometry_cursor.pointer = [p.x, p.y];
                        if let Some(name) = self.geometry_panel.drag.as_deref()
                            && let Some(p) = book.trace_position(name, p, scene.as_ref())
                        {
                            geo::push_trace(&mut self.geometry_panel.drag_trace, p);
                        }
                        // Redraw dependent traces through the normal plot worker. Keep
                        // the last complete scene visible while the next sample evaluates.
                        if let Some(name) = self.geometry_panel.drag.as_deref()
                            && book
                                .objects
                                .iter()
                                .any(|object| object.traced && object.name != name)
                            && !self.calculating()
                            && let Some(live) = self.memory.models.geometry_book.as_mut()
                            && live.move_point(name, p, scene.as_ref()).is_ok()
                        {
                            self.memory.models.results.remove("Geometry");
                            self.geometry_panel.refresh = true;
                        }
                        self.dirty();
                        ctx.request_repaint();
                    }
                    if response.drag_stopped()
                        && let Some(name) = self.geometry_panel.drag.take()
                    {
                        let p = Point2 {
                            x: self.memory.geometry_cursor.pointer[0],
                            y: self.memory.geometry_cursor.pointer[1],
                        };
                        let book = self.memory.models.geometry_book.as_mut().unwrap();
                        let samples = std::mem::take(&mut self.geometry_panel.drag_trace);
                        match book.move_point(&name, p, scene.as_ref()) {
                            Ok(()) => {
                                if let Some(object) =
                                    book.objects.iter_mut().find(|o| o.name == name && o.traced)
                                {
                                    for sample in samples {
                                        geo::push_trace(&mut object.trace_samples, sample);
                                    }
                                }
                                self.geometry_changed();
                            }
                            Err(e) => self.error = Some(e),
                        }
                        ctx.request_repaint();
                    }
                }
            });
    }
    fn geometry_pointer(
        &mut self,
        p: [f64; 2],
        book: &Book,
        scene: Option<&Scene>,
        window: &PlotWindow,
        requests: &mut Vec<Request>,
    ) {
        if self.geometry_panel.busy {
            return;
        }
        if let Some(mut menu) = self.geometry_panel.menu {
            if (0. ..=180.).contains(&p[0]) && (43. ..220.).contains(&p[1]) {
                menu.index = menu.first + ((p[1] - 43.) / 21.) as usize;
                if menu.index < self.geometry_menu_items(menu.category).len() {
                    requests.extend(self.geometry_choose_menu(menu));
                }
            } else {
                self.geometry_panel.menu = None;
            }
            self.dirty();
            return;
        }
        if p[1] >= 220. {
            let key = chrome::softkey_index(p[0] as i64);
            match key {
                0 => {
                    self.geometry_panel.menu = Some(Menu {
                        category: None,
                        index: 0,
                        first: 0,
                    })
                }
                1 if self.page != Some(Page::Plot) => {
                    requests.push(Request::EditMath(self.geometry_target(None)))
                }
                2 if self.page == Some(Page::Numeric) => {
                    if self.memory.geometry_cursor.measurement >= book.measurements.len() {
                        self.error = Some("Enter a measurement before adding its label".into());
                        return;
                    }

                    requests.push(Request::EditMath(MathTarget::GeometryLabel {
                        index: self.memory.geometry_cursor.measurement,
                    }))
                }
                4 if self.page == Some(Page::Plot) && self.trace_selected(scene) => {
                    self.geometry_panel.menu = Some(Menu {
                        category: Some(TRACE_MENU),
                        index: 0,
                        first: 0,
                    })
                }
                5 if self.page == Some(Page::Plot) && self.geometry_panel.selected.is_some() => {
                    self.geometry_panel.menu = Some(Menu {
                        category: Some(7),
                        index: 0,
                        first: 0,
                    })
                }
                3 if self.page == Some(Page::Symbolic) => {
                    if self.page == Some(Page::Numeric) {
                        self.memory.geometry_cursor.measurement = book.measurements.len();
                        requests.push(Request::EditMath(self.geometry_target(None)));
                    } else {
                        requests.push(Request::EditMath(MathTarget::GeometryObject {
                            name: book.next_name(),
                            preset: None,
                            insert: Some(
                                self.memory.geometry_cursor.object.min(book.objects.len()),
                            ),
                        }));
                    }
                }
                _ => {}
            }
            self.dirty();
            return;
        }
        if self.page == Some(Page::Plot) {
            let pt = unproject([p[0].floor(), p[1].floor()], window);
            self.memory.geometry_cursor.pointer = [pt.x, pt.y];
            let filter = self.geometry_panel.tool.and_then(Command::pick_filter);
            self.geometry_panel.hover = scene.and_then(|s| hit(s, book, p, window, filter));
        } else if (22. ..203.).contains(&p[1]) {
            let numeric = self.page == Some(Page::Numeric);
            let c = &mut self.memory.geometry_cursor;
            let (index, first, len) = if numeric {
                (
                    &mut c.measurement,
                    c.first_measurement,
                    book.measurements.len(),
                )
            } else {
                (&mut c.object, c.first_object, book.objects.len())
            };
            let row = first + ((p[1] - 22.) / 27.) as usize;
            if row <= len {
                let same = *index == row;
                *index = row;
                c.visibility = p[0] < 17.;
                if c.visibility {
                    requests.extend(self.geometry_action(A::Evaluate).unwrap_or_default());
                } else if !numeric && (46. ..=85.).contains(&p[0]) && row < len {
                    requests.push(Request::EditMath(MathTarget::GeometryName {
                        name: book.objects[row].name.clone(),
                    }));
                } else if same && p[0] >= 44. {
                    requests.push(Request::EditMath(self.geometry_target(None)));
                }
            }
        }
        self.dirty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn camera(xmin: f64, ymin: f64, pixel: f64) -> PlotWindow {
        let mut configuration = ppl_vm::GeometryPlotSettings::default();
        for (name, value) in [("Xmin", xmin), ("Ymin", ymin), ("PixSize", pixel)] {
            configuration
                .set(
                    name,
                    &ppl_vm::Value::Real(ppl_math::Real::from_f64(value).unwrap()),
                )
                .unwrap();
        }
        let [xmin, xmax, ymin, ymax] = configuration.derived_bounds();
        PlotWindow {
            xmin: xmin.to_f64(),
            xmax: xmax.to_f64(),
            ymin: ymin.to_f64(),
            ymax: ymax.to_f64(),
            ..Default::default()
        }
    }
    #[test]
    fn original_geometry_canvas_markers_and_touch_use_square_pixels() {
        for (window, controls) in [
            (
                camera(-16., -11., 0.1),
                vec![
                    (0., 0., 160, 109),
                    (4., 3., 200, 79),
                    (-12., -8., 40, 189),
                    (-15.5, -10.5, 5, 214),
                    (15.5, 10.5, 315, 4),
                    (-8., 5., 80, 59),
                ],
            ),
            (
                camera(-7., -3., 0.2),
                vec![
                    (0., 0., 35, 204),
                    (4., 3., 55, 189),
                    (-6., -2., 5, 214),
                    (-5., 20., 10, 104),
                    (-4.88, 30.88, 10, 49),
                    (-2.5, 28.5, 22, 61),
                    (4.9, 24.9, 59, 79),
                ],
            ),
        ] {
            for (x, y, px, py) in controls {
                let projected = project(Point2 { x, y }, &window);
                assert_eq!(
                    (projected[0] as i64, projected[1] as i64),
                    (px, py),
                    "Point({x},{y})"
                );
                let mut image = Grob::new(320, 240, 0xffffff);
                point_marker(&mut image, Point2 { x, y }, &window, 0);
                assert_eq!(image.get(px, py), Some(0));
                assert_eq!(image.get(px - 2, py), Some(0));
                assert_eq!(image.get(px, py - 2), Some(0));
            }
        }
        let p = unproject([100., 70.], &camera(-32., -21.8, 0.2));
        assert!((p.x + 12.).abs() < 1e-10 && (p.y - 8.).abs() < 1e-10);
    }
    #[test]
    fn old_square_geometry_window_promotes_without_moving_existing_pixels() {
        let old = PlotWindow {
            xmin: -15.9,
            xmax: 15.9,
            ymin: -10.9,
            ymax: 10.9,
            ..Default::default()
        };
        let settings = NativeTools::geometry_settings_for_legacy_window(&old).unwrap();
        assert_eq!(
            (
                settings.xmin.to_f64(),
                settings.ymin.to_f64(),
                settings.pix_size.to_f64()
            ),
            (-16., -11., 0.1)
        );
        let window = camera(
            settings.xmin.to_f64(),
            settings.ymin.to_f64(),
            settings.pix_size.to_f64(),
        );
        for p in [
            Point2 { x: 0., y: 0. },
            Point2 { x: 4., y: 3. },
            Point2 { x: -12., y: -8. },
        ] {
            let before = [
                1. + 318. * (p.x - old.xmin) / (old.xmax - old.xmin),
                218. - 218. * (p.y - old.ymin) / (old.ymax - old.ymin),
            ];
            let after = project(p, &window);
            assert!((before[0] - after[0]).abs() < 1e-10 && (before[1] - after[1]).abs() < 1e-10);
        }
        let asymmetric = PlotWindow {
            xmin: -10.,
            xmax: 10.,
            ymin: -20.,
            ymax: 20.,
            ..Default::default()
        };
        let state = NativeTools::geometry_settings_for_legacy_window(&asymmetric).unwrap();
        let bounds = state.derived_bounds();
        assert!(
            bounds[0].to_f64() <= -10.
                && bounds[1].to_f64() >= 10.
                && bounds[2].to_f64() <= -20.
                && bounds[3].to_f64() >= 20.
        );
    }
    #[test]
    fn dense_world_ticks_are_bounded_by_lcd_pixels() {
        assert_eq!(grid_pixels(-10000., 3180000., 1., 319).len(), 320);
        assert_eq!(grid_pixels(-10000., 2180000., 1e-100, 219).len(), 220);
    }
    #[test]
    fn geometry_setup_click_opens_the_pixel_size_editor() {
        let ctx = egui::Context::default();
        let lcd = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 240.));
        let mut tools = NativeTools::default();
        tools.memory.selected_app = "Geometry".into();
        tools.page = Some(Page::PlotSetup);
        tools.geometry_settings_changed(&ppl_vm::GeometryPlotSettings::default());
        let mut requests = vec![];
        let frame =
            |tools: &mut NativeTools, events: Vec<egui::Event>, requests: &mut Vec<Request>| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640., 480.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| tools.show_geometry_plot_setup(ctx, lcd, 1., requests),
                );
            };
        frame(&mut tools, vec![], &mut requests);
        frame(&mut tools, vec![], &mut requests);
        let position = egui::pos2(260., 60.);
        for pressed in [true, false] {
            frame(
                &mut tools,
                vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut requests,
            );
        }
        assert!(
            matches!(requests.as_slice(),[Request::EditMath(MathTarget::GeometryPlotField(name))] if name=="PixSize")
        );
    }
    #[test]
    fn geometry_keyboard_zoom_uses_live_cursor_and_queues_scoped_state() {
        let mut tools = NativeTools::default();
        tools.memory.selected_app = "Geometry".into();
        tools.page = Some(Page::Plot);
        let mut settings = ppl_vm::GeometryPlotSettings::default();
        for (name, value) in [("Xmin", -32.), ("Ymin", -21.8), ("PixSize", 0.2)] {
            settings
                .set(
                    name,
                    &ppl_vm::Value::Real(ppl_math::Real::from_f64(value).unwrap()),
                )
                .unwrap();
        }
        tools.geometry_settings_changed(&settings);
        tools.memory.geometry_cursor.pointer = [-12., 8.];
        tools.geometry_action(A::Insert("+")).unwrap();
        let settings = tools.geometry_plot_configuration().unwrap();
        assert_eq!(
            (
                settings.xmin.to_f64(),
                settings.ymin.to_f64(),
                settings.pix_size.to_f64()
            ),
            (-28., -2.9, 0.1)
        );
        assert!(matches!(
            tools.pending_actions.as_slice(),
            [Request::GeometryPlotConfiguration(_)]
        ));
    }
    #[test]
    fn geometry_visibility_flags_match_original_zero_and_one_effects() {
        let mut tools = NativeTools::default();
        tools.memory.selected_app = "Geometry".into();
        let mut settings = ppl_vm::GeometryPlotSettings {
            grid_lines: true,
            grid_dots: true,
            axes: true,
            ..Default::default()
        };
        tools.geometry_settings_changed(&settings);
        let mut image = Grob::new(320, 240, 0xffffff);
        tools.paint_geometry_plot(
            &mut image,
            &Book::default(),
            None,
            &tools.trace_window(),
            &ppl_vm::Settings::default(),
        );
        assert!(image.pixels().iter().all(|p| *p == 0xffffff));
        settings.axes = false;
        tools.memory.selected_app = "Geometry".into();
        tools.geometry_settings_changed(&settings);
        let mut image = Grob::new(320, 240, 0xffffff);
        tools.paint_geometry_plot(
            &mut image,
            &Book::default(),
            None,
            &tools.trace_window(),
            &ppl_vm::Settings::default(),
        );
        assert_eq!(image.get(160, 20), Some(0));
        assert_eq!(image.get(20, 109), Some(0));
    }
    #[test]
    fn original_g1_numeric_measurement_marks_approximate_whole_values() {
        assert_eq!(
            value_text(
                &Shape::Number { re: 5., im: 0. },
                &ppl_vm::Settings::default()
            ),
            "5."
        );
    }
}
