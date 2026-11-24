//! Repaint numeric history from typed values without rewriting recall source.
use super::{AnswerPresentation, HistoryDisplay, HomeEntry};
use ppl_math::{Complex, ComplexPresentation, RealPresentation};
use ppl_vm::{Settings, Value};
use std::rc::Rc;

fn restore(v: &serde_json::Value, depth: usize) -> Option<Value> {
    if depth > 64 {
        return None;
    }
    let real = |key: &str, origin: &str| {
        let mut image = serde_json::json!({"kind":"real","value":v[key].as_str()?});
        if let Some(origin) = v.get(origin) {
            image["origin"] = origin.clone();
        }
        match ppl_vm::session::decode_value(&image).ok()? {
            ppl_vm::PortableValue::Real(value) => Some(value),
            _ => None,
        }
    };
    Some(match v["type"].as_str()? {
        "real" => Value::Real(real("value", "origin")?.with_presentation(
            if v["presentation"] == "sexagesimal" {
                RealPresentation::Sexagesimal
            } else {
                RealPresentation::Decimal
            },
        )),
        "complex" => Value::Complex(
            Complex::new(
                real("real", "real_origin")?,
                real("imaginary", "imaginary_origin")?,
            )
            .with_presentation(match v["presentation"].as_str() {
                Some("polar") => ComplexPresentation::Polar,
                Some("sexagesimal") => ComplexPresentation::Sexagesimal,
                _ => ComplexPresentation::Rectangular,
            }),
        ),
        "integer" if v.get("word").is_some() => {
            let value = ppl_vm::session::decode_value(&v["word"]).ok()?.to_value();
            if !matches!(value, Value::Int(_)) {
                return None;
            }
            value
        }
        "integer" => {
            // A display literal can omit its word width. Legacy Debug-value
            // profiles do not carry recoverable typed bits, so keep their
            // existing paint rather than guessing a 32-bit word.
            let bits = u8::try_from(v["bits"].as_u64()?)
                .ok()
                .filter(|n| (1..=64).contains(n))?;
            let unsigned = v["unsigned"].as_str()?.parse::<u64>().ok()?;
            let literal = v["literal"].as_str().or_else(|| v["display"].as_str())?;
            let body = literal.strip_prefix('#')?;
            let explicit = if body.contains(':') {
                body.to_owned()
            } else {
                format!("{body}:{bits}")
            };
            let word = ppl_math::HpInt::parse_literal(&explicit, ppl_math::IntBase::Hex)?;
            if word.bits() != bits || word.unsigned() != unsigned {
                return None;
            }
            Value::Int(word)
        }
        "string" => {
            if let Some(units) = v["utf16"].as_array() {
                Value::from_utf16(
                    units
                        .iter()
                        .map(|u| u.as_u64().and_then(|u| u16::try_from(u).ok()))
                        .collect::<Option<Vec<_>>>()?,
                )
            } else {
                Value::Str(Rc::from(v["value"].as_str()?))
            }
        }
        "symbolic" => return None,
        "list" => Value::List(Rc::new(
            v["value"]
                .as_array()?
                .iter()
                .map(|v| restore(v, depth + 1))
                .collect::<Option<Vec<_>>>()?,
        )),
        "matrix" => {
            let rows = usize::try_from(v["rows"].as_u64()?).ok()?;
            let cols = usize::try_from(v["cols"].as_u64()?).ok()?;
            let data = v["value"]
                .as_array()?
                .iter()
                .map(|v| restore(v, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            if rows.checked_mul(cols)? != data.len() {
                return None;
            }
            Value::Matrix(Rc::new(ppl_vm::value::Matrix {
                rows,
                cols,
                data,
                vector: v["vector"].as_bool().unwrap_or(false),
            }))
        }
        _ => return None,
    })
}
pub fn recover_value(typed: &serde_json::Value) -> Option<ppl_vm::PortableValue> {
    restore(typed, 0).map(|value| ppl_vm::PortableValue::from(&value))
}
fn annotate(v: &mut serde_json::Value, value: &Value, s: &Settings) {
    match value {
        Value::List(values) => {
            for (v, value) in v["value"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .zip(values.iter())
            {
                annotate(v, value, s)
            }
        }
        Value::Matrix(m) => {
            for (v, value) in v["value"].as_array_mut().unwrap().iter_mut().zip(&m.data) {
                annotate(v, value, s)
            }
        }
        _ => {
            if v.get("display").is_some() {
                v["display"] = value.string_form(s.home_format()).into();
            }
        }
    }
}
fn needs_recall(value: &Value) -> bool {
    match value {
        Value::Int(_) => true,
        Value::Real(value) => value.origin() != ppl_math::RealOrigin::LegacyBcd,
        Value::Complex(value) => {
            value.re.origin() != ppl_math::RealOrigin::LegacyBcd
                || value.im.origin() != ppl_math::RealOrigin::LegacyBcd
        }
        Value::List(v) => v.iter().any(needs_recall),
        Value::Matrix(m) => m.data.iter().any(needs_recall),
        _ => false,
    }
}
pub(super) fn annotate_recall(typed: &mut serde_json::Value, value: &Value) {
    if needs_recall(value) {
        typed["recall"] = recall(value).into();
    }
}
pub(super) fn recall(value: &Value) -> String {
    match value {
        Value::Real(value) => value.ppl_source(),
        Value::Complex(value) => format!("({},{})", value.re.ppl_source(), value.im.ppl_source()),
        Value::Int(w) => {
            let literal = w.with_base(w.base).format_for(0, false, w.base);
            if w.is_system_base() {
                format!("Setbase({literal},0)")
            } else {
                literal
            }
        }
        Value::List(v) => format!("{{{}}}", v.iter().map(recall).collect::<Vec<_>>().join(",")),
        Value::Matrix(m) => {
            let rows = (0..m.rows)
                .map(|r| {
                    format!(
                        "[{}]",
                        m.row(r).iter().map(recall).collect::<Vec<_>>().join(",")
                    )
                })
                .collect::<Vec<_>>();
            if m.vector {
                rows.first().cloned().unwrap_or_else(|| "[]".into())
            } else {
                format!("[{}]", rows.join(","))
            }
        }
        _ => {
            let wrapped = Value::List(Rc::new(vec![value.clone()]))
                .string_form(ppl_vm::value::default_format());
            wrapped[1..wrapped.len() - 1].into()
        }
    }
}
pub fn refresh(entry: &mut HomeEntry, s: &Settings) -> bool {
    if entry.error
        || matches!(
            entry.presentation,
            Some(AnswerPresentation::CommonFraction | AnswerPresentation::MixedFraction)
        )
    {
        return false;
    }
    let Some(typed) = &mut entry.typed_result else {
        return false;
    };
    if !matches!(
        typed["type"].as_str(),
        Some("real" | "complex" | "integer" | "list" | "matrix")
    ) {
        return false;
    }
    let Some(mut value) = restore(typed, 0) else {
        return false;
    };
    match (&mut value, entry.presentation) {
        (Value::Real(real), Some(AnswerPresentation::Decimal)) => {
            *real = real.with_presentation(RealPresentation::Decimal)
        }
        (Value::Complex(complex), Some(AnswerPresentation::Decimal)) => {
            *complex = complex.with_presentation(ComplexPresentation::Rectangular)
        }
        (Value::Real(real), Some(AnswerPresentation::Sexagesimal)) => {
            *real = real.with_presentation(RealPresentation::Sexagesimal)
        }
        (Value::Complex(complex), Some(AnswerPresentation::Sexagesimal)) => {
            *complex = complex.with_presentation(ComplexPresentation::Sexagesimal)
        }
        (Value::Complex(complex), Some(AnswerPresentation::Polar)) => {
            *complex = complex.with_presentation(ComplexPresentation::Polar)
        }
        _ => {}
    }
    let text = value.string_form(s.home_format());
    let old = entry.home_display.as_ref().map(|d| d.text.clone());
    entry.home_display = (text != entry.result).then(|| HistoryDisplay {
        source: entry.result.clone(),
        text,
    });
    let before = typed.clone();
    annotate_recall(typed, &value);
    annotate(typed, &value, s);
    old != entry.home_display.as_ref().map(|d| d.text.clone()) || before != *typed
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::native::{NativeUiState, portable_display_json};
    use ppl_math::Real;
    #[test]
    fn submitted_long_number_uses_calculator_notation_without_rewriting_its_source() {
        let value = ppl_vm::PortableValue::Real(
            Real::parse_versioned(
                "1234567890123456789",
                ppl_math::NumericVersion::Current15751,
            )
            .unwrap(),
        );
        let mut state = NativeUiState::default();
        state.add_history(
            "1234567890123456789".into(),
            value.display(&Settings::default()),
            false,
            Some(super::super::portable_json(&value)),
        );
        assert_eq!(state.history[0].display_expression(), "1.23456789012E18");
        assert_eq!(state.history[0].expression, "1234567890123456789");
        state.history_position = Some(0);
        state.history_result = false;
        assert_eq!(state.history_text(), Some("1234567890123456789"));
    }
    #[test]
    fn exact_history_origin_is_validated_and_recursive_copy_keeps_the_number_model() {
        use ppl_vm::PortableValue as P;
        let exact = Real::parse_versioned(
            "1234567890123456789",
            ppl_math::NumericVersion::Current15751,
        )
        .unwrap();
        let ratio = Real::exact_rational(false, 1, 3).unwrap();
        let pi = Real::exact_pi_multiple(false, 1, 7).unwrap();
        let decimal = Real::parse("0.25").unwrap().decimal();
        let value = P::List(vec![
            P::Real(exact),
            P::Real(ratio),
            P::Real(pi),
            P::Real(decimal),
        ]);
        let mut settings = Settings {
            numeric_version: ppl_math::NumericVersion::Current15751,
            ..Default::default()
        };
        let mut state = NativeUiState::default();
        state.add_history(
            "L1".into(),
            value.display(&settings),
            false,
            Some(portable_display_json(&value, &settings)),
        );
        assert_eq!(
            state.history[0].typed_result.as_ref().unwrap()["recall"],
            "{1.234567890123456789e18,(1/3),(1*π/7),approx(0.25)}"
        );
        assert!(refresh(&mut state.history[0], &settings));
        assert_eq!(
            recover_value(state.history[0].typed_result.as_ref().unwrap())
                .map(|v| ppl_vm::session::encode_value(&v)),
            Some(ppl_vm::session::encode_value(&value))
        );
        state.history_position = Some(0);
        state.history_result = true;
        assert_eq!(
            state.history_text(),
            Some("{1.234567890123456789e18,(1/3),(1*π/7),approx(0.25)}")
        );
        state.copy_history_to_entry();
        let source = state.home_input.clone();
        let mut context = ppl_vm::SessionContext::new(settings.clone(), 1);
        let recalled = context
            .evaluate_home(
                &source,
                Box::new(ppl_vm::HeadlessHost::new()),
                Default::default(),
            )
            .result
            .unwrap();
        assert_eq!(
            ppl_vm::session::encode_value(&P::from(&recalled)),
            ppl_vm::session::encode_value(&value)
        );
        settings.set_format(1, 2);
        refresh(&mut state.history[0], &settings);
        state.history_position = Some(0);
        state.history_result = true;
        assert_eq!(state.history_text(), Some(source.as_str()));
        let saved = serde_json::to_value(&state.history[0]).unwrap();
        let reopened: HomeEntry = serde_json::from_value(saved).unwrap();
        assert_eq!(
            recover_value(reopened.typed_result.as_ref().unwrap())
                .map(|v| ppl_vm::session::encode_value(&v)),
            Some(ppl_vm::session::encode_value(&value))
        );
        let mut bad = super::super::portable_json(&P::Real(ratio));
        bad["origin"]["denominator"] = "2".into();
        assert!(recover_value(&bad).is_none());
    }
    #[test]
    fn complex_history_components_keep_their_typed_origins_and_presentation() {
        let re = Real::parse_versioned(
            "1234567890123456789",
            ppl_math::NumericVersion::Current15751,
        )
        .unwrap();
        let im = Real::parse("0.25").unwrap().decimal();
        let value = ppl_vm::PortableValue::Complex(Complex::new(re, im));
        let typed = super::super::portable_json(&value);
        let Some(ppl_vm::PortableValue::Complex(restored)) = recover_value(&typed) else {
            panic!("typed complex")
        };
        assert_eq!(restored.re.origin(), re.origin());
        assert_eq!(restored.im.origin(), im.origin());
        assert_eq!(
            recall(&Value::Complex(restored)),
            "(1.234567890123456789e18,approx(0.25))"
        );
    }
    #[test]
    fn word_restore_requires_exact_width_and_payload_and_leaves_legacy_paint_intact() {
        for (bits, unsigned, literal) in [
            (8, "255", "#FFh"),
            (64, "18446744073709551615", "#FFFFFFFFFFFFFFFFh"),
        ] {
            let typed = serde_json::json!({"type":"integer","literal":literal,"bits":bits,"unsigned":unsigned});
            let Some(Value::Int(word)) = restore(&typed, 0) else {
                panic!("valid word rejected")
            };
            assert_eq!(word.bits(), bits);
            assert_eq!(word.unsigned().to_string(), unsigned);
        }
        for typed in [
            serde_json::json!({"type":"integer","display":"#FFh"}),
            serde_json::json!({"type":"integer","literal":"#FF:64h","bits":8,"unsigned":"255"}),
            serde_json::json!({"type":"integer","literal":"#100h","bits":8,"unsigned":"256"}),
            serde_json::json!({"type":"integer","literal":"#FFh","bits":8,"unsigned":"1"}),
        ] {
            assert!(restore(&typed, 0).is_none());
        }
        let settings = Settings::default();
        let mut state = NativeUiState::default();
        let value = ppl_vm::PortableValue::List(vec![
            ppl_vm::PortableValue::Real(Real::from_i64(1000)),
            ppl_vm::PortableValue::Integer(ppl_math::HpInt::new(255, 8, ppl_math::IntBase::Hex)),
        ]);
        state.add_history(
            "L1".into(),
            "{1000,#FFh}".into(),
            false,
            Some({
                let mut typed = portable_display_json(&value, &settings);
                typed["value"][1].as_object_mut().unwrap().remove("word");
                typed
            }),
        );
        let before = serde_json::to_value(&state.history[0]).unwrap();
        assert!(!refresh(state.history.back_mut().unwrap(), &settings));
        assert_eq!(serde_json::to_value(&state.history[0]).unwrap(), before);
    }
    #[test]
    fn words_repaint_and_recall_preserve_carrier_width_sign_and_system_base() {
        use ppl_math::{HpInt, IntBase};
        for (number, width) in [(255_i128, -8), (-5, -8), (u64::MAX as i128, 64)] {
            for system in [false, true] {
                let word = HpInt::new_with_width(number, width, IntBase::Hex)
                    .unwrap()
                    .with_system_base(system);
                let value = ppl_vm::PortableValue::Integer(word);
                let mut old = Settings::default();
                old.set_separator(3).unwrap();
                let mut state = NativeUiState::default();
                state.add_history(
                    "WORD".into(),
                    value.display(&old),
                    false,
                    Some(portable_display_json(&value, &old)),
                );
                let initial_source = state.history[0].typed_result.as_ref().unwrap()["recall"]
                    .as_str()
                    .expect("new word history must already contain exact recall source");
                let initial = ppl_vm::SessionContext::new(old.clone(), 11)
                    .evaluate_home(
                        initial_source,
                        Box::new(ppl_vm::HeadlessHost::default()),
                        ppl_vm::Limits::default(),
                    )
                    .result
                    .unwrap();
                assert!(matches!(initial, Value::Int(w) if w == word));
                let mut settings = old.clone();
                settings.set_integer_bits(64).unwrap();
                settings.int_base = IntBase::Dec;
                refresh(state.history.back_mut().unwrap(), &settings);
                let restored = restore(state.history[0].typed_result.as_ref().unwrap(), 0).unwrap();
                assert!(matches!(restored,Value::Int(w)if w==word));
                assert_eq!(state.history[0].display_result(), value.display(&settings));
                state.history_jump(false);
                state.copy_history_to_entry();
                let mut context = ppl_vm::SessionContext::new(settings, 11);
                let result = context
                    .evaluate_home(
                        &state.home_input,
                        Box::new(ppl_vm::HeadlessHost::default()),
                        ppl_vm::Limits::default(),
                    )
                    .result
                    .unwrap();
                assert!(
                    matches!(result,Value::Int(w)if w==word),
                    "{}: {result:?}",
                    state.home_input
                );
            }
        }
    }
    #[test]
    fn mixed_list_word_recall_keeps_strings_and_word_metadata() {
        let word = ppl_math::HpInt::new_with_width(-5, -8, ppl_math::IntBase::Hex)
            .unwrap()
            .with_system_base(true);
        let value = ppl_vm::PortableValue::List(vec![
            ppl_vm::PortableValue::String("a,b".into()),
            ppl_vm::PortableValue::Integer(word),
            ppl_vm::PortableValue::Real(Real::from_i64(1000)),
        ]);
        let settings = Settings::default();
        let mut state = NativeUiState::default();
        state.add_history(
            "L1".into(),
            value.display(&settings),
            false,
            Some(portable_display_json(&value, &settings)),
        );
        refresh(state.history.back_mut().unwrap(), &settings);
        state.history_jump(false);
        state.copy_history_to_entry();
        let mut context = ppl_vm::SessionContext::default();
        let result = context
            .evaluate_home(
                &state.home_input,
                Box::new(ppl_vm::HeadlessHost::default()),
                ppl_vm::Limits::default(),
            )
            .result
            .unwrap();
        assert_eq!(ppl_vm::PortableValue::from(&result), value);
    }
    #[test]
    fn settings_repaint_existing_results_without_inserting_separators_in_recall() {
        let mut state = NativeUiState::default();
        let mut settings = Settings::default();
        let value = ppl_vm::PortableValue::Real(Real::parse("123456.789").unwrap());
        state.add_history(
            "123456.789".into(),
            "123456.789".into(),
            false,
            Some(portable_display_json(&value, &settings)),
        );
        assert!(refresh(state.history.back_mut().unwrap(), &settings));
        assert_eq!(state.history[0].display_result(), "123﹐456.789");
        settings.set_separator(3).unwrap();
        assert!(refresh(state.history.back_mut().unwrap(), &settings));
        assert_eq!(state.history[0].display_result(), "123456.789");
        settings.set_format(1, 2);
        refresh(state.history.back_mut().unwrap(), &settings);
        assert_eq!(state.history[0].display_result(), "123456.79");
        state.history_jump(false);
        state.copy_history_to_entry();
        assert_eq!(state.home_input, "123456.789");
        assert_eq!(
            state.history[0].typed_result.as_ref().unwrap()["value"],
            "123456.789"
        );
    }
    #[test]
    fn repaint_respects_selected_decimal_projection_of_formatted_values() {
        let mut state = NativeUiState::default();
        let mut settings = Settings::default();
        let value = ppl_vm::PortableValue::Real(
            Real::parse("1.375")
                .unwrap()
                .with_presentation(RealPresentation::Sexagesimal),
        );
        state.add_history(
            "1°22′30″".into(),
            "1.375".into(),
            false,
            Some(portable_display_json(&value, &settings)),
        );
        state.history[0].presentation = Some(AnswerPresentation::Decimal);
        settings.set_format(1, 2);
        refresh(state.history.back_mut().unwrap(), &settings);
        assert_eq!(state.history[0].display_result(), "1.38");
        assert_eq!(
            state.history[0].typed_result.as_ref().unwrap()["presentation"],
            "sexagesimal"
        );
    }
    #[test]
    fn changing_complex_paint_keeps_allow_flag_and_fraction_projection() {
        let mut state = NativeUiState::default();
        let mut settings = Settings::default();
        let value =
            ppl_vm::PortableValue::Complex(Complex::new(Real::from_i64(3), Real::from_i64(4)));
        state.add_history(
            "3+4*i".into(),
            "3+4*i".into(),
            false,
            Some(portable_display_json(&value, &settings)),
        );
        settings.complex_display = ppl_math::ComplexFormat::Pair;
        assert!(refresh(state.history.back_mut().unwrap(), &settings));
        assert_eq!(state.history[0].display_result(), "(3,4)");
        assert!(!settings.complex_results);
        state.history[0].presentation = Some(AnswerPresentation::CommonFraction);
        state.history[0].result = "7/2".into();
        assert!(!refresh(state.history.back_mut().unwrap(), &settings));
        assert_eq!(state.history[0].display_result(), "7/2");
    }
}
