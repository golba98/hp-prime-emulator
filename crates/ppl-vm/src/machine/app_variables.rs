//! Calculator-owned app variables execute in the current VM, before any
//! presentation request. A qualified statistics column aliases Home memory;
//! sending it to a GUI cache would lose writes made during a live program.
use super::*;

impl Machine {
    pub(super) fn statistics_column(
        &mut self,
        name: &str,
        indices: &[crate::PortableValue],
        assigned: Option<crate::PortableValue>,
    ) -> RtResult<Value> {
        let indices = indices
            .iter()
            .map(crate::PortableValue::to_value)
            .collect::<Vec<_>>();
        let mut column = self
            .calculator_global(name)
            .cloned()
            .unwrap_or_else(|| Value::list(Vec::new()));
        if let Some(value) = assigned {
            let value = value.to_value();
            // Copy-on-write plus whole-column validation makes every rejected
            // element write atomic, including nested and append operations.
            store_index(
                &mut column,
                &indices,
                value.clone(),
                self.limits.max_collection_len,
            )?;
            self.store_home_value(name, column)?;
            Ok(value)
        } else {
            index_value(&column, &indices, self)
        }
    }
}
