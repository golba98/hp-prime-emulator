//! One local attribute contract for qualified PPL access and worksheet formulas.
//! A supported setter materializes a blank record even when its value is ignored.
use super::*;
use ppl_vm::{ErrorKind, RuntimeError};

fn bad(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, message.into())
}
fn number(value: i64) -> PortableValue {
    PortableValue::Real(Real::from_i64(value))
}
pub(super) fn index(value: &PortableValue) -> Result<i64, RuntimeError> {
    let index = value
        .to_value()
        .as_real()
        .and_then(Real::to_i64)
        .ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::BadArgType,
                "Spreadsheet attribute must be an integer",
            )
        })?;
    check_index(index)?;
    Ok(index)
}
fn check_index(index: i64) -> Result<(), RuntimeError> {
    if (-1..=12).contains(&index) {
        Ok(())
    } else {
        Err(bad("Invalid Spreadsheet attribute"))
    }
}
fn integer(value: &PortableValue) -> Option<i64> {
    value.to_value().as_real()?.to_i64()
}
#[derive(Clone, Copy)]
enum WriteMode {
    Single,
    Prefix,
}
impl Spreadsheet {
    pub(super) fn address(&self, name: &str) -> Option<Address> {
        Address::parse(name).or_else(|| {
            self.names
                .get(name)
                .filter(|b| b.left == b.right && b.top == b.bottom)
                .map(|b| Address {
                    col: b.left,
                    row: b.top,
                })
        })
    }
    fn materialized(&self, address: Address) -> bool {
        let name = address.name();
        self.cells.contains_key(&name)
            || self.metadata_cells.contains(&name)
            || self
                .names
                .values()
                .any(|block| *block == format::Block::cell(address.col, address.row))
            || self.formats.iter().any(|rule| {
                rule.block.scope() == 3 && rule.block.contains(address.col, address.row)
            })
    }
    pub(super) fn materialize(&mut self, address: Address) -> Result<(), RuntimeError> {
        if !self.materialized(address) {
            if self.metadata_cells.len() >= 10_000 {
                return Err(bad("Spreadsheet metadata-cell limit reached"));
            }
            self.metadata_cells.insert(address.name());
        }
        Ok(())
    }
    pub fn read_attribute(
        &self,
        name: &str,
        field: i64,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<PortableValue, RuntimeError> {
        self.read_attribute_with_geometry(name, field, snapshot, cancel, None)
    }
    pub(super) fn read_attribute_with_geometry(
        &self,
        name: &str,
        field: i64,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<PortableValue, RuntimeError> {
        check_index(field)?;
        let address = self.address(name);
        let header = self.names.get(name).copied().filter(|_| field == 0);
        if address.is_none() && header.is_none() {
            return Err(bad("Invalid Spreadsheet cell or name"));
        }
        let mut evaluator = Evaluator::new(self, snapshot, cancel, geometry).map_err(bad)?;
        let value = if let Some(address) = address {
            evaluator.attribute(address, field)
        } else {
            evaluator.named_value(header.unwrap())
        }
        .map_err(bad)?;
        Ok(PortableValue::from(&value))
    }
    /// A complete cloned change for the worker's existing source/epoch fence.
    /// Prefix record setters commit earlier fields then stop at the first
    /// rejected member, unlike validation of an entire new record in advance.
    pub fn write_attribute(
        &self,
        name: &str,
        field: i64,
        value: &PortableValue,
    ) -> Result<Self, RuntimeError> {
        check_index(field)?;
        let address = self
            .address(name)
            .ok_or_else(|| bad("Invalid Spreadsheet cell or single-cell name"))?;
        let mut next = self.clone();
        next.materialize(address)?;
        if field == -1 {
            if let PortableValue::List(values) = value {
                for (index, value) in values.iter().take(11).enumerate() {
                    if !next.write_field(address, index as i64 + 1, value, WriteMode::Prefix)? {
                        break;
                    }
                }
            }
        } else {
            next.write_field(address, field, value, WriteMode::Single)?;
        }
        next.validate().map_err(bad)?;
        Ok(next)
    }
    fn write_field(
        &mut self,
        address: Address,
        field: i64,
        value: &PortableValue,
        mode: WriteMode,
    ) -> Result<bool, RuntimeError> {
        let name = address.name();
        match field {
            0 | 1 => {
                if let PortableValue::Symbolic(_) = value {
                    let source = crate::ide::native::value_source(&value.to_value());
                    if source.len() > 65536 || source.trim().is_empty() {
                        return Ok(false);
                    }
                    // A symbolic object in either setter is a retained formula
                    // on G1. The worksheet evaluator parses it after expanding
                    // its own ranges and cell references.
                    self.set(&name, &format!("={source}")).map_err(bad)?;
                } else {
                    self.set_typed(&name, value).map_err(bad)?;
                }
            }
            2 => {
                let alias = match value {
                    PortableValue::String(alias) => Some(alias.as_str()),
                    PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(alias)) => {
                        Some(alias.as_str())
                    }
                    _ => None,
                };
                let clear = integer(value) == Some(-1)
                    || matches!(mode, WriteMode::Prefix) && alias == Some("");
                if clear {
                    self.names
                        .retain(|_, block| *block != format::Block::cell(address.col, address.row));
                } else {
                    let Some(alias) = alias else { return Ok(false) };
                    if alias.len() > 8 || !alias.is_ascii() || validate_name(alias).is_err() {
                        return Ok(false);
                    }
                    // Keep the old target's metadata if an alias is reassigned.
                    if let Some(old) = self
                        .names
                        .get(alias)
                        .copied()
                        .filter(|b| b.left == b.right && b.top == b.bottom)
                    {
                        self.materialize(Address {
                            col: old.left,
                            row: old.top,
                        })?;
                        self.metadata_cells.insert(cell_name(old.left, old.top));
                    }
                    self.names
                        .retain(|_, block| *block != format::Block::cell(address.col, address.row));
                    self.set_name(alias, format::Block::cell(address.col, address.row))
                        .map_err(bad)?;
                }
            }
            3..=11 => {
                let Some(value) = integer(value) else {
                    return Ok(false);
                };
                let mut style = format::CellFormat::default();
                match field {
                    3 if (-1..=5).contains(&value) => style.number = Some(value),
                    4 if (-1..=11).contains(&value) => style.digits = Some(value),
                    5 if (-1..=6).contains(&value) => {
                        style.font = Some(if value == -1 { 0 } else { 10 + value as u8 * 2 })
                    }
                    6 | 7 if value == -1 || u32::try_from(value).is_ok() => {
                        let color = if value == -1 {
                            format::AUTO_COLOR
                        } else {
                            format::canonical_color(value as u32)
                        };
                        if field == 6 {
                            style.color = Some(color)
                        } else {
                            style.fill = Some(color)
                        }
                    }
                    8 | 9 if (-1..=2).contains(&value) => {
                        let alignment = if value == -1 { 3 } else { value as u8 };
                        if field == 8 {
                            style.horizontal = Some(alignment)
                        } else {
                            style.vertical = Some(alignment)
                        }
                    }
                    10 | 11 if (-1..=1).contains(&value) => {
                        let choice = (value + 1) as u8;
                        if field == 10 {
                            style.quotes = Some(choice)
                        } else {
                            style.textbook = Some(choice)
                        }
                    }
                    _ => return Ok(false),
                }
                self.apply_format(format::Block::cell(address.col, address.row), style)
                    .map_err(bad)?;
            }
            12 => {} // Observed read-only flag: writes ignored, record still exists.
            _ => return Err(bad("Invalid Spreadsheet attribute")),
        }
        Ok(true)
    }
}
impl Evaluator<'_> {
    pub(super) fn attribute(&mut self, address: Address, field: i64) -> Result<Value, String> {
        self.check()?;
        check_index(field).map_err(|e| e.to_string())?;
        let name = address.name();
        if field == 0 {
            return self.cell(&name);
        }
        if !self.sheet.materialized(address) {
            return Ok(number(-1).to_value());
        }
        if field == -1 {
            let fields = (1..=11)
                .map(|field| self.attribute(address, field))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::list(fields));
        }
        if field == 1 {
            if let Some(source) = self
                .sheet
                .cells
                .get(&name)
                .and_then(|source| source.trim_start().strip_prefix('='))
                && !self.sheet.entered_values.contains_key(&name)
            {
                return Ok(
                    PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source.into())).to_value(),
                );
            }
            return if self.sheet.cells.contains_key(&name) {
                self.cell(&name)
            } else {
                Ok(number(0).to_value())
            };
        }
        if field == 2 {
            let alias = self
                .sheet
                .names
                .iter()
                .find(|(_, block)| **block == format::Block::cell(address.col, address.row))
                .map(|(alias, _)| alias.as_str())
                .unwrap_or("");
            return Ok(Value::str(alias));
        }
        let local = self.sheet.local_format(address.col, address.row);
        let value = match field {
            3 => local.number.unwrap_or(-1),
            4 => local.digits.unwrap_or(-1),
            5 => local
                .font
                .filter(|&v| v != 0)
                .map(|v| i64::from((v - 10) / 2))
                .unwrap_or(-1),
            6 | 7 => if field == 6 { local.color } else { local.fill }
                .filter(|&v| v != format::AUTO_COLOR)
                .map(i64::from)
                .unwrap_or(-1),
            8 | 9 => if field == 8 {
                local.horizontal
            } else {
                local.vertical
            }
            .filter(|&v| v != 3)
            .map(i64::from)
            .unwrap_or(-1),
            10 | 11 => if field == 10 {
                local.quotes
            } else {
                local.textbook
            }
            .map(|v| i64::from(v) - 1)
            .unwrap_or(-1),
            12 => 0,
            _ => return Err("Invalid Spreadsheet attribute".into()),
        };
        Ok(number(value).to_value())
    }
}
