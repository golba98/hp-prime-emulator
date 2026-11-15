//! Function app settings share the calculator's LCD coordinates and app memory.
use super::{NativeTools, Page, Request};
use crate::ide::native::{chrome, expression::EditCommand as E, keys::KeyAction as A};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use ppl_vm::{AppSettings, SessionSnapshot};
use serde::{Deserialize, Serialize};

const PAPER: u32 = 0xe8f1f6;
const SELECTED: u32 = 0xadccff;
const ANGLES: [&str; 4] = ["System", "Radians", "Degrees", "Gradians"];
const FORMATS: [&str; 7] = [
    "System",
    "Standard",
    "Fixed",
    "Scientific",
    "Engineering",
    "Floating",
    "Rounded",
];
const COMPLEX: [&str; 3] = ["System", "ON", "OFF"];
const DIGITS: [&str; 12] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"];
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    pub setup_focus: usize,
}
impl Memory {
    pub fn validate(&self) -> Result<(), String> {
        if self.setup_focus > 3 {
            return Err("Invalid Function settings selection".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub(super) struct Panel {
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
    menu: Option<usize>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Angle,
    Format,
    Digits,
    Complex,
}
impl Field {
    fn at(code: usize) -> Self {
        [Self::Angle, Self::Format, Self::Digits, Self::Complex][code.min(3)]
    }
    fn code(self) -> usize {
        match self {
            Self::Angle => 0,
            Self::Format => 1,
            Self::Digits => 2,
            Self::Complex => 3,
        }
    }
    fn rect(self) -> [i64; 4] {
        match self {
            Self::Angle => [153, 24, 314, 45],
            Self::Format => [153, 50, 266, 70],
            Self::Digits => [268, 50, 314, 70],
            Self::Complex => [153, 76, 314, 96],
        }
    }
    fn choices(self) -> &'static [&'static str] {
        match self {
            Self::Angle => &ANGLES,
            Self::Format => &FORMATS,
            Self::Complex => &COMPLEX,
            Self::Digits => &DIGITS,
        }
    }
    fn value(self, s: &AppSettings) -> i64 {
        match self {
            Self::Angle => s.angle,
            Self::Format => s.format,
            Self::Digits => s.digits,
            Self::Complex => s.complex,
        }
    }
    fn set(self, s: &mut AppSettings, n: i64) {
        match self {
            Self::Angle => s.angle = n,
            Self::Format => s.format = n,
            Self::Digits => s.digits = n,
            Self::Complex => s.complex = n,
        }
    }
    fn help(self) -> &'static str {
        match self {
            Self::Angle => "Choose angle measure",
            Self::Format => "Choose format for numbers",
            Self::Digits => "Choose decimal places to display",
            Self::Complex => "Choose complex number mode",
        }
    }
}
fn text(g: &mut Grob, s: &str, x: i64, y: i64) {
    draw_text(g, s, x, y, 14., 0, None, None);
}
fn settings(s: &SessionSnapshot) -> AppSettings {
    s.app_settings.get("Function").cloned().unwrap_or_default()
}
impl NativeTools {
    pub fn is_function_view(&self) -> bool {
        self.memory.selected_app == "Function" && self.page == Some(Page::SymbolicSetup)
    }
    fn function_field(&self) -> Field {
        Field::at(self.memory.function_views.setup_focus)
    }
    fn function_select(&mut self, field: Field) {
        self.memory.function_views.setup_focus = field.code();
        self.function_panel.menu = None;
        self.error = None;
        self.dirty();
    }
    fn function_choose(&mut self, s: &SessionSnapshot, index: usize, out: &mut Vec<Request>) {
        let mut next = settings(s);
        self.function_field().set(&mut next, index as i64);
        self.function_panel.menu = None;
        out.push(Request::SetAppSettings("Function".into(), next));
    }
    fn function_activate(&mut self, s: &SessionSnapshot, _out: &mut Vec<Request>) {
        let field = self.function_field();
        self.function_panel.menu = Some(
            field
                .value(&settings(s))
                .clamp(0, field.choices().len() as i64 - 1) as usize,
        );
    }
    pub fn function_view_text(&mut self, text: &str, s: &SessionSnapshot) -> Option<Vec<Request>> {
        if !self.is_function_view() {
            return None;
        }
        if self.function_field() == Field::Digits
            && let Ok(index) = text.parse::<usize>()
            && index <= 11
        {
            let mut out = vec![];
            self.function_choose(s, index, &mut out);
            return Some(out);
        }
        Some(vec![])
    }
    fn function_menu_geometry(&self, index: usize) -> (i64, i64, i64, usize, usize) {
        let field = self.function_field();
        let items = field.choices();
        if field == Field::Digits {
            return (266, 23, 49, index.saturating_sub(8), 9);
        }
        let [x, y, r, _] = field.rect();
        let count = items.len().min(9);
        let top = (y - 2).min(219 - count as i64 * 21);
        let width = if field == Field::Format {
            r - x + 2
        } else {
            (items.iter().map(|s| measure(s, 14.)).max().unwrap_or(70) + 27)
                .min(r - x + 1)
                .max(80)
        };
        (x, top, width, 0, count)
    }
    pub fn function_view_action(&mut self, action: A, s: &SessionSnapshot) -> Option<Vec<Request>> {
        if !self.is_function_view() {
            return None;
        }
        if matches!(action, A::Home | A::Tool(_)) {
            self.function_panel.menu = None;
            return None;
        }
        let mut out = vec![];
        if let Some(index) = self.function_panel.menu {
            match action {
                A::History(n) => {
                    let max = self.function_field().choices().len() - 1;
                    self.function_panel.menu = Some(if n < 0 {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(max)
                    });
                }
                A::Evaluate => self.function_choose(s, index, &mut out),
                A::Back | A::On => self.function_panel.menu = None,
                A::Insert(text) => return self.function_view_text(text, s),
                _ => {}
            }
            return Some(out);
        }
        match action {
            A::History(n) => {
                let fields = if settings(s).format >= 2 {
                    vec![Field::Angle, Field::Format, Field::Digits, Field::Complex]
                } else {
                    vec![Field::Angle, Field::Format, Field::Complex]
                };
                let i = fields
                    .iter()
                    .position(|f| *f == self.function_field())
                    .unwrap_or(0);
                let next = if n < 0 {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(fields.len() - 1)
                };
                self.function_select(fields[next]);
            }
            A::Edit(E::Left(_)) => {
                if self.function_field() == Field::Digits {
                    self.function_select(Field::Format);
                } else {
                    self.function_activate(s, &mut out);
                }
            }
            A::Edit(E::Right(_)) => {
                if self.function_field() == Field::Format && settings(s).format >= 2 {
                    self.function_select(Field::Digits);
                } else {
                    self.function_activate(s, &mut out);
                }
            }
            A::Edit(E::SelectAll) => {}
            A::Evaluate => self.function_activate(s, &mut out),
            A::Edit(E::Backspace | E::Delete) => {
                let mut next = settings(s);
                let field = self.function_field();
                field.set(&mut next, if field == Field::Digits { 4 } else { 0 });
                out.push(Request::SetAppSettings("Function".into(), next));
            }
            A::Edit(E::Clear) => out.push(Request::SetAppSettings(
                "Function".into(),
                AppSettings::default(),
            )),
            A::Insert(text) => return self.function_view_text(text, s),
            A::Back | A::On => out.push(Request::Close),
            _ => return None,
        }
        Some(out)
    }
    pub(super) fn show_function_view(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        s: &SessionSnapshot,
        out: &mut Vec<Request>,
    ) {
        let settings = settings(s);
        if settings.format < 2 && self.function_field() == Field::Digits {
            self.function_select(Field::Format);
        }
        let focus = self.function_field();
        let mut g = Grob::new(320, 240, PAPER);
        chrome::header(&mut g, "Function Symbolic Setup");
        for (label, field) in [
            ("Angle Measure:", Field::Angle),
            ("Number Format:", Field::Format),
            ("Complex:", Field::Complex),
        ] {
            let [x, y, r, b] = field.rect();
            text(&mut g, label, 151 - measure(label, 14.), y + 3);
            g.rect(
                x,
                y,
                r,
                b,
                if focus == field { 0 } else { 0xe1e1e1 },
                0xffffff,
            );
            let value = field
                .value(&settings)
                .clamp(0, field.choices().len() as i64 - 1) as usize;
            text(&mut g, field.choices()[value], x + 2, y + 3);
            g.fill_rect(r - 15, y + 1, r - 1, b - 1, 0xc5dcfc);
            for row in 0..4 {
                g.line(
                    r - 11 + row,
                    y + 8 + row,
                    r - 5 - row,
                    y + 8 + row,
                    0x7f7f7f,
                );
            }
        }
        if settings.format >= 2 {
            let [x, y, r, b] = Field::Digits.rect();
            g.rect(
                x,
                y,
                r,
                b,
                if focus == Field::Digits { 0 } else { 0xe1e1e1 },
                0xffffff,
            );
            text(&mut g, &settings.digits.to_string(), x + 2, y + 3);
            g.fill_rect(r - 15, y + 1, r - 1, b - 1, 0xc5dcfc);
            for row in 0..4 {
                g.line(
                    r - 11 + row,
                    y + 8 + row,
                    r - 5 - row,
                    y + 8 + row,
                    0x7f7f7f,
                );
            }
        }
        draw_text(
            &mut g,
            self.error.as_deref().unwrap_or(focus.help()),
            0,
            203,
            14.,
            if self.error.is_some() { 0xc00000 } else { 0 },
            Some(318),
            None,
        );
        chrome::softkeys(&mut g, &["", "Choose", "", "", "", ""]);
        if let Some(index) = self.function_panel.menu {
            let (x, top, width, first, count) = self.function_menu_geometry(index);
            let items = focus.choices();
            g.rect(x, top, x + width, top + count as i64 * 21, 0, 0xffffff);
            for (i, label) in items.iter().enumerate().skip(first).take(count) {
                let py = top + (i - first) as i64 * 21;
                if i == index {
                    g.fill_rect(x + 1, py + 1, x + width - 1, py + 20, SELECTED);
                }
                if i == focus.value(&settings) as usize {
                    text(&mut g, "✓", x + 2, py + 2);
                }
                text(&mut g, label, x + 14, py + 2);
            }
            if items.len() > count {
                let height = count as i64 * 21 - 2;
                let thumb = height * count as i64 / items.len() as i64;
                let y = top + 1 + (height - thumb) * first as i64 / (items.len() - count) as i64;
                g.line(
                    x + width - 2,
                    top + 1,
                    x + width - 2,
                    top + height,
                    0xe1e1e1,
                );
                g.line(x + width - 2, y, x + width - 2, y + thumb, 0x888888);
            }
        }
        if self.function_panel.pixels != g.pixels() || self.function_panel.texture.is_none() {
            self.function_panel.pixels = g.pixels().to_vec();
            let image = egui::ColorImage::new(
                [320, 240],
                g.pixels()
                    .iter()
                    .map(|c| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, *c as u8))
                    .collect(),
            );
            if let Some(t) = &mut self.function_panel.texture {
                t.set(image, egui::TextureOptions::NEAREST);
            } else {
                self.function_panel.texture = Some(ctx.load_texture(
                    "native_function_view",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        egui::Area::new("native_function_view".into())
            .fixed_pos(lcd.min)
            .fade_in(false)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.function_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let response =
                    ui.interact(lcd, ui.id().with("function_settings"), egui::Sense::click());
                if response.clicked()
                    && let Some(p) = response.interact_pointer_pos()
                {
                    let p = (p - lcd.min) / (lcd.width() / 320.);
                    let (x, y) = (p.x as i64, p.y as i64);
                    if let Some(index) = self.function_panel.menu {
                        let (left, top, width, first, count) = self.function_menu_geometry(index);
                        if x >= left && x <= left + width && y >= top && y < top + count as i64 * 21
                        {
                            self.function_choose(s, first + ((y - top) / 21) as usize, out);
                        } else {
                            self.function_panel.menu = None;
                        }
                    } else if y >= 220 {
                        let key = chrome::softkey_index(x);
                        if key == 1 {
                            self.function_activate(s, out);
                        }
                    } else {
                        for field in [Field::Angle, Field::Format, Field::Digits, Field::Complex] {
                            if field == Field::Digits && settings.format < 2 {
                                continue;
                            }
                            let [l, t, r, b] = field.rect();
                            if (l..=r).contains(&x) && (t..=b).contains(&y) {
                                let active = field == focus;
                                self.function_select(field);
                                if active || x >= r - 16 {
                                    self.function_activate(s, out);
                                }
                                break;
                            }
                        }
                    }
                    ctx.request_repaint();
                }
            });
    }
}
