//! PPL legacy arguments and lossless JSON data arguments.
use ppl_vm::{RuntimeError, Settings, Value};
use serde_json::{Value as Json, json, value::RawValue};
use std::rc::Rc;
pub fn ppl_args(text: Option<&str>, settings: &Settings) -> Result<Vec<Value>, String> {
    let Some(text) = text else { return Ok(vec![]) };
    let mut parts = vec![];
    let (mut depth, mut in_str, mut cur) = (0i32, false, String::new());
    for c in text.chars() {
        match c {
            '"' => in_str = !in_str,
            '{' | '[' | '(' if !in_str => depth += 1,
            '}' | ']' | ')' if !in_str => depth -= 1,
            ',' if depth == 0 && !in_str => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c)
    }
    parts.push(cur);
    parts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            ppl_vm::eval_text_with(s, settings).map_err(|e| format!("bad argument `{s}`: {e}"))
        })
        .collect()
}
pub fn json_args(text: &str, settings: &Settings) -> Result<Vec<Value>, String> {
    let a: Vec<Box<RawValue>> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    a.iter().map(|x| decode(x.get(), settings)).collect()
}
fn decode(s: &str, settings: &Settings) -> Result<Value, String> {
    if s.starts_with('"') {
        return serde_json::from_str::<String>(s)
            .map(|s| Value::Str(Rc::from(s)))
            .map_err(|e| e.to_string());
    }
    if s.starts_with('[') {
        return json_args(s, settings).map(|v| Value::List(Rc::new(v)));
    }
    if s.starts_with('{') {
        let o: std::collections::BTreeMap<String, Box<RawValue>> =
            serde_json::from_str(s).map_err(|e| e.to_string())?;
        let field = |name: &str| {
            o.get(name)
                .map(|x| x.get())
                .ok_or_else(|| format!("missing tagged argument field {name}"))
        };
        let string =
            |name: &str| serde_json::from_str::<String>(field(name)?).map_err(|e| e.to_string());
        let kind = string("type")?;
        let presentation = o
            .get("presentation")
            .map(|value| serde_json::from_str::<String>(value.get()).map_err(|e| e.to_string()))
            .transpose()?;
        if presentation.is_some()
            && !matches!(
                kind.as_str(),
                "real" | "complex" | "rational" | "pi_multiple"
            )
        {
            return Err("Presentation metadata requires a numeric argument".into());
        }
        return match kind.as_str() {
            "rational" | "pi_multiple" => {
                if settings.numeric_version != ppl_math::NumericVersion::Current15751 {
                    return Err("Exact real arguments require numeric version15751".into());
                }
                let coefficient = |name: &str| {
                    let text = string(name)?;
                    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(format!("Invalid unsigned exact coefficient {name}"));
                    }
                    text.parse::<u64>()
                        .map_err(|_| format!("Exact coefficient {name} exceeds64bits"))
                };
                let numerator = coefficient("numerator")?;
                let denominator = coefficient("denominator")?;
                if denominator == 0 {
                    return Err("Exact denominator must be nonzero".into());
                }
                let negative: bool =
                    serde_json::from_str(field("negative")?).map_err(|e| e.to_string())?;
                let value = if kind == "rational" {
                    ppl_math::Real::exact_rational(negative, numerator, denominator)
                } else {
                    ppl_math::Real::exact_pi_multiple(negative, numerator, denominator)
                }
                .map_err(|e| e.to_string())?;
                let presentation = match presentation.as_deref() {
                    None => ppl_math::RealPresentation::Decimal,
                    Some("sexagesimal") => ppl_math::RealPresentation::Sexagesimal,
                    _ => return Err("Unknown real presentation".into()),
                };
                Ok(Value::Real(value.with_presentation(presentation)))
            }
            "real" => {
                let format = match presentation.as_deref() {
                    None => ppl_math::RealPresentation::Decimal,
                    Some("sexagesimal") => ppl_math::RealPresentation::Sexagesimal,
                    _ => return Err("Unknown real presentation".into()),
                };
                ppl_math::Real::parse_versioned(&string("value")?, settings.numeric_version)
                    .map(|value| Value::Real(value.with_presentation(format)))
                    .ok_or("invalid real".into())
            }
            "utf16" => {
                let units: Vec<u16> =
                    serde_json::from_str(field("value")?).map_err(|e| e.to_string())?;
                if units.len() > 1_000_000 {
                    return Err("UTF-16 argument exceeds unit limit".into());
                }
                Ok(Value::from_utf16(units))
            }
            "string" => {
                if o.contains_key("utf16") {
                    if o.contains_key("value") {
                        return Err("String must supply value or utf16, not both".into());
                    }
                    let units: Vec<u16> =
                        serde_json::from_str(field("utf16")?).map_err(|e| e.to_string())?;
                    if units.len() > 1_000_000 {
                        return Err("UTF-16 argument exceeds unit limit".into());
                    }
                    Ok(Value::from_utf16(units))
                } else {
                    Ok(Value::Str(Rc::from(string("value")?)))
                }
            }
            "list" => json_args(field("value")?, settings).map(|v| Value::List(Rc::new(v))),
            "integer" => {
                let literal = string("literal")?;
                let declared_bits = o
                    .get("bits")
                    .map(|value| {
                        serde_json::from_str::<u8>(value.get()).map_err(|error| error.to_string())
                    })
                    .transpose()?;
                if declared_bits.is_some_and(|bits| !(1..=64).contains(&bits)) {
                    return Err("integer width must be1..64".into());
                }
                let number = ppl_math::HpInt::parse_literal_with_defaults(
                    literal.strip_prefix('#').unwrap_or(&literal),
                    ppl_math::IntBase::Hex,
                    declared_bits.unwrap_or(32) as i64,
                )
                .ok_or("invalid integer literal")?;
                let bits = declared_bits.unwrap_or(number.bits());
                let flag = |name: &str, default| {
                    o.get(name)
                        .map(|value| {
                            serde_json::from_str::<bool>(value.get())
                                .map_err(|error| error.to_string())
                        })
                        .transpose()
                        .map(|value| value.unwrap_or(default))
                };
                let signed = flag("signed_word", number.is_signed())?;
                let width = if signed { -(bits as i64) } else { bits as i64 };
                Ok(Value::Int(
                    ppl_math::HpInt::new_with_width(number.numeric_value(), width, number.base)
                        .ok_or("invalid signed word width")?
                        .with_system_base(flag("system_base", number.is_system_base())?),
                ))
            }
            "complex" => {
                let re = ppl_math::Real::parse(&string("real")?)
                    .ok_or("invalid complex real component")?;
                let im = ppl_math::Real::parse(&string("imaginary")?)
                    .ok_or("invalid complex imaginary component")?;
                let format = match presentation.as_deref() {
                    None => ppl_math::ComplexPresentation::Rectangular,
                    Some("sexagesimal") => ppl_math::ComplexPresentation::Sexagesimal,
                    Some("polar") => ppl_math::ComplexPresentation::Polar,
                    _ => return Err("Unknown complex presentation".into()),
                };
                Ok(Value::Complex(
                    ppl_math::Complex::new(re, im).with_presentation(format),
                ))
            }
            "matrix" => {
                let rows: usize =
                    serde_json::from_str(field("rows")?).map_err(|e| e.to_string())?;
                let cols: usize =
                    serde_json::from_str(field("columns")?).map_err(|e| e.to_string())?;
                let data = json_args(field("value")?, settings)?;
                if rows == 0
                    || cols == 0
                    || rows.checked_mul(cols) != Some(data.len())
                    || !data
                        .iter()
                        .all(|v| matches!(v, Value::Real(_) | Value::Complex(_)))
                {
                    return Err("invalid matrix dimensions or entries".into());
                }
                let vector = match o.get("vector") {
                    Some(v) => serde_json::from_str(v.get()).map_err(|e| e.to_string())?,
                    None => false,
                };
                if vector && rows != 1 {
                    return Err("a vector requires one row".into());
                }
                Ok(Value::Matrix(Rc::new(ppl_vm::value::Matrix {
                    rows,
                    cols,
                    data,
                    vector,
                })))
            }
            "grob" => {
                let index: u8 = serde_json::from_str(field("index")?).map_err(|e| e.to_string())?;
                if index > 9 {
                    return Err("GROB index must be 0..9".into());
                }
                Ok(Value::Grob(index))
            }
            "ppl" => ppl_vm::eval_text_with(&string("value")?, settings).map_err(|e| e.to_string()),
            _ => Err(format!("unknown native argument type {kind}")),
        };
    }
    // RawValue preserves exponent and decimal spelling, including > f64 range.
    ppl_math::Real::parse_versioned(s, settings.numeric_version)
        .map(Value::Real)
        .ok_or_else(|| format!("invalid JSON numeric argument {s}"))
}
pub fn encode(v: &Value) -> Json {
    match v {
        Value::Real(x) => {
            let mut value = match x.origin() {
                ppl_math::RealOrigin::Rational {
                    numerator,
                    denominator,
                } => {
                    if let Some(text) = x.exact_decimal() {
                        json!({"type":"real","value":text})
                    } else {
                        json!({"type":"rational","negative":x.is_negative(),"numerator":numerator.to_string(),"denominator":denominator.to_string()})
                    }
                }
                ppl_math::RealOrigin::PiMultiple {
                    numerator,
                    denominator,
                } => {
                    json!({"type":"pi_multiple","negative":x.is_negative(),"numerator":numerator.to_string(),"denominator":denominator.to_string()})
                }
                _ => json!({"type":"real","value":x.to_string()}),
            };
            if x.presentation() == ppl_math::RealPresentation::Sexagesimal {
                value["presentation"] = "sexagesimal".into();
            }
            value
        }
        Value::Str(s) => json!({"type":"string","value":s.as_ref()}),
        Value::RawString(units) => json!({"type":"string","utf16":units.as_ref()}),
        Value::Int(x) => {
            let mut value = json!({"type":"integer","literal":x.to_string(),"bits":x.bits(),"unsigned":x.unsigned().to_string(),"signed":x.signed().to_string()});
            if x.is_signed() {
                value["signed_word"] = true.into();
                value["numeric"] = x.numeric_value().to_string().into();
            }
            if x.is_system_base() {
                value["system_base"] = true.into();
            }
            value
        }
        Value::Complex(x) => {
            let mut value =
                json!({"type":"complex","real":x.re.to_string(),"imaginary":x.im.to_string()});
            match x.presentation() {
                ppl_math::ComplexPresentation::Rectangular => {}
                ppl_math::ComplexPresentation::Sexagesimal => {
                    value["presentation"] = "sexagesimal".into()
                }
                ppl_math::ComplexPresentation::Polar => value["presentation"] = "polar".into(),
            }
            value
        }
        Value::List(xs) => json!({"type":"list","value":xs.iter().map(encode).collect::<Vec<_>>()}),
        Value::Matrix(m) => {
            json!({"type":"matrix","rows":m.rows,"columns":m.cols,"vector":m.vector,"value":m.data.iter().map(encode).collect::<Vec<_>>()})
        }
        Value::Symbolic(v) => json!({"type":"symbolic","value":v.as_ref()}),
        Value::Grob(n) => json!({"type":"grob","index":n}),
    }
}
pub fn error_text(e: RuntimeError) -> String {
    e.to_string()
}
