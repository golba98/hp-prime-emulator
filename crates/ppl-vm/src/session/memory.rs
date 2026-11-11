//! Versioned calculator memory, independent of running VM stacks and host handles.
use super::*;
use ppl_math::IntBase;
use serde_json::{Value as Json, json};

const VERSION: u64 = 1;
const MAX_CELLS: usize = 1_000_000;

pub fn encode_value(value: &PortableValue) -> Json {
    match value {
        PortableValue::Symbolic(v) => json!({"kind":"symbolic","value":v}),
        PortableValue::Real(v) => json!({"kind":"real","value":v.to_string()}),
        PortableValue::Integer(v) => {
            json!({"kind":"integer","value":v.unsigned().to_string(),"bits":v.bits(),"base":base_code(v.base)})
        }
        PortableValue::Complex(v) => {
            json!({"kind":"complex","real":v.re.to_string(),"imaginary":v.im.to_string()})
        }
        PortableValue::String(v) => json!({"kind":"string","value":v}),
        PortableValue::List(v) => {
            json!({"kind":"list","value":v.iter().map(encode_value).collect::<Vec<_>>()})
        }
        PortableValue::Matrix {
            rows,
            cols,
            vector,
            data,
        } => {
            json!({"kind":"matrix","rows":rows,"cols":cols,"vector":vector,"value":data.iter().map(encode_value).collect::<Vec<_>>()})
        }
        PortableValue::Grob(v) => json!({"kind":"grob","value":v}),
    }
}
fn base_code(b: IntBase) -> u8 {
    match b {
        IntBase::Bin => 2,
        IntBase::Oct => 8,
        IntBase::Dec => 10,
        IntBase::Hex => 16,
    }
}
fn base(b: u64) -> Result<IntBase, String> {
    match b {
        2 => Ok(IntBase::Bin),
        8 => Ok(IntBase::Oct),
        10 => Ok(IntBase::Dec),
        16 => Ok(IntBase::Hex),
        _ => Err("Invalid integer base".into()),
    }
}
fn number(j: &Json, key: &str) -> Result<Real, String> {
    Real::parse(j[key].as_str().ok_or("Missing exact decimal")?)
        .ok_or_else(|| "Invalid decimal".to_string())
}

pub fn decode_value(j: &Json) -> Result<PortableValue, String> {
    {
        let mut remaining = MAX_CELLS;
        decode(j, 0, &mut remaining)
    }
}
fn decode(j: &Json, depth: usize, remaining: &mut usize) -> Result<PortableValue, String> {
    if depth > 64 || *remaining == 0 {
        return Err("Calculator value exceeds nesting/element limit".into());
    }
    *remaining -= 1;
    Ok(match j["kind"].as_str().ok_or("Missing value kind")? {
        "symbolic" => {
            let value: ppl_cas::CasValue =
                serde_json::from_value(j["value"].clone()).map_err(|e| e.to_string())?;
            value.validate().map_err(|e| e.to_string())?;
            PortableValue::Symbolic(value)
        }
        "real" => PortableValue::Real(number(j, "value")?),
        "integer" => {
            let bits = j["bits"]
                .as_u64()
                .filter(|b| (1..=64).contains(b))
                .ok_or("Invalid integer width")? as u8;
            let word = j["value"]
                .as_str()
                .ok_or("Missing integer")?
                .parse::<u64>()
                .map_err(|_| "Invalid integer")?;
            if bits < 64 && word >= (1u64 << bits) {
                return Err("Integer exceeds its width".into());
            }
            PortableValue::Integer(HpInt::new(
                word as i128,
                bits,
                base(j["base"].as_u64().ok_or("Missing base")?)?,
            ))
        }
        "complex" => PortableValue::Complex(Complex {
            re: number(j, "real")?,
            im: number(j, "imaginary")?,
        }),
        "string" => PortableValue::String(j["value"].as_str().ok_or("Missing string")?.into()),
        kind @ ("list" | "matrix") => {
            let values = j["value"].as_array().ok_or("Missing collection data")?;
            let data = values
                .iter()
                .map(|v| decode(v, depth + 1, remaining))
                .collect::<Result<Vec<_>, _>>()?;
            if kind == "list" {
                PortableValue::List(data)
            } else {
                let rows = usize::try_from(j["rows"].as_u64().ok_or("Invalid rows")?)
                    .map_err(|_| "Invalid rows")?;
                let cols = usize::try_from(j["cols"].as_u64().ok_or("Invalid columns")?)
                    .map_err(|_| "Invalid columns")?;
                let vector = j["vector"].as_bool().ok_or("Missing vector flag")?;
                if rows.checked_mul(cols) != Some(data.len()) || (vector && rows != 1) {
                    return Err("Matrix dimensions do not match data".into());
                }
                if data.iter().any(|v| {
                    !matches!(
                        v,
                        PortableValue::Real(_)
                            | PortableValue::Integer(_)
                            | PortableValue::Complex(_)
                    )
                }) {
                    return Err("Matrix cells must be numeric".into());
                }
                PortableValue::Matrix {
                    rows,
                    cols,
                    vector,
                    data,
                }
            }
        }
        "grob" => PortableValue::Grob(
            j["value"]
                .as_u64()
                .filter(|v| *v < 10)
                .ok_or("Invalid GROB index")? as u8,
        ),
        _ => return Err("Unknown calculator value kind".into()),
    })
}
fn encode_map(map: &BTreeMap<String, PortableValue>) -> Json {
    Json::Object(
        map.iter()
            .map(|(k, v)| (k.clone(), encode_value(v)))
            .collect(),
    )
}
fn decode_map(j: &Json) -> Result<BTreeMap<String, PortableValue>, String> {
    j.as_object()
        .ok_or("Missing value map")?
        .iter()
        .map(|(k, v)| Ok((k.clone(), decode_value(v)?)))
        .collect()
}
impl SessionContext {
    pub fn memory_json(&self) -> Json {
        json!({"version":VERSION,"cas":self.cas,"active_app":self.active_app,"app_settings":self.app_settings.iter().map(|(name,s)|(name.clone(),json!({"angle":s.angle,"complex":s.complex,"format":s.format,"digits":s.digits}))).collect::<serde_json::Map<String,Json>>(),"app_files":self.app_files.iter().map(|(k,v)|(k.clone(),encode_map(v))).collect::<serde_json::Map<String,Json>>(),"generation":self.generation,"globals":encode_map(&self.globals),
            "modules":self.modules.iter().map(|(k,m)|(k.clone(),json!({"revision":m.revision,"initialized":m.initialized,"values":encode_map(&m.values)}))).collect::<serde_json::Map<String,Json>>(),
            "notes":self.notes,"programs":self.programs,"last_value":self.last_value.as_ref().map(encode_value),
            "settings":{"angle":self.settings.angle.code(),"format":self.settings.hformat,"digits":self.settings.hdigits,"complex":self.settings.complex_results,"base":base_code(self.settings.int_base),"bits":self.settings.int_bits,"signed":self.settings.int_signed,"font_px":self.settings.font_px,"exp_char":self.settings.exp_char.to_string()},
            "rng":self.rng.state(),"terminal":self.terminal,"display":if self.display==DisplayMode::Graphics {"graphics"}else{"terminal"},
            "grobs":self.grobs.iter().map(|g|g.as_ref().map(|g|json!({"width":g.width(),"height":g.height(),"pixels":g.pixels()}))).collect::<Vec<_>>()})
    }
    /// Parse into a fresh context first; malformed/future state never mutates this session.
    pub fn restore_memory(&mut self, j: &Json) -> Result<(), String> {
        if j["version"].as_u64() != Some(VERSION) {
            return Err("Unsupported Native memory version".into());
        }
        let mut next = Self::default();
        if let Some(name) = j["active_app"].as_str() {
            next.active_app = name.into();
        }
        // Version 1 profiles predating app overrides migrate to System defaults.
        // A present but malformed map fails transactionally, never resets data.
        if let Some(apps) = j.get("app_settings") {
            let apps = apps.as_object().ok_or("Invalid app settings map")?;
            if apps.len() > 1024 {
                return Err("Too many app settings".into());
            }
            for (name, value) in apps {
                if name.is_empty() || name.len() > 1024 {
                    return Err("Invalid app settings name".into());
                }
                let settings = crate::AppSettings {
                    angle: value["angle"].as_i64().ok_or("Missing AAngle")?,
                    complex: value["complex"].as_i64().ok_or("Missing AComplex")?,
                    format: value["format"].as_i64().ok_or("Missing AFormat")?,
                    digits: value["digits"].as_i64().ok_or("Missing ADigits")?,
                };
                settings.validate()?;
                next.app_settings.insert(name.clone(), settings);
            }
        }
        if let Some(files) = j["app_files"].as_object() {
            next.app_files = files
                .iter()
                .map(|(k, v)| Ok((k.clone(), decode_map(v)?)))
                .collect::<Result<_, String>>()?;
        }
        if let Some(cas) = j.get("cas") {
            next.cas = serde_json::from_value(cas.clone()).map_err(|e| e.to_string())?;
            next.cas.validate().map_err(|e| e.to_string())?;
        }
        next.globals = decode_map(&j["globals"])?;
        next.modules = j["modules"]
            .as_object()
            .ok_or("Missing modules")?
            .iter()
            .map(|(k, m)| {
                Ok((
                    k.clone(),
                    ModuleState {
                        revision: m["revision"]
                            .as_str()
                            .ok_or("Missing source revision")?
                            .into(),
                        initialized: m["initialized"]
                            .as_bool()
                            .ok_or("Missing initializer state")?,
                        values: decode_map(&m["values"])?,
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        next.notes = serde_json::from_value(j["notes"].clone()).map_err(|e| e.to_string())?;
        next.programs = serde_json::from_value(j["programs"].clone()).map_err(|e| e.to_string())?;
        next.last_value = if j["last_value"].is_null() {
            None
        } else {
            Some(decode_value(&j["last_value"])?)
        };
        let s = &j["settings"];
        next.settings.angle =
            crate::AngleMode::from_code(s["angle"].as_i64().ok_or("Missing angle")?)
                .ok_or("Invalid angle")?;
        let format = s["format"]
            .as_i64()
            .filter(|v| (0..=5).contains(v))
            .ok_or("Invalid format")?;
        let digits = s["digits"]
            .as_i64()
            .filter(|v| (0..=11).contains(v))
            .ok_or("Invalid digits")?;
        next.settings.set_format(format, digits);
        next.settings.int_base = base(s["base"].as_u64().ok_or("Missing base")?)?;
        next.settings.int_bits = s["bits"]
            .as_u64()
            .filter(|v| (1..=64).contains(v))
            .ok_or("Invalid bits")? as u8;
        next.settings.int_signed = s["signed"].as_bool().ok_or("Missing signed setting")?;
        next.settings.complex_results = s["complex"].as_bool().ok_or("Missing complex setting")?;
        next.settings.font_px = s["font_px"]
            .as_f64()
            .filter(|v| v.is_finite() && (1.0..=128.0).contains(v))
            .ok_or("Invalid font size")? as f32;
        let exp = s["exp_char"].as_str().ok_or("Missing exponent character")?;
        if exp.chars().count() != 1 {
            return Err("Invalid exponent character".into());
        }
        next.settings.exp_char = exp.chars().next().unwrap();
        let state: [u64; 4] =
            serde_json::from_value(j["rng"].clone()).map_err(|e| e.to_string())?;
        next.rng = Rng::restore(state).ok_or("Invalid RNG state")?;
        next.terminal = serde_json::from_value(j["terminal"].clone()).map_err(|e| e.to_string())?;
        next.display = match j["display"].as_str() {
            Some("graphics") => DisplayMode::Graphics,
            Some("terminal") => DisplayMode::Terminal,
            _ => return Err("Invalid display mode".into()),
        };
        let grobs = j["grobs"]
            .as_array()
            .filter(|v| v.len() == 10)
            .ok_or("Invalid GROB table")?;
        next.grobs = grobs
            .iter()
            .map(|g| {
                if g.is_null() {
                    return Ok(None);
                }
                let w = g["width"]
                    .as_u64()
                    .filter(|v| (1..=4096).contains(v))
                    .ok_or("Invalid GROB width")? as usize;
                let h = g["height"]
                    .as_u64()
                    .filter(|v| (1..=4096).contains(v))
                    .ok_or("Invalid GROB height")? as usize;
                if w.checked_mul(h).filter(|n| *n <= 16_777_216).is_none() {
                    return Err("GROB size exceeds limit".into());
                }
                let pixels: Vec<u32> =
                    serde_json::from_value(g["pixels"].clone()).map_err(|e| e.to_string())?;
                if pixels.len() != w * h {
                    return Err("GROB pixel count mismatch".into());
                }
                let mut grob = Grob::new(w, h, 0);
                grob.pixels_mut().copy_from_slice(&pixels);
                Ok(Some(grob))
            })
            .collect::<Result<_, String>>()?;
        next.generation = j["generation"].as_u64().ok_or("Missing generation")?;
        *self = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_memory_roundtrip_and_invalid_restore_is_transactional() {
        let mut a = SessionContext::default();
        a.set_variable(
            "A",
            PortableValue::Real(Real::parse("1.23456789012E-490").unwrap()),
        )
        .unwrap();
        a.set_variable(
            "B",
            PortableValue::Integer(HpInt::new(u64::MAX as i128, 64, IntBase::Hex)),
        )
        .unwrap();
        a.set_variable(
            "L1",
            PortableValue::List(vec![PortableValue::String("text\nα".into())]),
        )
        .unwrap();
        a.store_program("SAVED", "EXPORT SAVED() BEGIN RETURN A; END;".into())
            .unwrap();
        a.store_note("NOTE", "hello".into()).unwrap();
        let image = a.memory_json();
        let mut b = SessionContext::default();
        b.restore_memory(&image).unwrap();
        assert_eq!(a.snapshot().globals, b.snapshot().globals);
        assert_eq!(image, b.memory_json());
        let mut broken = image.clone();
        broken["settings"]["bits"] = json!(0);
        assert!(b.restore_memory(&broken).is_err());
        assert_eq!(b.memory_json(), image);
        broken = image.clone();
        broken["version"] = json!(999);
        assert!(b.restore_memory(&broken).is_err());
        assert_eq!(b.memory_json(), image);
    }
    #[test]
    fn malformed_matrix_and_words_are_rejected() {
        assert!(
            decode_value(&json!({"kind":"matrix","rows":2,"cols":2,"vector":false,"value":[]}))
                .is_err()
        );
        assert!(decode_value(&json!({"kind":"integer","value":"256","bits":8,"base":16})).is_err());
    }
}
