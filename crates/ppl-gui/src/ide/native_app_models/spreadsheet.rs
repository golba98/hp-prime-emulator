//! Spreadsheet cells retain formulas separately from entered values. Numeric
//! operations use the VM's BCD values, never floating-point text substitution.
//! HP Prime User Guide 813269-002, pp.211–226 and 388–389.
mod access;
pub mod format;
pub use access::AccessResult;
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spreadsheet {
    pub cells: BTreeMap<String, String>,
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
        } else if let Some(a) = Address::parse(source) {
            Some(Self {
                left: a.col,
                right: a.col,
                top: a.row,
                bottom: a.row,
            })
        } else {
            let col = column(source)?;
            Some(Self {
                left: col,
                right: col,
                top: 1,
                bottom: MAX_ROWS,
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
    pub fn format(&self, col: usize, row: usize) -> format::CellFormat {
        let mut result = format::CellFormat::default();
        for rule in &self.formats {
            if rule.block.contains(col, row) {
                result.overlay(&rule.style);
            }
        }
        result
    }
    pub fn apply_format(
        &mut self,
        block: format::Block,
        style: format::CellFormat,
    ) -> Result<(), String> {
        block.validate()?;
        style.validate()?;
        if self.formats.len() >= 1024 {
            return Err("Spreadsheet format limit reached".into());
        }
        self.formats.push(format::FormatRule { block, style });
        Ok(())
    }
    pub fn clear_block(&mut self, block: format::Block) {
        self.cells
            .retain(|n, _| !Address::parse(n).is_some_and(|a| block.contains(a.col, a.row)));
        self.entered_values
            .retain(|n, _| self.cells.contains_key(n));
    }
    pub fn set(&mut self, name: &str, source: &str) -> Result<(), String> {
        let name = Address::parse(name).ok_or("Invalid cell address")?.name();
        self.entered_values.remove(&name);
        if source.is_empty() {
            self.cells.remove(&name);
        } else {
            self.cells.insert(name, source.into());
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.names.len() > 1024 {
            return Err("Spreadsheet name limit exceeded".into());
        }
        for (name, block) in &self.names {
            validate_name(name)?;
            block.validate()?;
        }
        if self.formats.len() > 1024 {
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
        if self.cells.len() > 10_000 {
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
        let name = Address::parse(name).ok_or("Invalid cell address")?.name();
        if !self.cells.contains_key(&name) {
            return Err(format!("Cell {name} is empty"));
        }
        let mut eval = Evaluator::new(self, snapshot, cancel)?;
        let value = eval.cell(&name)?;
        Ok(CellValue::from_value(&value, &eval.context.settings))
    }
    pub fn calculate(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<serde_json::Value, String> {
        let mut evaluator = Evaluator::new(self, snapshot, cancel)?;
        let mut values = serde_json::Map::new();
        for name in self.cells.keys() {
            evaluator.check()?;
            let cell = match evaluator.cell(name) {
                Ok(value) => {
                    serde_json::to_value(CellValue::from_value(&value, &evaluator.context.settings))
                        .map_err(|e| e.to_string())?
                }
                Err(error) => serde_json::json!({"Error":error}),
            };
            values.insert(name.clone(), cell);
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
        let address = Address::parse(name).ok_or("Invalid cell address")?;
        let name = address.name();
        let mut next = self.clone();
        if source.trim().is_empty() {
            next.set(&name, "")?;
            return Ok(next);
        }
        if source.trim_start().starts_with('=') {
            next.set(&name, source)?;
            return Ok(next);
        }
        let mut eval = Evaluator::new(self, snapshot, cancel)?;
        let value = eval.expression(source, address, 0)?;
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
    visiting: BTreeSet<String>,
    cache: BTreeMap<String, Result<Value, String>>,
    prefix: String,
    slots: usize,
}
impl<'a> Evaluator<'a> {
    fn new(
        sheet: &'a Spreadsheet,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let settings = snapshot.settings.clone();
        let mut context = SessionContext::new(settings.clone(), 0x5348454554);
        context.active_app = "Spreadsheet".into();
        context.app_settings = snapshot.app_settings.clone();
        context.app_numeric_settings = snapshot.app_numeric_settings.clone();
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
                .chain(sheet.cells.values())
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
            visiting: BTreeSet::new(),
            cache: BTreeMap::new(),
            prefix,
            slots: 0,
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
            let Some(source) = self.sheet.cells.get(&name) else {
                return Ok(Value::Real(Real::ZERO));
            };
            let source = source.clone();
            let text = source.trim();
            if let Some(formula) = text.strip_prefix('=') {
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
    fn aggregate(
        &mut self,
        name: &str,
        body: &str,
        address: Address,
        depth: usize,
    ) -> Result<Value, String> {
        let mut values = Vec::new();
        let named = self.sheet.names.get(body.trim()).map(|b| Range {
            left: b.left,
            right: b.right,
            top: b.top,
            bottom: b.bottom,
        });
        if let Some(range) = named.or_else(|| Range::parse(body.trim())) {
            // Empty range members do not contribute to either sum or count.
            // Iterate sparse occupied cells, including the current cell so a
            // self-reference is detected rather than silently omitted.
            let mut names = self
                .sheet
                .cells
                .keys()
                .filter(|name| Address::parse(name).is_some_and(|a| range.contains(a)))
                .cloned()
                .collect::<Vec<_>>();
            names.sort_by_key(|name| {
                let a = Address::parse(name).unwrap();
                (a.row, a.col)
            });
            for cell in names {
                values.push(self.cell(&cell)?);
            }
        } else {
            match self.expression(body, address, depth + 1)? {
                Value::List(items) => values.extend(items.iter().cloned()),
                value => values.push(value),
            }
        }
        let operation = name.to_ascii_uppercase();
        if operation == "COUNT" {
            let count = values
                .iter()
                .filter(|value| matches!(value, Value::Real(_) | Value::Int(_) | Value::Complex(_)))
                .count();
            return Ok(Value::Real(Real::from_i64(count as i64)));
        }
        let count = values.len();
        let mut sum = Value::Real(if operation == "PRODUCT" {
            Real::ONE
        } else {
            Real::ZERO
        });
        for (index, value) in values.into_iter().enumerate() {
            self.check()?;
            if !matches!(value, Value::Real(_) | Value::Int(_) | Value::Complex(_)) {
                return Err(format!("{name} range contains a non-numeric object"));
            }
            use ppl_syntax::ast::BinOp;
            if operation == "MIN" || operation == "MAX" {
                if index == 0 {
                    sum = value;
                    continue;
                }
                let op = if operation == "MIN" {
                    BinOp::Lt
                } else {
                    BinOp::Gt
                };
                let selected = ppl_vm::ops::binary(op, &value, &sum, &self.context.settings)
                    .map_err(|e| e.to_string())?;
                if selected.as_real().is_some_and(|value| !value.is_zero()) {
                    sum = value;
                }
            } else {
                let op = if operation == "PRODUCT" {
                    BinOp::Mul
                } else {
                    BinOp::Add
                };
                sum = ppl_vm::ops::binary(op, &sum, &value, &self.context.settings)
                    .map_err(|e| e.to_string())?;
            }
        }
        if count == 0 && ["AVERAGE", "MIN", "MAX"].contains(&operation.as_str()) {
            return Err(format!("{name} has no numeric cells"));
        }
        if operation == "AVERAGE" {
            sum = ppl_vm::ops::binary(
                ppl_syntax::ast::BinOp::Div,
                &sum,
                &Value::Real(Real::from_i64(count as i64)),
                &self.context.settings,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(sum)
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
                pos += ch.len_utf8();
                while pos < source.len() {
                    let c = source[pos..].chars().next().unwrap();
                    if !(c.is_alphanumeric() || matches!(c, '_' | '$')) {
                        break;
                    }
                    pos += c.len_utf8();
                }
                let token = &source[start..pos];
                let next = source[pos..].trim_start();
                if ["SUM", "AVERAGE", "COUNT", "MIN", "MAX", "PRODUCT"]
                    .iter()
                    .any(|name| token.eq_ignore_ascii_case(name))
                    && next.starts_with('(')
                    && !source[..start].trim_end().ends_with('.')
                {
                    let open = source.len() - next.len();
                    let end = call_end(source, open)?;
                    let body = source[open + 1..end - 1].trim();
                    let range_only =
                        token.eq_ignore_ascii_case("MIN") || token.eq_ignore_ascii_case("MAX");
                    if range_only
                        && Range::parse(body).is_none()
                        && !self.sheet.names.contains_key(body)
                    {
                        // Leave ordinary scalar/list calls to the PPL compiler.
                        // In particular, parsing a comma-separated argument body
                        // as one expression would turn MIN(1,2) into a complex value.
                        out.push_str(token);
                    } else {
                        let value = self.aggregate(token, body, address, depth)?;
                        out.push_str(&self.slot(value)?);
                        pos = end;
                    }
                } else if !next.starts_with('(') && !source[..start].ends_with('.') {
                    if let Some(cell) = Address::parse(token) {
                        let value = self.cell(&cell.name())?;
                        out.push_str(&self.slot(value)?);
                    } else if let Some(block) = self.sheet.names.get(token).copied() {
                        if block.left != block.right || block.top != block.bottom {
                            return Err("Use a named block inside a spreadsheet aggregate".into());
                        }
                        let value = self.cell(&cell_name(block.left, block.top))?;
                        out.push_str(&self.slot(value)?);
                    } else if let Some(value) = self.home_setting(token) {
                        out.push_str(&value.to_string());
                    } else if token == "Row" {
                        out.push_str(&address.row.to_string());
                    } else if token == "Col" {
                        out.push_str(&address.col.to_string());
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
                Box::new(ppl_vm::HeadlessHost {
                    interrupt: Some(self.cancel.clone()),
                    ..Default::default()
                }),
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
            || before.app_files != after.app_files
            || before.notes != after.notes
            || before.programs != after.programs
            || before.cas.bindings != after.cas.bindings;
        if wrote {
            self.context = Evaluator::new(self.sheet, &before, self.cancel.clone())?.context;
            return Err("Spreadsheet function attempted to change calculator memory".into());
        }
        self.context.settings = before.settings.clone();
        self.context.app_settings = before.app_settings.clone();
        self.context.app_numeric_settings = before.app_numeric_settings.clone();
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
    fn ordinary_min_max_and_qualified_calls_keep_normal_ppl_semantics() {
        let mut sheet = Spreadsheet::default();
        let memory = snapshot();
        sheet.set("A1", "3").unwrap();
        sheet.set("A2", "5").unwrap();
        for (formula, expected) in [
            ("MIN(1,2)", 1.),
            ("MAX({1,2})", 2.),
            ("MIN(A1,A2)", 3.),
            ("MAX(A1:A2)", 5.),
            ("MIN({5,3})", 3.),
        ] {
            sheet.set("B1", &format!("={formula}")).unwrap();
            assert_eq!(number(&sheet, "B1", &memory), expected, "{formula}");
        }
        let mut evaluator = Evaluator::new(&sheet, &memory, no_cancel()).unwrap();
        for source in ["CAS.MIN(1,2)", "CAS.SUM({1,2})", "CAS . MAX({1,2})"] {
            assert_eq!(
                evaluator
                    .expand(source, Address::parse("B1").unwrap(), 0)
                    .unwrap(),
                source
            );
        }
    }
    #[test]
    fn spreadsheet_range_functions_keep_typed_values_sparse_cells_and_cycles() {
        let mut sheet = Spreadsheet::default();
        let memory = snapshot();
        for (cell, source) in [("A1", "0.1"), ("A2", "2"), ("A4", "3"), ("D1", "\"text\"")] {
            sheet.set(cell, source).unwrap();
        }
        for (formula, expected) in [
            ("COUNT(A1:A4)", 3.),
            ("PRODUCT(A1:A4)", 0.6),
            ("MIN(A1:A4)", 0.1),
            ("MAX(A1:A4)", 3.),
            ("COUNT(D1:D3)", 0.),
            ("PRODUCT(E1:E3)", 1.),
        ] {
            sheet.set("B1", &format!("={formula}")).unwrap();
            assert_eq!(number(&sheet, "B1", &memory), expected, "{formula}");
        }
        for formula in ["MIN(E1:E3)", "MAX(E1:E3)", "MIN(D1:D3)", "COUNT(B1:B2)"] {
            sheet.set("B1", &format!("={formula}")).unwrap();
            assert!(
                sheet.evaluate_in("B1", &memory, no_cancel()).is_err(),
                "{formula}"
            );
        }
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
        let mut eval = Evaluator::new(&sheet, &memory, no_cancel()).unwrap();
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
        assert_eq!(restored.format(1, 1).number, Some(-1));
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
