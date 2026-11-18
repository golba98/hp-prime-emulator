//! The worksheet CHOOSE formula reads and writes a companion cell's index.
use super::{Address, Book, Evaluator, Spreadsheet, call_end, cell_name, formula_arguments};
use ppl_math::Real;
use ppl_vm::{PortableValue, SessionSnapshot, Value};
use serde::{Deserialize, Serialize};
use std::{
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct CellChoiceMenu {
    pub title: String,
    pub items: Vec<String>,
    pub current: usize,
}

pub(super) struct ChoiceDefinition {
    target: String,
    title: String,
    pub(super) items: Rc<Vec<Value>>,
}

impl ChoiceDefinition {
    pub(super) fn current(&self, evaluator: &mut Evaluator<'_>) -> Result<Option<usize>, String> {
        let index = evaluator
            .cell(&self.target)?
            .as_real()
            .and_then(Real::to_i64)
            .ok_or("CHOOSE requires an integer cell index")?;
        Ok(usize::try_from(index)
            .ok()
            .filter(|i| (1..=self.items.len()).contains(i)))
    }
}

pub(super) fn body(source: &str) -> Option<&str> {
    let source = source.trim().strip_prefix('=')?.trim_start();
    let (name, rest) = source.split_at_checked("CHOOSE".len())?;
    if !name.eq_ignore_ascii_case("CHOOSE") {
        return None;
    }
    let call = rest.trim_start();
    if !call.starts_with('(') || call_end(call, 0).ok()? != call.len() {
        return None;
    }
    Some(&call[1..call.len() - 1])
}

impl Evaluator<'_> {
    pub(super) fn choose_definition(
        &mut self,
        body: &str,
        address: Address,
    ) -> Result<ChoiceDefinition, String> {
        let args = formula_arguments(body)?;
        let [target, title, items @ ..] = args.as_slice() else {
            return Err("Use CHOOSE(index cell, title, list of choices)".into());
        };
        if items.is_empty() {
            return Err("CHOOSE requires a list of choices".into());
        }
        let target = self
            .sheet
            .address(target)
            .ok_or("CHOOSE requires a single worksheet cell")?;
        let title = self
            .expression(title, address, 1)?
            .expect_str("CHOOSE title")
            .map_err(|error| error.to_string())?
            .to_string();
        let items = items
            .iter()
            .map(|item| self.expression(item, address, 1))
            .collect::<Result<Vec<_>, _>>()?;
        let items = match items.as_slice() {
            [Value::List(list)] => list.clone(),
            _ => Rc::new(items),
        };
        Ok(ChoiceDefinition {
            target: target.name(),
            title,
            items,
        })
    }
}

impl Spreadsheet {
    fn choose_source(&self, address: Address) -> Option<&str> {
        let name = address.name();
        if self.entered_values.contains_key(&name) {
            return None;
        }
        self.cells.get(&name).map(String::as_str).and_then(body)
    }

    pub(crate) fn has_choice(&self, col: usize, row: usize) -> bool {
        self.choose_source(Address { col, row }).is_some()
    }

    pub fn open_cell_choice(
        &self,
        name: &str,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<CellChoiceMenu, String> {
        let address = Address::parse(name).ok_or("Invalid worksheet cell")?;
        let body = self
            .choose_source(address)
            .ok_or("Cell has no CHOOSE formula")?;
        let mut evaluator = Evaluator::new(self, snapshot, cancel, geometry)?;
        let choice = evaluator.choose_definition(body, address)?;
        let current = choice.current(&mut evaluator)?.unwrap_or(1);
        let items = choice
            .items
            .iter()
            .map(|item| {
                if let Some(text) = item.string_units() {
                    String::from_utf16_lossy(&text)
                } else {
                    item.string_form(snapshot.settings.home_format())
                }
            })
            .collect();
        Ok(CellChoiceMenu {
            title: choice.title,
            items,
            current,
        })
    }

    pub fn select_cell_choice(
        mut self,
        name: &str,
        selection: usize,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
        geometry: Option<Arc<Book>>,
    ) -> Result<Self, String> {
        let address = Address::parse(name).ok_or("Invalid worksheet cell")?;
        let body = self
            .choose_source(address)
            .ok_or("Cell has no CHOOSE formula")?;
        let mut evaluator = Evaluator::new(&self, snapshot, cancel, geometry)?;
        let choice = evaluator.choose_definition(body, address)?;
        if !(1..=choice.items.len()).contains(&selection) {
            return Err("Worksheet choice is out of range".into());
        }
        let target = choice.target;
        if target == cell_name(address.col, address.row) {
            return Err("CHOOSE requires a separate index cell".into());
        }
        self.set_typed(
            &target,
            &PortableValue::Real(Real::from_i64(selection as i64)),
        )?;
        self.validate()?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worksheet_choose_keeps_typed_zero_distinct_from_no_choice() {
        let memory = ppl_vm::SessionContext::default().snapshot();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut sheet = Spreadsheet::default();
        sheet
            .set("A1", r#"=CHOOSE($B$1,"A, B",{0,"two, three"})"#)
            .unwrap();
        let values = sheet.calculate(&memory, cancel.clone()).unwrap();
        assert_eq!(values["A1"]["Number"], 0.0);
        assert_eq!(values["A1"]["UnselectedChoice"], true);
        let menu = sheet
            .open_cell_choice("A1", &memory, cancel.clone(), None)
            .unwrap();
        assert_eq!(menu.title, "A, B");
        assert_eq!(menu.items, ["0", "two, three"]);
        assert_eq!(menu.current, 1);
        let mut selected = sheet
            .select_cell_choice("A1", 1, &memory, cancel.clone(), None)
            .unwrap();
        let values = selected.calculate(&memory, cancel.clone()).unwrap();
        assert_eq!(values["A1"]["Number"], 0.0);
        assert!(values["A1"].get("UnselectedChoice").is_none());
        selected.set("B1", "1.5").unwrap();
        assert!(selected.evaluate_in("A1", &memory, cancel.clone()).is_err());
        selected.set("B1", "9").unwrap();
        assert_eq!(
            selected.calculate(&memory, cancel).unwrap()["A1"]["UnselectedChoice"],
            true
        );
    }
}
