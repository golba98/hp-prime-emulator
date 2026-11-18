//! Supported applications in the Prime's four-column Application Library.
//! Icons are drawn here from primitives; no firmware artwork is distributed.
use super::{NativeTools, Page, Request, ResetApp};
use crate::ide::native::{chrome, expression::EditCommand as E, keys::KeyAction as A};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use serde::{Deserialize, Serialize};

const APPS: [&str; 18] = [
    "Function",
    "Advanced Graphing",
    "Geometry",
    "Spreadsheet",
    "Statistics 1Var",
    "Statistics 2Var",
    "Inference",
    "Solve",
    "Linear Solver",
    "Triangle Solver",
    "Parametric",
    "Polar",
    "Sequence",
    "Finance",
    "Linear Explorer",
    "Quadratic Explorer",
    "Trig Explorer",
    super::super::native_graph3d::NAME,
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    pub highlight: String,
    pub sort: Sort,
    pub recent: Vec<String>,
    pub first_row: usize,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    Chronological,
    Alphabetic,
    #[default]
    Fixed,
}
impl Sort {
    fn index(self) -> usize {
        match self {
            Self::Chronological => 0,
            Self::Alphabetic => 1,
            Self::Fixed => 2,
        }
    }
    fn at(index: usize) -> Self {
        [Self::Chronological, Self::Alphabetic, Self::Fixed][index.min(2)]
    }
}
impl Selection {
    pub fn validate(&self) -> Result<(), String> {
        let valid = |n: &str| APPS.contains(&n);
        if self.first_row > 2
            || (!self.highlight.is_empty() && !valid(&self.highlight))
            || self.recent.len() > APPS.len()
            || self.recent.iter().any(|n| !valid(n))
            || self
                .recent
                .iter()
                .enumerate()
                .any(|(i, n)| self.recent[..i].contains(n))
        {
            return Err("Invalid Application Library selection".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Panel {
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
    pub(super) menu: Option<usize>,
    confirm_reset: Option<ResetApp>,
    prefix: String,
    typed: Option<std::time::Instant>,
}
fn names(selection: &Selection) -> Vec<&'static str> {
    let mut names = APPS.to_vec();
    match selection.sort {
        Sort::Alphabetic => names.sort_unstable(),
        Sort::Chronological => names.sort_by_key(|n| {
            selection
                .recent
                .iter()
                .position(|r| r == n)
                .unwrap_or(APPS.len())
        }),
        Sort::Fixed => {}
    }
    names
}
fn label(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 12., 0, None, None);
}

impl NativeTools {
    pub(super) fn gallery_open(&mut self) {
        self.memory.gallery.highlight = self.memory.selected_app.clone();
        self.gallery_panel.menu = None;
        self.gallery_panel.confirm_reset = None;
        self.gallery_panel.prefix.clear();
        self.gallery_panel.typed = None;
        self.gallery_reveal();
    }
    fn gallery_index(&self, names: &[&str]) -> usize {
        names
            .iter()
            .position(|n| *n == self.memory.gallery.highlight)
            .or_else(|| names.iter().position(|n| *n == self.memory.selected_app))
            .unwrap_or(0)
    }
    fn gallery_reveal(&mut self) {
        let index = self.gallery_index(&names(&self.memory.gallery));
        let row = index / 4;
        let first = &mut self.memory.gallery.first_row;
        *first = (*first).min(APPS.len().div_ceil(4).saturating_sub(3));
        if row < *first {
            *first = row;
        }
        if row >= *first + 3 {
            *first = row - 2;
        }
    }
    fn gallery_sort(&mut self, index: usize) {
        self.memory.gallery.sort = Sort::at(index);
        self.gallery_panel.menu = None;
        self.gallery_reveal();
        self.dirty();
    }
    pub fn gallery_started(&mut self, name: &str) {
        if !APPS.contains(&name) {
            return;
        }
        self.memory.gallery.recent.retain(|n| n != name);
        self.memory.gallery.recent.insert(0, name.into());
        self.memory.gallery.recent.truncate(APPS.len());
    }
    pub fn gallery_text(&mut self, text: &str) -> bool {
        if self.page != Some(Page::Apps) {
            return false;
        }
        if self.gallery_panel.menu.is_some() || self.gallery_panel.confirm_reset.is_some() {
            return true;
        }
        let now = std::time::Instant::now();
        if self
            .gallery_panel
            .typed
            .is_none_or(|t| now.duration_since(t).as_secs_f64() > 1.)
        {
            self.gallery_panel.prefix.clear();
        }
        self.gallery_panel.typed = Some(now);
        for c in text.chars().filter(|c| c.is_ascii_alphabetic()) {
            if self.gallery_panel.prefix.len() >= 2 {
                self.gallery_panel.prefix.clear();
            }
            self.gallery_panel.prefix.push(c.to_ascii_lowercase());
            let names = names(&self.memory.gallery);
            let find = |s: &str| names.iter().find(|n| n.to_ascii_lowercase().starts_with(s));
            if find(&self.gallery_panel.prefix).is_none() {
                self.gallery_panel.prefix = c.to_ascii_lowercase().to_string();
            }
            if let Some(name) = find(&self.gallery_panel.prefix) {
                self.memory.gallery.highlight = (*name).into();
            }
        }
        self.gallery_reveal();
        self.dirty();
        true
    }
    pub fn gallery_action(&mut self, action: A) -> Option<Vec<Request>> {
        if self.page != Some(Page::Apps) {
            return None;
        }
        if self.gallery_panel.confirm_reset.is_some() {
            let request = match action {
                A::Evaluate => self.finish_app_reset(true),
                A::Back | A::On => self.finish_app_reset(false),
                _ => None,
            };
            return Some(request.into_iter().collect());
        }
        if let Some(index) = self.gallery_panel.menu {
            match action {
                A::History(n) => {
                    self.gallery_panel.menu = Some(if n < 0 {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(2)
                    })
                }
                A::Evaluate => self.gallery_sort(index),
                A::Back | A::On => self.gallery_panel.menu = None,
                A::Home | A::Tool(_) => return None,
                _ => {}
            }
            return Some(vec![]);
        }
        let names = names(&self.memory.gallery);
        let index = self.gallery_index(&names);
        let next = match action {
            A::History(n) if n < 0 => index.saturating_sub(4),
            A::History(_) => (index + 4).min(names.len() - 1),
            A::Edit(E::Left(_)) => index.saturating_sub(1),
            A::Edit(E::Right(_)) => (index + 1).min(names.len() - 1),
            A::Evaluate => return Some(vec![Request::StartApp(names[index].into())]),
            A::Back | A::On => return Some(vec![Request::Close]),
            _ => return None,
        };
        self.memory.gallery.highlight = names[next].into();
        self.gallery_reveal();
        self.dirty();
        Some(vec![])
    }
    fn finish_app_reset(&mut self, confirmed: bool) -> Option<Request> {
        let target = self.gallery_panel.confirm_reset.take()?;
        self.dirty();
        confirmed.then_some(Request::ResetApp(target))
    }
    pub(super) fn show_gallery(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        requests: &mut Vec<Request>,
    ) {
        self.gallery_reveal();
        let names = names(&self.memory.gallery);
        let selected = self.gallery_index(&names);
        let first = self.memory.gallery.first_row;
        let mut g = Grob::new(320, 240, 0xffffff);
        chrome::header(&mut g, "Application Library");
        for (i, name) in names.iter().enumerate().skip(first * 4).take(12) {
            let x = (i % 4) as i64 * 79;
            let y = 20 + (i / 4 - first) as i64 * 66;
            if i == selected {
                g.fill_rect(x, y, x + 77, y + 65, 0xadccff);
            }
            icon(&mut g, name, x + 20, y + 3);
            let lines: Vec<_> = match *name {
                "Advanced Graphing" => vec!["Advanced", "Graphing"],
                "Linear Explorer" => vec!["Linear", "Explorer"],
                "Quadratic Explorer" => vec!["Quadratic", "Explorer"],
                "Trig Explorer" => vec!["Trig", "Explorer"],
                "Triangle Solver" => vec!["Triangle", "Solver"],
                _ if measure(name, 12.) > 78 => name
                    .rsplit_once(' ')
                    .map_or_else(|| vec![*name], |(a, b)| vec![a, b]),
                _ => vec![*name],
            };
            for (row, line) in lines.iter().enumerate() {
                label(
                    &mut g,
                    line,
                    x + (78 - measure(line, 12.)) / 2,
                    y + 40 + row as i64 * 12,
                );
            }
        }
        let rows = names.len().div_ceil(4);
        if rows > 3 {
            g.line(317, 22, 317, 217, 0x888888);
            let height = 196 * 3 / rows as i64;
            let top = 22 + (196 - height) * first as i64 / (rows - 3) as i64;
            g.fill_rect(316, top, 318, top + height - 1, 0x3d3d3d);
        }
        let reset_target = ResetApp::from_name(names[selected]);
        let reset_label = if reset_target.is_some() { "Reset" } else { "" };
        chrome::softkeys_with_menus(&mut g, &["", reset_label, "Sort", "", "", "Start"], &[2]);
        if let Some(index) = self.gallery_panel.menu {
            g.rect(34, 134, 158, 219, 0, 0xffffff);
            g.fill_rect(35, 135, 157, 153, 0x517ebd);
            draw_text(&mut g, "Sort Apps", 69, 136, 14., 0xffffff, None, None);
            for (i, s) in ["Chronologically", "Alphabetically", "Fixed"]
                .iter()
                .enumerate()
            {
                let y = 154 + i as i64 * 21;
                if i == index {
                    g.fill_rect(35, y, 157, y + 20, 0xadccff);
                }
                label(
                    &mut g,
                    &format!(
                        "{}{} {s}",
                        if self.memory.gallery.sort.index() == i {
                            "✓"
                        } else {
                            ""
                        },
                        i + 1
                    ),
                    36,
                    y + 3,
                );
            }
        }
        if let Some(target) = self.gallery_panel.confirm_reset {
            g.rect(26, 72, 294, 170, 0x234765, 0xffffff);
            g.fill_rect(27, 73, 293, 98, 0x234765);
            let title = format!("Reset {}?", target.name());
            draw_text(&mut g, &title, 39, 78, 16., 0xffffff, None, None);
            let scope = match target {
                ResetApp::Sequence => "Clear definitions and initial terms.",
                ResetApp::Geometry => "Clear constructions and measurements.",
                ResetApp::StatisticsOne | ResetApp::StatisticsTwo => {
                    "Clear data columns and analysis definitions."
                }
            };
            label(&mut g, scope, 39, 108);
            let settings = match target {
                ResetApp::Sequence => "Reset Plot, Numeric and Symbolic.",
                ResetApp::Geometry => "Restore the default plot settings.",
                ResetApp::StatisticsOne | ResetApp::StatisticsTwo => {
                    "Restore default plot and Symbolic settings."
                }
            };
            label(&mut g, settings, 39, 131);
            chrome::softkeys(&mut g, &["Cancel", "", "", "", "", "Reset"]);
        }
        if let Some(error) = &self.error {
            g.fill_rect(0, 201, 313, 219, 0xffffff);
            draw_text(&mut g, error, 2, 202, 12., 0xc00000, Some(310), None);
        }
        if self.gallery_panel.pixels != g.pixels() || self.gallery_panel.texture.is_none() {
            self.gallery_panel.pixels = g.pixels().to_vec();
            let image = egui::ColorImage::new(
                [320, 240],
                g.pixels()
                    .iter()
                    .map(|p| egui::Color32::from_rgb((p >> 16) as u8, (p >> 8) as u8, *p as u8))
                    .collect(),
            );
            if let Some(t) = &mut self.gallery_panel.texture {
                t.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.gallery_panel.texture =
                    Some(ctx.load_texture("native_library", image, egui::TextureOptions::NEAREST));
            }
        }
        egui::Area::new("native_library".into())
            .fade_in(false)
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.gallery_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let r = ui.interact(lcd, ui.id().with("library_surface"), egui::Sense::click());
                if r.hovered() {
                    let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
                    if wheel.abs() > 1.
                        && let Some(out) =
                            self.gallery_action(A::History(if wheel < 0. { 1 } else { -1 }))
                    {
                        requests.extend(out);
                        ctx.request_repaint();
                    }
                }
                if r.clicked()
                    && let Some(p) = r.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / (lcd.width() / 320.);
                    let (x, y) = (p.x as i64, p.y as i64);
                    if self.gallery_panel.confirm_reset.is_some() {
                        if y >= 220 {
                            match chrome::softkey_index(x) {
                                0 => {
                                    self.finish_app_reset(false);
                                }
                                5 => requests.extend(self.finish_app_reset(true)),
                                _ => {}
                            }
                        }
                    } else if y >= 220 {
                        match chrome::softkey_index(x) {
                            1 if self.gallery_panel.menu.is_none() => {
                                self.gallery_panel.confirm_reset = reset_target;
                                self.error = None;
                                self.dirty();
                            }
                            2 => self.gallery_panel.menu = Some(self.memory.gallery.sort.index()),
                            5 => requests.push(Request::StartApp(names[selected].into())),
                            _ => {}
                        }
                    } else if self.gallery_panel.menu.is_some() {
                        if (34..=158).contains(&x) && (154..217).contains(&y) {
                            self.gallery_sort((y - 154) as usize / 21);
                        } else {
                            self.gallery_panel.menu = None;
                        }
                    } else if (20..218).contains(&y) && x < 316 {
                        let index = (first + (y - 20) as usize / 66) * 4 + x as usize / 79;
                        if let Some(name) = names.get(index) {
                            self.memory.gallery.highlight = (*name).into();
                            self.dirty();
                            requests.push(Request::StartApp((*name).into()));
                        }
                    }
                    ctx.request_repaint();
                }
            });
    }
}

fn icon(g: &mut Grob, name: &str, x: i64, y: i64) {
    let color = match name {
        "Geometry" | "Spreadsheet" => 0xba171e,
        "Statistics 1Var" | "Statistics 2Var" | "Inference" => 0x9739b5,
        "Solve" | "Linear Solver" | "Triangle Solver" => 0xf57916,
        "Finance" | "Linear Explorer" | "Quadratic Explorer" | "Trig Explorer" => 0x3aaf1a,
        _ => 0x347fe3,
    };
    for row in 0..38 {
        let inset = if row == 0 || row == 37 {
            5
        } else if !(3..=34).contains(&row) {
            2
        } else {
            0
        };
        g.line(x + inset, y + row, x + 37 - inset, y + row, color);
    }
    let white = 0xffffff;
    if name == "Spreadsheet" {
        g.rect(x + 5, y + 4, x + 32, y + 33, white, color);
        for n in 1..5 {
            g.line(x + 5, y + 4 + n * 6, x + 32, y + 4 + n * 6, white);
        }
        for n in 1..3 {
            g.line(x + 5 + n * 9, y + 4, x + 5 + n * 9, y + 33, white);
        }
    } else if name == "Geometry" || name == "Triangle Solver" {
        if name == "Geometry" {
            g.circle(x + 19, y + 19, 15, white);
        }
        g.line(x + 7, y + 31, x + 13, y + 6, white);
        g.line(x + 13, y + 6, x + 31, y + 24, white);
        g.line(x + 31, y + 24, x + 7, y + 31, white);
    } else if name == super::super::native_graph3d::NAME {
        let corners = [
            (7, 13),
            (19, 5),
            (32, 12),
            (20, 20),
            (7, 26),
            (19, 18),
            (32, 25),
            (20, 33),
        ];
        for (a, b) in [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            let (ax, ay) = corners[a];
            let (bx, by) = corners[b];
            g.line(x + ax, y + ay, x + bx, y + by, white);
        }
    } else if name == "Advanced Graphing" {
        g.line(x + 4, y + 31, x + 34, y + 31, white);
        g.line(x + 8, y + 3, x + 8, y + 34, white);
        let mut last = None;
        for i in 0..=60 {
            let t = i as f64 * std::f64::consts::TAU / 60.;
            let point = (
                x + 19 + (14. * t.cos()) as i64,
                y + 19 + (9. * t.sin()) as i64,
            );
            if let Some((a, b)) = last {
                g.line(a, b, point.0, point.1, white);
            }
            last = Some(point);
        }
        g.line(x + 10, y + 33, x + 28, y + 4, white);
    } else if name == "Polar" || name == "Parametric" {
        g.line(x + 3, y + 19, x + 34, y + 19, white);
        g.line(x + 19, y + 3, x + 19, y + 34, white);
        let mut last = None;
        for i in 0..=120 {
            let t = i as f64 * std::f64::consts::TAU / 120.;
            let (a, b) = if name == "Polar" {
                let r = 15. * (3. * t).cos();
                (r * t.cos(), r * t.sin())
            } else {
                (14. * t.sin(), 14. * (2. * t).sin())
            };
            let point = (x + 19 + a as i64, y + 19 - b as i64);
            if let Some((a, b)) = last {
                g.line(a, b, point.0, point.1, white);
            }
            last = Some(point);
        }
    } else if name == "Statistics 1Var" {
        for (i, h) in [15, 25, 19, 30].into_iter().enumerate() {
            let a = x + 5 + i as i64 * 7;
            g.rect(a, y + 33 - h, a + 4, y + 33, white, color);
        }
    } else if name == "Statistics 2Var" {
        for (a, b) in [(5, 26), (11, 23), (17, 25), (20, 13), (27, 17), (31, 7)] {
            g.fill_rect(x + a, y + b, x + a + 2, y + b + 2, white);
        }
        g.line(x + 4, y + 31, x + 33, y + 7, white);
    } else if name == "Finance" {
        draw_text(g, "$", x + 10, y + 4, 28., white, None, None);
    } else if name == "Linear Solver" || name == "Solve" {
        g.circle(x + 19, y + 19, 12, white);
        g.line(x + 19, y + 2, x + 19, y + 35, white);
        g.line(x + 2, y + 19, x + 35, y + 19, white);
        g.circle(x + 19, y + 19, 3, white);
    } else {
        g.line(x + 4, y + 31, x + 34, y + 31, white);
        g.line(x + 8, y + 3, x + 8, y + 34, white);
        let mut last = None;
        for a in 0..29 {
            let t = a as f64 / 28.;
            let v = match name {
                "Inference" => 29. - 27. * (-25. * (t - 0.5).powi(2)).exp(),
                "Linear Explorer" => 30. - 27. * t,
                "Quadratic Explorer" => 4. + 100. * (t - 0.5).powi(2),
                "Trig Explorer" | "Sequence" => 19. - 12. * (t * 7.).sin(),
                _ => 30. - 27. * t.sqrt(),
            };
            let point = (x + 5 + a, y + v as i64);
            if name == "Sequence" {
                g.set(point.0, point.1, white);
            } else if let Some((px, py)) = last {
                g.line(px, py, point.0, point.1, white);
            }
            last = Some(point);
        }
    }
}
