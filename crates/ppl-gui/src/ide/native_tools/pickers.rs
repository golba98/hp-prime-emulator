//! Searchable Native catalogs. Command signatures come from the compiler;
//! offered measurements must pass the existing dimensional syntax boundary.
use super::{NativeTools, Page, Request};
use crate::ide::native::{expression::TemplateKind as T, keys::KeyAction};
use ppl_compiler::builtins::{BUILTINS, BuiltinSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PickerState {
    pub query: String,
    pub category: String,
    pub selected: String,
}
struct Choice {
    key: &'static str,
    label: String,
    category: &'static str,
}
// HP User Guide ch.24 names these unit categories. This is the supported
// Native subset; the CAS translator remains authoritative for inserted units.
const UNITS: &[(&str, &str, &str)] = &[
    ("m", "metre", "Length"),
    ("cm", "centimetre", "Length"),
    ("km", "kilometre", "Length"),
    ("in", "inch", "Length"),
    ("ft", "foot", "Length"),
    ("yd", "yard", "Length"),
    ("mi", "mile", "Length"),
    ("m^2", "square metre", "Area"),
    ("cm^2", "square centimetre", "Area"),
    ("m^3", "cubic metre", "Volume"),
    ("L", "litre", "Volume"),
    ("mL", "millilitre", "Volume"),
    ("s", "second", "Time"),
    ("min", "minute", "Time"),
    ("h", "hour", "Time"),
    ("Hz", "hertz", "Time"),
    ("m/s", "metres per second", "Speed"),
    ("km/h", "kilometres per hour", "Speed"),
    ("g", "gram", "Mass"),
    ("kg", "kilogram", "Mass"),
    ("lb", "pound", "Mass"),
    ("oz", "ounce", "Mass"),
    ("ton", "ton", "Mass"),
    ("m/s^2", "metres per second squared", "Acceleration"),
    ("N", "newton", "Force"),
    ("J", "joule", "Energy"),
    ("W", "watt", "Power"),
    ("Pa", "pascal", "Pressure"),
    ("bar", "bar", "Pressure"),
    ("atm", "atmosphere", "Pressure"),
    ("psi", "pounds per square inch", "Pressure"),
    ("K", "kelvin", "Temperature"),
    ("A", "ampere", "Electricity"),
    ("C", "coulomb", "Electricity"),
    ("V", "volt", "Electricity"),
    ("F", "farad", "Electricity"),
    ("Ω", "ohm", "Electricity"),
    ("S", "siemens", "Electricity"),
    ("Wb", "weber", "Electricity"),
    ("T", "tesla", "Electricity"),
    ("H", "henry", "Electricity"),
    ("rad", "radian", "Angle"),
    ("deg", "degree", "Angle"),
    ("Pa*s", "pascal second", "Viscosity"),
    ("Bq", "becquerel", "Radiation"),
    ("Gy", "gray", "Radiation"),
    ("Sv", "sievert", "Radiation"),
    ("mol", "mole", "Amount"),
];
const UNIT_TOOLS: &[(&str, &str)] = &[
    ("CONVERT", "convert to another unit"),
    ("MKSA", "convert to SI base units"),
    ("UFACTOR", "factor out a unit"),
    ("USIMPLIFY", "simplify a measurement"),
];
const PREFIXES: &[(&str, &str)] = &[
    ("", "None"),
    ("Y", "yotta"),
    ("Z", "zetta"),
    ("E", "exa"),
    ("P", "peta"),
    ("T", "tera"),
    ("G", "giga"),
    ("M", "mega"),
    ("k", "kilo"),
    ("h", "hecto"),
    ("D", "deca"),
    ("d", "deci"),
    ("c", "centi"),
    ("m", "milli"),
    ("µ", "micro"),
    ("n", "nano"),
    ("p", "pico"),
    ("f", "femto"),
    ("a", "atto"),
    ("z", "zepto"),
    ("y", "yocto"),
];
const TEMPLATES: &[(&str, T)] = &[
    ("Fraction", T::Fraction),
    ("Power", T::Power),
    ("Square root", T::Root),
    ("Nth root", T::NthRoot),
    ("Absolute value", T::Function("ABS")),
    ("List", T::List),
    ("Matrix", T::Matrix),
    ("Parentheses", T::Parentheses),
];
const MEMORY: &[(Page, &str)] = &[
    (Page::Variables, "Variables"),
    (Page::CasVariables, "CAS variables"),
    (Page::Programs, "Programs"),
    (Page::Lists, "Lists"),
    (Page::Matrices, "Matrices"),
    (Page::Notes, "Notes"),
];

// Consume menu navigation before egui's end-of-frame focus traversal. The
// cursor scrolls through the actual choices, including rows outside the LCD.
fn choose(
    ui: &mut egui::Ui,
    id: &str,
    selected: &mut String,
    items: &[(&str, String)],
    cursor: &mut Option<usize>,
    size: egui::Vec2,
) {
    let popup = egui::ComboBox::from_id_salt(id)
        .selected_text(
            items
                .iter()
                .find(|(key, _)| *key == selected)
                .map_or("All", |(_, label)| label),
        )
        .width(size.x)
        .height(size.y)
        .show_ui(ui, |ui| {
            let mut index = cursor.unwrap_or_else(|| {
                items
                    .iter()
                    .position(|(key, _)| *key == selected)
                    .unwrap_or(0)
            });
            let mut moved = cursor.is_none();
            ui.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    index = index.saturating_sub(1);
                    moved = true;
                }
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    index = (index + 1).min(items.len().saturating_sub(1));
                    moved = true;
                }
            });
            *cursor = Some(index);
            let accept =
                ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            for (row, (key, label)) in items.iter().enumerate() {
                let response = ui.selectable_label(row == index, label);
                if row == index && moved {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
                if response.clicked() || (accept && row == index) {
                    *selected = (*key).into();
                    ui.close();
                    if let Some(id) = ui.memory(|memory| memory.focused()) {
                        ui.memory_mut(|memory| memory.surrender_focus(id));
                    }
                }
            }
        });
    if popup.inner.is_none() {
        *cursor = None;
    }
}

fn command_request(spec: &BuiltinSpec) -> Request {
    if spec.min == 0 {
        Request::Insert(spec.name.into())
    } else {
        Request::InsertTemplate(T::Function(spec.name))
    }
}
impl NativeTools {
    pub fn is_picker_page(&self) -> bool {
        matches!(
            self.page,
            Some(Page::Toolbox | Page::Units | Page::Templates | Page::Memory)
        )
    }
    fn picker_search_id(page: Page) -> egui::Id {
        egui::Id::new(("native_picker_search", format!("{page:?}")))
    }
    pub fn picker_owns_navigation(&self, ctx: &egui::Context) -> bool {
        self.is_picker_page()
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|m| m.focused()).is_none_or(|id| {
                self.page
                    .is_some_and(|page| id == Self::picker_search_id(page))
            })
    }
    fn picker_state(&self, page: Page) -> (&str, &str, &str) {
        match page {
            Page::Toolbox => (
                &self.memory.commands.query,
                &self.memory.commands.category,
                &self.memory.commands.selected,
            ),
            Page::Units => (
                &self.memory.units.query,
                &self.memory.units.category,
                &self.memory.units.selected,
            ),
            Page::Templates => ("", "", &self.memory.template_selection),
            Page::Memory => ("", "", &self.memory.memory_selection),
            _ => ("", "", ""),
        }
    }
    fn picker_selected_mut(&mut self, page: Page) -> &mut String {
        match page {
            Page::Toolbox => &mut self.memory.commands.selected,
            Page::Units => &mut self.memory.units.selected,
            Page::Templates => &mut self.memory.template_selection,
            Page::Memory => &mut self.memory.memory_selection,
            _ => unreachable!(),
        }
    }
    fn picker_choices(&self, page: Page) -> Vec<Choice> {
        let all: Vec<_> = match page {
            Page::Toolbox => BUILTINS
                .iter()
                .map(|spec| Choice {
                    key: spec.name,
                    label: format!(
                        "{} ({}) · {}",
                        spec.name,
                        spec.arity_text(),
                        spec.status.as_str()
                    ),
                    category: spec.category.as_str(),
                })
                .collect(),
            Page::Units => UNITS
                .iter()
                .map(|&(key, label, category)| Choice {
                    key,
                    label: format!("{key} · {label}"),
                    category,
                })
                .chain(UNIT_TOOLS.iter().map(|&(key, label)| Choice {
                    key,
                    label: format!("{key} · {label}"),
                    category: "Tools",
                }))
                .collect(),
            Page::Templates => TEMPLATES
                .iter()
                .map(|&(key, _)| Choice {
                    key,
                    label: key.into(),
                    category: "",
                })
                .collect(),
            Page::Memory => MEMORY
                .iter()
                .map(|&(_, key)| Choice {
                    key,
                    label: key.into(),
                    category: "",
                })
                .collect(),
            _ => Vec::new(),
        };
        let (query, category, _) = self.picker_state(page);
        let query = query.trim().to_lowercase();
        let mut filtered = all
            .into_iter()
            .filter(|c| {
                (category.is_empty() || c.category == category)
                    && (query.is_empty() || c.label.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        if page == Page::Toolbox {
            filtered.sort_by_key(|c| {
                (
                    c.key.to_lowercase() != query,
                    !c.key.to_lowercase().starts_with(&query),
                    c.key,
                )
            });
        }
        filtered
    }
    fn picker_accept(&mut self, page: Page, key: &str) -> Vec<Request> {
        match page {
            Page::Toolbox => ppl_compiler::builtins::lookup_case_insensitive(key)
                .map(command_request)
                .into_iter()
                .collect(),
            Page::Templates => TEMPLATES
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, t)| Request::InsertTemplate(*t))
                .into_iter()
                .collect(),
            Page::Units if UNIT_TOOLS.iter().any(|&(name, _)| name == key) => {
                let name = UNIT_TOOLS.iter().find(|&&(name, _)| name == key).unwrap().0;
                vec![Request::InsertTemplate(T::Function(name))]
            }
            Page::Units => {
                let unit = format!("_{}{key}", self.memory.unit_prefix);
                match ppl_cas::units::translate_home(&format!("1{unit}")) {
                    Ok(Some(_)) => vec![Request::Insert(unit)],
                    Ok(None) => Vec::new(),
                    Err(error) => {
                        self.error = Some(error.to_string());
                        Vec::new()
                    }
                }
            }
            Page::Memory => {
                if let Some(&(page, _)) = MEMORY.iter().find(|(_, label)| *label == key) {
                    self.open(page);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
    /// None leaves text editing to egui; Some consumes selection/activation.
    pub fn picker_action(&mut self, action: KeyAction) -> Option<Vec<Request>> {
        if !self.is_picker_page() {
            return None;
        }
        let page = self.page?;
        let choices = self.picker_choices(page);
        let selected = self.picker_state(page).2;
        let current = choices.iter().position(|c| c.key == selected).unwrap_or(0);
        let next = match action {
            KeyAction::History(delta) => {
                if delta < 0 {
                    current.saturating_sub(1)
                } else {
                    (current + 1).min(choices.len().saturating_sub(1))
                }
            }
            KeyAction::Evaluate => current,
            _ => return None,
        };
        let Some(choice) = choices.get(next) else {
            return Some(Vec::new());
        };
        *self.picker_selected_mut(page) = choice.key.into();
        self.picker_scroll_pending = true;
        self.dirty();
        Some(if action == KeyAction::Evaluate {
            self.picker_accept(page, choice.key)
        } else {
            Vec::new()
        })
    }
    pub(super) fn show_picker(
        &mut self,
        ui: &mut egui::Ui,
        page: Page,
        lcd: egui::Rect,
        scale: f32,
        snapshot: &ppl_vm::SessionSnapshot,
        requests: &mut Vec<Request>,
    ) {
        if matches!(page, Page::Toolbox | Page::Units) {
            let state = if page == Page::Toolbox {
                &mut self.memory.commands
            } else {
                &mut self.memory.units
            };
            let old = (state.query.clone(), state.category.clone());
            let search = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(Self::picker_search_id(page))
                    .hint_text("Find name or symbol"),
            );
            if std::mem::take(&mut self.picker_focus_pending) {
                search.request_focus();
            }
            ui.horizontal(|ui| {
                ui.label("Category");
                let categories: std::collections::BTreeSet<_> = if page == Page::Toolbox {
                    BUILTINS.iter().map(|s| s.category.as_str()).collect()
                } else {
                    UNITS
                        .iter()
                        .map(|u| u.2)
                        .chain(std::iter::once("Tools"))
                        .collect()
                };
                let items = std::iter::once(("", "All".into()))
                    .chain(categories.into_iter().map(|name| (name, name.into())))
                    .collect::<Vec<_>>();
                choose(
                    ui,
                    &format!("native_picker_category_{page:?}"),
                    &mut state.category,
                    &items,
                    &mut self.picker_category_cursor,
                    egui::vec2(165. * scale, 120. * scale),
                );
            });
            if old != (state.query.clone(), state.category.clone()) {
                state.selected.clear();
                if old.1 != state.category {
                    search.request_focus();
                }
                self.picker_scroll_pending = true;
                self.error = None;
                self.dirty();
            }
        }
        if page == Page::Units {
            let old_prefix = self.memory.unit_prefix.clone();
            ui.horizontal(|ui| {
                ui.label("Prefix");
                let items = PREFIXES
                    .iter()
                    .map(|&(prefix, label)| {
                        (
                            prefix,
                            if prefix.is_empty() {
                                label.into()
                            } else {
                                format!("{prefix} · {label}")
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                choose(
                    ui,
                    "native_unit_prefix",
                    &mut self.memory.unit_prefix,
                    &items,
                    &mut self.unit_prefix_cursor,
                    egui::vec2(100. * scale, 120. * scale),
                );
            });
            if self.memory.unit_prefix != old_prefix {
                ui.memory_mut(|m| m.request_focus(Self::picker_search_id(page)));
                self.error = None;
                self.dirty();
            }
            ui.small("Native units · calculations require the algebra helper");
        }
        if page == Page::Memory {
            ui.label(format!(
                "{} Home variables · {} CAS variables",
                snapshot.globals.len(),
                snapshot.cas.bindings.len()
            ));
            ui.label(format!(
                "{} programs · {} notes",
                snapshot.programs.len(),
                snapshot.notes.len()
            ));
            ui.small("Objects in the Native profile; firmware memory is separate.");
        }
        let choices = self.picker_choices(page);
        let selected = self.picker_state(page).2.to_owned();
        if !choices.iter().any(|c| c.key == selected) {
            *self.picker_selected_mut(page) =
                choices.first().map_or(String::new(), |c| c.key.into());
        }
        let selected = self.picker_state(page).2.to_owned();
        if matches!(page, Page::Toolbox | Page::Units) {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!choices.is_empty(), egui::Button::new("Insert"))
                    .clicked()
                {
                    requests.extend(self.picker_accept(page, &selected));
                }
                if page == Page::Toolbox
                    && ui
                        .add_enabled(!choices.is_empty(), egui::Button::new("Help"))
                        .clicked()
                {
                    self.memory.help_topic = selected.clone();
                    self.memory.help_query = selected.clone();
                    self.open(Page::Help);
                }
                ui.small(format!("{} matches", choices.len()));
            });
        }
        ui.small(if page == Page::Memory {
            "↑/↓ select · Enter or tap opens"
        } else {
            "↑/↓ select · Enter or tap inserts"
        });
        if choices.is_empty() {
            ui.label("No matching entries");
        }
        let scroll = std::mem::take(&mut self.picker_scroll_pending);
        let height = (lcd.bottom() - ui.cursor().top() - 6. * scale).max(1.);
        egui::ScrollArea::vertical()
            .id_salt(("native_picker_list", format!("{page:?}")))
            .max_height(height)
            .animated(false)
            .show(ui, |ui| {
                for choice in choices {
                    let response = ui.selectable_label(selected == choice.key, &choice.label);
                    if selected == choice.key && scroll {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        *self.picker_selected_mut(page) = choice.key.into();
                        requests.extend(self.picker_accept(page, choice.key));
                        self.dirty();
                    }
                }
            });
    }
}
