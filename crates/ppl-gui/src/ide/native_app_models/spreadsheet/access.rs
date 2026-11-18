//! Typed qualified access uses the same local attributes as worksheet formulas.
use super::*;
use ppl_vm::{ErrorKind, RuntimeError, host::AppVariableRequest};
#[derive(Debug)]
pub struct AccessResult {
    pub value: PortableValue,
    pub sheet: Option<Spreadsheet>,
}
impl Spreadsheet {
    pub fn access(
        &self,
        request: &AppVariableRequest,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<AccessResult, RuntimeError> {
        self.access_with_geometry(request, snapshot, cancel, None)
    }
    pub fn access_with_geometry(
        &self,
        request: &AppVariableRequest,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<AccessResult, RuntimeError> {
        if request.app != "Spreadsheet" {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                "This Native application does not provide qualified variables",
            ));
        }
        if cancel.load(Ordering::Acquire) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        if request.indices.len() > 1 {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                "Spreadsheet access accepts one attribute index",
            ));
        }
        let field = request
            .indices
            .first()
            .map(attributes::index)
            .transpose()?
            .unwrap_or(0);
        if let Some(value) = &request.value {
            let sheet = self.write_attribute(&request.name, field, value)?;
            return Ok(AccessResult {
                value: value.clone(),
                sheet: Some(sheet),
            });
        }
        if field == 0
            && let Some(address) = self.address(&request.name)
            && let Some(body) = self.cells.get(&address.name()).and_then(|s| stat1::body(s))
        {
            let invalid = |e: String| RuntimeError::new(ErrorKind::InvalidInput, e);
            let mut evaluator =
                Evaluator::new(self, snapshot, cancel, geometry).map_err(invalid)?;
            evaluator.visiting.insert(address.name());
            let table = evaluator.stat1_table(body, address).map_err(invalid)?;
            let value = PortableValue::from(&table[0][0]);
            let mut next = self.clone();
            let changed = next.store_stat1(address, table, true).map_err(invalid)?;
            return Ok(AccessResult {
                value,
                sheet: changed.then_some(next),
            });
        }
        Ok(AccessResult {
            value: self.read_attribute_with_geometry(
                &request.name,
                field,
                snapshot,
                cancel,
                geometry,
            )?,
            sheet: None,
        })
    }
}
