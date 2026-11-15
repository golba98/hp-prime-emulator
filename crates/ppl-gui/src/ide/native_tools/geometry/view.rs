use super::*;
use crate::ide::{native::chrome, native_apps::PlotWindow};
use ppl_graphics::{Grob, text::draw_text};
const PAPER: u32 = 0xe8f1f6;
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
fn project(p: Point2, w: &PlotWindow) -> [f64; 2] {
    [
        1. + 318. * (p.x - w.xmin) / (w.xmax - w.xmin),
        218. - 218. * (p.y - w.ymin) / (w.ymax - w.ymin),
    ]
}
fn unproject(p: [f64; 2], w: &PlotWindow) -> Point2 {
    Point2 {
        x: w.xmin + (p[0] - 1.) * (w.xmax - w.xmin) / 318.,
        y: w.ymin + (218. - p[1]) * (w.ymax - w.ymin) / 218.,
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
        Shape::Circle { .. } => "Circle".into(),
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
fn point_marker(g: &mut Grob, p: Point2, w: &PlotWindow, color: u32) {
    let [x, y] = project(p, w);
    if (-3. ..=323.).contains(&x) && (-3. ..=222.).contains(&y) {
        g.circle(x.round() as i64, y.round() as i64, 2, color);
    }
}
fn paint_shape(g: &mut Grob, shape: &Shape, w: &PlotWindow, color: u32) {
    match shape {
        Shape::Point(p) => point_marker(g, *p, w, color),
        Shape::Points(points) => {
            for p in points {
                point_marker(g, *p, w, color);
            }
        }
        Shape::Line { a, b, extent } => segment(g, *a, *b, *extent, w, color),
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
        Shape::Number { .. } => {}
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
fn hit(scene: &Scene, book: &Book, p: [f64; 2], w: &PlotWindow) -> Option<String> {
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
        let distance = match shape {
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
            Shape::Polygon(points) => (0..points.len())
                .map(|i| {
                    distance_segment(
                        p,
                        project(points[i], w),
                        project(points[(i + 1) % points.len()], w),
                    )
                })
                .fold(f64::INFINITY, f64::min),
            Shape::Number { .. } => continue,
        };
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
    fn paint_geometry_plot(
        &self,
        g: &mut Grob,
        book: &Book,
        scene: Option<&Scene>,
        w: &PlotWindow,
        settings: &ppl_vm::Settings,
    ) {
        let step = 10_f64.powf(((w.xmax - w.xmin) / 32.).log10().ceil());
        if step.is_finite() && step > 0. {
            for minor in [true, false] {
                let step = if minor { step / 2. } else { step };
                let color = if minor { 0xe4e4e4 } else { 0xbdbdbd };
                for i in 0..130 {
                    let x = (w.xmin / step).ceil() * step + i as f64 * step;
                    if x <= w.xmax {
                        let p = project(Point2 { x, y: 0. }, w);
                        g.line(p[0].round() as i64, 0, p[0].round() as i64, 218, color);
                    }
                    let y = (w.ymin / step).ceil() * step + i as f64 * step;
                    if y <= w.ymax {
                        let p = project(Point2 { x: 0., y }, w);
                        g.line(0, p[1].round() as i64, 319, p[1].round() as i64, color);
                    }
                }
            }
        }
        if w.axes {
            if w.xmin <= 0. && w.xmax >= 0. {
                let x = project(Point2 { x: 0., y: 0. }, w)[0].round() as i64;
                g.line(x, 0, x, 218, 0);
            }
            if w.ymin <= 0. && w.ymax >= 0. {
                let y = project(Point2 { x: 0., y: 0. }, w)[1].round() as i64;
                g.line(0, y, 319, y, 0);
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
        if plot {
            self.paint_geometry_plot(&mut image, &book, scene.as_ref(), &window, &settings);
            image.fill_rect(0, 219, 319, 239, PAPER);
            chrome::softkeys(
                &mut image,
                &[
                    "Cmds",
                    "",
                    "",
                    "",
                    "",
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
                if self.geometry_panel.selected.is_some() {
                    264
                } else {
                    319
                },
                239,
                PAPER,
            );
            let label = if let Some(command) = self.geometry_panel.tool {
                format!(
                    "{}: select {:?}",
                    command.label,
                    command
                        .needs
                        .get(self.geometry_panel.args.len())
                        .copied()
                        .unwrap_or(Need::Point)
                )
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
                Some(if self.geometry_panel.selected.is_some() {
                    210
                } else {
                    265
                }),
                None,
            );
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
        if self.geometry_panel.pixels != image.pixels() {
            self.geometry_panel.pixels = image.pixels().to_vec();
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
                        self.geometry_panel.drag = self.geometry_panel.selected.clone();
                    }
                    if response.dragged()
                        && let Some(p) = response.interact_pointer_pos()
                    {
                        let p = (p - lcd.min) / scale;
                        let p = unproject([p.x.floor() as f64, p.y.floor() as f64], &window);
                        self.memory.geometry_cursor.pointer = [p.x, p.y];
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
                        match self
                            .memory
                            .models
                            .geometry_book
                            .as_mut()
                            .unwrap()
                            .move_point(&name, p)
                        {
                            Ok(()) => self.geometry_changed(),
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
            self.geometry_panel.hover = scene.and_then(|s| hit(s, book, p, window));
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
