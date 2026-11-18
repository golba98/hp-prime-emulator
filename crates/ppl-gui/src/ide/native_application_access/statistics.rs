//! Statistics app requests operate on a worker-local book and the live VM frame.
//! Returning a changed book never commits it: the GUI retains the source/epoch
//! fence, and discards this value on cancellation or an intervening edit.
use super::{checkbox, geometry::ReadSnapshot};
use crate::ide::native_statistics::{
    ANALYSIS_COUNT, Columns, OneBook, TwoAnalysis, TwoBook, one_data::VARIABLES,
};
use ppl_vm::{ErrorKind, PortableValue, RuntimeError, SessionSnapshot, host::AppVariableRequest};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, message.into())
}
fn check(cancel: &AtomicBool) -> Result<(), RuntimeError> {
    if cancel.load(Ordering::Acquire) {
        Err(RuntimeError::kind(ErrorKind::Interrupted))
    } else {
        Ok(())
    }
}
fn index(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    (bytes.len() == 2 && bytes[0] == b'H' && (b'1'..=b'5').contains(&bytes[1]))
        .then(|| usize::from(bytes[1] - b'1'))
}
fn target(value: &PortableValue) -> Result<usize, RuntimeError> {
    let PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) = value else {
        return Err(invalid(
            "A statistics command requires an H1 through H5 definition",
        ));
    };
    let name = if let Some((owner, name)) = source.split_once('.') {
        if owner != "Statistics_1Var" {
            return Err(invalid("Invalid statistics definition owner"));
        }
        name
    } else {
        source.as_str()
    };
    index(name).ok_or_else(|| invalid("A statistics command requires an H1 through H5 definition"))
}
fn field(value: &PortableValue, maximum: i64) -> Result<usize, RuntimeError> {
    let number = value
        .to_value()
        .expect_real("statistics definition index")?;
    if !number.is_integer() {
        return Err(invalid(format!(
            "Statistics definition index must be 1 through {maximum}"
        )));
    }
    number
        .to_i64()
        .filter(|n| (1..=maximum).contains(n))
        .map(|n| n as usize)
        .ok_or_else(|| {
            invalid(format!(
                "Statistics definition index must be 1 through {maximum}"
            ))
        })
}
fn check_position(request: &AppVariableRequest) -> Result<usize, RuntimeError> {
    match (request.indices.as_slice(), request.value.as_ref()) {
        ([position], None) => field(position, 5).map(|index| index - 1),
        _ => Err(invalid("A statistics checkbox command requires one index")),
    }
}

pub(crate) fn access_two_checkbox(
    mut analyses: [TwoAnalysis; ANALYSIS_COUNT],
    request: &AppVariableRequest,
    cancel: Arc<AtomicBool>,
) -> Result<(PortableValue, Option<(usize, bool)>), RuntimeError> {
    check(&cancel)?;
    let index = check_position(request)?;
    let analysis = &mut analyses[index];
    let defined = analysis.defined();
    let (value, changed) = checkbox(&request.name, &mut analysis.enabled, defined)?;
    check(&cancel)?;
    Ok((value, changed.then_some((index, analysis.enabled))))
}

pub enum TwoUpdate {
    Checkbox(usize, bool),
    Definition {
        index: usize,
        analysis: Box<TwoAnalysis>,
        equation: Option<String>,
        columns_revision: Option<u64>,
        error: Option<RuntimeError>,
    },
    Fits {
        results: serde_json::Value,
        equations: [Option<String>; ANALYSIS_COUNT],
        columns_revisions: [Option<u64>; ANALYSIS_COUNT],
        disabled_target: bool,
    },
}

fn two_slot(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    (bytes.len() == 2 && bytes[0] == b'S' && (b'1'..=b'5').contains(&bytes[1]))
        .then(|| usize::from(bytes[1] - b'1'))
}

pub(crate) fn access_two(
    mut book: TwoBook,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<(PortableValue, Option<TwoUpdate>), RuntimeError> {
    check(&cancel)?;
    match request.name.as_str() {
        "CHECK" | "UNCHECK" | "ISCHECK" => {
            let (value, changed) = access_two_checkbox(book.analyses, request, cancel)?;
            return Ok((
                value,
                changed.map(|(index, on)| TwoUpdate::Checkbox(index, on)),
            ));
        }
        "Do2VStats" => {
            let ([PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(name))], None) =
                (request.indices.as_slice(), request.value.as_ref())
            else {
                return Err(invalid("Do2VStats requires an S1 through S5 definition"));
            };
            let index = two_slot(name)
                .ok_or_else(|| invalid("Do2VStats requires an S1 through S5 definition"))?;
            let columns_revision = snapshot.statistics2_columns_revision;
            let disabled_target = !book.analyses[index].enabled;
            if disabled_target && book.fit_columns_revisions[index] == Some(columns_revision) {
                return Ok((PortableValue::Real(ppl_math::Real::ONE), None));
            }
            book.columns = Columns::from_snapshot(snapshot, true).map_err(invalid)?.0;
            let results = book.calculate(snapshot, &cancel).map_err(|error| {
                if cancel.load(Ordering::Acquire) {
                    RuntimeError::kind(ErrorKind::Interrupted)
                } else {
                    invalid(error)
                }
            })?;
            book.remember_fit_equations(&results, columns_revision);
            check(&cancel)?;
            let results =
                serde_json::to_value(results).map_err(|error| invalid(error.to_string()))?;
            return Ok((
                PortableValue::Real(ppl_math::Real::ONE),
                Some(TwoUpdate::Fits {
                    results,
                    equations: book.fit_equations,
                    columns_revisions: book.fit_columns_revisions,
                    disabled_target,
                }),
            ));
        }
        _ => {}
    }
    let index =
        two_slot(&request.name).ok_or_else(|| invalid("Unknown Statistics 2Var variable"))?;
    let position = match request.indices.as_slice() {
        [] => None,
        [position] => Some(field(position, 8)?),
        _ => {
            return Err(invalid(
                "A statistics definition accepts at most one field index",
            ));
        }
    };
    let Some(value) = request.value.as_ref() else {
        let mut values = book.definition(index);
        let value = match position {
            Some(position) => values.remove(position - 1),
            None => PortableValue::List(values),
        };
        return Ok((value, None));
    };
    let mut error = None;
    if let Some(position) = position {
        book.assign_field(index, position, value).map_err(invalid)?;
    } else {
        let PortableValue::List(values) = value else {
            return Err(invalid("A statistics definition requires a list"));
        };
        if !(1..=8).contains(&values.len()) {
            return Err(invalid("A statistics definition needs 1 through 8 fields"));
        }
        // G1 keeps the accepted leading fields when a later field is invalid.
        for (offset, field) in values.iter().enumerate() {
            if let Err(message) = book.assign_field(index, offset + 1, field) {
                if offset == 0 {
                    return Err(invalid(message));
                }
                error = Some(invalid(message));
                break;
            }
        }
    }
    check(&cancel)?;
    Ok((
        value.clone(),
        Some(TwoUpdate::Definition {
            index,
            analysis: Box::new(book.analyses[index].clone()),
            equation: book.fit_equations[index].take(),
            columns_revision: book.fit_columns_revisions[index],
            error,
        }),
    ))
}

/// Returns a caller-visible value and, only for a successful mutation, a new
/// statistics book. Getters never promote calculator Ans or recalculate data.
/// Result variables retain the first column of the last successful calculation,
/// independently of later checkbox edits or the GUI cursor. Do1VStats computes
/// every checked definition; its argument must name one of those definitions.
pub fn access(
    book: OneBook,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<(PortableValue, Option<OneBook>), RuntimeError> {
    access_with_geometry(book, request, snapshot, cancel, None)
}

pub(crate) fn access_with_geometry(
    mut book: OneBook,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
    geometry: ReadSnapshot,
) -> Result<(PortableValue, Option<OneBook>), RuntimeError> {
    check(&cancel)?;
    if request.app != "Statistics 1Var" {
        return Err(RuntimeError::new(
            ErrorKind::Unsupported,
            "These statistics variables require Statistics 1Var",
        ));
    }
    book.validate().map_err(invalid)?;
    let name = request.name.as_str();
    let (value, changed) = if matches!(name, "CHECK" | "UNCHECK" | "ISCHECK") {
        let analysis = &mut book.analyses[check_position(request)?];
        let defined = analysis.data.is_some() || analysis.data_source.is_some();
        checkbox(name, &mut analysis.enabled, defined)?
    } else if let Some(index) = index(name) {
        let analysis = &mut book.analyses[index];
        let value = match (request.indices.as_slice(), request.value.as_ref()) {
            ([], None) => analysis.definition(index),
            ([position], None) => {
                let PortableValue::List(fields) = analysis.definition(index) else {
                    unreachable!()
                };
                fields[field(position, 5)? - 1].clone()
            }
            ([], Some(value)) => analysis.assign_prefix(value).map_err(invalid)?,
            ([position], Some(value)) => analysis
                .assign_field(field(position, 5)?, value)
                .map_err(invalid)?,
            _ => {
                return Err(invalid(
                    "A statistics definition accepts at most one field index",
                ));
            }
        };
        (value, request.value.is_some())
    } else if VARIABLES.contains(&name) {
        if request.value.is_some() || !request.indices.is_empty() {
            return Err(invalid("Statistics result variables are read-only scalars"));
        }
        let checked = book
            .analyses
            .iter()
            .position(|analysis| analysis.enabled)
            .ok_or_else(|| invalid("No definition checked in Symbolic view"))?;
        let index = book.result_analysis.unwrap_or(checked);
        let value = book
            .result_for(index, name, snapshot.settings.numeric_version)
            .map_err(invalid)?;
        if matches!(value, PortableValue::Symbolic(ppl_cas::CasValue::Undefined)) {
            return Err(RuntimeError::kind(ErrorKind::Undefined));
        }
        (value, false)
    } else if matches!(name, "Do1VStats" | "SetSample" | "SetFreq") {
        if request.value.is_some() {
            return Err(invalid("A statistics command is not an assignment target"));
        }
        match (name, request.indices.as_slice()) {
            ("Do1VStats", [analysis]) => {
                let index = target(analysis)?;
                if !book.analyses[index].enabled {
                    return Err(invalid("Insufficient statistics data"));
                }
                // Work only on this clone. A later checked definition failing
                // cannot replace an earlier retained result column.
                for analysis in 0..super::super::native_statistics::ANALYSIS_COUNT {
                    if book.analyses[analysis].enabled {
                        book.compute_one_with_geometry(
                            analysis,
                            snapshot,
                            cancel.clone(),
                            geometry.clone(),
                        )
                        .map_err(|error| {
                            if cancel.load(Ordering::Acquire) {
                                RuntimeError::kind(ErrorKind::Interrupted)
                            } else {
                                invalid(error)
                            }
                        })?;
                    }
                }
                book.result_analysis = book.analyses.iter().position(|analysis| analysis.enabled);
            }
            ("SetSample" | "SetFreq", [analysis, expression]) => {
                let index = target(analysis)?;
                // The compiler captures AST source in a symbolic value. Do not
                // evaluate D1+1, a local identifier or a program at this setter.
                let PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(_)) = expression else {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        "A statistics command requires an unevaluated expression",
                    ));
                };
                if name == "SetFreq"
                    && matches!(expression,PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source)) if source=="1")
                {
                    // Original SetFreq(Hn,1) restores the factory empty field
                    // while retaining its checkbox. An explicit indexed clear
                    // is a distinct operation and unchecks the definition.
                    let checked = book.analyses[index].enabled;
                    book.analyses[index]
                        .assign_field(2, &PortableValue::String(String::new()))
                        .map_err(invalid)?;
                    book.analyses[index].enabled = checked;
                } else {
                    book.analyses[index]
                        .assign_field(if name == "SetSample" { 1 } else { 2 }, expression)
                        .map_err(invalid)?;
                }
            }
            _ => return Err(invalid("Invalid statistics command argument count")),
        }
        (PortableValue::Real(ppl_math::Real::ONE), true)
    } else {
        return Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("Unknown Statistics 1Var variable: {name}"),
        ));
    };
    check(&cancel)?;
    Ok((value, changed.then_some(book)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_math::{NumericVersion, Real};
    use ppl_vm::SessionContext;
    fn numeric(n: i64) -> PortableValue {
        PortableValue::Real(Real::from_i64(n))
    }
    #[test]
    fn a_statistics_check_returns_real_one_and_requires_a_definition() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let snapshot = SessionContext::default().snapshot();
        let book = OneBook::default();
        let (value, changed) = access(
            book.clone(),
            &request("CHECK", vec![numeric(1)], None),
            &snapshot,
            cancelled.clone(),
        )
        .unwrap();
        assert_eq!(value, numeric(1));
        assert!(changed.is_none());
        assert!(
            access(
                book.clone(),
                &request("CHECK", vec![numeric(2)], None),
                &snapshot,
                cancelled.clone(),
            )
            .unwrap_err()
            .to_string()
            .contains("No definition in Symbolic view")
        );
        let (_, changed) = access(
            book,
            &request("UNCHECK", vec![numeric(1)], None),
            &snapshot,
            cancelled.clone(),
        )
        .unwrap();
        let (value, changed) = access(
            changed.unwrap(),
            &request("ISCHECK", vec![numeric(1)], None),
            &snapshot,
            cancelled,
        )
        .unwrap();
        assert_eq!(value, numeric(0));
        assert!(changed.is_none());
    }

    #[test]
    fn statistics_two_check_writes_only_its_resolved_analysis() {
        let mut analyses = crate::ide::native_statistics::TwoBook::default().analyses;
        analyses[1].x = Some(3);
        analyses[1].y = Some(4);
        let mut command = request("CHECK", vec![numeric(2)], None);
        command.app = "Statistics 2Var".into();
        let (value, changed) =
            access_two_checkbox(analyses, &command, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(value, numeric(1));
        assert_eq!(changed, Some((1, true)));
    }

    fn syntax(s: &str) -> PortableValue {
        PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(s.into()))
    }
    fn request(
        name: &str,
        indices: Vec<PortableValue>,
        value: Option<PortableValue>,
    ) -> AppVariableRequest {
        AppVariableRequest {
            app: "Statistics 1Var".into(),
            name: name.into(),
            indices,
            value,
        }
    }
    fn access_ok(
        book: OneBook,
        request: &AppVariableRequest,
        snapshot: &SessionSnapshot,
    ) -> (PortableValue, Option<OneBook>) {
        access(book, request, snapshot, Arc::new(AtomicBool::new(false))).unwrap()
    }
    fn context() -> SessionSnapshot {
        let mut context = SessionContext::default();
        context.settings.numeric_version = NumericVersion::Current15751;
        context
            .set_variable(
                "D1",
                PortableValue::List(vec![numeric(1), numeric(2), numeric(4)]),
            )
            .unwrap();
        context.snapshot()
    }
    #[test]
    fn all_thirteen_calculator_results_come_from_real_weighted_computation() {
        let snapshot = context();
        let book = OneBook::default();
        let (result, changed) = access_ok(
            book,
            &request("SetFreq", vec![syntax("H1"), syntax("3")], None),
            &snapshot,
        );
        assert_eq!(result, numeric(1));
        let book = changed.unwrap();
        assert!(book.analyses[0].enabled);
        let (result, changed) = access_ok(
            book,
            &request("Do1VStats", vec![syntax("H1")], None),
            &snapshot,
        );
        assert_eq!(result, numeric(1));
        let book = changed.unwrap();
        let controls: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/native-statistics-one-definition-controls.json"
        ))
        .unwrap();
        for (name, text) in controls["frequency_three"]["results"].as_object().unwrap() {
            let (result, changed) =
                access_ok(book.clone(), &request(name, vec![], None), &snapshot);
            assert!(changed.is_none());
            assert_eq!(
                result.display(&snapshot.settings),
                text.as_str().unwrap(),
                "{name}"
            );
        }
        assert_eq!(
            snapshot.globals["D1"],
            PortableValue::List(vec![numeric(1), numeric(2), numeric(4)])
        );
        assert!(!snapshot.globals.contains_key("MeanX"));
    }
    #[test]
    fn definition_assignment_results_getters_and_checkbox_behavior_match_original_controls() {
        let snapshot = context();
        let input = PortableValue::List(vec![
            PortableValue::String("D1".into()),
            PortableValue::String("3".into()),
            numeric(2),
            numeric(2),
            numeric(0x123456),
        ]);
        let (result, book) = access_ok(
            OneBook::default(),
            &request("H5", vec![], Some(input.clone())),
            &snapshot,
        );
        assert_eq!(result, input);
        let mut book = book.unwrap();
        assert!(!book.analyses[4].enabled);
        book.analyses[4].enabled = true;
        let (_, book) = access_ok(
            book,
            &request("H5", vec![numeric(2)], Some(numeric(2))),
            &snapshot,
        );
        let book = book.unwrap();
        assert!(book.analyses[4].enabled);
        let (frequency, unchanged) = access_ok(
            book.clone(),
            &request("H5", vec![numeric(2)], None),
            &snapshot,
        );
        assert_eq!(frequency, syntax("2"));
        assert!(unchanged.is_none());
        let (getter, _) = access_ok(book.clone(), &request("H5", vec![], None), &snapshot);
        let PortableValue::List(fields) = getter else {
            panic!()
        };
        assert_eq!(fields[0], syntax("D1"));
        let PortableValue::Integer(color) = fields[4] else {
            panic!()
        };
        assert_eq!(color.raw_value(), 0x123456);
        assert_eq!(color.bits(), 24);
        for (field, value) in [(0, numeric(2)), (4, numeric(0)), (5, numeric(0x1000000))] {
            assert!(
                access(
                    book.clone(),
                    &request("H5", vec![numeric(field)], Some(value)),
                    &snapshot,
                    Arc::new(AtomicBool::new(false))
                )
                .is_err()
            );
        }
    }
    #[test]
    fn commands_capture_source_preserve_failed_results_and_resist_cancellation() {
        let snapshot = context();
        let book = OneBook::default();
        let (_, book) = access_ok(
            book,
            &request("SetSample", vec![syntax("H1"), syntax("D1+1")], None),
            &snapshot,
        );
        let (_, book) = access_ok(
            book.unwrap(),
            &request("Do1VStats", vec![syntax("H1")], None),
            &snapshot,
        );
        let book = book.unwrap();
        assert_eq!(book.analyses[0].data_expression(), "D1+1");
        let (mean, _) = access_ok(book.clone(), &request("MeanX", vec![], None), &snapshot);
        assert_eq!(mean.display(&snapshot.settings), "3.33333333333");
        let (_, book) = access_ok(
            book,
            &request("SetFreq", vec![syntax("H1"), syntax("-1")], None),
            &snapshot,
        );
        let book = book.unwrap();
        assert!(
            access(
                book.clone(),
                &request("Do1VStats", vec![syntax("H1")], None),
                &snapshot,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        let (retained, _) = access_ok(book.clone(), &request("MeanX", vec![], None), &snapshot);
        assert_eq!(mean, retained);
        assert!(
            access(
                book.clone(),
                &request("SetSample", vec![syntax("H1"), syntax("\"D1\"")], None),
                &snapshot,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        let error = access(
            book,
            &request("SetFreq", vec![syntax("H1"), syntax("1")], None),
            &snapshot,
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Interrupted);
    }
    #[test]
    fn original_only_second_then_both_checked_retains_the_previous_result_column_until_recalculation()
     {
        let snapshot = context();
        let mut book = OneBook::default();
        book.analyses[0].enabled = false;
        book.analyses[1] = book.analyses[0].clone();
        book.analyses[1].enabled = true;
        book.analyses[1].data_source = Some("D1*10".into());
        let (_, changed) = access_ok(
            book,
            &request("Do1VStats", vec![syntax("H2")], None),
            &snapshot,
        );
        let mut book = changed.unwrap();
        assert_eq!(book.result_analysis, Some(1));
        book.analyses[0].enabled = true;
        let (before, _) = access_ok(book.clone(), &request("MeanX", vec![], None), &snapshot);
        assert_eq!(before.display(&snapshot.settings), "23.3333333333");
        let restored: OneBook =
            serde_json::from_value(serde_json::to_value(&book).unwrap()).unwrap();
        let (restored_result, _) = access_ok(restored, &request("MeanX", vec![], None), &snapshot);
        assert_eq!(before, restored_result);
        let (_, changed) = access_ok(
            book,
            &request("Do1VStats", vec![syntax("H2")], None),
            &snapshot,
        );
        let book = changed.unwrap();
        assert_eq!(book.result_analysis, Some(0));
        let (after, _) = access_ok(book.clone(), &request("MeanX", vec![], None), &snapshot);
        assert_eq!(after.display(&snapshot.settings), "2.33333333333");
        assert!(book.result_values.contains_key("H1") && book.result_values.contains_key("H2"));
        let mut invalid = book.clone();
        invalid.analyses[1].frequency_source = Some("-1".into());
        let before = invalid.result_values.clone();
        assert!(
            access(
                invalid.clone(),
                &request("Do1VStats", vec![syntax("H1")], None),
                &snapshot,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        assert_eq!(invalid.result_values, before);
    }
    #[test]
    fn original_setfreq_literal_one_restores_factory_field_without_unchecking_but_indexed_and_expression_one_retain_source()
     {
        let snapshot = context();
        let (_, book) = access_ok(
            OneBook::default(),
            &request("SetFreq", vec![syntax("H1"), syntax("1")], None),
            &snapshot,
        );
        let book = book.unwrap();
        assert!(book.analyses[0].enabled);
        assert!(book.analyses[0].frequency_expression().is_empty());
        let (_, book) = access_ok(
            book,
            &request("H1", vec![numeric(2)], Some(numeric(1))),
            &snapshot,
        );
        let book = book.unwrap();
        assert!(book.analyses[0].enabled);
        assert_eq!(book.analyses[0].frequency_expression(), "1");
        let (_, book) = access_ok(
            book,
            &request("SetFreq", vec![syntax("H1"), syntax("1+0")], None),
            &snapshot,
        );
        let book = book.unwrap();
        assert!(book.analyses[0].enabled);
        assert_eq!(book.analyses[0].frequency_expression(), "1+0");
    }
    #[test]
    fn named_results_follow_checked_definitions_and_do_not_follow_the_last_computation() {
        let snapshot = context();
        let mut book = OneBook::default();
        book.analyses[1] = book.analyses[0].clone();
        book.analyses[1].data_source = Some("D1*10".into());
        let (_, book) = access_ok(
            book,
            &request("Do1VStats", vec![syntax("H1")], None),
            &snapshot,
        );
        let (_, book) = access_ok(
            book.unwrap(),
            &request("Do1VStats", vec![syntax("H2")], None),
            &snapshot,
        );
        let book = book.unwrap();
        let (mean, _) = access_ok(book.clone(), &request("MeanX", vec![], None), &snapshot);
        assert_eq!(mean.display(&snapshot.settings), "2.33333333333");
        let error = access(
            book.clone(),
            &request("MeanX", vec![syntax("H2")], None),
            &snapshot,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        let mut no_checked = book;
        for definition in &mut no_checked.analyses {
            definition.enabled = false;
        }
        assert!(
            access(
                no_checked,
                &request("MeanX", vec![], None),
                &snapshot,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
    }
    #[test]
    fn source_setters_preserve_scalar_expression_spelling_then_computation_rejects_it() {
        let snapshot = context();
        for source in ["(2)", "1+1"] {
            let (_, changed) = access_ok(
                OneBook::default(),
                &request("SetFreq", vec![syntax("H1"), syntax(source)], None),
                &snapshot,
            );
            let book = changed.unwrap();
            assert!(book.analyses[0].enabled);
            let (stored, _) = access_ok(
                book.clone(),
                &request("H1", vec![numeric(2)], None),
                &snapshot,
            );
            assert_eq!(stored, syntax(source));
            let before = book.result_values.clone();
            assert!(
                access(
                    book.clone(),
                    &request("Do1VStats", vec![syntax("H1")], None),
                    &snapshot,
                    Arc::new(AtomicBool::new(false))
                )
                .is_err()
            );
            assert_eq!(book.result_values, before);
        }
    }
}
