//! Function definitions use the Prime's measured row and command-line geometry.
use super::{MathTarget, NativeTools, Page, Request};
use crate::ide::native::{
    chrome,
    expression::{EditCommand as E, ExpressionEditor, TemplateKind},
    keys::KeyAction as A,
    math_layout,
};
use ppl_graphics::{Grob, text::draw_text};
use serde::{Deserialize, Serialize};

const PAPER: u32 = 0xe8f1f6;
const SELECTED: u32 = 0xadccff;
const DEFAULT_COLORS: [u32; 10] = [
    0x0000ff, 0xff0000, 0x008000, 0xffa500, 0x800080, 0xff00ff, 0x00ff00, 0x000000, 0x00ffff,
    0x808080,
];
const COLORS: [u32; 8] = [
    0x0000ff, 0xff0000, 0x008000, 0xffa500, 0x800080, 0xff00ff, 0x00ff00, 0x000000,
];
const FUNCTIONS: [&str; 10] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F0"];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Cursor {
    pub row: usize,
    pub first: usize,
    pub color: bool,
    pub colors: [u32; 10],
}
impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            first: 0,
            color: false,
            colors: DEFAULT_COLORS,
        }
    }
}
impl Cursor {
    pub fn validate(&self) -> Result<(), String> {
        if self.row >= 10 || self.first > 3 || self.colors.iter().any(|c| *c > 0xffffff) {
            return Err("Invalid Function Symbolic selection".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Panel {
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
    editor: Option<ExpressionEditor>,
    scroll: i64,
    show: bool,
    offset: [i64; 2],
    menu: Option<Menu>,
    busy: bool,
}
#[derive(Clone, Copy)]
enum Menu {
    Color(usize),
    Functions(usize),
}
fn text(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 14., 0, None, None);
}
impl NativeTools {
    pub fn function_definitions_changed(&mut self, snapshot: &ppl_vm::SessionSnapshot) {
        if self.memory.selected_app != "Function" {
            self.dirty();
            return;
        }
        let pending = self.presentation_pending;
        self.cancel_calculations();
        self.function_numeric_invalidate();
        self.points.clear();
        self.table.clear();
        self.error = None;
        match self.page {
            Some(Page::Numeric) => self.table(snapshot),
            Some(Page::Plot) => self.plot(snapshot.clone()),
            Some(Page::PlotDetail) => {
                self.open_special(8, true, snapshot.clone());
            }
            Some(Page::PlotTable) => {
                self.open_special(9, true, snapshot.clone());
            }
            _ => {}
        }
        self.presentation_pending = pending;
        self.dirty();
    }
    pub fn is_graph_symbolic_page(&self) -> bool {
        self.page == Some(Page::Symbolic) && self.memory.selected_app == "Function"
    }
    fn graph_cursor(&mut self) -> &mut Cursor {
        self.memory
            .graph_symbolic
            .entry(self.memory.selected_app.clone())
            .or_default()
    }
    pub fn graph_symbolic_busy(&mut self, busy: bool) {
        self.graph_symbolic_panel.busy = busy;
    }
    pub fn graph_symbolic_editor(&mut self, editor: Option<ExpressionEditor>) {
        self.graph_symbolic_panel.editor = editor;
    }
    pub fn graph_symbolic_menu(&self) -> bool {
        self.is_graph_symbolic_page() && self.graph_symbolic_panel.menu.is_some()
    }
    pub fn graph_prepare_curve(&mut self, app: &str, index: usize) -> Result<(), String> {
        if index >= 10 {
            return Err("Choose one of the ten function fields".into());
        }
        let graph = self.memory.graphs.get_mut(app).ok_or("Unknown graph app")?;
        while graph.curves.len() <= index {
            graph.curves.push(crate::ide::native_apps::Curve {
                expression: String::new(),
                second: String::new(),
                enabled: false,
                sequence: None,
                literal: None,
            });
        }
        Ok(())
    }
    pub fn graph_finished(&mut self, index: usize, advance: bool) {
        let c = self.graph_cursor();
        c.row = if advance { (index + 1).min(9) } else { index };
        c.color = false;
        self.graph_reveal();
        self.dirty();
    }
    fn graph_reveal(&mut self) {
        let c = self.graph_cursor();
        if c.row < c.first {
            c.first = c.row;
        }
        if c.row >= c.first + 7 {
            c.first = c.row - 6;
        }
        c.first = c.first.min(3);
    }
    fn graph_edit(&mut self, clear: bool) -> Request {
        let index = self.graph_cursor().row;
        self.graph_symbolic_panel.menu = None;
        Request::EditMath(MathTarget::Curve {
            app: self.memory.selected_app.clone(),
            index,
            second: false,
            clear,
        })
    }
    fn graph_changed(&mut self, requests: &mut Vec<Request>) {
        if let Some(graph) = self.memory.graphs.get(&self.memory.selected_app) {
            requests.push(Request::GraphConfiguration(
                self.memory.selected_app.clone(),
                graph.clone(),
            ));
        }
        self.error = None;
        self.dirty();
    }
    pub fn graph_symbolic_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if !self.is_graph_symbolic_page() {
            return None;
        }
        if self.graph_symbolic_panel.busy
            || self.graph_symbolic_panel.menu.is_some()
            || self.graph_symbolic_panel.show
        {
            return Some(vec![]);
        }
        if self.graph_symbolic_panel.editor.is_some() {
            return Some(vec![Request::TypeText(text.into())]);
        }
        self.graph_cursor().color = false;
        Some(vec![self.graph_edit(true), Request::TypeText(text.into())])
    }
    fn graph_choose(&mut self, index: usize, requests: &mut Vec<Request>) {
        match self.graph_symbolic_panel.menu.take() {
            Some(Menu::Color(_)) => {
                let row = self.graph_cursor().row;
                self.graph_cursor().colors[row] = COLORS[index.min(7)];
                self.graph_changed(requests);
            }
            Some(Menu::Functions(_)) => requests.push(Request::InsertTemplate(
                TemplateKind::Function(FUNCTIONS[index.min(9)]),
            )),
            None => {}
        }
    }
    pub fn graph_symbolic_action(&mut self, action: A) -> Option<Vec<Request>> {
        if !self.is_graph_symbolic_page() {
            return None;
        }
        if matches!(action, A::Home | A::Tool(_)) {
            self.graph_symbolic_panel.menu = None;
            self.graph_symbolic_panel.show = false;
            return None;
        }
        if self.graph_symbolic_panel.busy {
            return Some(vec![]);
        }
        let mut out = vec![];
        if let Some(menu) = self.graph_symbolic_panel.menu {
            let (mut index, max, columns) = match menu {
                Menu::Color(i) => (i, 7, 3),
                Menu::Functions(i) => (i, 9, 1),
            };
            match action {
                A::History(n) => {
                    index = if n < 0 {
                        index.saturating_sub(columns)
                    } else {
                        (index + columns).min(max)
                    }
                }
                A::Edit(E::Left(_)) => index = index.saturating_sub(1),
                A::Edit(E::Right(_)) => index = (index + 1).min(max),
                A::Evaluate => {
                    self.graph_choose(index, &mut out);
                    return Some(out);
                }
                A::Back | A::On => {
                    self.graph_symbolic_panel.menu = None;
                    return Some(out);
                }
                _ => return Some(out),
            }
            self.graph_symbolic_panel.menu = Some(match menu {
                Menu::Color(_) => Menu::Color(index),
                Menu::Functions(_) => Menu::Functions(index),
            });
            return Some(out);
        }
        if self.graph_symbolic_panel.show {
            let p = &mut self.graph_symbolic_panel;
            match action {
                A::History(n) => p.offset[1] = (p.offset[1] + i64::from(n) * 16).clamp(0, 65536),
                A::Edit(E::Left(_)) => p.offset[0] = p.offset[0].saturating_sub(32).max(0),
                A::Edit(E::Right(_)) => p.offset[0] = (p.offset[0] + 32).min(65536),
                A::Back | A::On | A::Evaluate => p.show = false,
                _ => {}
            }
            return Some(out);
        }
        if self.graph_symbolic_panel.editor.is_some() {
            return None;
        }
        match action {
            A::History(n) => {
                let c = self.graph_cursor();
                c.row = if n < 0 {
                    c.row.saturating_sub(1)
                } else {
                    (c.row + 1).min(9)
                };
                self.graph_reveal();
                self.dirty();
            }
            A::Edit(E::Left(_) | E::Right(_)) => {
                let c = self.graph_cursor();
                c.color = !c.color;
                self.dirty();
            }
            A::Edit(E::Backspace | E::Delete) => {
                let row = self.graph_cursor().row;
                if let Some(c) = self
                    .memory
                    .graphs
                    .get_mut("Function")
                    .and_then(|g| g.curves.get_mut(row))
                {
                    c.expression.clear();
                    c.literal = None;
                    c.enabled = false;
                }
                self.graph_changed(&mut out);
            }
            A::Evaluate => self.graph_softkey(0, &mut out),
            A::Insert(s) => return self.graph_symbolic_text(s),
            A::Template(t) => {
                out.push(self.graph_edit(true));
                out.push(Request::InsertTemplate(t));
            }
            A::Copy => {
                let row = self.graph_cursor().row;
                if let Some(c) = self
                    .memory
                    .graphs
                    .get("Function")
                    .and_then(|g| g.curves.get(row))
                {
                    out.push(Request::CopyText(c.expression.clone()));
                }
            }
            A::Back | A::On => out.push(Request::Close),
            _ => return None,
        }
        Some(out)
    }
    fn graph_softkey(&mut self, key: usize, out: &mut Vec<Request>) {
        if self.graph_symbolic_panel.busy {
            return;
        }
        if self.graph_symbolic_panel.show {
            if key == 5 {
                self.graph_symbolic_panel.show = false;
            }
            return;
        }
        if self.graph_symbolic_panel.editor.is_some() {
            match key {
                1 => self.graph_symbolic_panel.menu = Some(Menu::Functions(0)),
                2 => out.push(Request::TypeText("X".into())),
                4 => out.push(Request::FinishMath(false)),
                5 => out.push(Request::FinishMath(true)),
                _ => {}
            }
            return;
        }
        let c = self.graph_cursor().clone();
        match key {
            0 if c.color => {
                let value = c.colors[c.row];
                self.graph_symbolic_panel.menu = Some(Menu::Color(
                    COLORS.iter().position(|x| *x == value).unwrap_or(0),
                ));
            }
            0 => out.push(self.graph_edit(false)),
            1 => {
                if let Some(curve) = self
                    .memory
                    .graphs
                    .get_mut("Function")
                    .and_then(|g| g.curves.get_mut(c.row))
                    && !curve.expression.is_empty()
                {
                    curve.enabled = !curve.enabled;
                    self.graph_changed(out);
                }
            }
            2 if !c.color => {
                out.push(self.graph_edit(true));
                out.push(Request::TypeText("X".into()));
            }
            4 if !c.color => {
                self.graph_symbolic_panel.show = true;
                self.graph_symbolic_panel.offset = [0, 0];
            }
            5 if !c.color => {
                let result = self
                    .memory
                    .graphs
                    .get("Function")
                    .unwrap()
                    .expanded_definition(c.row, false, &std::sync::atomic::AtomicBool::new(false));
                match result {
                    Ok(source) => {
                        if let Some(curve) = self
                            .memory
                            .graphs
                            .get_mut("Function")
                            .and_then(|g| g.curves.get_mut(c.row))
                        {
                            curve.expression = source;
                            curve.literal = None;
                        }
                        self.graph_changed(out);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            _ => {}
        }
    }
    pub fn graph_plot_colors(&self) -> Vec<egui::Color32> {
        if self.memory.selected_app != "Function" {
            return vec![];
        }
        let colors = self
            .memory
            .graph_symbolic
            .get(&self.memory.selected_app)
            .map_or(DEFAULT_COLORS, |c| c.colors);
        self.memory
            .graphs
            .get(&self.memory.selected_app)
            .map(|g| {
                g.curves
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| c.enabled && !c.expression.trim().is_empty())
                    .map(|(i, _)| {
                        let c = colors[i % 10];
                        egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn show_graph_symbolic(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        out: &mut Vec<Request>,
    ) {
        self.graph_reveal();
        let cursor = self.graph_cursor().clone();
        let graph = &self.memory.graphs["Function"];
        let selected = graph
            .curves
            .get(cursor.row)
            .map_or("", |c| c.expression.as_str())
            .to_owned();
        let mut g = Grob::new(320, 240, PAPER);
        chrome::header(&mut g, "Function Symbolic View");
        for (row, name) in FUNCTIONS.iter().enumerate().skip(cursor.first).take(7) {
            let y = 20 + (row - cursor.first) as i64 * 26;
            let active = row == cursor.row;
            if active {
                g.fill_rect(0, y, 314, y + 25, SELECTED);
            }
            g.rect(1, y + 6, 14, y + 18, 0xe1e1e1, 0xffffff);
            let curve = graph.curves.get(row);
            if curve.is_some_and(|c| c.enabled && !c.expression.is_empty()) {
                g.line(3, y + 12, 5, y + 16, 0);
                g.line(5, y + 16, 11, y + 7, 0);
            }
            g.rect(17, y + 3, 35, y + 21, 0xe1e1e1, 0xffffff);
            g.fill_rect(19, y + 5, 33, y + 19, cursor.colors[row]);
            text(&mut g, &format!("{name}(X)="), 38, y + 5);
            let color_active = active && cursor.color;
            if color_active {
                g.rect(15, y + 1, 36, y + 23, 0, 0xffffff);
                g.fill_rect(19, y + 5, 33, y + 19, cursor.colors[row]);
            }
            g.rect(78, y + 2, 313, y + 23, 0xe1e1e1, 0xffffff);
            if active && !cursor.color {
                g.rect(77, y + 1, 314, y + 24, 0, 0);
                g.fill_rect(79, y + 3, 312, y + 22, 0xffffff);
            }
            if let Some(curve) = curve
                && !curve.expression.is_empty()
            {
                let layout = math_layout::expression(&curve.expression, 14.);
                let mut cell = Grob::new(233, 20, 0xffffff);
                layout.paint_static(&mut cell, 0, 0, 0);
                g.blit((79, y + 3, 312, y + 23), &cell, (0, 0, 233, 20), None, 255);
            }
        }
        g.line(317, 22, 317, 199, 0x888888);
        let top = 22 + cursor.first as i64 * 18;
        g.fill_rect(316, top, 318, top + 122, 0x5f5f5f);
        g.fill_rect(0, 202, 319, 219, PAPER);
        text(
            &mut g,
            self.error.as_deref().unwrap_or(if cursor.color {
                "Choose graph color"
            } else {
                "Enter function"
            }),
            0,
            203,
        );
        chrome::softkeys(
            &mut g,
            if cursor.color {
                &["Choose", "√", "", "", "", ""]
            } else {
                &["Edit", "√", "X", "", "Show", "Eval"]
            },
        );
        let mut entry_top = 203;
        if let Some(editor) = &self.graph_symbolic_panel.editor {
            let layout = math_layout::layout(editor, 60);
            let height = layout.height.clamp(17, 60);
            entry_top = 220 - height;
            let caret = layout.cursor(editor.cursor()).0;
            let scroll = &mut self.graph_symbolic_panel.scroll;
            if caret < *scroll {
                *scroll = caret;
            } else if caret > *scroll + 311 {
                *scroll = caret - 311;
            }
            g.fill_rect(0, entry_top - 17, 319, 219, PAPER);
            text(
                &mut g,
                self.error.as_deref().unwrap_or("Enter function"),
                0,
                entry_top - 17,
            );
            let mut entry = Grob::new(320, height as usize, 0xffffff);
            layout.paint(
                &mut entry,
                1 - *scroll,
                0,
                editor.selection(),
                editor.cursor(),
            );
            g.blit(
                (0, entry_top, 320, 220),
                &entry,
                (0, 0, 320, height),
                None,
                255,
            );
            chrome::softkeys(&mut g, &["", "F", "X", "", "Cancel", "OK"]);
        }
        if self.graph_symbolic_panel.show {
            g.fill(0xffffff);
            let layout = math_layout::expression(&selected, 14.);
            let off = self.graph_symbolic_panel.offset;
            layout.paint_static(
                &mut g,
                ((320 - layout.width) / 2).max(2) - off[0],
                ((220 - layout.height) / 2).max(2) - off[1],
                0,
            );
            chrome::softkeys(&mut g, &["", "", "", "", "", "OK"]);
        }
        if let Some(menu) = self.graph_symbolic_panel.menu {
            match menu {
                Menu::Color(index) => {
                    g.rect(16, 35, 122, 141, 0, 0xffffff);
                    for (i, color) in COLORS.iter().enumerate() {
                        let x = 19 + (i % 3) as i64 * 35;
                        let y = 38 + (i / 3) as i64 * 35;
                        g.fill_rect(x, y, x + 30, y + 30, *color);
                        if i == index {
                            g.rect(x - 2, y - 2, x + 32, y + 32, 0, *color);
                            g.fill_rect(x, y, x + 30, y + 30, *color);
                        }
                    }
                }
                Menu::Functions(index) => {
                    let first = index.saturating_sub(7);
                    g.rect(53, 30, 156, 218, 0, 0xffffff);
                    g.fill_rect(54, 31, 155, 49, 0x517ebd);
                    draw_text(&mut g, "Functions", 62, 32, 14., 0xffffff, None, None);
                    for (i, name) in FUNCTIONS.iter().enumerate().skip(first).take(8) {
                        let y = 50 + (i - first) as i64 * 21;
                        if i == index {
                            g.fill_rect(54, y, 155, y + 20, SELECTED);
                        }
                        text(&mut g, name, 58, y + 2);
                    }
                }
            }
        }
        if self.graph_symbolic_panel.pixels != g.pixels()
            || self.graph_symbolic_panel.texture.is_none()
        {
            self.graph_symbolic_panel.pixels = g.pixels().to_vec();
            let image = egui::ColorImage::new(
                [320, 240],
                g.pixels()
                    .iter()
                    .map(|c| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, *c as u8))
                    .collect(),
            );
            if let Some(t) = &mut self.graph_symbolic_panel.texture {
                t.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.graph_symbolic_panel.texture = Some(ctx.load_texture(
                    "native_function_symbolic",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        egui::Area::new("native_function_symbolic".into())
            .fixed_pos(lcd.min)
            .fade_in(false)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.graph_symbolic_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let r = ui.interact(lcd, ui.id().with("function_surface"), egui::Sense::click());
                if r.hovered() {
                    let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
                    if wheel.abs() > 1. {
                        out.extend(
                            self.graph_symbolic_action(A::History(if wheel < 0. { 1 } else { -1 }))
                                .unwrap_or_default(),
                        );
                        ctx.request_repaint();
                    }
                }
                if r.clicked()
                    && let Some(p) = r.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / (lcd.width() / 320.);
                    let (x, y) = (p.x as i64, p.y as i64);
                    if let Some(menu) = self.graph_symbolic_panel.menu {
                        match menu {
                            Menu::Color(_) => {
                                if (16..123).contains(&x) && (35..142).contains(&y) {
                                    let i = ((y - 35) / 35 * 3 + (x - 16) / 35) as usize;
                                    if i < 8 {
                                        self.graph_choose(i, out);
                                    }
                                } else {
                                    self.graph_symbolic_panel.menu = None;
                                }
                            }
                            Menu::Functions(index) => {
                                if (53..157).contains(&x) && (50..218).contains(&y) {
                                    let i = index.saturating_sub(7) + (y - 50) as usize / 21;
                                    if i < 10 {
                                        self.graph_choose(i, out);
                                    }
                                } else {
                                    self.graph_symbolic_panel.menu = None;
                                }
                            }
                        }
                    } else if y >= 220 {
                        self.graph_softkey(chrome::softkey_index(x), out);
                    } else if let Some(editor) = &self.graph_symbolic_panel.editor {
                        if y >= entry_top {
                            let layout = math_layout::layout(editor, 60);
                            out.push(Request::MathCursor {
                                position: layout.hit_test(
                                    x - 1 + self.graph_symbolic_panel.scroll,
                                    y - entry_top,
                                ),
                                extend: false,
                            });
                        }
                    } else if !self.graph_symbolic_panel.show && (20..202).contains(&y) {
                        let row = cursor.first + (y - 20) as usize / 26;
                        if row < 10 {
                            let c = self.graph_cursor();
                            c.row = row;
                            c.color = (15..37).contains(&x);
                            if x < 15 {
                                self.graph_softkey(1, out);
                            }
                            self.dirty();
                        }
                    }
                    ctx.request_repaint();
                }
            });
    }
}
