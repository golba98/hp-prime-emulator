//! Typed qualified access to the authoritative worksheet; no Home bindings.
use super::*;
use ppl_vm::{ErrorKind, RuntimeError, host::AppVariableRequest};
pub struct AccessResult {
    pub value: PortableValue,
    pub sheet: Option<Spreadsheet>,
}
fn bad(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, message.into())
}
fn unsupported(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::Unsupported, message.into())
}
fn integer(value: &PortableValue) -> Result<i64, RuntimeError> {
    value
        .to_value()
        .expect_real("Spreadsheet attribute")?
        .to_i64()
        .ok_or_else(|| bad("Spreadsheet attribute must be an integer"))
}
fn real(value: i64) -> PortableValue {
    PortableValue::Real(Real::from_i64(value))
}
impl Spreadsheet {
    pub fn access(
        &self,
        request: &AppVariableRequest,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<AccessResult, RuntimeError> {
        if request.app != "Spreadsheet" {
            return Err(unsupported(
                "This Native application does not provide qualified variables",
            ));
        }
        if cancel.load(Ordering::Acquire) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        let address = Address::parse(&request.name)
            .or_else(|| {
                self.names
                    .get(&request.name)
                    .filter(|b| b.left == b.right && b.top == b.bottom)
                    .map(|b| Address {
                        col: b.left,
                        row: b.top,
                    })
            })
            .ok_or_else(|| bad("Invalid Spreadsheet cell or single-cell name"))?;
        if request.indices.len() > 1 {
            return Err(unsupported(
                "Spreadsheet access accepts one attribute index",
            ));
        }
        let field = request
            .indices
            .first()
            .map(integer)
            .transpose()?
            .unwrap_or(0);
        let name = address.name();
        if let Some(value) = &request.value {
            let mut sheet = self.clone();
            match field {
                0 | 1 => {
                    if field == 1
                        && let PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) = value
                    {
                        if source.len() > 65536 || source.trim().is_empty() {
                            return Err(bad("Invalid Spreadsheet formula definition"));
                        }
                        sheet.set(&name, &format!("={source}")).map_err(bad)?;
                    } else {
                        let typed = ppl_vm::session::encode_value(value);
                        ppl_vm::session::decode_value(&typed).map_err(bad)?;
                        sheet.cells.insert(
                            name.clone(),
                            value
                                .to_value()
                                .string_form(ppl_vm::value::default_format()),
                        );
                        sheet.entered_values.insert(name, typed);
                    }
                }
                2 => {
                    let PortableValue::String(alias) = value else {
                        return Err(bad("Spreadsheet cell name must be text"));
                    };
                    let alias = alias.trim();
                    sheet
                        .set_name(alias, format::Block::cell(address.col, address.row))
                        .map_err(bad)?;
                    sheet.names.retain(|key, block| {
                        key == alias
                            || !(block.left == address.col
                                && block.right == address.col
                                && block.top == address.row
                                && block.bottom == address.row)
                    });
                }
                3 => {
                    let n = integer(value)?;
                    sheet
                        .apply_format(
                            format::Block::cell(address.col, address.row),
                            format::CellFormat {
                                number: Some(n),
                                ..Default::default()
                            },
                        )
                        .map_err(bad)?;
                }
                _ => {
                    return Err(unsupported(
                        "This Spreadsheet attribute is not writable in Native yet",
                    ));
                }
            }
            sheet.validate().map_err(bad)?;
            return Ok(AccessResult {
                value: value.clone(),
                sheet: Some(sheet),
            });
        }
        let value = match field {
            0 => {
                let mut evaluator = Evaluator::new(self, snapshot, cancel).map_err(bad)?;
                PortableValue::from(&evaluator.cell(&name).map_err(bad)?)
            }
            1 => {
                if let Some(source) = self
                    .cells
                    .get(&name)
                    .and_then(|s| s.trim_start().strip_prefix('='))
                    && !self.entered_values.contains_key(&name)
                {
                    return Ok(AccessResult {
                        value: PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source.into())),
                        sheet: None,
                    });
                }
                let mut evaluator = Evaluator::new(self, snapshot, cancel).map_err(bad)?;
                PortableValue::from(&evaluator.cell(&name).map_err(bad)?)
            }
            2 => self
                .names
                .iter()
                .find(|(_, b)| {
                    b.left == address.col
                        && b.right == address.col
                        && b.top == address.row
                        && b.bottom == address.row
                })
                .map(|(alias, _)| PortableValue::String(alias.clone()))
                .unwrap_or_else(|| {
                    if self.cells.contains_key(&name) {
                        PortableValue::String(String::new())
                    } else {
                        real(-1)
                    }
                }),
            3 => real(self.format(address.col, address.row).number.unwrap_or(-1)),
            _ => {
                return Err(unsupported(
                    "This Spreadsheet attribute is not readable in Native yet",
                ));
            }
        };
        Ok(AccessResult { value, sheet: None })
    }
}
