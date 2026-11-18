//! One-variable definitions. Plot/option codes follow independent original
//! firmware assignment controls; presentation labels follow the calculator UI.
use super::{Columns, OneAnalysis};
use crate::ide::native_application_access::geometry::ReadSnapshot;
use ppl_vm::PortableValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Chart {
    #[default]
    Histogram,
    BoxWhisker,
    NormalProbability,
    Line,
    Bar,
    Pareto,
    Control,
    Dot,
    StemLeaf,
    Pie,
}
impl Chart {
    pub const ALL: [Self; 10] = [
        Self::Histogram,
        Self::BoxWhisker,
        Self::NormalProbability,
        Self::Line,
        Self::Bar,
        Self::Pareto,
        Self::Control,
        Self::Dot,
        Self::StemLeaf,
        Self::Pie,
    ];
    pub fn code(self) -> u8 {
        Self::ALL.iter().position(|&chart| chart == self).unwrap() as u8 + 1
    }
    pub fn from_code(code: i64) -> Option<Self> {
        code.checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| Self::ALL.get(index))
            .copied()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Histogram => "Histogram",
            Self::BoxWhisker => "Box and whisker",
            Self::NormalProbability => "Normal probability",
            Self::Line => "Line",
            Self::Bar => "Bar",
            Self::Pareto => "Pareto",
            Self::Control => "Control",
            Self::Dot => "Dot",
            Self::StemLeaf => "Stem and leaf",
            Self::Pie => "Pie",
        }
    }
    pub fn options(self) -> &'static [&'static str] {
        match self {
            Self::BoxWhisker => &["No outliers", "Show outliers"],
            Self::Control => &["Individuals", "Moving Range"],
            Self::StemLeaf => &["Single stem", "Split stem"],
            _ => &[],
        }
    }
    pub fn default_option(self) -> u8 {
        u8::from(!self.options().is_empty())
    }
    pub fn valid_option(self, option: u8) -> bool {
        if self.options().is_empty() {
            option == 0
        } else {
            (1..=2).contains(&option)
        }
    }
}
impl OneAnalysis {
    pub fn data_expression(&self) -> String {
        self.data_source.clone().unwrap_or_else(|| {
            self.data
                .map_or_else(String::new, |column| Columns::label(column, false))
        })
    }
    pub fn frequency_expression(&self) -> String {
        self.frequency_source.clone().unwrap_or_else(|| {
            self.frequency
                .map_or_else(String::new, |column| Columns::label(column, false))
        })
    }
    pub fn plot_color(&self, index: usize) -> u32 {
        self.color.unwrap_or(super::ANALYSIS_COLORS[index.min(4)])
    }
    pub fn definition(&self, index: usize) -> PortableValue {
        let expression = |source: String| {
            if source.is_empty() {
                PortableValue::String(source)
            } else {
                PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source))
            }
        };
        PortableValue::List(vec![
            expression(self.data_expression()),
            expression(self.frequency_expression()),
            PortableValue::Real(ppl_math::Real::from_i64(i64::from(self.chart.code()))),
            PortableValue::Real(ppl_math::Real::from_i64(i64::from(self.option))),
            PortableValue::Integer(ppl_math::HpInt::new(
                i128::from(self.plot_color(index)),
                24,
                ppl_math::IntBase::Hex,
            )),
        ])
    }
    pub(super) fn validate_definition(&self) -> Result<(), String> {
        if !self.chart.valid_option(self.option) || self.color.is_some_and(|color| color > 0xffffff)
        {
            return Err("Invalid statistics plot type, option or color".into());
        }
        for source in [&self.data_source, &self.frequency_source]
            .into_iter()
            .flatten()
        {
            if source.len() > 65_536 {
                return Err("Statistics expression exceeds 64 KiB".into());
            }
            if !source.is_empty() {
                let (expression, diagnostics) = ppl_syntax::parse_expression(source);
                if diagnostics.has_errors()
                    || ppl_compiler::ppl_expression_source(&expression).is_err()
                {
                    return Err("Invalid saved statistics expression".into());
                }
            }
        }
        Ok(())
    }
}
fn expression(value: &PortableValue) -> Result<String, String> {
    let source = match value {
        PortableValue::String(source) => source.clone(),
        PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) => source.clone(),
        _ => super::super::native::value_source(&value.to_value()),
    };
    if source.is_empty() {
        return Ok(source);
    }
    if source.len() > 65_536 {
        return Err("Statistics expression exceeds 64 KiB".into());
    }
    let (parsed, diagnostics) = ppl_syntax::parse_expression(&source);
    if diagnostics.has_errors() || matches!(parsed.kind, ppl_syntax::ast::ExprKind::Str(_)) {
        return Err("A statistics definition requires a valid expression".into());
    }
    ppl_compiler::ppl_expression_source(&parsed).map_err(|error| error.to_string())?;
    Ok(source.trim().to_owned())
}
fn column(source: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.len() == 2 && bytes[0] == b'D' && bytes[1].is_ascii_digit() {
        Some((usize::from(bytes[1] - b'0') + 9) % 10)
    } else {
        None
    }
}
fn integer(value: &PortableValue) -> Result<i64, String> {
    let number = value
        .to_value()
        .expect_real("statistics definition")
        .map_err(|e| e.to_string())?;
    if !number.is_integer() {
        return Err("A statistics plot setting must be an integer".into());
    }
    number
        .to_i64()
        .ok_or_else(|| "Statistics plot setting is outside its range".into())
}
impl OneAnalysis {
    fn set_field(&mut self, field: usize, value: &PortableValue) -> Result<(), String> {
        match field {
            1 => {
                let source = expression(value)?;
                self.data = column(&source);
                self.data_source = Some(source);
            }
            2 => {
                let source = expression(value)?;
                self.frequency = column(&source);
                self.frequency_source = Some(source);
            }
            3 => {
                self.chart =
                    Chart::from_code(integer(value)?).ok_or("Invalid statistics plot type")?;
                if !self.chart.valid_option(self.option) {
                    self.option = self.chart.default_option();
                }
            }
            4 => {
                self.option =
                    u8::try_from(integer(value)?).map_err(|_| "Invalid statistics plot option")?;
            }
            5 => {
                self.color = Some(
                    u32::try_from(integer(value)?)
                        .ok()
                        .filter(|&color| color <= 0xffffff)
                        .ok_or("Invalid statistics plot color")?,
                );
            }
            _ => return Err("Statistics definition index must be 1 through 5".into()),
        }
        Ok(())
    }
    /// A complete/prefix assignment is validated on a clone before replacing
    /// the definition. Missing trailing fields retain their current values.
    pub fn assign_prefix(&mut self, value: &PortableValue) -> Result<PortableValue, String> {
        let PortableValue::List(fields) = value else {
            return Err("A statistics definition requires a list".into());
        };
        if !(1..=5).contains(&fields.len()) {
            return Err("A statistics definition needs 1 through 5 fields".into());
        }
        let mut candidate = self.clone();
        for (index, value) in fields.iter().enumerate() {
            candidate.set_field(index + 1, value)?;
        }
        candidate.validate_definition()?;
        candidate.enabled = false;
        *self = candidate;
        Ok(value.clone())
    }
    /// Nonempty indexed edits and SetSample/SetFreq retain the checkbox.
    /// Explicitly clearing either expression unchecks the definition, including
    /// a frequency field that was already empty (original G1 UI control).
    pub fn assign_field(
        &mut self,
        field: usize,
        value: &PortableValue,
    ) -> Result<PortableValue, String> {
        let mut candidate = self.clone();
        candidate.set_field(field, value)?;
        candidate.validate_definition()?;
        if (field == 1 && candidate.data_expression().is_empty())
            || (field == 2 && candidate.frequency_expression().is_empty())
        {
            candidate.enabled = false;
        }
        *self = candidate;
        Ok(value.clone())
    }
}

impl super::OneBook {
    pub(super) fn validate_results(&self) -> Result<(), String> {
        if self
            .result_analysis
            .is_some_and(|index| index >= super::ANALYSIS_COUNT)
        {
            return Err("Invalid saved statistics result analysis".into());
        }
        for (name, values) in &self.result_values {
            if !["H1", "H2", "H3", "H4", "H5"].contains(&name.as_str())
                || values.len() != super::one_data::VARIABLES.len()
            {
                return Err("Invalid saved statistics results".into());
            }
            for name in super::one_data::VARIABLES {
                let value = values
                    .get(name)
                    .ok_or("Incomplete saved statistics results")?;
                match ppl_vm::session::decode_value(value)? {
                    PortableValue::Real(_) => {}
                    PortableValue::Symbolic(
                        ppl_cas::CasValue::Undefined
                        | ppl_cas::CasValue::Infinity(ppl_cas::InfinitySign::Positive),
                    ) if matches!(name, "sX" | "serrX") => {}
                    _ => return Err("Invalid calculator value in saved statistics results".into()),
                }
            }
        }
        Ok(())
    }
    pub fn compute_one(
        &mut self,
        index: usize,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<super::one_data::Summary, String> {
        self.compute_one_with_geometry(index, snapshot, cancel, None)
    }

    pub(crate) fn compute_one_with_geometry(
        &mut self,
        index: usize,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        geometry: ReadSnapshot,
    ) -> Result<super::one_data::Summary, String> {
        let analysis = self
            .analyses
            .get(index)
            .ok_or("Invalid statistics analysis")?;
        if !self.analyses.iter().any(|analysis| analysis.enabled) {
            return Err("No definition checked in Symbolic view".into());
        }
        if !analysis.enabled {
            return Err("Insufficient statistics data".into());
        }
        analysis.validate_definition()?;
        let data = super::one_data::Data::evaluate_with_geometry(
            analysis,
            snapshot,
            cancel.clone(),
            geometry,
        )?;
        let result = data.summary(&cancel)?;
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Statistics calculation interrupted".into());
        }
        self.result_values.insert(
            format!("H{}", index + 1),
            result
                .variables
                .iter()
                .map(|(name, value)| (name.clone(), ppl_vm::session::encode_value(value)))
                .collect(),
        );
        Ok(result)
    }
    pub fn result_for(
        &self,
        index: usize,
        name: &str,
        version: ppl_math::NumericVersion,
    ) -> Result<PortableValue, String> {
        if index >= 5 || !super::one_data::VARIABLES.contains(&name) {
            return Err("Invalid statistics result".into());
        }
        if let Some(value) = self
            .result_values
            .get(&format!("H{}", index + 1))
            .and_then(|values| values.get(name))
        {
            ppl_vm::session::decode_value(value)
        } else {
            // The original app supplies zero for all thirteen factory results.
            let zero = ppl_math::Real::ZERO;
            Ok(PortableValue::Real(
                if version == ppl_math::NumericVersion::Current15751 {
                    zero.decimal()
                } else {
                    zero
                },
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_thirty_original_plot_option_assignments_match_the_definition_validator() {
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-definition-controls.json"
        ))
        .unwrap();
        for case in controls["chart_options"].as_array().unwrap() {
            let code = case["plot"].as_i64().unwrap();
            let option = case["option"].as_u64().unwrap() as u8;
            let analysis = OneAnalysis {
                chart: Chart::from_code(code).unwrap(),
                option,
                ..Default::default()
            };
            assert_eq!(
                analysis.validate_definition().is_ok(),
                case["accepted"].as_bool().unwrap(),
                "{code}/{option}"
            );
        }
        assert!(Chart::from_code(i64::MIN).is_none());
        assert!(Chart::from_code(0).is_none());
        assert!(Chart::from_code(11).is_none());
    }
    #[test]
    fn original_fresh_definitions_and_legacy_profile_columns_keep_typed_config() {
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-definition-controls.json"
        ))
        .unwrap();
        let book = super::super::OneBook::default();
        let values = book
            .analyses
            .iter()
            .enumerate()
            .map(|(index, a)| a.definition(index))
            .collect::<Vec<_>>();
        for (index, value) in values.into_iter().enumerate() {
            let PortableValue::List(fields) = value else {
                panic!()
            };
            assert_eq!(fields.len(), 5);
            assert_eq!(
                fields[0],
                if index == 0 {
                    PortableValue::Symbolic(ppl_cas::CasValue::Symbolic("D1".into()))
                } else {
                    PortableValue::String(String::new())
                }
            );
            let PortableValue::Integer(color) = fields[4] else {
                panic!()
            };
            assert_eq!(color.bits(), 24);
            assert_eq!(
                color.raw_value(),
                controls["fresh_definitions"][index]["color_rgb"]
                    .as_u64()
                    .unwrap()
            );
        }
        let old: OneAnalysis =
            serde_json::from_str(r#"{"enabled":true,"data":2,"frequency":3}"#).unwrap();
        assert_eq!(old.data_expression(), "D3");
        assert_eq!(old.frequency_expression(), "D4");
        let explicit = OneAnalysis {
            data_source: Some("D1+1".into()),
            frequency_source: Some("1+1".into()),
            ..old
        };
        let reloaded: OneAnalysis =
            serde_json::from_value(serde_json::to_value(&explicit).unwrap()).unwrap();
        assert_eq!(reloaded.data_expression(), "D1+1");
        assert_eq!(reloaded.frequency_expression(), "1+1");
        assert!(reloaded.validate_definition().is_ok());
    }
    #[test]
    fn assignments_preserve_prefix_style_and_indexed_checks_and_reject_bad_lists_atomically() {
        let text = |source: &str| PortableValue::String(source.into());
        let number = |n: i64| PortableValue::Real(ppl_math::Real::from_i64(n));
        let mut analysis = OneAnalysis {
            enabled: true,
            data: Some(0),
            ..Default::default()
        };
        let initial = PortableValue::List(vec![
            text("D1"),
            text("3"),
            number(2),
            number(2),
            number(0x123456),
        ]);
        assert_eq!(analysis.assign_prefix(&initial).unwrap(), initial);
        assert!(!analysis.enabled);
        analysis
            .assign_prefix(&PortableValue::List(vec![text("D2")]))
            .unwrap();
        assert_eq!(analysis.data_expression(), "D2");
        assert_eq!(analysis.frequency_expression(), "3");
        assert_eq!(
            (analysis.chart, analysis.option, analysis.color),
            (Chart::BoxWhisker, 2, Some(0x123456))
        );
        let before = serde_json::to_value(&analysis).unwrap();
        for bad in [
            PortableValue::List(vec![]),
            PortableValue::List(vec![text("D1+")]),
            PortableValue::List(vec![text("D1"), text(""), number(11)]),
        ] {
            assert!(analysis.assign_prefix(&bad).is_err());
            assert_eq!(serde_json::to_value(&analysis).unwrap(), before);
        }
        analysis.enabled = true;
        analysis.assign_field(2, &text("2")).unwrap();
        assert!(analysis.enabled);
        assert_eq!(analysis.frequency_expression(), "2");
        analysis.assign_field(3, &number(8)).unwrap();
        assert_eq!(analysis.option, 0);
        assert!(analysis.assign_field(4, &number(2)).is_err());
        assert_eq!(analysis.option, 0);
    }
    #[test]
    fn original_explicit_empty_frequency_unchecks_the_definition_even_when_already_empty() {
        let mut definition = OneAnalysis {
            enabled: true,
            data: Some(0),
            ..Default::default()
        };
        definition
            .assign_field(2, &PortableValue::String(String::new()))
            .unwrap();
        assert!(!definition.enabled);
        assert!(definition.frequency_expression().is_empty());
        definition
            .assign_field(2, &PortableValue::Real(ppl_math::Real::from_i64(2)))
            .unwrap();
        assert!(!definition.enabled);
        definition.enabled = true;
        definition
            .assign_field(2, &PortableValue::String("3".into()))
            .unwrap();
        assert!(definition.enabled);
        definition
            .assign_field(1, &PortableValue::String(String::new()))
            .unwrap();
        assert!(!definition.enabled);
    }
    #[test]
    fn captured_command_strings_are_distinct_from_actual_definition_text() {
        let mut analysis = OneAnalysis::default();
        analysis
            .assign_field(
                1,
                &PortableValue::Symbolic(ppl_cas::CasValue::Symbolic("D1+1".into())),
            )
            .unwrap();
        assert_eq!(analysis.data_expression(), "D1+1");
        assert!(
            analysis
                .assign_field(
                    1,
                    &PortableValue::Symbolic(ppl_cas::CasValue::Symbolic("\"D1\"".into()))
                )
                .is_err()
        );
        assert_eq!(analysis.data_expression(), "D1+1");
        let real = ppl_math::Real::parse_versioned(
            "1234567890123456789",
            ppl_math::NumericVersion::Current15751,
        )
        .unwrap();
        analysis
            .assign_field(2, &PortableValue::Real(real))
            .unwrap();
        assert_eq!(analysis.frequency_expression(), "1.234567890123456789e18");
    }
    #[test]
    fn computed_cache_keeps_typed_results_after_restart_and_failed_recalculation() {
        use std::sync::{Arc, atomic::AtomicBool};
        let mut session = ppl_vm::SessionContext::default();
        session.settings.numeric_version = ppl_math::NumericVersion::Current15751;
        session
            .set_variable(
                "D1",
                PortableValue::List(
                    [1, 2, 4]
                        .into_iter()
                        .map(|n| PortableValue::Real(ppl_math::Real::from_i64(n)))
                        .collect(),
                ),
            )
            .unwrap();
        let snapshot = session.snapshot();
        let mut book = super::super::OneBook::default();
        assert_eq!(
            book.result_for(0, "MeanX", snapshot.settings.numeric_version)
                .unwrap()
                .to_value()
                .expect_real("mean")
                .unwrap(),
            ppl_math::Real::ZERO
        );
        book.analyses[0].frequency_source = Some("3".into());
        book.compute_one(0, &snapshot, Arc::new(AtomicBool::new(false)))
            .unwrap();
        let stored = serde_json::to_value(&book).unwrap();
        let restored: super::super::OneBook = serde_json::from_value(stored).unwrap();
        restored.validate().unwrap();
        let value = restored
            .result_for(0, "MeanX", snapshot.settings.numeric_version)
            .unwrap();
        assert_eq!(
            value.to_value().expect_real("mean").unwrap().ppl_source(),
            "approx(2.33333333333)"
        );
        let before = book.result_values.clone();
        book.analyses[0].frequency_source = Some("-1".into());
        assert!(
            book.compute_one(0, &snapshot, Arc::new(AtomicBool::new(false)))
                .is_err()
        );
        assert_eq!(book.result_values, before);
        assert!(
            book.compute_one(0, &snapshot, Arc::new(AtomicBool::new(true)))
                .is_err()
        );
        assert_eq!(book.result_values, before);
        assert!(
            book.compute_one(1, &snapshot, Arc::new(AtomicBool::new(false)))
                .is_err()
        );
        assert_eq!(book.result_values, before);
    }
}
