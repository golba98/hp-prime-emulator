//! Typed Statistics 2Var Symbolic records and previously computed fit equations.
use super::{ANALYSIS_COLORS, ANALYSIS_COUNT, Columns, TwoAnalysis, TwoBook, TwoResult};
use crate::ide::native_regression::FitKind;
use ppl_cas::CasValue;
use ppl_math::{HpInt, IntBase, Real};
use ppl_vm::PortableValue;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

pub(super) fn empty_equations(equations: &[Option<String>; ANALYSIS_COUNT]) -> bool {
    equations.iter().all(Option::is_none)
}

fn expression(source: String) -> PortableValue {
    if source.is_empty() {
        PortableValue::String(source)
    } else {
        PortableValue::Symbolic(CasValue::Symbolic(source))
    }
}

fn integer(value: &PortableValue) -> Result<i64, String> {
    let real = value
        .to_value()
        .expect_real("statistics definition")
        .map_err(|err| err.to_string())?;
    real.is_integer()
        .then(|| real.to_i64())
        .flatten()
        .ok_or_else(|| "A statistics setting must be an integer in range".into())
}

fn color(value: &PortableValue) -> Result<u32, String> {
    let value = match value {
        PortableValue::Integer(word) => word.unsigned() as i128,
        _ => i128::from(integer(value)?),
    };
    u32::try_from(value)
        .ok()
        .filter(|&color| color <= 0xffffff)
        .ok_or_else(|| "Statistics color must be between 0 and 16777215".into())
}

fn definition_source(value: &PortableValue) -> Result<String, String> {
    let source = match value {
        PortableValue::String(source) | PortableValue::Symbolic(CasValue::Symbolic(source)) => {
            source.trim().to_owned()
        }
        PortableValue::Real(_) | PortableValue::Integer(_) => {
            crate::ide::native::value_source(&value.to_value())
        }
        _ => return Err("A statistics definition requires an expression".into()),
    };
    if !valid_source(&source) {
        return Err("Invalid statistics expression".into());
    }
    Ok(source)
}

fn valid_source(source: &str) -> bool {
    if source.len() > 65536 {
        return false;
    }
    if source.is_empty() {
        return true;
    }
    let (parsed, diagnostics) = ppl_syntax::parse_expression(source);
    !diagnostics.has_errors()
        && !matches!(parsed.kind, ppl_syntax::ast::ExprKind::Str(_))
        && ppl_compiler::ppl_expression_source(&parsed).is_ok()
}

fn column(source: &str) -> Option<usize> {
    match source.as_bytes() {
        [b'C', digit] if digit.is_ascii_digit() => Some((usize::from(*digit - b'0') + 9) % 10),
        _ => None,
    }
}

impl TwoAnalysis {
    fn source(&self, field: usize) -> (&Option<String>, Option<usize>) {
        match field {
            1 => (&self.x_source, self.x),
            2 => (&self.y_source, self.y),
            3 => (&self.frequency_source, self.frequency),
            _ => unreachable!("Statistics observation field"),
        }
    }

    pub(super) fn evaluate_reference(
        &self,
        field: usize,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Option<ppl_vm::Value>, String> {
        let (source, _) = self.source(field);
        source
            .as_deref()
            .map(|source| {
                crate::ide::native_readonly::evaluate(
                    source,
                    "Statistics 2Var",
                    snapshot,
                    [],
                    cancel.clone(),
                )
            })
            .transpose()
    }

    pub fn reference(&self, field: usize) -> String {
        let (source, column) = self.source(field);
        source.clone().unwrap_or_else(|| {
            column.map_or_else(String::new, |column| Columns::label(column, true))
        })
    }

    pub fn set_column(&mut self, field: usize, column: Option<usize>) {
        self.set_reference(field, column, None);
    }

    fn set_reference(&mut self, field: usize, column: Option<usize>, source: Option<String>) {
        match field {
            1 => (self.x, self.x_source) = (column, source),
            2 => (self.y, self.y_source) = (column, source),
            3 => (self.frequency, self.frequency_source) = (column, source),
            _ => unreachable!("Statistics observation field"),
        }
    }

    pub fn defined(&self) -> bool {
        [1, 2].into_iter().all(|field| {
            let (source, column) = self.source(field);
            source.as_ref().is_some_and(|source| !source.is_empty()) || column.is_some()
        })
    }

    pub(super) fn valid_definition(&self) -> bool {
        self.user_expression.len() <= 65536
            && self
                .definition_number
                .is_none_or(|number| (1..=9).contains(&number))
            && [self.observation_color, self.fit_color]
                .into_iter()
                .flatten()
                .all(|color| color <= 0xffffff)
            && [
                &self.x_source,
                &self.y_source,
                &self.frequency_source,
                &self.fit_override,
            ]
            .into_iter()
            .flatten()
            .all(|source| valid_source(source))
    }

    pub fn observation_color(&self, index: usize) -> u32 {
        self.observation_color.unwrap_or(ANALYSIS_COLORS[index])
    }

    pub fn fit_color(&self, index: usize) -> u32 {
        self.fit_color.unwrap_or(ANALYSIS_COLORS[index])
    }
}

impl TwoBook {
    pub fn definition(&self, index: usize) -> Vec<PortableValue> {
        let row = &self.analyses[index];
        let word = |color| PortableValue::Integer(HpInt::new(i128::from(color), 24, IntBase::Hex));
        let formula = if row.fit == FitKind::UserDefined {
            &row.user_expression
        } else {
            row.fit_override
                .as_deref()
                .or(self.fit_equations[index].as_deref())
                .unwrap_or_default()
        };
        vec![
            expression(row.reference(1)),
            expression(row.reference(2)),
            expression(row.reference(3)),
            PortableValue::Real(Real::from_i64(row.fit.code())),
            expression(formula.to_owned()),
            word(row.observation_color(index)),
            PortableValue::Real(Real::from_i64(
                row.definition_number.unwrap_or(index as i64 + 1),
            )),
            word(row.fit_color(index)),
        ]
    }

    pub fn assign_field(
        &mut self,
        index: usize,
        field: usize,
        value: &PortableValue,
    ) -> Result<(), String> {
        let row = &mut self.analyses[index];
        match field {
            1..=3 => {
                let source = definition_source(value)?;
                let column = column(&source);
                if source.is_empty() {
                    row.enabled = false;
                }
                let expression = (column.is_none() && !source.is_empty()).then_some(source);
                row.set_reference(field, column, expression);
            }
            4 => {
                row.fit =
                    FitKind::from_code(integer(value)?).ok_or("Invalid statistics fit type")?;
                row.user_expression.clear();
                row.fit_override = None;
                self.fit_equations[index] = None;
            }
            5 => {
                let source = definition_source(value)?;
                if row.fit == FitKind::UserDefined {
                    row.user_expression = source;
                } else {
                    row.fit_override = (!source.is_empty()).then_some(source);
                    self.fit_equations[index] = None;
                }
            }
            6 => row.observation_color = Some(color(value)?),
            7 => {
                let number = integer(value)?;
                if !(1..=9).contains(&number) {
                    return Err("Statistics definition number must be 1 through 9".into());
                }
                row.definition_number = Some(number);
            }
            8 => row.fit_color = Some(color(value)?),
            _ => return Err("Statistics definition index must be 1 through 8".into()),
        }
        if field <= 5 {
            self.fit_columns_revisions[index] = None;
        }
        Ok(())
    }

    pub fn remember_fit_equations(
        &mut self,
        results: &BTreeMap<String, TwoResult>,
        columns_revision: u64,
    ) {
        for index in 0..ANALYSIS_COUNT {
            if let Some(result) = results.get(&format!("S{}", index + 1)) {
                self.set_calculated_fit(index, result.regression.equation(), columns_revision);
            }
        }
    }

    pub fn set_calculated_fit(&mut self, index: usize, equation: String, columns_revision: u64) {
        if let Some(storage) = self.fit_equations.get_mut(index) {
            *storage = Some(equation);
            self.fit_columns_revisions[index] = Some(columns_revision);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ide::{native_app_ui::AppMemory, native_statistics::TwoBook};

    #[test]
    fn retained_equations_do_not_change_the_model_worker_input() {
        let mut memory = AppMemory {
            statistics2_book: Some(TwoBook::default()),
            ..Default::default()
        };
        let original = memory.input_fingerprint();
        memory.statistics2_book.as_mut().unwrap().fit_equations[0] = Some("(2)*X+(1)".into());
        assert_eq!(memory.input_fingerprint(), original);
        memory.statistics2_book.as_mut().unwrap().analyses[0].y = Some(2);
        assert_ne!(memory.input_fingerprint(), original);
    }
}
