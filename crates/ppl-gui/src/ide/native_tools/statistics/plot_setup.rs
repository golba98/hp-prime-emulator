//! Statistics 1Var Plot Setup's numeric fields on the calculator LCD.
use crate::ide::native::chrome;
use ppl_graphics::{
    Grob,
    text::{draw_text, measure},
};
use ppl_vm::{AppPlotSettings, PlotCursorMode, Settings};

pub(super) struct NumericField {
    pub(super) name: &'static str,
    caption: &'static str,
    pub(super) bounds: [i64; 4],
    prompt: &'static str,
}

pub(super) const NUMERIC_FIELDS: [NumericField; 9] = [
    NumericField {
        name: "Hwidth",
        caption: "H Width:",
        bounds: [60, 23, 314, 45],
        prompt: "Enter histogram bar width",
    },
    NumericField {
        name: "Hmin",
        caption: "H Rng:",
        bounds: [60, 49, 186, 71],
        prompt: "Enter minimum histogram value",
    },
    NumericField {
        name: "Hmax",
        caption: "",
        bounds: [188, 49, 314, 71],
        prompt: "Enter maximum histogram value",
    },
    NumericField {
        name: "Xmin",
        caption: "X Rng:",
        bounds: [60, 75, 186, 97],
        prompt: "Enter minimum horizontal value",
    },
    NumericField {
        name: "Xmax",
        caption: "",
        bounds: [188, 75, 314, 97],
        prompt: "Enter maximum horizontal value",
    },
    NumericField {
        name: "Ymin",
        caption: "Y Rng:",
        bounds: [60, 101, 186, 123],
        prompt: "Enter minimum vertical value",
    },
    NumericField {
        name: "Ymax",
        caption: "",
        bounds: [188, 101, 314, 123],
        prompt: "Enter maximum vertical value",
    },
    NumericField {
        name: "Xtick",
        caption: "X Tick:",
        bounds: [60, 127, 314, 149],
        prompt: "Enter horizontal tick spacing",
    },
    NumericField {
        name: "Ytick",
        caption: "Y Tick:",
        bounds: [60, 153, 314, 174],
        prompt: "Enter vertical tick spacing",
    },
];

pub(super) fn paint_numeric_page(
    screen: &mut Grob,
    values: &AppPlotSettings,
    format: &Settings,
    selected: usize,
    error: Option<&str>,
) {
    chrome::header(screen, "Statistics 1Var Plot Setup");
    for (index, field) in NUMERIC_FIELDS.iter().enumerate() {
        let [left, top, right, bottom] = field.bounds;
        draw_text(
            screen,
            field.caption,
            left - 2 - measure(field.caption, 14.),
            top + 3,
            14.,
            0,
            None,
            None,
        );
        if index == selected {
            screen.rect(left - 1, top - 1, right + 1, bottom + 1, 0, 0xffffff);
        }
        screen.rect(
            left,
            top,
            right,
            bottom,
            if index == selected { 0 } else { 0xe1e1e1 },
            0xffffff,
        );
        let value = values.get(field.name).expect("Statistics Plot Setup value");
        draw_text(
            screen,
            &value.display(format.home_format()).replace('-', "−"),
            left + 2,
            top + 3,
            14.,
            0,
            Some(right - left - 5),
            None,
        );
    }
    draw_text(
        screen,
        error.unwrap_or(NUMERIC_FIELDS[selected].prompt),
        0,
        202,
        14.,
        0,
        Some(320),
        None,
    );
}

use super::{NativeTools, Request, app, plot_background as bg};
use crate::ide::{
    native::{
        expression::{EditCommand, ExpressionEditor},
        keys::KeyAction,
    },
    native_app_ui::Calculation,
    native_tools::Page,
};
use ppl_vm::{
    PlotBackgroundField, PlotBackgroundPicture, PlotBackgroundPlacement, SessionSnapshot, Value,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlotSetupField {
    Plot(&'static str),
    Background(PlotBackgroundField),
}

impl PlotSetupField {
    fn value(self, settings: &AppPlotSettings) -> Option<Value> {
        match self {
            Self::Plot(name) => settings.get(name),
            Self::Background(field) => {
                Some(Value::Real(settings.statistics_background()?.value(field)))
            }
        }
    }

    pub(crate) fn set(self, settings: &mut AppPlotSettings, value: &Value) -> ppl_vm::RtResult<()> {
        match self {
            Self::Plot(name) => settings.set(name, value),
            Self::Background(field) => settings
                .statistics_background_mut()
                .expect("Statistics 1Var Plot background fields")
                .set(field, value),
        }
    }
}

const PAPER: u32 = 0xe8f1f6;
const OPTIONS: [(&str, &str, &str, [i64; 4]); 4] = [
    ("Axes", "Axes:", "Draw the axes", [92, 26, 108, 43]),
    ("Labels", "Labels:", "Label the axes", [248, 26, 264, 43]),
    (
        "GridDots",
        "Grid Dots:",
        "Draw the grid dots",
        [92, 52, 108, 69],
    ),
    (
        "GridLines",
        "Grid Lines:",
        "Draw the grid lines",
        [248, 52, 264, 69],
    ),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupPage {
    Values,
    Appearance,
    Background,
}

const CURSOR_FIELD: usize = OPTIONS.len();
const CURSOR_LABELS: [&str; 3] = ["Standard", "Inverting", "Blinking"];
const CURSOR_X: i64 = 92;
const CURSOR_TOP: i64 = 74;
const CURSOR_RIGHT: i64 = 314;
const CURSOR_ROW: i64 = 21;

pub(super) struct SetupSelection {
    page: SetupPage,
    selected: usize,
    editor: Option<ExpressionEditor>,
    picker: Option<usize>,
    import: Option<bg::ImportSelection>,
    gallery_scroll: Option<f32>,
    gallery_drag: Option<(egui::Pos2, f32)>,
}

impl Default for SetupSelection {
    fn default() -> Self {
        Self {
            page: SetupPage::Values,
            selected: 3,
            editor: None,
            picker: None,
            import: None,
            gallery_scroll: None,
            gallery_drag: None,
        }
    }
}

fn gallery_contains(point: egui::Vec2) -> bool {
    let [left, top, right, bottom] = bg::PICTURE_BOX.map(|value| value as f32);
    (left..right).contains(&point.x) && (top..bottom).contains(&point.y)
}

impl SetupSelection {
    fn page(&mut self, page: SetupPage) {
        self.page = page;
        self.picker = None;
        self.import = None;
        self.gallery_scroll = None;
        self.gallery_drag = None;
        self.selected = match page {
            SetupPage::Values => 3,
            SetupPage::Appearance => 1,
            SetupPage::Background => bg::MODE,
        };
    }

    fn drag_gallery(
        &mut self,
        response: &egui::Response,
        lcd: egui::Rect,
        scale: f32,
        selection: usize,
        count: usize,
    ) -> bool {
        let button = egui::PointerButton::Primary;
        if response.drag_started_by(button) {
            let origin = response.ctx.input(|input| input.pointer.press_origin());
            self.gallery_drag = origin
                .filter(|point| gallery_contains((*point - lcd.min) / scale))
                .map(|point| (point, self.gallery_scroll.unwrap_or(selection as f32)));
        }
        let Some((origin, previous)) = self.gallery_drag else {
            return false;
        };
        let stopped = response.drag_stopped();
        if stopped {
            self.gallery_drag = None;
        }
        let Some(pointer) = response.interact_pointer_pos() else {
            return false;
        };
        let delta = (pointer - origin) / scale;
        if delta.x.abs() > delta.y.abs() && delta.x.abs() >= 4. {
            self.gallery_scroll =
                Some((previous - delta.x / bg::THUMBNAIL_SPACING).clamp(0., count as f32));
            self.selected = bg::GALLERY;
            true
        } else {
            false
        }
    }

    fn numeric(
        &self,
        placement: PlotBackgroundPlacement,
    ) -> Option<(PlotSetupField, &'static str)> {
        match self.page {
            SetupPage::Values => NUMERIC_FIELDS
                .get(self.selected)
                .map(|field| (PlotSetupField::Plot(field.name), field.prompt)),
            SetupPage::Appearance => None,
            SetupPage::Background => bg::numeric_field(self.selected, placement)
                .map(|field| (PlotSetupField::Background(field.field), field.prompt)),
        }
    }

    fn move_focus(&mut self, action: KeyAction, placement: PlotBackgroundPlacement) {
        use EditCommand as E;
        use KeyAction as A;
        self.selected = match self.page {
            SetupPage::Values => match action {
                A::History(delta) if delta < 0 => [0, 0, 0, 1, 2, 3, 4, 5, 7][self.selected],
                A::History(_) => [1, 3, 4, 5, 6, 7, 7, 8, 8][self.selected],
                A::Edit(E::Left(false)) => [0, 1, 1, 3, 3, 5, 5, 7, 8][self.selected],
                A::Edit(E::Right(false)) => [0, 2, 2, 4, 4, 6, 6, 7, 8][self.selected],
                _ => self.selected,
            },
            SetupPage::Appearance => match action {
                A::History(delta) if delta < 0 => [0, 1, 0, 1, 2][self.selected],
                A::History(_) => [2, 3, 4, 4, 4][self.selected],
                A::Edit(E::Left(false)) => [0, 0, 2, 2, 4][self.selected],
                A::Edit(E::Right(false)) => [1, 1, 3, 3, 4][self.selected],
                _ => self.selected,
            },
            SetupPage::Background => {
                let controls = bg::controls(placement);
                let position = controls
                    .iter()
                    .position(|value| *value == self.selected)
                    .unwrap_or(0);
                let delta = match action {
                    A::History(delta) => delta as isize,
                    A::Edit(E::Left(false)) => -1,
                    A::Edit(E::Right(false)) => 1,
                    _ => 0,
                };
                controls[position
                    .saturating_add_signed(delta)
                    .min(controls.len() - 1)]
            }
        };
    }
}

fn label(screen: &mut Grob, text: &str, x: i64, y: i64) {
    draw_text(screen, text, x, y, 14., 0, Some(320 - x), None);
}

fn option_selected(settings: &AppPlotSettings, index: usize) -> bool {
    [
        !settings.axes,
        settings.labels,
        !settings.grid_dots,
        !settings.grid_lines,
    ][index]
}

fn paint_appearance(
    screen: &mut Grob,
    settings: &AppPlotSettings,
    setup: &SetupSelection,
    error: Option<&str>,
) {
    let selected = setup.selected;
    chrome::header(screen, "Statistics 1Var Plot Setup");
    for (index, (_, caption, _, [left, top, right, bottom])) in OPTIONS.iter().copied().enumerate()
    {
        label(screen, caption, left - 3 - measure(caption, 14.), top);
        screen.rect(left, top, right, bottom, 0xe1e1e1, 0xffffff);
        if index == selected {
            screen.rect(left - 1, top - 1, right + 1, bottom + 1, 0, 0xffffff);
            screen.rect(left, top, right, bottom, 0, 0xffffff);
        }
        if option_selected(settings, index) {
            screen.line(left + 4, top + 8, left + 7, top + 13, 0);
            screen.line(left + 7, top + 13, left + 12, top + 3, 0);
        }
    }
    label(screen, "Cursor:", 44, 78);
    let mode = settings.cursor_mode().unwrap_or(PlotCursorMode::Standard) as usize;
    if let Some(choice) = setup.picker {
        let bottom = CURSOR_TOP + CURSOR_ROW * CURSOR_LABELS.len() as i64 + 1;
        screen.rect(CURSOR_X, CURSOR_TOP, CURSOR_RIGHT, bottom, 0, 0xffffff);
        for (index, caption) in CURSOR_LABELS.iter().enumerate() {
            let y = CURSOR_TOP + 1 + index as i64 * CURSOR_ROW;
            if index == choice {
                screen.fill_rect(
                    CURSOR_X + 1,
                    y,
                    CURSOR_RIGHT - 1,
                    y + CURSOR_ROW - 1,
                    0xadccff,
                );
            }
            if index == mode {
                label(screen, "✓", CURSOR_X + 3, y + 1);
            }
            label(screen, caption, CURSOR_X + 15, y + 1);
        }
    } else {
        let border = if selected == CURSOR_FIELD {
            0
        } else {
            0xe1e1e1
        };
        screen.rect(
            CURSOR_X,
            CURSOR_TOP,
            CURSOR_RIGHT,
            CURSOR_TOP + 23,
            border,
            0xffffff,
        );
        screen.fill_rect(
            CURSOR_RIGHT - 16,
            CURSOR_TOP + 2,
            CURSOR_RIGHT - 1,
            CURSOR_TOP + 21,
            0xadccff,
        );
        label(screen, CURSOR_LABELS[mode], CURSOR_X + 3, CURSOR_TOP + 3);
        label(screen, "▾", CURSOR_RIGHT - 12, CURSOR_TOP + 2);
    }
    if let Some(prompt) = error.or_else(|| OPTIONS.get(selected).map(|option| option.2)) {
        label(screen, prompt, 0, 202);
    }
}

fn paint_pager(screen: &mut Grob, page: SetupPage) {
    for y in 221..239 {
        let color = screen.get(130, y).unwrap_or(0);
        screen.line(157, y, 160, y, color);
    }
    let message = match page {
        SetupPage::Values => "Page 1/3",
        SetupPage::Appearance => "Page 2/3",
        SetupPage::Background => "Page 3/3",
    };
    draw_text(
        screen,
        message,
        160 - measure(message, 12.) / 2,
        222,
        12.,
        0xffffff,
        None,
        None,
    );
    for row in 0..5 {
        if page != SetupPage::Values {
            screen.line(111, 226 + row, 111 + row, 226 + row, 0xffffff);
        }
        if page != SetupPage::Background {
            screen.line(201, 226 + row, 205 - row, 226 + row, 0xffffff);
        }
    }
}

impl NativeTools {
    pub(crate) fn statistics_plot_setup_active(&self) -> bool {
        self.page == Some(Page::PlotSetup) && self.memory.selected_app == app(false)
    }

    pub(crate) fn statistics_reset_plot_setup(&mut self) {
        self.statistics_panel.plot_setup = SetupSelection::default();
    }

    pub(crate) fn statistics_plot_setup_matches(
        &self,
        field: PlotSetupField,
        source: &str,
    ) -> bool {
        let setup = &self.statistics_panel.plot_setup;
        self.statistics_plot_setup_active()
            && setup
                .numeric(PlotBackgroundPlacement::Range)
                .is_some_and(|(selected, _)| selected == field)
            && setup
                .editor
                .as_ref()
                .is_some_and(|editor| editor.text() == source)
    }

    pub(crate) fn statistics_finish_plot_setup_edit(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.statistics_panel.plot_setup.editor = None;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.dirty();
    }

    pub(crate) fn statistics_plot_setup_action(
        &mut self,
        action: KeyAction,
        snapshot: &SessionSnapshot,
    ) -> Option<Vec<Request>> {
        use EditCommand as E;
        use KeyAction as A;
        if !self.statistics_plot_setup_active() {
            return None;
        }
        if matches!(action, A::Home | A::Tool(_)) {
            self.statistics_reset_plot_setup();
            return None;
        }
        let mut settings = snapshot.plot_settings_for(app(false))?;
        let placement = settings.statistics_background()?.placement;
        self.statistics_panel.plot_pictures.refresh(snapshot);
        let setup = &mut self.statistics_panel.plot_setup;
        if let Some(import) = &mut setup.import {
            let source = import.source();
            let pictures = self.statistics_panel.plot_pictures.list(source, false);
            let request = match import.action(action, pictures.len()) {
                bg::ImportAction::Keep => None,
                bg::ImportAction::Close => {
                    setup.import = None;
                    None
                }
                bg::ImportAction::Copy(index) => {
                    let picture = &pictures[index].key;
                    let PlotBackgroundPicture::AppFile(name) = picture else {
                        unreachable!("Only app files can be imported");
                    };
                    let request = Request::ImportStatisticsPicture {
                        from: source.into(),
                        name: name.clone(),
                    };
                    setup.import = None;
                    setup.gallery_scroll = None;
                    setup.selected = bg::GALLERY;
                    Some(request)
                }
            };
            self.dirty();
            return Some(request.into_iter().collect());
        }
        let background_page = setup.page == SetupPage::Background;
        let numeric = setup.numeric(placement).map(|(field, _)| field);
        let mut requests = Vec::new();
        if let Some(choice) = &mut setup.picker {
            let choices = if background_page {
                bg::PLACEMENT_LABELS.len()
            } else {
                CURSOR_LABELS.len()
            };
            match action {
                A::Back | A::On => setup.picker = None,
                A::History(delta) => {
                    *choice = choice
                        .saturating_add_signed(delta as isize)
                        .min(choices - 1);
                }
                A::Evaluate => {
                    if background_page {
                        settings.statistics_background_mut()?.placement =
                            PlotBackgroundPlacement::CHOICES[*choice];
                        requests.push(Request::PlotConfiguration(app(false).into(), settings));
                    } else {
                        match settings.set("Cursor", &PlotCursorMode::CHOICES[*choice].value()) {
                            Ok(()) => requests
                                .push(Request::PlotConfiguration(app(false).into(), settings)),
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                    setup.picker = None;
                }
                _ => {}
            }
        } else if let Some(editor) = &mut setup.editor {
            match action {
                A::Back | A::On => {
                    setup.editor = None;
                    self.error = None;
                }
                A::Insert(text) => editor.insert(text),
                A::Template(kind) => editor.insert_template(kind),
                A::Edit(command) => editor.apply(command),
                A::Copy => requests.push(Request::CopyText(editor.text().into())),
                A::Evaluate | A::Approximate if !editor.text().trim().is_empty() => {
                    if let Some(field) = numeric {
                        requests.push(Request::Calculate(Calculation::StatisticsPlotField {
                            field,
                            source: editor.text().into(),
                        }));
                    }
                }
                _ => {}
            }
        } else {
            match action {
                A::Edit(E::Left(false) | E::Right(false))
                    if background_page && setup.selected == bg::GALLERY =>
                {
                    let pictures = self.statistics_panel.plot_pictures.list(app(false), true);
                    let delta = if matches!(action, A::Edit(E::Left(false))) {
                        -1
                    } else {
                        1
                    };
                    bg::move_picture(settings.statistics_background_mut()?, &pictures, delta);
                    setup.gallery_scroll = None;
                    requests.push(Request::PlotConfiguration(app(false).into(), settings));
                }
                A::History(_) | A::Edit(_) => setup.move_focus(action, placement),
                A::Evaluate if numeric.is_some() => {
                    let value = numeric
                        .and_then(|field| field.value(&settings))
                        .expect("Statistics Plot Setup value");
                    setup.editor = Some(ExpressionEditor::from_text(
                        &value.display(Default::default()),
                    ));
                }
                A::Evaluate if background_page => {
                    if setup.selected == bg::MODE {
                        setup.picker = Some(placement as usize);
                    }
                }
                A::Evaluate if setup.selected == CURSOR_FIELD => {
                    let mode = settings.cursor_mode().unwrap_or(PlotCursorMode::Standard);
                    setup.picker = Some(mode as usize);
                }
                A::Evaluate => {
                    let index = setup.selected;
                    let (name, _, _, _) = OPTIONS[index];
                    let checked = !option_selected(&settings, index);
                    let raw = if name == "Labels" { checked } else { !checked };
                    match settings.set(name, &Value::from_bool(raw)) {
                        Ok(()) => {
                            requests.push(Request::PlotConfiguration(app(false).into(), settings))
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
                A::Insert(text) if numeric.is_some() => {
                    setup.editor = Some(ExpressionEditor::from_text(text));
                }
                A::Back | A::On => {
                    self.open(Page::Plot);
                    self.plot(snapshot.clone());
                }
                _ => {}
            }
        }
        self.dirty();
        Some(requests)
    }

    pub(crate) fn show_statistics_plot_setup(
        &mut self,
        ctx: &egui::Context,
        lcd: egui::Rect,
        scale: f32,
        snapshot: &SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        let Some(settings) = snapshot.plot_settings_for(app(false)) else {
            return;
        };
        let format = snapshot
            .app_settings
            .get(app(false))
            .map(|settings| settings.effective(&snapshot.settings))
            .unwrap_or_else(|| snapshot.settings.clone());
        self.statistics_panel.plot_pictures.refresh(snapshot);
        let setup = &self.statistics_panel.plot_setup;
        let background = settings
            .statistics_background()
            .expect("Statistics 1Var Plot background");
        let numeric = setup.numeric(background.placement);
        let gallery_enabled = setup.page == SetupPage::Background
            && setup.picker.is_none()
            && setup.editor.is_none()
            && setup.import.is_none();
        let (picture, picture_count) = if gallery_enabled {
            let pictures = self.statistics_panel.plot_pictures.list(app(false), true);
            (
                bg::picture_index(background.picture.as_ref(), &pictures),
                pictures.len(),
            )
        } else {
            (0, 0)
        };
        let mut image = Grob::new(320, 240, PAPER);
        if let Some(import) = &setup.import {
            let pictures = self
                .statistics_panel
                .plot_pictures
                .list(import.source(), false);
            import.paint(&mut image, &pictures);
        } else {
            match setup.page {
                SetupPage::Values => paint_numeric_page(
                    &mut image,
                    &settings,
                    &format,
                    setup.selected,
                    self.error.as_deref(),
                ),
                SetupPage::Appearance => {
                    paint_appearance(&mut image, &settings, setup, self.error.as_deref())
                }
                SetupPage::Background => bg::paint_setup(
                    &mut image,
                    background,
                    &format,
                    &self.statistics_panel.plot_pictures.list(app(false), true),
                    (setup.selected, setup.picker),
                    setup.gallery_scroll,
                    self.error
                        .as_deref()
                        .or_else(|| self.statistics_panel.plot_pictures.admission_error()),
                ),
            }
            if let (Some(editor), Some((_, prompt))) = (&setup.editor, numeric) {
                if setup.page == SetupPage::Background {
                    image.fill_rect(0, 201, 319, 219, PAPER);
                    if let Some(error) = self.error.as_deref() {
                        image.rect(9, 97, 309, 139, 0, PAPER);
                        label(&mut image, error, 15, 107);
                    }
                } else {
                    image.fill_rect(0, 186, 319, 219, PAPER);
                    label(&mut image, self.error.as_deref().unwrap_or(prompt), 0, 187);
                }
                image.fill_rect(0, 204, 319, 219, 0xffffff);
                image.line(0, 204, 319, 204, 0);
                label(&mut image, editor.text(), 1, 205);
                let x = (1 + measure(&editor.text()[..editor.cursor()], 14.)).min(319);
                image.line(x, 206, x, 219, 0);
                chrome::softkeys(&mut image, &["", "", "", "", "Cancel", "OK"]);
            } else {
                let left = if numeric.is_some() { "Edit" } else { "" };
                let check = match (setup.page, setup.selected, setup.picker) {
                    (SetupPage::Appearance, CURSOR_FIELD, None)
                    | (SetupPage::Background, bg::MODE, None) => "Choose",
                    (SetupPage::Appearance, _, None) => "✓",
                    _ => "",
                };
                let import = if setup.page == SetupPage::Background && setup.picker.is_none() {
                    "Import"
                } else {
                    ""
                };
                chrome::softkeys(&mut image, &[left, check, "", "", import, ""]);
                paint_pager(&mut image, setup.page);
            }
        }
        self.update_statistics_texture(ctx, &image);
        egui::Area::new("native_statistics_plot_setup".into())
            .fixed_pos(lcd.min)
            .order(egui::Order::Middle)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_clip_rect(lcd);
                ui.set_min_size(lcd.size());
                ui.painter().image(
                    self.statistics_panel.texture.as_ref().unwrap().id(),
                    lcd,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    egui::Color32::WHITE,
                );
                let sense = if gallery_enabled {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::click()
                };
                let response = ui.interact(lcd, ui.id().with("plot_setup"), sense);
                if gallery_enabled
                    && self.statistics_panel.plot_setup.drag_gallery(
                        &response,
                        lcd,
                        scale,
                        picture,
                        picture_count,
                    )
                {
                    self.error = None;
                    self.dirty();
                    ctx.request_repaint();
                }
                if response.clicked()
                    && let Some(pointer) = response.interact_pointer_pos()
                {
                    let point = (pointer - lcd.min) / scale;
                    if gallery_enabled
                        && gallery_contains(point)
                        && let Some(scroll) = self.statistics_panel.plot_setup.gallery_scroll
                    {
                        let pictures = self.statistics_panel.plot_pictures.list(app(false), true);
                        if let Some(position) = bg::picture_at(&pictures, scroll, point.x as i64) {
                            let mut settings = settings.clone();
                            let background = settings.statistics_background_mut().unwrap();
                            background.picture = position
                                .checked_sub(1)
                                .map(|index| pictures[index].key.clone());
                            self.statistics_panel.plot_setup.gallery_scroll = None;
                            requests.push(Request::PlotConfiguration(app(false).into(), settings));
                        }
                    } else if let Some(action) = self.statistics_plot_setup_click(
                        point.x as i64,
                        point.y as i64,
                        background.placement,
                    ) && let Some(next) =
                        self.statistics_plot_setup_action(action, snapshot)
                    {
                        requests.extend(next);
                    }
                    self.dirty();
                    ctx.request_repaint();
                }
            });
    }

    fn statistics_plot_setup_click(
        &mut self,
        x: i64,
        y: i64,
        placement: PlotBackgroundPlacement,
    ) -> Option<KeyAction> {
        let setup = &mut self.statistics_panel.plot_setup;
        if let Some(import) = &mut setup.import {
            return import.click(x, y);
        }
        if setup.editor.is_some() {
            return (y >= 220).then(|| match chrome::softkey_index(x) {
                4 => KeyAction::Back,
                5 => KeyAction::Evaluate,
                _ => KeyAction::None,
            });
        }
        if let Some(choice) = &mut setup.picker {
            let (left, top, right, count) = if setup.page == SetupPage::Background {
                let [left, top, right, _] = bg::PLACEMENT_BOX;
                (left, top, right, bg::PLACEMENT_LABELS.len())
            } else {
                (CURSOR_X, CURSOR_TOP, CURSOR_RIGHT, CURSOR_LABELS.len())
            };
            let row = ((y - top - 1) / CURSOR_ROW) as usize;
            if (left..=right).contains(&x) && y > top && row < count {
                *choice = row;
                return Some(KeyAction::Evaluate);
            }
            setup.picker = None;
            return None;
        }
        if y >= 220 {
            match (setup.page, x) {
                (SetupPage::Values | SetupPage::Background, 0..53)
                | (SetupPage::Appearance | SetupPage::Background, 53..106) => {
                    return Some(KeyAction::Evaluate);
                }
                (SetupPage::Values, 159..212) => setup.page(SetupPage::Appearance),
                (SetupPage::Appearance, 106..159) => setup.page(SetupPage::Values),
                (SetupPage::Appearance, 159..212) => setup.page(SetupPage::Background),
                (SetupPage::Background, 106..159) => setup.page(SetupPage::Appearance),
                (SetupPage::Background, 212..265) => {
                    setup.import = Some(bg::ImportSelection::default());
                    self.error = None;
                }
                _ => {}
            }
            return None;
        }
        let hit = |[left, top, right, bottom]: [i64; 4]| {
            (left..=right).contains(&x) && (top..=bottom).contains(&y)
        };
        if setup.page == SetupPage::Background {
            if hit(bg::PLACEMENT_BOX) {
                setup.selected = bg::MODE;
                self.error = None;
                return Some(KeyAction::Evaluate);
            }
            if hit(bg::PICTURE_BOX) {
                setup.selected = bg::GALLERY;
                self.error = None;
                let direction = if x < 79 {
                    Some(EditCommand::Left(false))
                } else if x > 244 {
                    Some(EditCommand::Right(false))
                } else {
                    None
                };
                return direction.map(KeyAction::Edit);
            }
            if let Some(&index) = bg::controls(placement).iter().find(|&&index| {
                bg::numeric_field(index, placement).is_some_and(|field| hit(field.bounds))
            }) {
                setup.selected = index;
                self.error = None;
            }
        } else if setup.page == SetupPage::Values {
            if let Some(index) = NUMERIC_FIELDS.iter().position(|field| hit(field.bounds)) {
                setup.selected = index;
                self.error = None;
            }
        } else if hit([CURSOR_X, CURSOR_TOP, CURSOR_RIGHT, CURSOR_TOP + 23]) {
            setup.selected = CURSOR_FIELD;
            self.error = None;
            return Some(KeyAction::Evaluate);
        } else if let Some(index) = OPTIONS.iter().position(|option| hit(option.3)) {
            setup.selected = index;
            self.error = None;
            return Some(KeyAction::Evaluate);
        }
        None
    }
}
