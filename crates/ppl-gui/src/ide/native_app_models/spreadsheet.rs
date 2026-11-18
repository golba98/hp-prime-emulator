//! Spreadsheet cells retain formulas separately from entered values. Numeric
//! operations use the VM's BCD values, never floating-point text substitution.
//! HP Prime User Guide 813269-002, pp.211–226 and 388–389.
mod access;
#[cfg(test)]
mod attribute_tests;
mod attributes;
mod choose;
mod clipboard;
pub mod format;
mod regression_report;
mod reports;
mod sort;
pub(crate) mod worksheet_cas;
use crate::ide::native_geometry::Book;
pub use access::AccessResult;
pub use choose::CellChoiceMenu;
pub use clipboard::{ClipboardBlock, PasteMode};
use ppl_math::{Real, RealPresentation};
use ppl_vm::{PortableValue, SessionContext, SessionSnapshot, Settings, Value};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const MAX_ROWS: usize = 10_000;
pub const MAX_COLUMNS: usize = 676;
const MAX_POPULATED_CELLS: usize = 10_000;
const MAX_FORMAT_RULES: usize = 1024;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spreadsheet {
    pub cells: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    row_entries: BTreeMap<usize, HeaderEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    column_entries: BTreeMap<usize, HeaderEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<format::FormatRule>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub column_widths: BTreeMap<usize, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub row_heights: BTreeMap<usize, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, format::Block>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    entered_values: BTreeMap<String, serde_json::Value>,
    /// Format-only cells remain visually blank but have a calculator record.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    metadata_cells: BTreeSet<String>,
    /// 0 retains the legacy Native Auto-color interpretation until a write.
    #[serde(default, skip_serializing_if = "is_legacy_format")]
    format_encoding: u8,
}
fn is_legacy_format(value: &u8) -> bool {
    *value == 0
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CellValue {
    Number(f64),
    Text(String),
    Object {
        display: String,
        typed: serde_json::Value,
    },
}
impl CellValue {
    /// Recover a typed matrix/list/object for display without reparsing its label.
    pub fn portable_value(&self) -> Option<PortableValue> {
        match self {
            Self::Number(value) => Real::from_f64(*value).ok().map(PortableValue::Real),
            Self::Text(value) => Some(PortableValue::String(value.clone())),
            Self::Object { typed, .. } => crate::ide::native::home_format::recover_value(typed),
        }
    }
    fn from_value(value: &Value, settings: &Settings) -> Self {
        match value {
            Value::Real(n)
                if n.presentation() == RealPresentation::Decimal
                    && Real::from_f64(n.to_f64()).is_ok_and(|roundtrip| roundtrip == *n) =>
            {
                Self::Number(n.to_f64())
            }
            Value::Str(s) => Self::Text(s.to_string()),
            _ => Self::Object {
                display: value.string_form(settings.format()),
                typed: crate::ide::native::portable_json(&PortableValue::from(value)),
            },
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Address {
    col: usize,
    row: usize,
}
impl Address {
    fn parse(name: &str) -> Option<Self> {
        let name = name.strip_prefix('$').unwrap_or(name);
        let split = name.bytes().position(|c| c.is_ascii_digit() || c == b'$')?;
        if split == 0 {
            return None;
        }
        let col = column(&name[..split])?;
        let row_text = name[split..].strip_prefix('$').unwrap_or(&name[split..]);
        if row_text.is_empty() || !row_text.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let row = row_text.parse::<usize>().ok()?;
        (row > 0 && row <= MAX_ROWS).then_some(Self { col, row })
    }
    fn name(self) -> String {
        cell_name(self.col, self.row)
    }
}
fn column(name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let mut col = 0usize;
    for ch in name.bytes() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        col = col
            .checked_mul(26)?
            .checked_add((ch.to_ascii_uppercase() - b'A' + 1) as usize)?;
    }
    (col <= MAX_COLUMNS).then_some(col)
}
pub fn cell_coordinates(name: &str) -> Option<(usize, usize)> {
    Address::parse(name).map(|a| (a.col, a.row))
}
pub fn cell_name(mut col: usize, row: usize) -> String {
    let mut letters = Vec::new();
    while col > 0 {
        letters.push((b'A' + ((col - 1) % 26) as u8) as char);
        col = (col - 1) / 26;
    }
    letters.reverse();
    format!("{}{row}", letters.into_iter().collect::<String>())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Header {
    Row(usize),
    Column(usize),
}
impl Header {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        if let Some(col) = column(name) {
            return Some(Self::Column(col));
        }
        if !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()) {
            let row = name.parse().ok()?;
            if (1..=MAX_ROWS).contains(&row) {
                return Some(Self::Row(row));
            }
        }
        None
    }
    pub(crate) fn name(self) -> String {
        match self {
            Self::Row(row) => row.to_string(),
            Self::Column(col) => cell_name(col, 1).trim_end_matches('1').into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum HeaderEntry {
    Formula(String),
    Value {
        source: String,
        typed: serde_json::Value,
    },
}
impl HeaderEntry {
    fn source(&self) -> &str {
        match self {
            Self::Formula(source) | Self::Value { source, .. } => source,
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.source().trim().is_empty() || self.source().len() > 65536 {
            return Err("Enter a row or column formula or value of up to 64KiB".into());
        }
        match self {
            Self::Formula(source) => {
                if source
                    .strip_prefix('=')
                    .is_none_or(|formula| formula.trim().is_empty())
                {
                    return Err("Enter a formula after =".into());
                }
            }
            Self::Value { typed, .. } => {
                ppl_vm::session::decode_value(typed)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Range {
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
}
impl Range {
    fn parse(source: &str) -> Option<Self> {
        if let Some((first, last)) = source.split_once(':') {
            let a = Address::parse(first.trim())?;
            let b = Address::parse(last.trim())?;
            Some(Self {
                left: a.col.min(b.col),
                right: a.col.max(b.col),
                top: a.row.min(b.row),
                bottom: a.row.max(b.row),
            })
        } else {
            Address::parse(source).map(|a| Self {
                left: a.col,
                right: a.col,
                top: a.row,
                bottom: a.row,
            })
        }
    }
    fn contains(self, a: Address) -> bool {
        (self.left..=self.right).contains(&a.col) && (self.top..=self.bottom).contains(&a.row)
    }
}
fn validate_name(name: &str) -> Result<(), String> {
    if name.len() > 32
        || name.is_empty()
        || !name.as_bytes()[0].is_ascii_alphabetic()
        || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || Address::parse(name).is_some()
        || ["Row", "Col", "Ans"].contains(&name)
    {
        return Err("Choose a name of up to32 letters/digits, starting with a letter".into());
    }
    Ok(())
}
impl Spreadsheet {
    pub fn set_name(&mut self, name: &str, block: format::Block) -> Result<(), String> {
        block.validate()?;
        let name = name.trim();
        validate_name(name)?;
        if !self.names.contains_key(name) && self.names.len() >= 1024 {
            return Err("Spreadsheet name limit reached".into());
        }
        self.names.insert(name.into(), block);
        Ok(())
    }
    /// The local record, before row/column/sheet defaults. Auto is retained.
    pub fn local_format(&self, col: usize, row: usize) -> format::CellFormat {
        self.scoped_format(col, row, 3)
    }
    fn scoped_format(&self, col: usize, row: usize, scope: usize) -> format::CellFormat {
        let mut result = format::CellFormat::default();
        for rule in &self.formats {
            if rule.block.scope() == scope && rule.block.contains(col, row) {
                result.overlay(&rule.style.decoded(self.format_encoding));
            }
        }
        result
    }
    /// Resolve local overrides over column, row and sheet defaults. A later
    /// Auto clears that scope's override rather than hiding its inherited value.
    pub fn format(&self, col: usize, row: usize) -> format::CellFormat {
        let mut result = format::CellFormat::defaults();
        for scope in 0..=3 {
            result.overlay_explicit(&self.scoped_format(col, row, scope));
        }
        result
    }
    fn upgrade_format_encoding(&mut self) {
        if self.format_encoding == 0 {
            for rule in &mut self.formats {
                rule.style = rule.style.decoded(0);
            }
            self.format_encoding = format::CURRENT_ENCODING;
        }
    }
    pub fn apply_format(
        &mut self,
        block: format::Block,
        style: format::CellFormat,
    ) -> Result<(), String> {
        block.validate()?;
        style.validate()?;
        let style = style.decoded(format::CURRENT_ENCODING);
        let coalesces = self.formats.last().is_some_and(|last| last.block == block);
        if self.formats.len() >= MAX_FORMAT_RULES && !coalesces {
            return Err("Spreadsheet format limit reached".into());
        }
        // Upgrade only old rules, before appending the new raw ARGB value.
        // Reading/serializing a legacy profile never rewrites its old colors.
        self.upgrade_format_encoding();
        if let Some(last) = self.formats.last_mut().filter(|last| last.block == block) {
            last.style.overlay(&style);
            return Ok(());
        }
        self.formats.push(format::FormatRule { block, style });
        Ok(())
    }
    pub(crate) fn has_header_entries(&self) -> bool {
        !self.row_entries.is_empty() || !self.column_entries.is_empty()
    }
    pub(crate) fn editor_source(&self, target: &str) -> Option<&str> {
        match Header::parse(target) {
            Some(Header::Row(row)) => self.row_entries.get(&row).map(HeaderEntry::source),
            Some(Header::Column(col)) => self.column_entries.get(&col).map(HeaderEntry::source),
            None => self.cells.get(target).map(String::as_str),
        }
    }
    fn inherited_entry(&self, address: Address) -> Option<&HeaderEntry> {
        self.column_entries
            .get(&address.col)
            .or_else(|| self.row_entries.get(&address.row))
    }
    pub(crate) fn has_value(&self, col: usize, row: usize) -> bool {
        self.cells.contains_key(&cell_name(col, row))
            || self.inherited_entry(Address { col, row }).is_some()
    }
    pub(crate) fn clear_header(&mut self, header: Header) {
        let (index, entries) = self.header_entries_mut(header);
        entries.remove(&index);
    }
    fn header_entries_mut(&mut self, header: Header) -> (usize, &mut BTreeMap<usize, HeaderEntry>) {
        match header {
            Header::Row(row) => (row, &mut self.row_entries),
            Header::Column(col) => (col, &mut self.column_entries),
        }
    }
    fn used_extent(&self) -> (usize, usize) {
        let columns = self
            .column_entries
            .last_key_value()
            .map_or(0, |(&col, _)| col);
        let rows = self.row_entries.last_key_value().map_or(0, |(&row, _)| row);
        let cell_corners = self
            .formats
            .iter()
            .filter(|rule| rule.block.scope() == 3)
            .map(|rule| Address {
                col: rule.block.right,
                row: rule.block.bottom,
            });
        self.cells
            .keys()
            .chain(&self.metadata_cells)
            .filter_map(|name| Address::parse(name))
            .chain(cell_corners)
            .fold((columns, rows), |(last_col, last_row), cell| {
                (last_col.max(cell.col), last_row.max(cell.row))
            })
    }
    fn inherited_addresses(
        &self,
        block: Range,
        addresses: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        if block.left > block.right || block.top > block.bottom {
            return Ok(());
        }
        let mut add = |col, row| -> Result<(), String> {
            addresses.insert(cell_name(col, row));
            if addresses.len() > MAX_POPULATED_CELLS {
                return Err("Spreadsheet range exceeds 10000 populated cells".into());
            }
            Ok(())
        };
        for (&row, _) in self.row_entries.range(block.top..=block.bottom) {
            for col in block.left..=block.right {
                add(col, row)?;
            }
        }
        for (&col, _) in self.column_entries.range(block.left..=block.right) {
            for row in block.top..=block.bottom {
                add(col, row)?;
            }
        }
        Ok(())
    }
    pub fn clear_block(&mut self, block: format::Block) {
        self.metadata_cells.retain(|name| {
            !self.cells.contains_key(name)
                || !Address::parse(name).is_some_and(|a| block.contains(a.col, a.row))
        });
        self.cells
            .retain(|n, _| !Address::parse(n).is_some_and(|a| block.contains(a.col, a.row)));
        self.entered_values
            .retain(|n, _| self.cells.contains_key(n));
    }
    pub fn set(&mut self, name: &str, source: &str) -> Result<(), String> {
        let name = Address::parse(name).ok_or("Invalid cell address")?.name();
        self.entered_values.remove(&name);
        if source.is_empty() {
            if self.cells.remove(&name).is_some() {
                self.metadata_cells.remove(&name);
            }
        } else {
            self.cells.insert(name, source.into());
        }
        Ok(())
    }
    fn set_typed(&mut self, name: &str, value: &PortableValue) -> Result<(), String> {
        let encoded = ppl_vm::session::encode_value(value);
        ppl_vm::session::decode_value(&encoded)?;
        self.cells.insert(
            name.into(),
            value
                .to_value()
                .string_form(ppl_vm::value::default_format()),
        );
        self.entered_values.insert(name.into(), encoded);
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.format_encoding > format::CURRENT_ENCODING {
            return Err("Unknown spreadsheet format encoding".into());
        }
        if self.metadata_cells.len() > 10_000
            || self
                .metadata_cells
                .iter()
                .any(|name| Address::parse(name).is_none_or(|address| address.name() != *name))
        {
            return Err("Invalid spreadsheet metadata cells".into());
        }
        if self.names.len() > 1024 {
            return Err("Spreadsheet name limit exceeded".into());
        }
        for (name, block) in &self.names {
            validate_name(name)?;
            block.validate()?;
        }
        if self.formats.len() > MAX_FORMAT_RULES {
            return Err("Spreadsheet format limit exceeded".into());
        }
        for rule in &self.formats {
            rule.block.validate()?;
            rule.style.validate()?;
        }
        if self
            .column_widths
            .iter()
            .any(|(&k, &v)| k == 0 || k > MAX_COLUMNS || !(20..=300).contains(&v))
            || self
                .row_heights
                .iter()
                .any(|(&k, &v)| k == 0 || k > MAX_ROWS || !(12..=160).contains(&v))
        {
            return Err("Invalid spreadsheet cell dimensions".into());
        }
        for (entries, limit) in [
            (&self.row_entries, MAX_ROWS),
            (&self.column_entries, MAX_COLUMNS),
        ] {
            for (&index, entry) in entries {
                if !(1..=limit).contains(&index) {
                    return Err("Invalid spreadsheet formula header".into());
                }
                entry.validate()?;
            }
        }
        if self.cells.len() > MAX_POPULATED_CELLS {
            return Err("Spreadsheet exceeds populated-cell limit".into());
        }
        for name in self.cells.keys() {
            if Address::parse(name)
                .ok_or("Invalid saved cell address")?
                .name()
                != *name
            {
                return Err("Saved cell address is not canonical".into());
            }
        }
        for (name, value) in &self.entered_values {
            if !self.cells.contains_key(name) {
                return Err("Entered value has no worksheet cell".into());
            }
            ppl_vm::session::decode_value(value)?;
        }
        Ok(())
    }
    pub fn evaluate(&self, name: &str, settings: &Settings) -> Result<CellValue, String> {
        let mut snapshot = SessionContext::default().snapshot();
        snapshot.settings = settings.clone();
        self.evaluate_in(name, &snapshot, Arc::new(AtomicBool::new(false)))
    }
    pub fn evaluate_in(
        &self,
        name: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<CellValue, String> {
        self.evaluate_in_with_geometry(name, snapshot, cancel, None)
    }
    pub fn evaluate_in_with_geometry(
        &self,
        name: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<CellValue, String> {
        let address = Address::parse(name).ok_or("Invalid cell address")?;
        let name = address.name();
        if !self.cells.contains_key(&name) && self.inherited_entry(address).is_none() {
            return Err(format!("Cell {name} is empty"));
        }
        let mut eval = Evaluator::new(self, snapshot, cancel, geometry)?;
        let value = eval.cell(&name)?;
        Ok(CellValue::from_value(&value, &eval.context.settings))
    }
    pub fn calculate(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<serde_json::Value, String> {
        self.calculate_with_geometry(snapshot, cancel, None)
    }
    pub fn calculate_with_geometry(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<serde_json::Value, String> {
        self.calculate_for_view(snapshot, cancel, geometry, None)
    }
    pub(crate) fn calculate_for_view(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
        viewport: Option<format::Block>,
    ) -> Result<serde_json::Value, String> {
        let mut evaluator = Evaluator::new(self, snapshot, cancel, geometry)?;
        let mut addresses = BTreeSet::new();
        if let Some(block) = viewport {
            self.inherited_addresses(
                Range {
                    left: block.left,
                    right: block.right,
                    top: block.top,
                    bottom: block.bottom,
                },
                &mut addresses,
            )?;
        }
        addresses.extend(self.cells.keys().cloned());
        let mut values = serde_json::Map::new();
        for name in addresses {
            evaluator.check()?;
            let mut cell = match evaluator.cell(&name) {
                Ok(value) => {
                    serde_json::to_value(CellValue::from_value(&value, &evaluator.context.settings))
                        .map_err(|e| e.to_string())?
                }
                Err(error) => serde_json::json!({"Error":error}),
            };
            if evaluator.unselected_choices.contains(&name) {
                cell["UnselectedChoice"] = serde_json::Value::Bool(true);
            }
            values.insert(name, cell);
        }
        Ok(serde_json::Value::Object(values))
    }
    /// Entering without '=' freezes the result. A retained formula is evaluated
    /// against current calculator memory on each recalculation.
    pub fn enter(
        &self,
        name: &str,
        source: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        self.enter_with_geometry(name, source, snapshot, cancel, None)
    }
    pub fn enter_with_geometry(
        &self,
        name: &str,
        source: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<Self, String> {
        let mut sheet =
            self.enter_cell(name, source, snapshot, cancel.clone(), geometry.clone())?;
        sheet.refresh_reports(snapshot, cancel, geometry)?;
        Ok(sheet)
    }
    fn enter_cell(
        &self,
        name: &str,
        source: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<Self, String> {
        let mut next = self.clone();
        if let Some(header) = Header::parse(name) {
            let source = source.trim();
            if source.is_empty() {
                next.clear_header(header);
                return Ok(next);
            }
            if source.len() > 65536 {
                return Err("Enter a row or column formula or value of up to 64KiB".into());
            }
            let entry = if source.starts_with('=') {
                HeaderEntry::Formula(source.into())
            } else {
                let address = match header {
                    Header::Row(row) => Address { col: 0, row },
                    Header::Column(col) => Address { col, row: 0 },
                };
                let mut eval = Evaluator::new(self, snapshot, cancel, geometry)?;
                let value = eval.expression(source, address, 0)?;
                HeaderEntry::Value {
                    source: value.string_form(ppl_vm::value::default_format()),
                    typed: ppl_vm::session::encode_value(&PortableValue::from(&value)),
                }
            };
            entry.validate()?;
            let (index, entries) = next.header_entries_mut(header);
            entries.insert(index, entry);
            return Ok(next);
        }
        let address = Address::parse(name).ok_or("Invalid cell address")?;
        let name = address.name();
        if source.trim().is_empty() {
            next.set(&name, "")?;
            return Ok(next);
        }
        if source.trim_start().starts_with('=') {
            next.set(&name, source)?;
            return Ok(next);
        }
        let mut eval = Evaluator::new(self, snapshot, cancel, geometry)?;
        if let Some(body) = reports::body(&format!("={source}")) {
            let table = eval.report_table(body, address)?;
            next.store_report(address, table, false)?;
            return Ok(next);
        }
        let value = eval.expression(source, address, 0)?;
        if worksheet_cas::body(source).is_some()
            && let Some(formula) = eval.cas_frozen_formula(&value, address)?
        {
            next.set(&name, &format!("={formula}"))?;
            return Ok(next);
        }
        next.cells.insert(
            name.clone(),
            value.string_form(ppl_vm::value::default_format()),
        );
        next.entered_values.insert(
            name,
            ppl_vm::session::encode_value(&PortableValue::from(&value)),
        );
        Ok(next)
    }
}
struct Evaluator<'a> {
    deadline: std::time::Instant,
    sheet: &'a Spreadsheet,
    context: SessionContext,
    home_settings: Settings,
    cancel: Arc<AtomicBool>,
    geometry: Option<Arc<Book>>,
    visiting: BTreeSet<String>,
    cache: BTreeMap<String, Result<Value, String>>,
    unselected_choices: BTreeSet<String>,
    prefix: String,
    slots: usize,
    used_extent: (usize, usize),
}
impl<'a> Evaluator<'a> {
    fn new(
        sheet: &'a Spreadsheet,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<Self, String> {
        if let Some(book) = &geometry {
            book.validate()?;
        }
        let settings = snapshot.settings.clone();
        let mut context = SessionContext::new(settings.clone(), 0x5348454554);
        context.active_app = "Spreadsheet".into();
        context.app_settings = snapshot.app_settings.clone();
        context.app_numeric_settings = snapshot.app_numeric_settings.clone();
        context.app_plot_settings = snapshot.app_plot_settings.clone();
        context.geometry_plot_settings = snapshot.geometry_plot_settings.clone();
        context.app_files = snapshot.app_files.clone();
        context.cas = snapshot.cas.clone();
        for (name, value) in &snapshot.globals {
            if name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit() {
                continue;
            }
            context
                .set_variable(name, value.clone())
                .map_err(|e| e.to_string())?;
        }
        // A live caller can expose HAngle/AAngle slots in its globals. Those
        // aliases must not overwrite the worksheet's own calculation mode.
        context.settings = settings;
        context.app_settings = snapshot.app_settings.clone();
        context.app_numeric_settings = snapshot.app_numeric_settings.clone();
        context.app_plot_settings = snapshot.app_plot_settings.clone();
        context.geometry_plot_settings = snapshot.geometry_plot_settings.clone();
        for (name, source) in &snapshot.programs {
            context
                .store_program(name, source.clone())
                .map_err(|e| e.to_string())?;
        }
        let mut prefix = "PPLSHEETINTERNAL".to_string();
        while snapshot
            .globals
            .keys()
            .any(|name| name.starts_with(&prefix))
            || snapshot
                .programs
                .values()
                .map(String::as_str)
                .chain(sheet.cells.values().map(String::as_str))
                .chain(sheet.row_entries.values().map(HeaderEntry::source))
                .chain(sheet.column_entries.values().map(HeaderEntry::source))
                .any(|source| source.contains(&prefix))
        {
            prefix.push('X');
        }
        Ok(Self {
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(4),
            sheet,
            context,
            home_settings: snapshot.settings.clone(),
            cancel,
            geometry,
            visiting: BTreeSet::new(),
            cache: BTreeMap::new(),
            unselected_choices: BTreeSet::new(),
            prefix,
            slots: 0,
            used_extent: sheet.used_extent(),
        })
    }
    fn check(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::Acquire) {
            Err("Calculation interrupted".into())
        } else if std::time::Instant::now() >= self.deadline {
            Err("Spreadsheet calculation timed out".into())
        } else {
            Ok(())
        }
    }
    fn cell(&mut self, name: &str) -> Result<Value, String> {
        self.check()?;
        let address = Address::parse(name).ok_or("Invalid cell reference")?;
        let name = address.name();
        if let Some(value) = self.cache.get(&name) {
            return value.clone();
        }
        if self.visiting.len() >= 256 {
            return Err("Spreadsheet dependency depth exceeded".into());
        }
        if !self.visiting.insert(name.clone()) {
            return Err(format!("Circular spreadsheet reference at {name}"));
        }
        let result = (|| {
            if let Some(value) = self.sheet.entered_values.get(&name) {
                return Ok(ppl_vm::session::decode_value(value)?.to_value());
            }
            let source = if let Some(source) = self.sheet.cells.get(&name) {
                source
            } else {
                match self.sheet.inherited_entry(address) {
                    Some(HeaderEntry::Formula(source)) => source,
                    Some(HeaderEntry::Value { typed, .. }) => {
                        return Ok(ppl_vm::session::decode_value(typed)?.to_value());
                    }
                    None => return Ok(Value::Real(Real::ZERO)),
                }
            }
            .to_owned();
            let text = source.trim();
            if let Some(body) = reports::body(text) {
                self.report_table(body, address)?
                    .into_iter()
                    .next()
                    .and_then(|row| row.into_iter().next())
                    .ok_or_else(|| "Worksheet report has no output".into())
            } else if let Some(body) = choose::body(text) {
                let choice = self.choose_definition(body, address)?;
                let current = choice.current(self)?;
                if let Some(index) = current {
                    Ok(choice.items[index - 1].clone())
                } else {
                    self.unselected_choices.insert(name.clone());
                    Ok(Value::zero())
                }
            } else if let Some(formula) = text.strip_prefix('=') {
                self.expression(formula, address, 0)
            } else if let Some(real) = Real::parse(text) {
                Ok(Value::Real(real))
            } else if text.starts_with('"') {
                self.expression(text, address, 0)
            } else {
                Ok(Value::str(source.strip_prefix('\'').unwrap_or(&source)))
            }
        })();
        self.visiting.remove(&name);
        self.cache.insert(name, result.clone());
        result
    }
    fn slot(&mut self, value: Value) -> Result<String, String> {
        let name = format!("{}{}", self.prefix, self.slots);
        self.slots += 1;
        self.context
            .set_variable(&name, PortableValue::from(&value))
            .map_err(|e| e.to_string())?;
        Ok(name)
    }
    fn named_range(&self, block: format::Block) -> Range {
        let (columns, rows) = self.used_extent;
        Range {
            left: block.left,
            right: if block.scope() == 1 {
                columns
            } else {
                block.right
            },
            top: block.top,
            bottom: if block.scope() == 2 {
                rows
            } else {
                block.bottom
            },
        }
    }
    fn named_value(&mut self, block: format::Block) -> Result<Value, String> {
        if block.left == block.right && block.top == block.bottom {
            return self.cell(&cell_name(block.left, block.top));
        }
        match block.scope() {
            1 if block.top == block.bottom => {}
            2 if block.left == block.right => {}
            _ => return Err("Use a named block inside a spreadsheet aggregate".into()),
        }
        let range = self.named_range(block);
        let values = (range.top..=range.bottom)
            .flat_map(|row| (range.left..=range.right).map(move |col| (col, row)))
            .map(|(col, row)| self.cell(&cell_name(col, row)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::list(values))
    }
    fn aggregate(
        &mut self,
        name: &str,
        body: &str,
        address: Address,
        depth: usize,
    ) -> Result<Value, String> {
        let mut values = Vec::new();
        let mut non_numeric_range = false;
        for argument in formula_arguments(body)? {
            let named = self
                .sheet
                .names
                .get(argument)
                .map(|&block| self.named_range(block));
            if let Some(range) = named.or_else(|| Range::parse(argument)) {
                // Empty range members do not contribute to either sum or count.
                // Iterate sparse occupied cells, including the current cell so a
                // self-reference is detected rather than silently omitted.
                let mut addresses = self
                    .sheet
                    .cells
                    .keys()
                    .filter(|name| Address::parse(name).is_some_and(|a| range.contains(a)))
                    .cloned()
                    .collect::<BTreeSet<_>>();
                self.sheet.inherited_addresses(range, &mut addresses)?;
                let mut names = addresses.into_iter().collect::<Vec<_>>();
                names.sort_by_key(|name| {
                    let a = Address::parse(name).unwrap();
                    (a.row, a.col)
                });
                for cell in names {
                    let value = self.cell(&cell)?;
                    if !matches!(value, Value::Real(_) | Value::Int(_) | Value::Complex(_)) {
                        non_numeric_range = true;
                    }
                    values.push(value);
                }
            } else {
                values.push(self.expression(argument, address, depth + 1)?);
            }
        }
        if non_numeric_range {
            return Err(format!("{name} range contains a non-numeric object"));
        }
        let average = name.eq_ignore_ascii_case("AVERAGE");
        if values.is_empty() && average {
            return Err("AVERAGE has no numeric cells".into());
        }
        ppl_vm::builtins::math::sheet_aggregate(&values, &self.context.settings, average, || {
            self.check()
                .map_err(|e| ppl_vm::RuntimeError::new(ppl_vm::ErrorKind::Interrupted, e))
        })
        .map_err(|e| e.to_string())
    }
    fn expand(&mut self, source: &str, address: Address, depth: usize) -> Result<String, String> {
        let mut out = String::new();
        let mut pos = 0;
        while pos < source.len() {
            self.check()?;
            let ch = source[pos..].chars().next().unwrap();
            if ch.is_ascii_digit()
                || (ch == '.' && source[pos + 1..].starts_with(|c: char| c.is_ascii_digit()))
            {
                let start = pos;
                pos = number_end(source, start);
                out.push_str(&source[start..pos]);
                continue;
            }
            if ch == '"' {
                let end = quoted_end(source, pos)?;
                out.push_str(&source[pos..end]);
                pos = end;
                continue;
            }
            if ch.is_alphabetic() || matches!(ch, '_' | '$') {
                let start = pos;
                pos = identifier_end(source, start);
                let token = &source[start..pos];
                let next = source[pos..].trim_start();
                if source[pos..].starts_with('.') || source[..start].ends_with('.') {
                    out.push_str(token);
                } else if (token.eq_ignore_ascii_case("SUM")
                    || token.eq_ignore_ascii_case("AVERAGE"))
                    && next.starts_with('(')
                {
                    let open = source.len() - next.len();
                    let end = call_end(source, open)?;
                    let value =
                        self.aggregate(token, &source[open + 1..end - 1], address, depth)?;
                    out.push_str(&self.slot(value)?);
                    pos = end;
                } else if next.starts_with('(')
                    && let Some(cell) = self.sheet.address(token)
                {
                    let open = source.len() - next.len();
                    let end = call_end(source, open)?;
                    let value = self.expression(&source[open + 1..end - 1], address, depth + 1)?;
                    let field = attributes::index(&PortableValue::from(&value))
                        .map_err(|error| error.to_string())?;
                    let value = self.attribute(cell, field)?;
                    out.push_str(&self.slot(value)?);
                    pos = end;
                } else if !next.starts_with('(') {
                    if let Some(cell) = Address::parse(token) {
                        let value = self.cell(&cell.name())?;
                        out.push_str(&self.slot(value)?);
                    } else if let Some(block) = self.sheet.names.get(token).copied() {
                        let value = self.named_value(block)?;
                        out.push_str(&self.slot(value)?);
                    } else if let Some(value) = self.home_setting(token) {
                        out.push_str(&value.to_string());
                    } else if token == "Row" {
                        out.push_str(&address.row.to_string());
                    } else if token == "Col" {
                        out.push_str(&address.col.to_string());
                    } else if token.eq_ignore_ascii_case("pi") {
                        out.push('π');
                    } else {
                        out.push_str(token);
                    }
                } else {
                    out.push_str(token);
                }
            } else {
                out.push(ch);
                pos += ch.len_utf8();
            }
        }
        Ok(out)
    }
    fn home_setting(&self, name: &str) -> Option<i64> {
        let settings = &self.home_settings;
        Some(match name.to_ascii_lowercase().as_str() {
            "hangle" => settings.angle.code(),
            "hformat" => settings.hformat,
            "hdigits" => settings.hdigits,
            "hcomplex" => i64::from(settings.complex_results),
            "base" => match settings.int_base {
                ppl_math::IntBase::Bin => 0,
                ppl_math::IntBase::Oct => 1,
                ppl_math::IntBase::Dec => 2,
                ppl_math::IntBase::Hex => 3,
            },
            "bits" => i64::from(settings.int_bits),
            "signed" => i64::from(settings.int_signed),
            _ => return None,
        })
    }
    fn expression(
        &mut self,
        source: &str,
        address: Address,
        depth: usize,
    ) -> Result<Value, String> {
        let first_slot = self.slots;
        let result = self.expression_inner(source, address, depth);
        for slot in first_slot..self.slots {
            self.context
                .delete_variable(&format!("{}{slot}", self.prefix));
        }
        result
    }
    fn expression_inner(
        &mut self,
        source: &str,
        address: Address,
        depth: usize,
    ) -> Result<Value, String> {
        self.check()?;
        if depth > 64 {
            return Err("Spreadsheet formula nesting exceeded".into());
        }
        if let Some(body) = worksheet_cas::body(source) {
            return self.cas_expression(&body, address, depth);
        }
        let expression = self.expand(source, address, depth)?;
        let (ast, diagnostics) = ppl_syntax::parse_expression(&expression);
        if diagnostics.has_errors() {
            return Err(diagnostics.errors().next().unwrap().message.clone());
        }
        if has_assignment(&ast) {
            return Err("Assignments inside spreadsheet formulas are not supported".into());
        }
        validate_text_addition(&ast, &self.context.snapshot().globals)?;
        let before = self.context.snapshot();
        let result = self
            .context
            .evaluate_home(
                &expression,
                crate::ide::native_application_access::geometry::read_only_host(
                    self.geometry.clone(),
                    self.cancel.clone(),
                    self.deadline,
                    "Spreadsheet calculation timed out",
                ),
                ppl_vm::Limits {
                    max_steps: Some(1_000_000),
                    max_time: Some(std::time::Duration::from_secs(2)),
                    ..Default::default()
                },
            )
            .result
            .map_err(|e| e.to_string());
        let after = self.context.snapshot();
        let wrote = after
            .globals
            .iter()
            .any(|(name, value)| name != "Ans" && before.globals.get(name) != Some(value))
            || before
                .globals
                .keys()
                .any(|name| !after.globals.contains_key(name))
            || !same_settings(&before.settings, &after.settings)
            || before.active_app != after.active_app
            || before.app_settings != after.app_settings
            || before.app_numeric_settings != after.app_numeric_settings
            || before.app_plot_settings != after.app_plot_settings
            || before.geometry_plot_settings != after.geometry_plot_settings
            || before.app_files != after.app_files
            || before.notes != after.notes
            || before.programs != after.programs
            || before.cas.bindings != after.cas.bindings;
        if wrote {
            self.context = Evaluator::new(
                self.sheet,
                &before,
                self.cancel.clone(),
                self.geometry.clone(),
            )?
            .context;
            return Err("Spreadsheet function attempted to change calculator memory".into());
        }
        self.context.settings = before.settings.clone();
        self.context.app_settings = before.app_settings.clone();
        self.context.app_numeric_settings = before.app_numeric_settings.clone();
        self.context.app_plot_settings = before.app_plot_settings.clone();
        self.context.geometry_plot_settings = before.geometry_plot_settings.clone();
        self.context.app_files = before.app_files.clone();
        self.context.cas = before.cas.clone();
        for name in after
            .globals
            .keys()
            .filter(|name| !before.globals.contains_key(*name))
        {
            self.context.delete_variable(name);
        }
        for (name, value) in before.globals {
            if !(name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit()) {
                self.context
                    .set_variable(&name, value)
                    .map_err(|e| e.to_string())?;
            }
        }
        result
    }
}
fn same_settings(a: &Settings, b: &Settings) -> bool {
    a.angle == b.angle
        && a.number_format == b.number_format
        && a.hformat == b.hformat
        && a.hdigits == b.hdigits
        && a.complex_results == b.complex_results
        && a.complex_display == b.complex_display
        && a.digit_grouping == b.digit_grouping
        && a.int_base == b.int_base
        && a.int_bits == b.int_bits
        && a.int_signed == b.int_signed
        && a.font_px == b.font_px
        && a.exp_char == b.exp_char
}
// The VM permits implicit string conversion for program compatibility. In a
// worksheet, documented + operands are two strings or numeric objects.
fn validate_text_addition(
    expression: &ppl_syntax::ast::Expr,
    variables: &BTreeMap<String, PortableValue>,
) -> Result<Option<bool>, String> {
    use ppl_syntax::ast::{BinOp, ExprKind};
    let kind = match &expression.kind {
        ExprKind::Str(_) => Some(true),
        ExprKind::Number(_) | ExprKind::HashInt(_) | ExprKind::List(_) | ExprKind::Bracket(_) => {
            Some(false)
        }
        ExprKind::Ident(name) => variables
            .get(name)
            .map(|v| matches!(v, PortableValue::String(_))),
        ExprKind::Binary(op, left, right) => {
            let left = validate_text_addition(left, variables)?;
            let right = validate_text_addition(right, variables)?;
            if *op == BinOp::Add
                && matches!(
                    (left, right),
                    (Some(true), Some(false)) | (Some(false), Some(true))
                )
            {
                return Err("Addition requires two strings or numeric objects".into());
            }
            if *op == BinOp::Add && left == Some(true) && right == Some(true) {
                Some(true)
            } else {
                None
            }
        }
        ExprKind::Unary(_, e) | ExprKind::Postfix(_, e) | ExprKind::Quoted(e) => {
            validate_text_addition(e, variables)?;
            None
        }
        ExprKind::Call(_, args) | ExprKind::Index(_, args) => {
            for arg in args {
                validate_text_addition(arg, variables)?;
            }
            None
        }
        _ => None,
    };
    Ok(kind)
}
fn number_end(source: &str, mut pos: usize) -> usize {
    while pos < source.len()
        && (source.as_bytes()[pos].is_ascii_digit() || source.as_bytes()[pos] == b'.')
    {
        pos += 1;
    }
    if pos < source.len() && matches!(source.as_bytes()[pos], b'E' | b'e') {
        pos += 1;
        if pos < source.len() && matches!(source.as_bytes()[pos], b'+' | b'-') {
            pos += 1;
        }
        while pos < source.len() && source.as_bytes()[pos].is_ascii_digit() {
            pos += 1;
        }
    }
    pos
}
fn identifier_end(source: &str, mut pos: usize) -> usize {
    pos += source[pos..].chars().next().unwrap().len_utf8();
    while pos < source.len() {
        let c = source[pos..].chars().next().unwrap();
        if !(c.is_alphanumeric() || matches!(c, '_' | '$')) {
            break;
        }
        pos += c.len_utf8();
    }
    pos
}
fn quoted_end(source: &str, start: usize) -> Result<usize, String> {
    let mut escaped = false;
    for (offset, ch) in source[start + 1..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Ok(start + 1 + offset + 1);
        }
    }
    Err("Unclosed string in spreadsheet formula".into())
}
fn formula_arguments(source: &str) -> Result<Vec<&str>, String> {
    let mut args = Vec::new();
    if source.trim().is_empty() {
        return Ok(args);
    }
    let mut depth = 0usize;
    let mut pos = 0;
    let mut start = 0;
    while pos < source.len() {
        let ch = source[pos..].chars().next().unwrap();
        match ch {
            '"' => {
                pos = quoted_end(source, pos)?;
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.checked_sub(1).ok_or("Unmatched closing delimiter")?,
            ',' if depth == 0 => {
                args.push(source[start..pos].trim());
                start = pos + 1;
            }
            _ => {}
        }
        pos += ch.len_utf8();
    }
    args.push(source[start..].trim());
    if depth != 0 || args.contains(&"") {
        return Err("Invalid worksheet function arguments".into());
    }
    Ok(args)
}
fn call_end(source: &str, open: usize) -> Result<usize, String> {
    let mut depth = 0;
    let mut pos = open;
    while pos < source.len() {
        let ch = source[pos..].chars().next().unwrap();
        if ch == '"' {
            pos = quoted_end(source, pos)?;
            continue;
        }
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return Ok(pos + 1);
            }
        }
        pos += ch.len_utf8();
    }
    Err("Unclosed function in spreadsheet formula".into())
}
fn has_assignment(e: &ppl_syntax::ast::Expr) -> bool {
    use ppl_syntax::ast::ExprKind::*;
    match &e.kind {
        Assign(..) | Store(..) => true,
        Unary(_, e) | Postfix(_, e) | Quoted(e) => has_assignment(e),
        Binary(_, a, b) | ComplexPair(a, b) => has_assignment(a) || has_assignment(b),
        Call(f, args) | Index(f, args) => has_assignment(f) || args.iter().any(has_assignment),
        List(values) | Bracket(values) => values.iter().any(has_assignment),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> SessionSnapshot {
        SessionContext::default().snapshot()
    }
    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }
    fn number(sheet: &Spreadsheet, name: &str, snapshot: &SessionSnapshot) -> f64 {
        match sheet.evaluate_in(name, snapshot, no_cancel()).unwrap() {
            CellValue::Number(n) => n,
            other => panic!("Expected numeric cell: {other:?}"),
        }
    }
    fn triangle(snapshot: &SessionSnapshot) -> Arc<Book> {
        use crate::ide::native_geometry::Object;
        let mut book = Book {
            objects: [
                ("A", "point(0,0)"),
                ("B", "point(4,0)"),
                ("C", "point(0,3)"),
                ("D", "triangle(GA,GB,GC)"),
                ("E", "distance(GB,GC)"),
            ]
            .into_iter()
            .map(|(name, source)| Object {
                name: name.into(),
                source: source.into(),
                ..Default::default()
            })
            .collect(),
            ..Default::default()
        };
        book.refresh_values(snapshot, no_cancel()).unwrap();
        Arc::new(book)
    }

    #[test]
    fn stored_geometry_formulas_follow_redraw_and_entered_values_stay_fixed() {
        use ppl_vm::host::AppVariableRequest;

        let mut memory = snapshot();
        memory
            .globals
            .insert("GE".into(), PortableValue::Real(Real::from_i64(99)));
        let geometry = triangle(&memory);
        let mut request = AppVariableRequest {
            app: "Spreadsheet".into(),
            name: "A1".into(),
            indices: vec![PortableValue::Real(Real::ONE)],
            value: Some(PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(
                "Geometry.GE+2".into(),
            ))),
        };
        let mut sheet = Spreadsheet::default()
            .access_with_geometry(&request, &memory, no_cancel(), Some(geometry.clone()))
            .unwrap()
            .sheet
            .unwrap();
        request.value = None;
        assert!(matches!(
            sheet
                .access_with_geometry(&request, &memory, no_cancel(), Some(geometry.clone()))
                .unwrap()
                .value,
            PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) if source == "Geometry.GE+2"
        ));
        request.indices.clear();
        assert_eq!(
            sheet
                .access_with_geometry(&request, &memory, no_cancel(), Some(geometry.clone()))
                .unwrap()
                .value,
            PortableValue::Real(Real::from_i64(7))
        );

        sheet.set("D1", "21").unwrap();
        sheet
            .set_name("Geometry", format::Block::cell(4, 1))
            .unwrap();
        sheet
            .set("C1", "=distance(Geometry.GB,Geometry.GC)")
            .unwrap();
        sheet.set("E1", "=GE+Geometry").unwrap();
        sheet = sheet
            .enter_with_geometry(
                "B1",
                "Geometry.GE+2",
                &memory,
                no_cancel(),
                Some(geometry.clone()),
            )
            .unwrap();

        let mut redrawn = geometry.clone();
        let book = Arc::make_mut(&mut redrawn);
        book.objects[2].source = "point(4,6)".into();
        book.refresh_values(&memory, no_cancel()).unwrap();
        let first = sheet
            .calculate_with_geometry(&memory, no_cancel(), Some(geometry))
            .unwrap();
        let second = sheet
            .calculate_with_geometry(&memory, no_cancel(), Some(redrawn))
            .unwrap();
        assert_eq!(
            first,
            serde_json::json!({"A1":{"Number":7.0}, "B1":{"Number":7.0}, "C1":{"Number":5.0},
                "D1":{"Number":21.0}, "E1":{"Number":120.0}})
        );
        assert_eq!(
            second,
            serde_json::json!({"A1":{"Number":8.0}, "B1":{"Number":7.0}, "C1":{"Number":6.0},
                "D1":{"Number":21.0}, "E1":{"Number":120.0}})
        );
    }

    #[test]
    fn worksheet_geometry_bridge_denies_setters_and_no_value_deletion() {
        let mut context = SessionContext::default();
        context
            .store_program(
                "GWRITE",
                "EXPORT GWRITE() BEGIN Geometry.GE:=9; RETURN 1; END;".into(),
            )
            .unwrap();
        let memory = context.snapshot();
        let geometry = triangle(&memory);
        let fingerprint = geometry.access_fingerprint();
        let mut sheet = Spreadsheet::default();
        for (cell, source) in [("A1", "=GWRITE()"), ("B1", "=Geometry.DelInstruction()")] {
            sheet.set(cell, source).unwrap();
            let error = sheet
                .evaluate_in_with_geometry(cell, &memory, no_cancel(), Some(geometry.clone()))
                .unwrap_err();
            assert!(
                error.contains("cannot change Geometry objects or instructions"),
                "{error}"
            );
        }
        sheet.set("C1", "=Geometry.GE").unwrap();
        assert_eq!(
            sheet
                .evaluate_in_with_geometry("C1", &memory, no_cancel(), Some(geometry.clone()))
                .unwrap(),
            CellValue::Number(5.)
        );
        assert_eq!(geometry.access_fingerprint(), fingerprint);
    }

    #[test]
    fn bare_column_letter_in_aggregate_reads_home_with_inherited_entries() {
        let mut memory = snapshot();
        memory
            .globals
            .insert("B".into(), PortableValue::Real(Real::from_i64(91)));
        let sheet = Spreadsheet::default()
            .enter("B", "=10+Row", &memory, no_cancel())
            .unwrap();
        let mut sheet = sheet.enter("3", "=20+Col", &memory, no_cancel()).unwrap();
        sheet.set("B2", "777").unwrap();
        sheet.set("C1", "12").unwrap();
        sheet.set("E2", "=SUM(B)").unwrap();
        sheet.set("E4", "=SUM(B1:B5)").unwrap();
        sheet.set("E5", "=AVERAGE(C1:C3)").unwrap();
        assert_eq!(number(&sheet, "E2", &memory), 91.);
        assert_eq!(number(&sheet, "E4", &memory), 830.);
        assert_eq!(number(&sheet, "E5", &memory), 17.5);
    }

    #[test]
    fn named_header_ranges_follow_the_used_sheet_extent() {
        let memory = snapshot();
        let mut sheet = Spreadsheet::default()
            .enter("B", "=10+Row", &memory, no_cancel())
            .unwrap();
        sheet.set("A1", "10").unwrap();
        sheet.set("B2", "777").unwrap();
        sheet.set("C1", "12").unwrap();
        sheet.set("D6", "=InputCol").unwrap();
        sheet.set("D7", "=SUM(InputCol)").unwrap();
        sheet
            .set_name(
                "InputCol",
                format::Block {
                    left: 2,
                    right: 2,
                    top: 1,
                    bottom: MAX_ROWS,
                },
            )
            .unwrap();
        assert_eq!(number(&sheet, "D7", &memory), 863.);
        let list = |sheet: &Spreadsheet, cell: &str| {
            sheet
                .evaluate_in(cell, &memory, no_cancel())
                .unwrap()
                .portable_value()
                .unwrap()
        };
        let numbers = |contents: &[i64]| {
            PortableValue::List(
                contents
                    .iter()
                    .map(|&n| PortableValue::Real(Real::from_i64(n)))
                    .collect(),
            )
        };
        assert_eq!(list(&sheet, "D6"), numbers(&[11, 777, 13, 14, 15, 16, 17]));
        sheet.set("C8", "=TopLine").unwrap();
        sheet.set("C9", "=SUM(TopLine)").unwrap();
        sheet
            .set_name(
                "TopLine",
                format::Block {
                    left: 1,
                    right: MAX_COLUMNS,
                    top: 1,
                    bottom: 1,
                },
            )
            .unwrap();
        assert_eq!(list(&sheet, "C8"), numbers(&[10, 11, 12, 0]));
        assert_eq!(
            sheet
                .read_attribute("TopLine", 0, &memory, no_cancel())
                .unwrap(),
            list(&sheet, "C8")
        );
        assert_eq!(number(&sheet, "C9", &memory), 33.);
        assert_eq!(number(&sheet, "D7", &memory), 900.);
        sheet.set("A15", "17").unwrap();
        assert_eq!(number(&sheet, "D7", &memory), 1035.);
        assert_eq!(
            list(&sheet, "D6"),
            numbers(&[11, 777, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25])
        );
        let mut sheet = sheet
            .write_attribute("A25", 5, &PortableValue::Real(Real::from_i64(2)))
            .unwrap();
        sheet = sheet
            .write_attribute("F4", 5, &PortableValue::Real(Real::from_i64(2)))
            .unwrap();
        assert_eq!(list(&sheet, "C8"), numbers(&[10, 11, 12, 0, 0, 0]));
        assert_eq!(number(&sheet, "C9", &memory), 33.);
        let PortableValue::List(column) = list(&sheet, "D6") else {
            panic!("Expected column list")
        };
        assert_eq!(column.len(), 25);
        assert_eq!(column[24], PortableValue::Real(Real::from_i64(35)));
        assert_eq!(
            sheet
                .read_attribute("InputCol", 0, &memory, no_cancel())
                .unwrap(),
            PortableValue::List(column)
        );
    }

    #[test]
    fn used_extent_includes_persisted_bounded_formats_without_counting_header_defaults() {
        let sheet: Spreadsheet = serde_json::from_value(serde_json::json!({
            "cells": {"A1": "10"},
            "formats": [
                {"block": {"left": 2, "right": 4, "top": 6, "bottom": 8}, "style": {"font": 14}},
                {"block": {"left": 10, "right": 10, "top": 1, "bottom": MAX_ROWS}, "style": {"font": 16}},
                {"block": {"left": 1, "right": MAX_COLUMNS, "top": 30, "bottom": 30}, "style": {"font": 16}},
            ]
        })).unwrap();
        sheet.validate().unwrap();
        assert_eq!(sheet.used_extent(), (4, 8));
        assert!(!sheet.has_value(4, 8));
    }

    #[test]
    fn header_formulas_inherit_by_destination_and_preserve_local_records() {
        use ppl_vm::host::AppVariableRequest;

        let memory = snapshot();
        let mut sheet = Spreadsheet::default();
        for (name, source) in [
            ("A1", "10"),
            ("A2", "20"),
            ("A3", "30"),
            ("C1", "100"),
            ("C3", "300"),
        ] {
            sheet.set(name, source).unwrap();
        }
        sheet = sheet
            .write_attribute("B2", 0, &PortableValue::Real(Real::from_i64(777)))
            .unwrap();
        sheet = sheet.enter("B", "=A1+Row", &memory, no_cancel()).unwrap();
        sheet = sheet.enter("3", "=A2+Col", &memory, no_cancel()).unwrap();
        assert_eq!(sheet.cells.len(), 6);
        for (address, expected) in [
            ("A3", 30.),
            ("B1", 11.),
            ("B2", 777.),
            ("B3", 13.),
            ("B100", 110.),
            ("C3", 300.),
            ("D3", 24.),
            ("AZ3", 72.),
        ] {
            assert_eq!(number(&sheet, address, &memory), expected, "{address}");
        }
        let request = |address: &str| AppVariableRequest {
            app: "Spreadsheet".into(),
            name: address.into(),
            indices: Vec::new(),
            value: None,
        };
        assert_eq!(
            sheet
                .access(&request("B100"), &memory, no_cancel())
                .unwrap()
                .value,
            PortableValue::Real(Real::from_i64(110))
        );
        assert_eq!(
            sheet.read_attribute("B1", 1, &memory, no_cancel()).unwrap(),
            PortableValue::Real(Real::from_i64(-1))
        );
        sheet = sheet
            .write_attribute("B4", 5, &PortableValue::Real(Real::from_i64(2)))
            .unwrap();
        assert_eq!(
            sheet.read_attribute("B4", 1, &memory, no_cancel()).unwrap(),
            PortableValue::Real(Real::ZERO)
        );
        assert_eq!(number(&sheet, "B4", &memory), 14.);
        sheet = sheet
            .write_attribute("B5", 0, &PortableValue::String(String::new()))
            .unwrap();
        assert_eq!(
            sheet
                .access(&request("B5"), &memory, no_cancel())
                .unwrap()
                .value,
            PortableValue::String(String::new())
        );

        sheet.clear_block(format::Block::cell(2, 2));
        assert_eq!(number(&sheet, "B2", &memory), 12.);
        assert_eq!(
            sheet.read_attribute("B2", 1, &memory, no_cancel()).unwrap(),
            PortableValue::Real(Real::from_i64(-1))
        );
        sheet.set("C6", "=SUM(B1:B4)").unwrap();
        assert_eq!(number(&sheet, "C6", &memory), 50.);
        sheet.set("A1", "50").unwrap();
        let restored: Spreadsheet =
            serde_json::from_value(serde_json::to_value(sheet).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.editor_source("B"), Some("=A1+Row"));
        assert_eq!(restored.editor_source("3"), Some("=A2+Col"));
        assert_eq!(restored.editor_source("B2"), None);
        assert_eq!(number(&restored, "B2", &memory), 52.);
        assert_eq!(number(&restored, "C6", &memory), 210.);
    }

    #[test]
    fn non_formula_header_entry_freezes_at_header_coordinates() {
        let memory = snapshot();
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "50").unwrap();
        sheet.set("D4", "444").unwrap();
        sheet = sheet.enter("3", "=A1+Col", &memory, no_cancel()).unwrap();
        sheet = sheet.enter("D", "A1+2", &memory, no_cancel()).unwrap();
        sheet = sheet
            .enter("4", "Row*10+Col", &memory, no_cancel())
            .unwrap();
        sheet = sheet
            .enter("E", "Row*10+Col", &memory, no_cancel())
            .unwrap();
        sheet.set("A1", "60").unwrap();

        let mut restored: Spreadsheet =
            serde_json::from_value(serde_json::to_value(sheet).unwrap()).unwrap();
        restored.validate().unwrap();
        for (address, expected) in [
            ("D1", 52.),
            ("D3", 52.),
            ("D4", 444.),
            ("A4", 40.),
            ("E1", 5.),
        ] {
            assert_eq!(number(&restored, address, &memory), expected, "{address}");
        }
        assert_eq!(restored.editor_source("D"), Some("52"));
        restored.clear_header(Header::Column(4));
        assert_eq!(number(&restored, "D3", &memory), 64.);
        assert_eq!(number(&restored, "D4", &memory), 444.);
        assert!(restored.editor_source("D").is_none());
    }

    #[test]
    fn viewport_recalculation_does_not_expand_whole_formula_columns() {
        let memory = snapshot();
        let sheet = Spreadsheet::default()
            .enter("B", "=Row+1", &memory, no_cancel())
            .unwrap()
            .enter("E", "=E1+1", &memory, no_cancel())
            .unwrap();
        let values = sheet
            .calculate_for_view(
                &memory,
                no_cancel(),
                None,
                Some(format::Block {
                    left: 1,
                    right: 5,
                    top: 9000,
                    bottom: 9001,
                }),
            )
            .unwrap();
        assert_eq!(values.as_object().unwrap().len(), 4);
        assert_eq!(values["B9000"]["Number"], 9001.);
        assert_eq!(values["B9001"]["Number"], 9002.);
        assert!(
            values["E9000"]["Error"]
                .as_str()
                .unwrap()
                .contains("Circular spreadsheet reference")
        );
        assert_eq!(sheet.editor_source("E"), Some("=E1+1"));
    }

    #[test]
    fn retained_formulas_and_entered_values_are_independent_after_recalculation_and_reopen() {
        let mut sheet = Spreadsheet::default();
        let mut memory = snapshot();
        memory
            .globals
            .insert("P".into(), PortableValue::Real(Real::from_i64(10)));
        sheet = sheet.enter("A1", "P*5", &memory, no_cancel()).unwrap();
        sheet.set("B1", "=P*5").unwrap();
        memory
            .globals
            .insert("P".into(), PortableValue::Real(Real::from_i64(20)));
        assert_eq!(number(&sheet, "A1", &memory), 50.);
        assert_eq!(number(&sheet, "B1", &memory), 100.);
        let restored: Spreadsheet =
            serde_json::from_str(&serde_json::to_string(&sheet).unwrap()).unwrap();
        assert_eq!(number(&restored, "A1", &memory), 50.);
        assert_eq!(memory.globals["Ans"], snapshot().globals["Ans"]);
    }
    #[test]
    fn spreadsheet_uses_home_and_its_own_app_settings_instead_of_the_active_graph() {
        let mut memory = snapshot();
        memory.settings.angle = ppl_vm::AngleMode::Degrees;
        memory.active_app = "Function".into();
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "=SIN(90)").unwrap();
        assert_eq!(number(&sheet, "A1", &memory), 1.);
        memory.app_settings.insert(
            "Spreadsheet".into(),
            ppl_vm::AppSettings {
                angle: 3,
                ..Default::default()
            },
        );
        sheet.set("A1", "=SIN(100)").unwrap();
        assert_eq!(number(&sheet, "A1", &memory), 1.);
        sheet.set("A1", "=100*HAngle+10*AAngle+SIN(100)").unwrap();
        assert_eq!(number(&sheet, "A1", &memory), 131.);
        assert_eq!(memory.settings.angle, ppl_vm::AngleMode::Degrees);
    }
    #[test]
    fn stat1_writes_configured_headers_weighted_results_and_keeps_frozen_entries() {
        let mut sheet = Spreadsheet::default();
        let memory = snapshot();
        for (name, source) in [
            ("A1", "1"),
            ("A2", "3"),
            ("A3", "5"),
            ("B1", "1"),
            ("B2", "2"),
            ("B3", "4"),
        ] {
            sheet.set(name, source).unwrap();
        }
        sheet = sheet
            .enter("C1", "=STAT1(A1:B3,2,\"h n σ\")", &memory, no_cancel())
            .unwrap();
        assert_eq!(
            sheet.evaluate_in("C1", &memory, no_cancel()).unwrap(),
            CellValue::Text("n".into())
        );
        assert_eq!(number(&sheet, "D1", &memory), 7.);
        assert!((number(&sheet, "D2", &memory) - 1.45686271817).abs() < 1e-10);
        sheet = sheet
            .enter("C1", "=STAT1(A1:B3,1,\"H h n σ\")", &memory, no_cancel())
            .unwrap();
        assert_eq!(
            sheet.evaluate_in("D1", &memory, no_cancel()).unwrap(),
            CellValue::Text("A".into())
        );
        assert_eq!(
            sheet.evaluate_in("E1", &memory, no_cancel()).unwrap(),
            CellValue::Text("B".into())
        );
        assert_eq!(number(&sheet, "E2", &memory), 3.);
        sheet = sheet
            .enter("J1", "STAT1({1,3,5},\"h n σ\")", &memory, no_cancel())
            .unwrap();
        assert!(sheet.entered_values.contains_key("J1"));
        assert!((number(&sheet, "K2", &memory) - 1.63299316186).abs() < 1e-10);
        let stored = serde_json::to_vec(&sheet).unwrap();
        let mut sheet: Spreadsheet = serde_json::from_slice(&stored).unwrap();
        sheet = sheet.enter("C1", "17", &memory, no_cancel()).unwrap();
        assert!((number(&sheet, "E3", &memory) - 1.24721912892).abs() < 1e-10);
        assert!(
            sheet
                .enter("ZZ10000", "STAT1({1,3,5})", &memory, no_cancel())
                .is_err()
        );
    }

    #[test]
    fn stat2_recalculates_paired_columns_and_uses_case_sensitive_configuration() {
        let memory = snapshot();
        let mut sheet = Spreadsheet::default();
        for (name, value) in [
            ("A1", "1"),
            ("A2", "3"),
            ("A3", "5"),
            ("B1", "2"),
            ("B2", "4"),
            ("B3", "8"),
            ("C1", "1"),
            ("C2", "2"),
            ("C3", "4"),
        ] {
            sheet.set(name, value).unwrap();
        }
        sheet = sheet
            .enter("E1", "=STAT2(A1:C3)", &memory, no_cancel())
            .unwrap();
        assert_eq!(
            sheet.evaluate_in("F1", &memory, no_cancel()).unwrap(),
            CellValue::Text("A:B:C".into())
        );
        assert_eq!(number(&sheet, "F11", &memory), 7.);
        assert_eq!(number(&sheet, "F21", &memory), 186.);
        sheet = sheet.enter("A1", "20", &memory, no_cancel()).unwrap();
        assert_eq!(number(&sheet, "F21", &memory), 224.);
        sheet = sheet
            .enter("J1", "STAT2(A1:B3,\"h n σy\")", &memory, no_cancel())
            .unwrap();
        assert_eq!(number(&sheet, "K1", &memory), 3.);
        assert!((number(&sheet, "K2", &memory) - 2.49443825785).abs() < 1e-10);
        assert!(
            sheet
                .enter("J1", "STAT2(A1:B3,\"h σY\")", &memory, no_cancel())
                .is_err()
        );
        assert!(
            sheet
                .enter("J1", "STAT2(A1:C3,1)", &memory, no_cancel())
                .is_err()
        );
    }

    #[test]
    fn regression_report_uses_model_specific_coordinates_and_preserves_multi_pair_order() {
        let memory = snapshot();
        let mut sheet = Spreadsheet::default();
        for (row, [x, y]) in [[1, 2], [3, 4], [5, 8], [6, 11], [8, 17], [11, 25]]
            .iter()
            .enumerate()
        {
            for (c, value) in [*x, *y, *x, 2 * y].into_iter().enumerate() {
                sheet
                    .set(&cell_name(c + 1, row + 1), &value.to_string())
                    .unwrap();
            }
        }
        sheet = sheet
            .enter(
                "J1",
                "=REGRS(A1:D6,2,\"H h sl int cor cd sCov pCov\")",
                &memory,
                no_cancel(),
            )
            .unwrap();
        for (row, expected) in [
            8.77577255445,
            -1.96356595644,
            0.870658779777,
            0.758046710803,
            6.4064784461,
            5.33873203841,
        ]
        .into_iter()
        .enumerate()
        {
            assert!((number(&sheet, &cell_name(11, row + 2), &memory) - expected).abs() < 1e-9);
        }
        assert!((number(&sheet, "L2", &memory) - 17.5515451089).abs() < 1e-9);
        sheet = sheet
            .enter("N1", "REGRS(A1:B6,9,\"h a b c cd\")", &memory, no_cancel())
            .unwrap();
        for (row, expected) in [0.271691665085, 1.1400860705, 0.103933295361, 0.994499949869]
            .into_iter()
            .enumerate()
        {
            assert!((number(&sheet, &cell_name(15, row + 1), &memory) - expected).abs() < 1e-8);
        }
        for invalid in [
            "REGRS(A1:C6)",
            "REGRS(A1:B6,12)",
            "REGRS(A1:B6,2.5)",
            "REGRS(A1:B6,9,\"sl\")",
        ] {
            assert!(
                sheet.enter("N1", invalid, &memory, no_cancel()).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn aggregates_ignore_empty_cells_accept_reversed_blocks_and_reject_text_and_cycles() {
        let mut sheet = Spreadsheet::default();
        let memory = snapshot();
        sheet.set("A1", "2").unwrap();
        sheet.set("A2", "4").unwrap();
        sheet.set("B5", "=SUM(A1:B2)+average(B2:A1)").unwrap();
        assert_eq!(number(&sheet, "B5", &memory), 9.);
        sheet.set("B5", "=SUM(C1:C10000)").unwrap();
        assert_eq!(number(&sheet, "B5", &memory), 0.);
        sheet.set("B5", "=AVERAGE(C1:C10000)").unwrap();
        assert!(
            sheet
                .evaluate_in("B5", &memory, no_cancel())
                .unwrap_err()
                .contains("no numeric")
        );
        sheet.set("A3", "text").unwrap();
        sheet.set("B5", "=SUM(A1:A3)").unwrap();
        assert!(
            sheet
                .evaluate_in("B5", &memory, no_cancel())
                .unwrap_err()
                .contains("non-numeric")
        );
        sheet.set("B5", "=SUM(A1:B5)").unwrap();
        assert!(
            sheet
                .evaluate_in("B5", &memory, no_cancel())
                .unwrap_err()
                .contains("Circular")
        );
    }
    #[test]
    fn references_row_col_and_quoted_formula_text_keep_their_meaning() {
        let mut sheet = Spreadsheet::default();
        let memory = snapshot();
        sheet.set("A1", "2").unwrap();
        sheet.set("A2", "4").unwrap();
        sheet.set("B5", "=$A$1+A$2+$A2+$A$2").unwrap();
        assert_eq!(number(&sheet, "B5", &memory), 14.);
        sheet.set("B5", "=Row^(Col+1)").unwrap();
        assert_eq!(number(&sheet, "B5", &memory), 125.);
        sheet.set("B5", "=\"SUM(A1:A2)\"").unwrap();
        assert_eq!(
            sheet.evaluate_in("B5", &memory, no_cancel()).unwrap(),
            CellValue::Text("SUM(A1:A2)".into())
        );
    }
    #[test]
    fn pi_spellings_evaluate_without_changing_stored_formula_text() {
        let mut context = SessionContext::default();
        context.settings.numeric_version = ppl_math::NumericVersion::Current15751;
        let mut memory = context.snapshot();
        memory
            .globals
            .insert("pi_rate".into(), PortableValue::Real(Real::from_i64(9)));
        let mut sheet = Spreadsheet::default();
        for source in ["=π/3", "=pi/3", "=PI/3", "=Pi/3"] {
            sheet.set("A1", source).unwrap();
            let PortableValue::Real(value) =
                sheet.read_attribute("A1", 0, &memory, no_cancel()).unwrap()
            else {
                panic!("Expected numeric result for {source}");
            };
            assert_eq!(value.to_string(), "1.0471975512");
            assert_eq!(
                value.origin(),
                ppl_math::RealOrigin::PiMultiple {
                    numerator: 1,
                    denominator: 3,
                }
            );
            assert_eq!(sheet.cells["A1"], source);
        }
        sheet.set("A1", "=pi_rate/3").unwrap();
        assert_eq!(number(&sheet, "A1", &memory), 3.);
        sheet.set("A1", "=\"pi π PI pi_rate\"").unwrap();
        assert_eq!(
            sheet.evaluate_in("A1", &memory, no_cancel()).unwrap(),
            CellValue::Text("pi π PI pi_rate".into())
        );
    }
    #[test]
    fn ranges_use_bcd_arithmetic_and_values_do_not_overflow_through_f64() {
        let mut sheet = Spreadsheet::default();
        let memory = snapshot();
        for row in 1..=10 {
            sheet.set(&format!("A{row}"), "0.1").unwrap();
        }
        sheet.set("B1", "=SUM(A1:A10)").unwrap();
        assert_eq!(number(&sheet, "B1", &memory), 1.);
        sheet.set("A1", "1E400").unwrap();
        sheet.set("B1", "=A1/1E400").unwrap();
        assert_eq!(number(&sheet, "B1", &memory), 1.);
        assert!(
            matches!(sheet.evaluate_in("A1",&memory,no_cancel()).unwrap(),CellValue::Object{display,..} if display=="1E400")
        );
    }
    #[test]
    fn entered_collections_keep_typed_integers_and_text_after_reopen() {
        let sheet = Spreadsheet::default()
            .enter("A1", "{#FFh,\"text\",1/3}", &snapshot(), no_cancel())
            .unwrap();
        let restored: Spreadsheet =
            serde_json::from_str(&serde_json::to_string(&sheet).unwrap()).unwrap();
        let CellValue::Object { typed, .. } = restored
            .evaluate_in("A1", &snapshot(), no_cancel())
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(typed["value"][0]["type"], "integer");
        assert_eq!(typed["value"][1]["value"], "text");
        assert_eq!(typed["value"][2]["value"], "0.333333333333");
    }
    #[test]
    fn pure_installed_functions_can_read_home_values_without_promoting_ans() {
        let mut memory = snapshot();
        memory
            .globals
            .insert("P".into(), PortableValue::Real(Real::from_i64(10)));
        memory.programs.insert(
            "Double".into(),
            "EXPORT DOUBLE(X) BEGIN RETURN 2*X; END;".into(),
        );
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "=DOUBLE(P)").unwrap();
        assert_eq!(number(&sheet, "A1", &memory), 20.);
        assert_eq!(memory.globals["Ans"], snapshot().globals["Ans"]);
        sheet.set("A1", "=(P:=5)").unwrap();
        assert!(sheet.evaluate_in("A1", &memory, no_cancel()).is_err());
        assert_eq!(memory.globals["P"], PortableValue::Real(Real::from_i64(10)));
    }
    #[test]
    fn cancellation_and_documented_bounds_are_enforced_without_changing_the_sheet() {
        let mut sheet = Spreadsheet::default();
        sheet.set("YZ10000", "1").unwrap();
        assert!(sheet.set("ZA10001", "1").is_err());
        assert!(sheet.set("ZZ1", "1").is_err());
        assert!(
            sheet
                .calculate(&snapshot(), Arc::new(AtomicBool::new(true)))
                .is_err()
        );
        assert_eq!(sheet.cells.len(), 1);
    }
    #[test]
    fn tiny_bcd_values_and_temporary_bindings_survive_large_recalculations() {
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "1E-400").unwrap();
        for row in 2..=100 {
            sheet
                .set(&format!("A{row}"), &format!("=A{}+1E-400", row - 1))
                .unwrap();
        }
        let memory = snapshot();
        let mut eval = Evaluator::new(&sheet, &memory, no_cancel(), None).unwrap();
        let Value::Real(value) = eval.cell("A100").unwrap() else {
            panic!("expected BCD value");
        };
        assert_eq!(value, Real::parse("1E-398").unwrap());
        assert!(
            !eval
                .context
                .snapshot()
                .globals
                .keys()
                .any(|name| name.starts_with(&eval.prefix))
        );
        assert!(
            matches!(sheet.evaluate_in("A1", &memory, no_cancel()).unwrap(), CellValue::Object { display, .. } if display.contains("-400"))
        );
        for name in ["$$A1", "A1$", "A$$1", "A$1$", "A-1", "YZ10001", "ZA1"] {
            assert!(cell_coordinates(name).is_none(), "{name}");
        }
    }
    #[test]
    fn numeric_setting_and_custom_row_writes_cannot_escape_worksheet_evaluation() {
        for assignment in [
            "NumStart:=7",
            "NumStep:=2",
            "NumZoom:=3",
            "NumType:=1",
            "NumIndep:={2,4,6}",
        ] {
            let mut memory = snapshot();
            memory.programs.insert(
                "MUTATE".into(),
                format!("EXPORT MUTATE() BEGIN {assignment}; RETURN 7; END;"),
            );
            let before = memory.app_numeric_settings.clone();
            let mut sheet = Spreadsheet::default();
            sheet.set("A1", "=MUTATE()").unwrap();
            sheet.set("A2", "=NumStep").unwrap();
            let values = sheet.calculate(&memory, no_cancel()).unwrap();
            assert!(
                values["A1"]["Error"]
                    .as_str()
                    .unwrap()
                    .contains("change calculator memory"),
                "{assignment}"
            );
            assert_eq!(values["A2"]["Number"].as_f64(), Some(0.1));
            assert_eq!(memory.app_numeric_settings, before);
        }
    }
    #[test]
    fn installed_functions_cannot_change_settings_or_other_cells_during_recalculation() {
        let mut memory = snapshot();
        memory.programs.insert(
            "MUTATE".into(),
            "EXPORT MUTATE() BEGIN HAngle:=1; RETURN 7; END;".into(),
        );
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "=MUTATE()").unwrap();
        sheet.set("A2", "=SIN(90)").unwrap();
        let values = sheet.calculate(&memory, no_cancel()).unwrap();
        assert!(
            values["A1"]["Error"]
                .as_str()
                .unwrap()
                .contains("change calculator memory")
        );
        assert!((values["A2"]["Number"].as_f64().unwrap() - 1.).abs() > 0.1);
    }
    #[test]
    fn named_cells_blocks_and_scoped_formats_survive_reopen_without_changing_values() {
        use format::{Block, CellFormat};
        let mut sheet = Spreadsheet::default();
        sheet.set("A1", "2").unwrap();
        sheet.set("A2", "4").unwrap();
        sheet.set_name("TOTAL", Block::cell(1, 1)).unwrap();
        sheet
            .set_name(
                "DATA",
                Block {
                    left: 1,
                    right: 1,
                    top: 1,
                    bottom: 2,
                },
            )
            .unwrap();
        sheet.set("B1", "=TOTAL+SUM(DATA)").unwrap();
        sheet
            .apply_format(
                Block {
                    left: 1,
                    right: MAX_COLUMNS,
                    top: 1,
                    bottom: MAX_ROWS,
                },
                CellFormat {
                    color: Some(0xff0000),
                    number: Some(1),
                    digits: Some(6),
                    ..Default::default()
                },
            )
            .unwrap();
        sheet
            .apply_format(
                Block::cell(1, 1),
                CellFormat {
                    color: Some(0x0000ff),
                    fill: Some(0xffff00),
                    ..Default::default()
                },
            )
            .unwrap();
        sheet
            .apply_format(
                Block::cell(1, 1),
                CellFormat {
                    number: Some(-1),
                    ..Default::default()
                },
            )
            .unwrap();
        let restored: Spreadsheet =
            serde_json::from_str(&serde_json::to_string(&sheet).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(number(&restored, "B1", &snapshot()), 8.);
        assert_eq!(restored.local_format(1, 1).number, Some(-1));
        assert_eq!(restored.format(1, 1).number, Some(1));
        assert_eq!(restored.format(1, 1).color, Some(0x0000ff));
        assert_eq!(restored.format(2, 1).color, Some(0xff0000));
        let mut cleared = restored.clone();
        cleared.clear_block(Block::cell(1, 1));
        assert!(!cleared.cells.contains_key("A1"));
        assert!(cleared.cells.contains_key("A2"));
        assert_eq!(number(&cleared, "B1", &snapshot()), 4.);
        for name in ["A1", "Row", "Col", "Ans", "1bad", "bad.name", ""] {
            assert!(sheet.set_name(name, Block::cell(1, 1)).is_err());
        }
        sheet.row_heights.insert(0, 17);
        assert!(sheet.validate().is_err());
    }
}
