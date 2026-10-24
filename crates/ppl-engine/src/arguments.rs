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
        return match kind.as_str() {
            "real" => ppl_math::Real::parse(&string("value")?)
                .map(Value::Real)
                .ok_or("invalid real".into()),
            "string" => Ok(Value::Str(Rc::from(string("value")?))),
            "list" => json_args(field("value")?, settings).map(|v| Value::List(Rc::new(v))),
            "integer" => {
                let literal = string("literal")?;
                let number = ppl_math::HpInt::parse_literal(
                    literal.strip_prefix('#').unwrap_or(&literal),
                    ppl_math::IntBase::Hex,
                )
                .ok_or("invalid integer literal")?;
                let bits = match o.get("bits") {
                    Some(v) => serde_json::from_str::<u8>(v.get()).map_err(|e| e.to_string())?,
                    None => number.bits(),
                };
                if !(1..=64).contains(&bits) {
                    return Err("integer width must be 1..64".into());
                }
                Ok(Value::Int(ppl_math::HpInt::new(
                    number.unsigned() as i128,
                    bits,
                    number.base,
                )))
            }
            "complex" => {
                let re = ppl_math::Real::parse(&string("real")?)
                    .ok_or("invalid complex real component")?;
                let im = ppl_math::Real::parse(&string("imaginary")?)
                    .ok_or("invalid complex imaginary component")?;
                Ok(Value::Complex(ppl_math::Complex { re, im }))
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
    ppl_math::Real::parse(s)
        .map(Value::Real)
        .ok_or_else(|| format!("invalid JSON numeric argument {s}"))
}
pub fn encode(v: &Value) -> Json {
    match v {
        Value::Real(x) => json!({"type":"real","value":x.to_string()}),
        Value::Str(s) => json!({"type":"string","value":s.as_ref()}),
        Value::Int(x) => {
            json!({"type":"integer","literal":x.to_string(),"bits":x.bits(),"unsigned":x.unsigned().to_string(),"signed":x.signed().to_string()})
        }
        Value::Complex(x) => {
            json!({"type":"complex","real":x.re.to_string(),"imaginary":x.im.to_string()})
        }
        Value::List(xs) => json!({"type":"list","value":xs.iter().map(encode).collect::<Vec<_>>()}),
        Value::Matrix(m) => {
            json!({"type":"matrix","rows":m.rows,"columns":m.cols,"vector":m.vector,"value":m.data.iter().map(encode).collect::<Vec<_>>()})
        }
        Value::Grob(n) => json!({"type":"grob","index":n}),
    }
}
pub fn error_text(e: RuntimeError) -> String {
    e.to_string()
}
