//! Native Help consumes the user's installed book. No calculator text is bundled.
use super::{NativeTools, Page, Request};
use crate::ide::native::{expression::EditCommand, keys::KeyAction};
use egui::{Color32, emath::GuiRounding as _};
use ppl_engine::help::{Book, Language, Topic};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, mpsc},
    time::Duration,
};

const HISTORY_LIMIT: usize = 64;
const RESULT_PAGE: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    language: u8,
    id: Option<u32>,
    name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Memory {
    language: u8,
    current: Option<Visit>,
    back: Vec<Visit>,
    forward: Vec<Visit>,
    contents: bool,
}
impl Default for Memory {
    fn default() -> Self {
        Self {
            language: 1,
            current: None,
            back: Vec::new(),
            forward: Vec::new(),
            contents: true,
        }
    }
}
fn language(code: u8) -> Option<Language> {
    Language::ALL
        .into_iter()
        .find(|language| *language as u8 == code)
}
fn language_name(language: Language) -> &'static str {
    match language {
        Language::English => "English",
        Language::Chinese => "中文",
        Language::French => "Français",
        Language::German => "Deutsch",
        Language::Spanish => "Español",
        Language::Dutch => "Nederlands",
        Language::Portuguese => "Português",
        Language::Japanese => "日本語",
    }
}
impl Memory {
    pub(super) fn validate(&self) -> Result<(), String> {
        if language(self.language).is_none()
            || self.back.len() > HISTORY_LIMIT
            || self.forward.len() > HISTORY_LIMIT
            || self
                .current
                .iter()
                .chain(&self.back)
                .chain(&self.forward)
                .any(|visit| {
                    language(visit.language).is_none()
                        || visit.name.len() > 4096
                        || visit.id.is_some_and(|id| id == 0 || id > 10000)
                })
        {
            return Err("Invalid saved Help navigation".into());
        }
        Ok(())
    }
    fn visit(&mut self, visit: Visit) {
        if self.current.as_ref() != Some(&visit) {
            if let Some(previous) = self.current.replace(visit.clone()) {
                self.back.push(previous);
                if self.back.len() > HISTORY_LIMIT {
                    self.back.remove(0);
                }
            }
            self.forward.clear();
        }
        self.language = visit.language;
        self.contents = false;
    }
    fn back(&mut self) {
        if self.contents && self.current.is_some() {
            self.contents = false;
            return;
        }
        if let Some(previous) = self.back.pop() {
            if let Some(current) = self.current.replace(previous.clone()) {
                self.forward.push(current);
            }
            self.language = previous.language;
            self.contents = false;
        }
    }
    fn forward(&mut self) {
        if let Some(next) = self.forward.pop() {
            if let Some(current) = self.current.replace(next.clone()) {
                self.back.push(current);
            }
            self.language = next.language;
            self.contents = false;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    code: bool,
    math: bool,
    band: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    text: String,
    style: Style,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListLevel {
    Primary = 0,
    Secondary = 1,
}
impl ListLevel {
    fn from_tag(tag: char) -> Option<Self> {
        match tag {
            '+' => Some(Self::Primary),
            'o' => Some(Self::Secondary),
            _ => None,
        }
    }
    fn marker(self) -> &'static str {
        match self {
            Self::Primary => "•",
            Self::Secondary => "◦",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ListLine {
    level: ListLevel,
    marker: bool,
}
#[derive(Default)]
struct Lists {
    open: [bool; 2],
    line: Option<ListLine>,
}
impl Lists {
    fn item(&mut self, level: ListLevel) {
        self.open[level as usize..].fill(false);
        self.open[level as usize] = true;
        self.line = Some(ListLine {
            level,
            marker: true,
        });
    }
    fn close(&mut self, level: ListLevel) -> bool {
        if !self.open[level as usize] {
            return false;
        }
        self.open[level as usize..].fill(false);
        true
    }
    fn finish_line(&mut self, runs: &mut Vec<Run>, blocks: &mut Vec<Block>) {
        let runs = std::mem::take(runs);
        if let Some(line) = self.line
            && (!runs.is_empty() || line.marker)
        {
            blocks.push(Block::ListItem(line, runs));
        } else {
            blocks.push(Block::Paragraph(runs));
        }
        self.line = [ListLevel::Secondary, ListLevel::Primary]
            .into_iter()
            .find(|level| self.open[*level as usize])
            .map(|level| ListLine {
                level,
                marker: false,
            });
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Block {
    Paragraph(Vec<Run>),
    ListItem(ListLine, Vec<Run>),
    Example(usize, Style),
}
#[derive(Default)]
struct Document {
    blocks: Vec<Block>,
    examples: Vec<String>,
    unsupported: BTreeSet<char>,
}

/// Preserve unknown UTF16/private glyphs visibly rather than replacing them
/// silently with an unrelated glyph. The decoder's raw strings are unchanged.
fn visible_units(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied())
        .map(|unit| match unit {
            Ok(c) if ('\u{e000}'..='\u{f8ff}').contains(&c) => format!("[U+{:04X}]", c as u32),
            Ok(c) => c.to_string(),
            Err(error) => format!("[U+{:04X}]", error.unpaired_surrogate()),
        })
        .collect()
}
fn title(topic: &Topic) -> String {
    topic
        .spans
        .first()
        .map(|units| visible_units(units))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| topic.name.clone())
}
fn flush(text: &mut String, runs: &mut Vec<Run>, style: Style) {
    if !text.is_empty() {
        runs.push(Run {
            text: std::mem::take(text),
            style,
        });
    }
}
fn document(topic: &Topic) -> Document {
    let examples = topic
        .spans
        .get(2)
        .map(|units| String::from_utf16(units).unwrap_or_else(|_| visible_units(units)))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let mut out = Document {
        examples,
        ..Default::default()
    };
    let body = topic
        .spans
        .get(1)
        .map(|units| visible_units(units))
        .unwrap_or_default();
    let chars = body.chars().collect::<Vec<_>>();
    let (mut at, mut style, mut text, mut runs, mut stack) =
        (0, Style::default(), String::new(), Vec::new(), Vec::new());
    let mut used_examples = BTreeSet::new();
    let mut lists = Lists::default();
    while at < chars.len() {
        let c = chars[at];
        if c == '✎'
            && let Some(&tag) = chars.get(at + 1)
        {
            flush(&mut text, &mut runs, style);
            if let Some(level) = ListLevel::from_tag(tag) {
                lists.item(level);
            } else {
                stack.push((tag, style));
                match tag {
                    '*' => style.bold = true,
                    '_' => style.underline = true,
                    '#' | '/' => style.italic = true,
                    '^' => style.band = 1,
                    '%' => style.band = 2,
                    'P' | 'Y' | 'x' => style.code = true,
                    ';' | 'v' => style.math = true,
                    _ => {
                        out.unsupported.insert(tag);
                    }
                }
            }
            at += 2;
            continue;
        }
        if chars.get(at + 1) == Some(&'✐') {
            if let Some(level) = ListLevel::from_tag(c)
                && lists.close(level)
            {
                at += 2;
                continue;
            }
            if let Some(index) = stack.iter().rposition(|(tag, _)| *tag == c) {
                flush(&mut text, &mut runs, style);
                style = stack[index].1;
                stack.truncate(index);
                at += 2;
                continue;
            }
            // A malformed closing delimiter stays visible; do not discard text.
        }
        if c == '✍' {
            flush(&mut text, &mut runs, style);
            let end = chars[at + 1..]
                .iter()
                .position(|c| *c == '✍')
                .map(|i| i + at + 1);
            if let Some(end) = end {
                runs.push(Run {
                    text: chars[at + 1..end].iter().collect(),
                    style: Style {
                        code: true,
                        ..style
                    },
                });
                at = end + 1;
                continue;
            }
        }
        if c == '(' && chars[at..].starts_with(&['(', '-', '-', 'E']) {
            let rest: String = chars[at + 4..]
                .iter()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let end = at + 4 + rest.len();
            if !rest.is_empty()
                && chars.get(end..end + 3) == Some(&['-', '-', ')'])
                && let Ok(index) = rest.parse::<usize>()
                && index > 0
                && index <= out.examples.len()
            {
                flush(&mut text, &mut runs, style);
                if !runs.is_empty() || lists.line.is_some_and(|line| line.marker) {
                    lists.finish_line(&mut runs, &mut out.blocks);
                }
                out.blocks.push(Block::Example(index - 1, style));
                used_examples.insert(index - 1);
                at = end + 3;
                continue;
            }
        }
        if c == '\n' {
            flush(&mut text, &mut runs, style);
            lists.finish_line(&mut runs, &mut out.blocks);
        } else {
            text.push(c);
        }
        at += 1;
    }
    flush(&mut text, &mut runs, style);
    if !runs.is_empty() || lists.line.is_some_and(|line| line.marker) {
        lists.finish_line(&mut runs, &mut out.blocks);
    }
    let remaining = (0..out.examples.len())
        .filter(|index| !used_examples.contains(index))
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        out.blocks.push(Block::Paragraph(vec![Run {
            text: "Examples".into(),
            style: Style {
                bold: true,
                band: 2,
                ..Default::default()
            },
        }]));
        out.blocks.extend(remaining.into_iter().map(|index| {
            Block::Example(
                index,
                Style {
                    band: 2,
                    ..Default::default()
                },
            )
        }));
    }
    out
}

type Loaded = Result<Option<Arc<Book>>, String>;
#[derive(Default)]
pub(super) struct Panel {
    language: Option<Language>,
    book: Option<Arc<Book>>,
    loading: Option<mpsc::Receiver<Loaded>>,
    status: Option<String>,
    missing: bool,
    query: Option<String>,
    results: Vec<usize>,
    limit: usize,
    cursor: usize,
    document: Option<(u32, Document)>,
    textures: BTreeMap<String, egui::TextureHandle>,
    search_focused: bool,
    release_focus: bool,
    scroll: f32,
    scroll_to_result: bool,
    math_serial: usize,
    body_offset: f32,
}
impl Panel {
    pub(super) fn loading(&self) -> bool {
        self.loading.is_some()
    }
    fn load(&mut self, wanted: Language) {
        self.language = Some(wanted);
        self.book = None;
        self.document = None;
        self.textures.clear();
        self.status = None;
        self.missing = false;
        self.results.clear();
        self.query = None;
        self.limit = RESULT_PAGE;
        self.cursor = 0;
        let (tx, rx) = mpsc::channel();
        match std::thread::Builder::new()
            .name("native-help-book".into())
            .spawn(move || {
                let _ = tx
                    .send(ppl_engine::help::configured(wanted).map_err(|error| error.to_string()));
            }) {
            Ok(_) => self.loading = Some(rx),
            Err(error) => {
                self.loading = None;
                self.status = Some(format!("Could not start Help loader: {error}"));
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context, wanted: Language) -> bool {
        if self.language != Some(wanted) {
            self.load(wanted);
        }
        let result = self.loading.as_ref().map(mpsc::Receiver::try_recv);
        match result {
            Some(Ok(Ok(book))) => {
                self.missing = book.is_none();
                self.book = book;
                self.loading = None;
                self.query = None;
                true
            }
            Some(Ok(Err(error))) => {
                self.status = Some(error);
                self.loading = None;
                true
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.status = Some("Help loader stopped before returning a book".into());
                self.loading = None;
                true
            }
            Some(Err(mpsc::TryRecvError::Empty)) => {
                ctx.request_repaint_after(Duration::from_millis(16));
                false
            }
            None => false,
        }
    }
    fn search(&mut self, query: &str) {
        if self.query.as_deref() == Some(query) {
            return;
        }
        self.query = Some(query.into());
        self.cursor = 0;
        self.limit = RESULT_PAGE;
        let typed_query = query.to_owned();
        let query = query.to_lowercase();
        self.results = self
            .book
            .as_ref()
            .map(|book| {
                let mut indices = book
                    .topics
                    .iter()
                    .enumerate()
                    .filter(|(_, topic)| {
                        topic.id != 0
                            && (query.is_empty()
                                || topic.name.to_lowercase().contains(&query)
                                || title(topic).to_lowercase().contains(&query))
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                indices.sort_by_cached_key(|index| {
                    let topic = &book.topics[*index];
                    let folded = topic.name.to_lowercase();
                    let rank = if !query.is_empty() && topic.name == typed_query {
                        0
                    } else if !query.is_empty() && folded == query {
                        1
                    } else if !query.is_empty() && folded.starts_with(&query) {
                        2
                    } else {
                        3
                    };
                    (
                        rank,
                        if topic.name.is_empty() {
                            title(topic)
                        } else {
                            topic.name.clone()
                        }
                        .to_lowercase(),
                        topic.id,
                    )
                });
                indices
            })
            .unwrap_or_default();
    }
    fn select(&mut self, memory: &mut Memory, topic: &mut String, visit: Visit) {
        *topic = visit.name.clone();
        memory.visit(visit);
        self.document = None;
        self.textures.clear();
        self.release_focus = true;
    }
    fn synchronize_name(&mut self, memory: &mut Memory, topic: &mut String) {
        if !topic.is_empty()
            && memory
                .current
                .as_ref()
                .is_none_or(|visit| visit.name != *topic)
        {
            let id = self.book.as_ref().and_then(|book| {
                book.topics
                    .iter()
                    .find(|entry| entry.name == *topic)
                    .or_else(|| book.named(topic).next())
                    .map(|entry| entry.id)
            });
            self.select(
                memory,
                topic,
                Visit {
                    language: memory.language,
                    id,
                    name: topic.clone(),
                },
            );
        }
        if let Some(current) = &mut memory.current
            && current.id.is_none()
        {
            current.id = self.book.as_ref().and_then(|book| {
                book.topics
                    .iter()
                    .find(|entry| entry.name == current.name)
                    .or_else(|| book.named(&current.name).next())
                    .map(|entry| entry.id)
            });
        }
    }
    fn registry(ui: &mut egui::Ui, topic: &str, requests: &mut Vec<Request>) {
        if let Some(spec) = ppl_compiler::builtins::lookup_case_insensitive(topic) {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Insert command").clicked() {
                    requests.push(if spec.min == 0 {
                        Request::Insert(spec.name.into())
                    } else {
                        Request::InsertTemplate(
                            crate::ide::native::expression::TemplateKind::Function(spec.name),
                        )
                    });
                }
                ui.small(match spec.status {
                    ppl_compiler::builtins::Status::Implemented => {
                        "Native implementation available"
                    }
                    ppl_compiler::builtins::Status::Approximate => "Native compatibility limits",
                    ppl_compiler::builtins::Status::Unsupported => "Not implemented in Native",
                });
            });
            ui.small(format!(
                "Arguments: {} · {}",
                spec.arity_text(),
                spec.category.as_str()
            ));
        }
    }
    fn math(&mut self, ui: &mut egui::Ui, source: &str, scale: f32) {
        let serial = self.math_serial;
        self.math_serial += 1;
        let (_, diagnostics) = ppl_syntax::parse_expression(source);
        if source.len() > 8192 || diagnostics.has_errors() {
            ui.label(visible_units(&source.encode_utf16().collect::<Vec<_>>()));
            return;
        }
        let layout = crate::ide::native::math_layout::expression(source, 16.);
        if layout.width <= 0 || layout.width > 1024 || layout.height <= 0 || layout.height > 360 {
            ui.label(visible_units(&source.encode_utf16().collect::<Vec<_>>()));
            return;
        }
        let key = source.to_owned();
        let texture = self.textures.entry(key).or_insert_with(|| {
            let width = layout.width as usize + 4;
            let height = layout.height as usize + 4;
            let mut image = ppl_graphics::Grob::new(width, height, 0xffffff);
            layout.paint_static(&mut image, 2, 2, 0);
            let pixels = image
                .pixels()
                .iter()
                .map(|color| Color32::from_rgba_unmultiplied(0, 0, 0, 255 - (color & 255) as u8))
                .collect();
            ui.ctx().load_texture(
                "native-help-math",
                egui::ColorImage::new([width, height], pixels),
                egui::TextureOptions::NEAREST,
            )
        });
        let size = (texture.size_vec2() * scale).round_ui();
        egui::ScrollArea::horizontal()
            .id_salt(("help_math", serial))
            .show(ui, |ui| {
                ui.add(egui::Image::new((texture.id(), size)));
            });
    }
    fn paragraph(&mut self, ui: &mut egui::Ui, runs: &[Run], scale: f32) {
        if runs.is_empty() {
            ui.add_space(4. * scale);
            return;
        }
        if runs.len() == 1 && runs[0].style.math {
            self.math(ui, &runs[0].text, scale);
            return;
        }
        let mut job = egui::text::LayoutJob {
            wrap: egui::text::TextWrapping {
                max_width: ui.available_width(),
                ..Default::default()
            },
            ..Default::default()
        };
        for run in runs {
            let style = run.style;
            let role = if style.code {
                egui::TextStyle::Monospace
            } else if style.bold {
                egui::TextStyle::Heading
            } else {
                egui::TextStyle::Body
            };
            let font = egui::FontId::new(
                if style.code { 13. * scale } else { 14. * scale },
                ui.style().text_styles[&role].family.clone(),
            );
            job.append(
                &run.text,
                0.,
                egui::TextFormat {
                    font_id: font,
                    color: Color32::BLACK,
                    italics: style.italic,
                    underline: if style.underline {
                        egui::Stroke::new(scale, Color32::BLACK)
                    } else {
                        egui::Stroke::NONE
                    },
                    background: match style.band {
                        1 => Color32::from_rgb(255, 255, 128),
                        2 => Color32::from_rgb(180, 255, 180),
                        _ => Color32::TRANSPARENT,
                    },
                    ..Default::default()
                },
            );
        }
        ui.add(egui::Label::new(job).wrap());
    }
    fn list_item(&mut self, ui: &mut egui::Ui, line: ListLine, runs: &[Run], scale: f32) {
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.add_space(line.level as u8 as f32 * 12. * scale);
            let marker = if line.marker { line.level.marker() } else { "" };
            let marker = egui::RichText::new(marker)
                .size(14. * scale)
                .color(Color32::BLACK);
            ui.add_sized([11. * scale, 0.], egui::Label::new(marker));
            self.paragraph(ui, runs, scale);
        });
    }
    fn example(
        &mut self,
        ui: &mut egui::Ui,
        example: &str,
        scale: f32,
        requests: &mut Vec<Request>,
    ) {
        if let Some((source, result)) = example.split_once("---->") {
            self.math(ui, source.trim(), scale);
            ui.small("→");
            self.math(ui, result.trim(), scale);
        } else {
            ui.monospace(visible_units(&example.encode_utf16().collect::<Vec<_>>()));
        }
        if ui.small_button("Copy example").clicked() {
            requests.push(Request::CopyText(
                example
                    .split_once("---->")
                    .map_or(example, |(source, _)| source)
                    .trim()
                    .into(),
            ));
        }
    }
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        memory: &mut Memory,
        query: &mut String,
        topic: &mut String,
        viewport: (f32, f32),
        requests: &mut Vec<Request>,
    ) -> bool {
        let (scale, bottom) = viewport;
        self.math_serial = 0;
        ui.style_mut().spacing.item_spacing = ui.style().spacing.item_spacing.round_ui();
        let mut changed = self.poll(
            ui.ctx(),
            language(memory.language).unwrap_or(Language::English),
        );
        self.synchronize_name(memory, topic);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !memory.back.is_empty() || (memory.contents && memory.current.is_some()),
                    egui::Button::new("Back"),
                )
                .clicked()
            {
                memory.back();
                if let Some(visit) = &memory.current {
                    *topic = visit.name.clone();
                }
                self.document = None;
            }
            if ui
                .add_enabled(!memory.forward.is_empty(), egui::Button::new("Forward"))
                .clicked()
            {
                memory.forward();
                if let Some(visit) = &memory.current {
                    *topic = visit.name.clone();
                }
                self.document = None;
            }
            if ui.button("Contents").clicked() {
                memory.contents = true;
                query.clear();
            }
            egui::ComboBox::from_id_salt("native_help_language")
                .selected_text(language_name(
                    language(memory.language).unwrap_or(Language::English),
                ))
                .show_ui(ui, |ui| {
                    for item in Language::ALL {
                        if ui
                            .selectable_value(&mut memory.language, item as u8, language_name(item))
                            .changed()
                        {
                            if let Some(current) = &mut memory.current {
                                current.language = memory.language;
                            }
                            self.load(item);
                        }
                    }
                });
        });
        let response = ui.add(
            egui::TextEdit::singleline(query)
                .id(egui::Id::new("native_help_search"))
                .hint_text("Find Help topic")
                .char_limit(1024),
        );
        self.search_focused = response.has_focus();
        if std::mem::take(&mut self.release_focus) {
            response.surrender_focus();
            self.search_focused = false;
        }
        if response.changed() {
            memory.contents = true;
            self.query = None;
        }
        self.search(query);
        // Keep actions stable while an asynchronous resource load completes.
        // Moving them between mouse press and release loses a real click.
        if !memory.contents {
            Self::registry(ui, topic, requests);
        }
        if self.loading.is_some() {
            ui.small("Loading installed Help…");
        }
        if let Some(error) = &self.status {
            ui.colored_label(
                Color32::DARK_RED,
                format!("Installed Help unavailable: {error}"),
            );
        }
        if self.missing || self.status.is_some() {
            ui.small("Native command reference · installed Help book unavailable");
            if ui.small_button("Retry installed Help").clicked() {
                self.load(language(memory.language).unwrap_or(Language::English));
            }
        }
        let height = (bottom - ui.cursor().top() - 3. * scale)
            .max(40. * scale)
            .round_ui();
        let scroll = std::mem::take(&mut self.scroll);
        let body = egui::ScrollArea::vertical().id_salt(("native_help_body", memory.current.as_ref().and_then(|v| v.id), memory.contents)).max_height(height).show(ui, |ui| {
            if memory.contents {
                if let Some(book) = self.book.clone() {
                    ui.small(format!("{} installed topics", self.results.len()));
                    let choices = self.results.iter().copied().take(self.limit).collect::<Vec<_>>();
                    for (index, position) in choices.into_iter().enumerate() {
                        let entry = &book.topics[position];
                        let title = title(entry); let label = if entry.name.is_empty() || entry.name == title { title }
                            else { format!("{} — {title}", entry.name) };
                        let response = ui.selectable_label(index == self.cursor, label);
                        if index == self.cursor && self.scroll_to_result { response.scroll_to_me(Some(egui::Align::Center)); }
                        if response.clicked() {
                            self.cursor = index;
                            self.select(memory, topic, Visit { language: memory.language, id: Some(entry.id), name: entry.name.clone() });
                        }
                    }
                    if self.results.len() > self.limit && ui.button("More topics").clicked() { self.limit += RESULT_PAGE; }
                    if self.results.is_empty() { ui.label("No matching installed topic"); }
                } else if self.loading.is_none() {
                    let query = query.to_ascii_lowercase();
                    for spec in ppl_compiler::builtins::BUILTINS.iter().filter(|spec| query.is_empty() || spec.name.to_ascii_lowercase().contains(&query)).take(100) {
                        if ui.selectable_label(*topic == spec.name, spec.name).clicked() {
                            self.select(memory, topic, Visit { language: memory.language, id: None, name: spec.name.into() });
                        }
                    }
                }
            } else if let Some(book) = self.book.clone() {
                if let Some(entry) = memory.current.as_ref().and_then(|visit| visit.id).and_then(|id| book.topic(id)) {
                    ui.strong(title(entry));
                    if self.document.as_ref().is_none_or(|(id, _)| *id != entry.id) { self.document = Some((entry.id, document(entry))); self.textures.clear(); }
                    let (_, doc) = self.document.take().unwrap();
                    for block in &doc.blocks { match block {
                        Block::Paragraph(runs) => self.paragraph(ui, runs, scale),
                        Block::ListItem(line, runs) => self.list_item(ui, *line, runs, scale),
                        Block::Example(index, style) => {
                            egui::Frame::new().fill(if style.band == 2 { Color32::from_rgb(180, 255, 180) } else { Color32::TRANSPARENT }).show(ui, |ui| {
                                self.example(ui, &doc.examples[*index], scale, requests);
                            });
                        },
                    }}
                    if !doc.unsupported.is_empty() { ui.small("Some original styles are shown as plain text."); }
                    self.document = Some((entry.id, doc));
                } else { ui.label("No installed topic matches this selection. Search Contents for another topic."); }
            } else if self.loading.is_none() && ppl_compiler::builtins::lookup_case_insensitive(topic).is_none() {
                ui.label("Choose a Native command from Contents.");
            }
            // Nested horizontal math views must finish before the outer body
            // receives its keyboard scroll request.
            if scroll != 0. { ui.scroll_with_delta(egui::vec2(0., -scroll * scale)); }
        });
        self.body_offset = body.state.offset.y;
        self.scroll_to_result = false;
        changed |= response.changed();
        changed
    }
}
impl NativeTools {
    pub(super) fn show_help(
        &mut self,
        ui: &mut egui::Ui,
        lcd: egui::Rect,
        scale: f32,
        requests: &mut Vec<Request>,
    ) {
        let before = (
            self.memory.help.clone(),
            self.memory.help_query.clone(),
            self.memory.help_topic.clone(),
        );
        let changed = self.help_panel.show(
            ui,
            &mut self.memory.help,
            &mut self.memory.help_query,
            &mut self.memory.help_topic,
            (scale, lcd.bottom()),
            requests,
        );
        if changed
            || before
                != (
                    self.memory.help.clone(),
                    self.memory.help_query.clone(),
                    self.memory.help_topic.clone(),
                )
        {
            self.dirty();
        }
    }
    pub fn help_snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "loading": self.help_panel.loading(), "installed": self.help_panel.book.is_some(),
            "topic_count": self.help_panel.book.as_ref().map_or(0, |book| book.topics.len()),
            "topic_id": self.help_panel.document.as_ref().map(|(id, _)| id),
            "scroll_y": self.help_panel.body_offset, "rendered_math": self.help_panel.textures.len(),
            "error": self.help_panel.status, "missing": self.help_panel.missing,
        })
    }
    pub fn help_action(&mut self, action: KeyAction) -> Option<Vec<Request>> {
        if self.page != Some(Page::Help) || matches!(action, KeyAction::Home | KeyAction::Tool(_)) {
            return None;
        }
        match action {
            KeyAction::Back if self.memory.help.back.is_empty() => {
                return Some(vec![Request::Close]);
            }
            KeyAction::Back | KeyAction::Edit(EditCommand::Left(false))
                if !self.help_panel.search_focused =>
            {
                self.memory.help.back();
                if let Some(visit) = &self.memory.help.current {
                    self.memory.help_topic = visit.name.clone();
                }
                self.help_panel.document = None;
            }
            KeyAction::Edit(EditCommand::Right(false)) if !self.help_panel.search_focused => {
                self.memory.help.forward();
                if let Some(visit) = &self.memory.help.current {
                    self.memory.help_topic = visit.name.clone();
                }
                self.help_panel.document = None;
            }
            KeyAction::History(delta) if self.memory.help.contents => {
                let length = if self.help_panel.book.is_some() {
                    self.help_panel.results.len()
                } else {
                    ppl_compiler::builtins::BUILTINS
                        .iter()
                        .filter(|spec| {
                            self.memory.help_query.is_empty()
                                || spec
                                    .name
                                    .to_ascii_lowercase()
                                    .contains(&self.memory.help_query.to_ascii_lowercase())
                        })
                        .count()
                };
                self.help_panel.cursor = (self.help_panel.cursor as i64 + i64::from(delta))
                    .clamp(0, length.saturating_sub(1) as i64)
                    as usize;
                self.help_panel.scroll_to_result = true;
                self.help_panel.limit = self.help_panel.limit.max(self.help_panel.cursor + 1);
            }
            KeyAction::History(delta) => {
                self.help_panel.scroll += delta as f32 * 32.;
            }
            KeyAction::Evaluate if self.memory.help.contents => {
                let visit = if let Some(book) = &self.help_panel.book {
                    self.help_panel
                        .results
                        .get(self.help_panel.cursor)
                        .map(|index| &book.topics[*index])
                        .map(|entry| Visit {
                            language: self.memory.help.language,
                            id: Some(entry.id),
                            name: entry.name.clone(),
                        })
                } else {
                    ppl_compiler::builtins::BUILTINS
                        .iter()
                        .filter(|spec| {
                            self.memory.help_query.is_empty()
                                || spec
                                    .name
                                    .to_ascii_lowercase()
                                    .contains(&self.memory.help_query.to_ascii_lowercase())
                        })
                        .nth(self.help_panel.cursor)
                        .map(|spec| Visit {
                            language: self.memory.help.language,
                            id: None,
                            name: spec.name.into(),
                        })
                };
                if let Some(visit) = visit {
                    self.help_panel.select(
                        &mut self.memory.help,
                        &mut self.memory.help_topic,
                        visit,
                    );
                }
            }
            KeyAction::On => return Some(vec![Request::Close]),
            _ => return None,
        }
        self.dirty();
        Some(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn topic(id: u32, name: &str, body: &str, examples: &str) -> Topic {
        Topic {
            id,
            name: name.into(),
            kind: 0,
            mode: 0,
            spans: [name, body, examples]
                .into_iter()
                .map(|s| s.encode_utf16().collect())
                .collect(),
        }
    }
    fn book() -> Arc<Book> {
        Arc::new(Book {
            language: Language::English,
            topics: vec![
                topic(
                    1,
                    "ALPHA",
                    "✎^✎*Form*✐\nALPHA(X)^✐\nText\n✎%Sample (--E1--)%✐",
                    "(1+2)/3 ----> 1",
                ),
                topic(2, "BETA", "✎+A bullet\n✎PLOCAL X:=1;P✐", ""),
                topic(3, "ALPHA", "A distinct topic with the same name", ""),
            ],
            groups: vec![],
        })
    }
    fn ready_panel() -> Panel {
        Panel {
            language: Some(Language::English),
            book: Some(book()),
            limit: RESULT_PAGE,
            ..Default::default()
        }
    }
    #[test]
    fn nested_original_markup_examples_and_mathematical_content_are_preserved() {
        let topic = &book().topics[0];
        let doc = document(topic);
        assert!(doc.unsupported.is_empty());
        assert!(doc.blocks.iter().any(|block| matches!(block, Block::Paragraph(runs) if runs.iter().any(|run|run.text=="Form" && run.style.bold && run.style.band==1))));
        assert!(
            doc.blocks
                .iter()
                .any(|block| matches!(block, Block::Example(0, style) if style.band==2))
        );
        assert_eq!(doc.examples, ["(1+2)/3 ----> 1"]);
        let text = doc
            .blocks
            .iter()
            .filter_map(|block| {
                if let Block::Paragraph(runs) = block {
                    Some(runs.iter().map(|run| run.text.as_str()).collect::<String>())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains('✎'));
        assert!(!text.contains('✐'));
        assert!(!text.contains("(--E1--)"));
        let layout = crate::ide::native::math_layout::expression("(1+2)/3", 16.);
        assert!(
            layout.height > 25,
            "a real fraction layout must exceed a plain text line"
        );
        let mut image = ppl_graphics::Grob::new(100, 100, 0xffffff);
        layout.paint_static(&mut image, 0, 0, 0);
        assert!(image.pixels().contains(&0));
    }
    #[test]
    fn list_markers_and_closers_preserve_nesting_and_continuation_styles() {
        let document = document(&topic(
            1,
            "LIST",
            "Introduction\n✎+✎*Outer*✐\nParent continuation\n✎oInner\nChild continuationo✐\nBack to parent\n✎+Last+✐\nAfter\nUnmatched+✐",
            "",
        ));
        let lines = document
            .blocks
            .iter()
            .map(|block| {
                let (list, runs) = match block {
                    Block::Paragraph(runs) => (None, runs),
                    Block::ListItem(line, runs) => (Some((line.level, line.marker)), runs),
                    Block::Example(..) => panic!("unexpected example"),
                };
                (
                    list,
                    runs.iter().map(|run| run.text.as_str()).collect::<String>(),
                )
            })
            .collect::<Vec<_>>();
        use ListLevel::{Primary, Secondary};
        assert_eq!(
            lines,
            [
                (None, "Introduction".into()),
                (Some((Primary, true)), "Outer".into()),
                (Some((Primary, false)), "Parent continuation".into()),
                (Some((Secondary, true)), "Inner".into()),
                (Some((Secondary, false)), "Child continuation".into()),
                (Some((Primary, false)), "Back to parent".into()),
                (Some((Primary, true)), "Last".into()),
                (None, "After".into()),
                (None, "Unmatched+✐".into()),
            ]
        );
        assert!(matches!(&document.blocks[1], Block::ListItem(_, runs) if runs[0].style.bold));
    }

    #[test]
    fn list_layout_keeps_wrapped_text_and_nested_markers_in_separate_columns() {
        let mut panel = Panel::default();
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240., 380.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (level, text) in [
                        (
                            ListLevel::Primary,
                            "Parent description needs two lines in this viewport.",
                        ),
                        (
                            ListLevel::Secondary,
                            "Child description likewise wraps at its own indent.",
                        ),
                    ] {
                        panel.list_item(
                            ui,
                            ListLine {
                                level,
                                marker: true,
                            },
                            &[Run {
                                text: text.into(),
                                style: Style::default(),
                            }],
                            1.,
                        );
                    }
                });
            },
        );
        let shape = |prefix| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::epaint::Shape::Text(text) if text.galley.text().starts_with(prefix) => {
                        Some(text)
                    }
                    _ => None,
                })
                .expect("text is painted")
        };
        let parent = shape("Parent");
        let child = shape("Child");
        let parent_marker = shape("•");
        let child_marker = shape("◦");
        assert!(parent.pos.x > parent_marker.pos.x);
        assert!(child_marker.pos.x > parent_marker.pos.x);
        assert!(child.pos.x > child_marker.pos.x && child.pos.x > parent.pos.x);
        for text in [parent, child] {
            assert!(text.galley.rows.len() > 1);
            let first_row = text.galley.rows[0].pos.x;
            assert!(text.galley.rows.iter().all(|row| row.pos.x == first_row));
        }
    }

    #[test]
    fn installed_plot_help_uses_real_primary_and_secondary_items() {
        let Some(path) = std::env::var_os("PPL_PRIME_HELP_RESOURCE") else {
            return;
        };
        let book = Book::load_resource(std::path::Path::new(&path), Language::English).unwrap();
        let topic = book.named("Function Plot View").next().unwrap();
        let document = document(topic);
        for level in [ListLevel::Primary, ListLevel::Secondary] {
            assert!(document.blocks.iter().any(|block| matches!(
                block, Block::ListItem(line, _) if line.level == level && line.marker
            )));
        }
        for block in &document.blocks {
            if let Block::Paragraph(runs) | Block::ListItem(_, runs) = block {
                assert!(
                    !runs
                        .iter()
                        .any(|run| run.text.contains("+✐") || run.text.contains("o✐"))
                );
            }
        }
    }

    #[test]
    fn unknown_styles_and_unpaired_calculator_units_do_not_lose_text() {
        let doc = document(&topic(1, "TEST", "✎?keep?✐\n(--E9--)", ""));
        assert!(doc.unsupported.contains(&'?'));
        assert!(doc.blocks.iter().any(
            |block| matches!(block,Block::Paragraph(runs) if runs.iter().any(|run|run.text=="keep"))
        ));
        assert_eq!(
            visible_units(&[0xd800, 0xe003, b'A' as u16]),
            "[U+D800][U+E003]A"
        );
    }
    #[test]
    fn duplicate_topics_navigation_languages_and_legacy_profiles_keep_identity() {
        let mut panel = ready_panel();
        let mut memory = Memory::default();
        let mut name = String::new();
        for id in [1, 3, 2] {
            let topic = panel.book.as_ref().unwrap().topic(id).unwrap();
            let visit = Visit {
                language: 1,
                id: Some(id),
                name: topic.name.clone(),
            };
            panel.select(&mut memory, &mut name, visit);
        }
        memory.back();
        assert_eq!(memory.current.as_ref().unwrap().id, Some(3));
        memory.back();
        assert_eq!(memory.current.as_ref().unwrap().id, Some(1));
        memory.forward();
        assert_eq!(memory.current.as_ref().unwrap().id, Some(3));
        memory.language = 3;
        let saved = serde_json::to_value(&memory).unwrap();
        let restored: Memory = serde_json::from_value(saved).unwrap();
        assert_eq!(restored, memory);
        restored.validate().unwrap();
        let mut legacy = serde_json::to_value(super::super::ToolMemory::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("help");
        legacy["help_topic"] = "SIN".into();
        legacy["help_query"] = "sin".into();
        let restored: super::super::ToolMemory = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.help, Memory::default());
        assert_eq!(restored.help_topic, "SIN");
    }
    #[test]
    fn book_search_and_keyboard_open_back_and_forward_use_stable_topic_ids() {
        let mut tools = NativeTools {
            help_panel: ready_panel(),
            ..Default::default()
        };
        tools.open(Page::Help);
        tools.memory.help_query = "alpha".into();
        tools.help_panel.search("alpha");
        assert_eq!(tools.help_panel.results, [0, 2]);
        tools.help_action(KeyAction::History(1)).unwrap();
        tools.help_action(KeyAction::Evaluate).unwrap();
        assert_eq!(tools.memory.help.current.as_ref().unwrap().id, Some(3));
        tools.help_panel.select(
            &mut tools.memory.help,
            &mut tools.memory.help_topic,
            Visit {
                language: 1,
                id: Some(2),
                name: "BETA".into(),
            },
        );
        tools.help_panel.search_focused = false;
        tools
            .help_action(KeyAction::Edit(EditCommand::Left(false)))
            .unwrap();
        assert_eq!(tools.memory.help.current.as_ref().unwrap().id, Some(3));
        tools
            .help_action(KeyAction::Edit(EditCommand::Right(false)))
            .unwrap();
        assert_eq!(tools.memory.help.current.as_ref().unwrap().id, Some(2));
    }
    #[test]
    fn asynchronous_completion_and_selected_resource_errors_are_explicit() {
        let mut panel = Panel {
            language: Some(Language::English),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        panel.loading = Some(rx);
        let ctx = egui::Context::default();
        assert!(!panel.poll(&ctx, Language::English));
        assert!(panel.loading.is_some());
        tx.send(Err("self-authored unreadable resource".into()))
            .unwrap();
        assert!(panel.poll(&ctx, Language::English));
        assert_eq!(
            panel.status.as_deref(),
            Some("self-authored unreadable resource")
        );
        assert!(panel.book.is_none());
        let (tx, rx) = mpsc::channel();
        panel.loading = Some(rx);
        tx.send(Ok(Some(book()))).unwrap();
        panel.poll(&ctx, Language::English);
        assert_eq!(panel.book.as_ref().unwrap().topics.len(), 3);
    }
    #[test]
    fn installed_help_book_loads_asynchronously_and_renders_real_selected_topic_when_configured() {
        if std::env::var_os("PPL_PRIME_HELP_RESOURCE").is_none() {
            return;
        }
        let mut panel = Panel::default();
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            panel.poll(&ctx, Language::English);
            if panel.loading.is_none() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Help loader timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let book = panel
            .book
            .as_ref()
            .expect("explicit genuine installed Help failed to load")
            .clone();
        assert!(book.topics.len() > 2000);
        let mut memory = Memory::default();
        let mut query = "SIN".into();
        let mut selected = "SIN".into();
        let mut requests = vec![];
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 480.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    panel.show(
                        ui,
                        &mut memory,
                        &mut query,
                        &mut selected,
                        (2., 480.),
                        &mut requests,
                    );
                });
            },
        );
        assert!(!output.shapes.is_empty());
        fn diagnostic(shape: &egui::epaint::Shape) -> bool {
            match shape {
                egui::epaint::Shape::Text(text) => text.galley.text().contains("ScrollArea ID"),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().any(diagnostic),
                _ => false,
            }
        }
        assert!(
            !output.shapes.iter().any(|shape| diagnostic(&shape.shape)),
            "repeated original examples produced duplicate scroll-area IDs"
        );
        let id = memory.current.as_ref().unwrap().id.unwrap();
        let entry = book.topic(id).unwrap();
        assert_eq!(entry.name, "SIN");
        let doc = &panel.document.as_ref().unwrap().1;
        assert!(!doc.examples.is_empty());
        assert!(doc.unsupported.is_empty());
        assert!(
            doc.blocks
                .iter()
                .any(|block| matches!(block, Block::Example(..)))
        );
        assert!(
            !panel.textures.is_empty(),
            "parseable original examples must use mathematical rendering"
        );
        let restored: Memory =
            serde_json::from_value(serde_json::to_value(&memory).unwrap()).unwrap();
        assert_eq!(restored.current.unwrap().id, Some(id));
    }
    #[test]
    fn escape_closes_a_first_visit_instead_of_trapping_help_navigation() {
        let mut tools = NativeTools::default();
        tools.open(Page::Help);
        tools.memory.help_topic = "SIN".into();
        tools.memory.help.current = Some(Visit {
            language: 1,
            id: Some(1),
            name: "SIN".into(),
        });
        assert!(matches!(
            tools.help_action(KeyAction::Back).unwrap().as_slice(),
            [Request::Close]
        ));
    }
    #[test]
    fn insert_command_survives_loader_completion_between_mouse_press_and_release() {
        fn text_rect(shape: &egui::epaint::Shape) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == "Insert command" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()))
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(text_rect),
                _ => None,
            }
        }
        let mut panel = Panel {
            language: Some(Language::English),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        panel.loading = Some(rx);
        let mut memory = Memory {
            current: Some(Visit {
                language: 1,
                id: None,
                name: "SIN".into(),
            }),
            contents: false,
            ..Default::default()
        };
        let mut query = "SIN".into();
        let mut selected = "SIN".into();
        let mut requests = vec![];
        let ctx = egui::Context::default();
        let mut frame = |panel: &mut Panel, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640., 480.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        panel.show(
                            ui,
                            &mut memory,
                            &mut query,
                            &mut selected,
                            (1., 480.),
                            &mut requests,
                        );
                    });
                },
            )
        };
        frame(&mut panel, vec![]);
        let output = frame(&mut panel, vec![]);
        let position = output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape))
            .unwrap()
            .center();
        frame(
            &mut panel,
            vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        tx.send(Ok(None)).unwrap();
        frame(
            &mut panel,
            vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(matches!(
            requests.as_slice(),
            [Request::InsertTemplate(
                crate::ide::native::expression::TemplateKind::Function("SIN")
            )]
        ));
        assert!(panel.missing);
    }
}
