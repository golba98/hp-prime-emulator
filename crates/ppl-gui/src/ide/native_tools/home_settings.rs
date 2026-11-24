//! Home controls use the LCD's pixel grid for painting and hit testing.
//! Measured against original G1 Home Settings. Only working Native settings
//! are offered; translation, clock/theme and exam controls remain unavailable.
use super::{NativeTools, Page, Request};
use crate::ide::native::{
    chrome, expression::EditCommand, keys::KeyAction, presentation::EntryMode,
};
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use ppl_vm::{AngleMode, Settings};

const PAPER: u32 = 0xe8f1f6;
const FORMATS: [&str; 6] = [
    "Standard",
    "Fixed",
    "Scientific",
    "Engineering",
    "Floating",
    "Rounded",
];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Field {
    #[default]
    Angle,
    Format,
    Digits,
    Grouping,
    Entry,
    Base,
    Bits,
    Signed,
    Complex,
    Allow,
    Textbook,
    Font,
}
impl Field {
    fn rect(self) -> [i64; 4] {
        match self {
            Self::Angle => [140, 24, 313, 44],
            Self::Format => [140, 50, 271, 70],
            Self::Digits => [277, 50, 313, 70],
            Self::Grouping => [140, 76, 313, 96],
            Self::Entry => [140, 102, 313, 122],
            Self::Base => [140, 128, 235, 148],
            Self::Bits => [239, 128, 279, 148],
            Self::Signed => [298, 131, 313, 146],
            Self::Complex => [140, 154, 235, 174],
            Self::Allow => [241, 157, 256, 172],
            Self::Textbook => [140, 78, 155, 93],
            Self::Font => [140, 24, 313, 44],
        }
    }
    fn help(self) -> &'static str {
        match self {
            Self::Angle => "Choose angle measure",
            Self::Format => "Choose format for numbers",
            Self::Digits => "Choose number of digits (0–11)",
            Self::Grouping => "Choose digit grouping",
            Self::Entry => "Choose entry mode",
            Self::Base => "Choose integer base",
            Self::Bits => "Word size: unsigned1–64, signed1–63",
            Self::Signed => "Use signed integers",
            Self::Complex => "Choose complex number format",
            Self::Allow => "Allow complex results",
            Self::Textbook => "Display expressions in textbook form",
            Self::Font => "Choose the main font size",
        }
    }
    fn check(self) -> bool {
        matches!(self, Self::Signed | Self::Allow | Self::Textbook)
    }
    fn number(self) -> bool {
        matches!(self, Self::Bits | Self::Digits)
    }
}
#[derive(Default)]
pub(super) struct Panel {
    page: usize,
    focus: Field,
    menu: Option<(Field, usize)>,
    draft: Option<String>,
    error: Option<String>,
    texture: Option<egui::TextureHandle>,
    pixels: Vec<u32>,
}
fn grouping(code: i64) -> String {
    let mut s = Settings::default();
    s.set_separator(code).expect("bounded separator");
    ppl_math::Real::parse("123456.789")
        .unwrap()
        .format(s.home_format())
}
impl Panel {
    fn fields(&self, s: &Settings) -> Vec<Field> {
        if self.page == 1 {
            return vec![Field::Font, Field::Textbook];
        }
        let mut fields = vec![Field::Angle, Field::Format];
        if s.hformat != 0 {
            fields.push(Field::Digits);
        }
        fields.extend([
            Field::Grouping,
            Field::Entry,
            Field::Base,
            Field::Bits,
            Field::Signed,
            Field::Complex,
            Field::Allow,
        ]);
        fields
    }
    fn choices(field: Field) -> Vec<String> {
        match field {
            Field::Angle => ["Radians", "Degrees", "Gradians"]
                .map(str::to_owned)
                .to_vec(),
            Field::Format => FORMATS.map(str::to_owned).to_vec(),
            Field::Grouping => (0..=10).map(grouping).collect(),
            Field::Entry => ["Textbook", "Algebraic"].map(str::to_owned).to_vec(),
            Field::Base => ["Binary", "Octal", "Decimal", "Hex"]
                .map(str::to_owned)
                .to_vec(),
            Field::Complex => ["a+b*i", "(a,b)"].map(str::to_owned).to_vec(),
            Field::Font => crate::ide::native::presentation::FontSize::ALL
                .map(|font| font.label().into())
                .to_vec(),
            _ => vec![],
        }
    }
    fn selected(
        field: Field,
        s: &Settings,
        p: &crate::ide::native::presentation::PresentationSettings,
    ) -> usize {
        match field {
            Field::Angle => match s.angle {
                AngleMode::Radians => 0,
                AngleMode::Degrees => 1,
                AngleMode::Gradians => 2,
            },
            Field::Format => s.hformat as usize,
            Field::Grouping => s.digit_grouping.code() as usize,
            Field::Entry => usize::from(p.entry == EntryMode::Algebraic),
            Field::Base => match s.int_base {
                ppl_math::IntBase::Bin => 0,
                ppl_math::IntBase::Oct => 1,
                ppl_math::IntBase::Dec => 2,
                ppl_math::IntBase::Hex => 3,
            },
            Field::Complex => s.complex_display.code() as usize,
            Field::Font => crate::ide::native::presentation::FontSize::ALL
                .iter()
                .position(|font| *font == p.font_size)
                .unwrap(),
            _ => 0,
        }
    }
    fn menu_rect(field: Field, len: usize) -> [i64; 4] {
        let [x, y, right, _] = field.rect();
        let height = (len.min(9) * 21) as i64;
        [
            x - 1,
            y.min(219 - height),
            right + 2,
            y.min(219 - height) + height + 2,
        ]
    }
    fn commit_number(&mut self, s: &Settings, out: &mut Vec<Request>) -> bool {
        let Some(draft) = &self.draft else {
            return true;
        };
        let n = draft.parse::<i64>().ok();
        let valid = n.is_some_and(|n| {
            if self.focus == Field::Bits {
                (1..=64).contains(&n)
            } else {
                (0..=11).contains(&n)
            }
        });
        if !valid {
            self.error = Some(self.focus.help().into());
            return false;
        }
        let mut next = s.clone();
        let n = n.unwrap();
        if self.focus == Field::Bits {
            if let Err(error) = next.set_integer_bits(n) {
                self.error = Some(error.to_string());
                return false;
            }
        } else {
            next.set_format(next.hformat, n);
        }
        self.draft = None;
        self.error = None;
        out.push(Request::SetHomeSettings(next));
        true
    }
    fn select(
        &mut self,
        field: Field,
        index: usize,
        s: &Settings,
        p: &mut crate::ide::native::presentation::PresentationSettings,
        out: &mut Vec<Request>,
    ) {
        let mut next = s.clone();
        match field {
            Field::Angle => {
                next.angle = [AngleMode::Radians, AngleMode::Degrees, AngleMode::Gradians][index]
            }
            Field::Format => next.set_format(index as i64, next.hdigits),
            Field::Grouping => next.set_separator(index as i64).expect("bounded menu"),
            Field::Entry => {
                p.entry = if index == 0 {
                    EntryMode::Textbook
                } else {
                    EntryMode::Algebraic
                };
                return;
            }
            Field::Base => {
                next.int_base = [
                    ppl_math::IntBase::Bin,
                    ppl_math::IntBase::Oct,
                    ppl_math::IntBase::Dec,
                    ppl_math::IntBase::Hex,
                ][index]
            }
            Field::Complex => {
                next.complex_display = ppl_math::ComplexFormat::from_code(index as i64).unwrap()
            }
            Field::Font => {
                p.font_size = crate::ide::native::presentation::FontSize::ALL[index];
                next.font_px = p.font_size.pixels();
            }
            _ => return,
        }
        out.push(Request::SetHomeSettings(next));
    }
    fn activate(
        &mut self,
        s: &Settings,
        p: &mut crate::ide::native::presentation::PresentationSettings,
        out: &mut Vec<Request>,
    ) {
        if self.focus.check() {
            let mut next = s.clone();
            match self.focus {
                Field::Signed => {
                    if let Err(error) = next.set_integer_signed(!next.int_signed) {
                        self.error = Some(error.to_string());
                        return;
                    }
                }
                Field::Allow => next.complex_results = !next.complex_results,
                Field::Textbook => {
                    p.textbook_display = !p.textbook_display;
                    return;
                }
                _ => unreachable!(),
            }
            out.push(Request::SetHomeSettings(next));
        } else if self.focus.number() {
            self.commit_number(s, out);
        } else {
            self.menu = Some((self.focus, Self::selected(self.focus, s, p)));
        }
    }
    fn change_page(&mut self, page: usize, s: &Settings, out: &mut Vec<Request>) {
        if !self.commit_number(s, out) {
            return;
        }
        self.page = page.min(1);
        self.menu = None;
        self.focus = if self.page == 0 {
            Field::Angle
        } else {
            Field::Font
        };
    }
}
impl NativeTools {
    pub fn home_settings_snapshot(&self, s: &Settings) -> serde_json::Value {
        serde_json::json!({"open":self.page==Some(Page::Settings),"page":self.home_settings.page+1,
            "focus":format!("{:?}",self.home_settings.focus),"menu":self.home_settings.menu.map(|(field,index)|serde_json::json!({"field":format!("{field:?}"),"selected":index,"choices":Panel::choices(field)})),
            "draft":self.home_settings.draft,"error":self.home_settings.error,
            "angle":format!("{:?}",s.angle),"format":s.hformat,"digits":s.hdigits,"separator":s.digit_grouping.code(),
            "complex":s.complex_code(),"integer_bits":s.int_bits,"integer_base":format!("{:?}",s.int_base),"integer_signed":s.int_signed,"font_pixels":s.font_px})
    }
    pub fn home_settings_text(&mut self, text: &str) -> Option<Vec<Request>> {
        if self.page != Some(Page::Settings) {
            return None;
        }
        if self.home_settings.menu.is_none()
            && self.home_settings.focus.number()
            && text.bytes().all(|c| c.is_ascii_digit())
        {
            let draft = self.home_settings.draft.get_or_insert_default();
            if draft.len() + text.len() <= 3 {
                draft.push_str(text);
            }
            self.home_settings.error = None;
            self.dirty();
        }
        Some(vec![])
    }
    pub fn home_settings_action(
        &mut self,
        action: KeyAction,
        s: &Settings,
    ) -> Option<Vec<Request>> {
        if self.page != Some(Page::Settings) {
            return None;
        }
        let panel = &mut self.home_settings;
        let mut out = vec![];
        if let Some((field, index)) = panel.menu {
            let count = Panel::choices(field).len();
            match action {
                KeyAction::History(n) => {
                    panel.menu = Some((
                        field,
                        (index as i64 + if n < 0 { -1 } else { 1 }).rem_euclid(count as i64)
                            as usize,
                    ))
                }
                KeyAction::Evaluate => {
                    panel.select(field, index, s, &mut self.memory.presentation, &mut out);
                    panel.menu = None;
                }
                KeyAction::Back | KeyAction::On => panel.menu = None,
                KeyAction::Home | KeyAction::Tool(_) => {
                    panel.menu = None;
                    return None;
                }
                _ => {}
            }
        } else {
            match action {
                KeyAction::History(n) => {
                    if panel.commit_number(s, &mut out) {
                        let fields = panel.fields(s);
                        let index = fields.iter().position(|f| *f == panel.focus).unwrap_or(0);
                        panel.focus = fields[(index as i64 + if n < 0 { -1 } else { 1 })
                            .rem_euclid(fields.len() as i64)
                            as usize];
                    }
                }
                KeyAction::Edit(EditCommand::Left(_)) => panel.change_page(0, s, &mut out),
                KeyAction::Edit(EditCommand::Right(_)) => panel.change_page(1, s, &mut out),
                KeyAction::Evaluate => panel.activate(s, &mut self.memory.presentation, &mut out),
                KeyAction::Back | KeyAction::On => {
                    if panel.draft.take().is_some() {
                        panel.error = None;
                    } else {
                        out.push(Request::Close);
                    }
                }
                KeyAction::Edit(EditCommand::Backspace) => {
                    if let Some(draft) = &mut panel.draft {
                        draft.pop();
                    }
                }
                KeyAction::Insert(text) => return self.home_settings_text(text),
                KeyAction::Home | KeyAction::Tool(_) => return None,
                _ => {}
            }
        }
        self.dirty();
        Some(out)
    }
    pub(super) fn show_home_settings(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        s: &Settings,
        out: &mut Vec<Request>,
    ) {
        let panel = &mut self.home_settings;
        let p = &mut self.memory.presentation;
        let mut screen = Grob::new(320, 240, PAPER);
        chrome::header(&mut screen, "Home Settings");
        let text = |g: &mut Grob, t: &str, x, y| draw_text(g, t, x, y, 14., 0, None, None);
        let fields = panel.fields(s);
        for (label, y) in if panel.page == 0 {
            vec![
                ("Angle Measure:", 26),
                ("Number Format:", 52),
                ("Digit Grouping:", 78),
                ("Entry:", 104),
                ("Integers:", 130),
                ("Complex:", 156),
            ]
        } else {
            vec![("Font Size:", 26), ("Textbook Display:", 78)]
        } {
            text(&mut screen, label, 137 - measure(label, 14.), y);
        }
        for &field in &fields {
            let [x, y, right, bottom] = field.rect();
            screen.fill_rect(x, y, right, bottom, 0xdcdcdc);
            screen.fill_rect(x + 1, y + 1, right - 1, bottom - 1, 0xffffff);
            if field.check() {
                let checked = match field {
                    Field::Signed => s.int_signed,
                    Field::Allow => s.complex_results,
                    Field::Textbook => p.textbook_display,
                    _ => false,
                };
                if checked {
                    text(&mut screen, "✓", x + 1, y - 1);
                }
            } else {
                let label = if field.number() {
                    panel
                        .draft
                        .as_ref()
                        .filter(|_| panel.focus == field)
                        .cloned()
                        .unwrap_or_else(|| {
                            if field == Field::Bits {
                                s.int_bits.to_string()
                            } else {
                                s.hdigits.to_string()
                            }
                        })
                } else {
                    Panel::choices(field)[Panel::selected(field, s, p)].clone()
                };
                text(&mut screen, &label, x + 2, y + 1);
                if !field.number() {
                    screen.fill_rect(right - 14, y + 1, right - 1, bottom - 1, 0xc3ddfc);
                    for row in 0..5 {
                        screen.line(
                            right - 10 + row / 2,
                            y + 8 + row,
                            right - 4 - row / 2,
                            y + 8 + row,
                            0x8b9ba5,
                        );
                    }
                }
            }
            if panel.focus == field {
                outline(&mut screen, [x - 1, y - 1, right + 1, bottom + 1]);
            }
        }
        if panel.page == 0 {
            text(&mut screen, "±:", 282, 130);
        }
        text(
            &mut screen,
            panel
                .error
                .as_deref()
                .or(self.error.as_deref())
                .unwrap_or(panel.focus.help()),
            0,
            202,
        );
        chrome::softkeys(&mut screen, &["", "", "", "", "", ""]);
        // The two middle buttons form a single page control on the G1.
        for y in 221..239 {
            let c = screen.get(130, y).unwrap_or(0);
            screen.line(158, y, 160, y, c);
        }
        draw_text(
            &mut screen,
            &format!("Page {}/2", panel.page + 1),
            133,
            222,
            12.,
            0xffffff,
            None,
            None,
        );
        for row in 0..7 {
            let d = 3 - (row - 3i64).abs();
            screen.line(
                111 + 3 - d,
                226 + row,
                114,
                226 + row,
                if panel.page == 0 { 0x888888 } else { 0xffffff },
            );
            screen.line(
                204,
                226 + row,
                204 + d,
                226 + row,
                if panel.page == 1 { 0x888888 } else { 0xffffff },
            );
        }
        if let Some((field, index)) = panel.menu {
            let choices = Panel::choices(field);
            let [x, y, right, bottom] = Panel::menu_rect(field, choices.len());
            screen.fill_rect(x, y, right, bottom, 0xffffff);
            outline(&mut screen, [x, y, right, bottom]);
            let start = index.saturating_sub(8);
            for (row, (i, label)) in choices.iter().enumerate().skip(start).take(9).enumerate() {
                let top = y + 1 + row as i64 * 21;
                if i == index {
                    screen.fill_rect(x + 1, top, right - 1, top + 20, 0xadccff);
                }
                if i == Panel::selected(field, s, p) {
                    text(&mut screen, "✓", x + 2, top + 1);
                }
                text(&mut screen, label, x + 14, top + 1);
                if top + 21 < bottom {
                    screen.line(x + 1, top + 21, right - 1, top + 21, 0xe0e0e0);
                }
            }
        }
        let pixels_changed = panel.pixels != screen.pixels();
        if pixels_changed {
            panel.pixels = screen.pixels().to_vec();
            let colors = panel
                .pixels
                .iter()
                .map(|&c| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8))
                .collect();
            let image = egui::ColorImage::new([320, 240], colors);
            if let Some(texture) = &mut panel.texture {
                texture.set(image, egui::TextureOptions::NEAREST);
            } else {
                panel.texture = Some(ctx.load_texture(
                    "native_home_settings",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
        egui::Area::new("native_home_settings_panel".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                if let Some(texture) = &panel.texture {
                    ui.painter().image(
                        texture.id(),
                        lcd,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        egui::Color32::WHITE,
                    );
                }
                let response = ui.interact(lcd, ui.id().with("surface"), egui::Sense::click());
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let point = (pos - lcd.min) / scale;
                    let (x, y) = (point.x as i64, point.y as i64);
                    if let Some((field, index)) = panel.menu {
                        let choices = Panel::choices(field);
                        let [left, top, right, bottom] = Panel::menu_rect(field, choices.len());
                        if x > left && x < right && y > top && y < bottom {
                            let i = index.saturating_sub(8) + ((y - top - 1) / 21) as usize;
                            if i < choices.len() {
                                panel.select(field, i, s, p, out);
                            }
                        }
                        panel.menu = None;
                    } else if y >= 220 && (106..=210).contains(&x) {
                        panel.change_page(usize::from(x >= 159), s, out);
                    } else if let Some(&field) = fields.iter().find(|field| {
                        let [l, t, r, b] = field.rect();
                        x >= l && x <= r && y >= t && y <= b
                    }) && panel.commit_number(s, out)
                    {
                        panel.focus = field;
                        panel.error = None;
                        if field.check() || (!field.number() && x >= field.rect()[2] - 14) {
                            let current = out
                                .iter()
                                .rev()
                                .find_map(|request| {
                                    if let Request::SetHomeSettings(s) = request {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| s.clone());
                            panel.activate(&current, p, out);
                        }
                    }
                    ctx.request_repaint();
                }
            });
        if pixels_changed {
            self.dirty();
        }
    }
}
fn outline(g: &mut Grob, [x, y, right, bottom]: [i64; 4]) {
    g.line(x, y, right, y, 0);
    g.line(x, bottom, right, bottom, 0);
    g.line(x, y, x, bottom, 0);
    g.line(right, y, right, bottom, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_64_is_refused_before_dispatch_and_signed_63_remains_editable() {
        let settings = Settings {
            int_bits: 64,
            ..Default::default()
        };
        let mut panel = Panel {
            focus: Field::Signed,
            ..Default::default()
        };
        let mut p = Default::default();
        let mut out = Vec::new();
        panel.activate(&settings, &mut p, &mut out);
        assert!(out.is_empty());
        assert!(panel.error.is_some());
        let settings = Settings {
            int_bits: 63,
            int_signed: true,
            ..Default::default()
        };
        panel.focus = Field::Bits;
        panel.draft = Some("64".into());
        assert!(!panel.commit_number(&settings, &mut out));
        assert!(out.is_empty());
        assert_eq!(panel.draft.as_deref(), Some("64"));
        panel.draft = Some("63".into());
        assert!(panel.commit_number(&settings, &mut out));
        assert!(matches!(&out[0],Request::SetHomeSettings(s)if s.int_bits==63&&s.int_signed));
    }
}
