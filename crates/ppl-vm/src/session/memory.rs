//! Versioned calculator memory, independent of running VM stacks and host handles.
use super::*;
use ppl_math::IntBase;
use serde_json::{Value as Json, json};

const VERSION: u64 = 6;
const TYPED_NUMERIC_VERSION: u64 = 5;
const MAX_CELLS: usize = 1_000_000;

pub fn encode_value(value: &PortableValue) -> Json {
    match value {
        PortableValue::Symbolic(v) => json!({"kind":"symbolic","value":v}),
        PortableValue::Real(v) => {
            let mut value = json!({"kind":"real","value":v.to_string()});
            match v.origin() {
                ppl_math::RealOrigin::LegacyBcd => {}
                ppl_math::RealOrigin::Decimal => value["origin"] = json!({"kind":"decimal"}),
                ppl_math::RealOrigin::Rational {
                    numerator,
                    denominator,
                } => {
                    value["origin"] = json!({"kind":"rational","numerator":numerator.to_string(),"denominator":denominator.to_string(),"negative":v.is_negative()})
                }
                ppl_math::RealOrigin::PiMultiple {
                    numerator,
                    denominator,
                } => {
                    value["origin"] = json!({"kind":"pi_multiple","numerator":numerator.to_string(),"denominator":denominator.to_string(),"negative":v.is_negative()})
                }
            }
            if v.presentation() == ppl_math::RealPresentation::Sexagesimal {
                value["presentation"] = "sexagesimal".into();
            }
            value
        }
        PortableValue::Integer(v) => {
            let mut value = json!({"kind":"integer","value":v.unsigned().to_string(),"bits":v.bits(),"base":base_code(v.base)});
            if v.is_signed() {
                value["signed_word"] = true.into();
            }
            if v.is_negative() {
                value["negative"] = true.into();
            }
            if v.is_system_base() {
                value["system_base"] = true.into();
            }
            value
        }
        PortableValue::Complex(v) => {
            let mut value =
                json!({"kind":"complex","real":v.re.to_string(),"imaginary":v.im.to_string()});
            match v.presentation() {
                ppl_math::ComplexPresentation::Rectangular => {}
                ppl_math::ComplexPresentation::Sexagesimal => {
                    value["presentation"] = "sexagesimal".into()
                }
                ppl_math::ComplexPresentation::Polar => value["presentation"] = "polar".into(),
            }
            value
        }
        PortableValue::String(v) => json!({"kind":"string","value":v}),
        PortableValue::Utf16(v) => json!({"kind":"utf16","value":v}),
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

fn real_value(j: &Json) -> Result<Real, String> {
    let cached = number(j, "value")?;
    let Some(origin) = j.get("origin") else {
        return Ok(cached);
    };
    let value = match origin["kind"].as_str().ok_or("Missing real origin kind")? {
        "decimal" => cached.decimal(),
        "rational" | "pi_multiple" => {
            let integer = |key: &str| {
                origin[key]
                    .as_str()
                    .ok_or("Missing exact coefficient")?
                    .parse::<u64>()
                    .map_err(|_| "Invalid exact coefficient")
            };
            let negative = origin["negative"].as_bool().ok_or("Missing exact sign")?;
            let (n, d) = (integer("numerator")?, integer("denominator")?);
            if origin["kind"] == "rational" {
                Real::exact_rational(negative, n, d)
            } else {
                Real::exact_pi_multiple(negative, n, d)
            }
            .map_err(|error| error.to_string())?
        }
        _ => return Err("Unknown real origin kind".into()),
    };
    if value.to_string() != cached.to_string() {
        return Err("Exact origin disagrees with saved decimal".into());
    }
    Ok(value)
}
fn numeric_value(value: &Json, legacy: bool) -> Result<Real, String> {
    if legacy && let Some(text) = value.as_str() {
        return Real::parse(text).ok_or("Invalid numeric setting".into());
    }
    match decode_value(value)? {
        PortableValue::Real(value) => Ok(value),
        _ => Err("Numeric setting must be a real".into()),
    }
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
        "real" => PortableValue::Real(real_value(j)?.with_presentation(
            match j.get("presentation") {
                None => ppl_math::RealPresentation::Decimal,
                Some(value) if value == "sexagesimal" => ppl_math::RealPresentation::Sexagesimal,
                _ => return Err("Unknown real presentation".into()),
            },
        )),
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
            let flag = |name| match j.get(name) {
                None => Ok(false),
                Some(value) => value.as_bool().ok_or("Invalid integer metadata flag"),
            };
            let signed = flag("signed_word")?;
            let negative = flag("negative")?;
            if negative && !signed {
                return Err("Negative carrier requires a signed word".into());
            }
            let width = if signed { -(bits as i64) } else { bits as i64 };
            let number = word as i128 - if negative { 1i128 << bits } else { 0 };
            PortableValue::Integer(
                HpInt::new_with_width(
                    number,
                    width,
                    base(j["base"].as_u64().ok_or("Missing base")?)?,
                )
                .ok_or("Invalid signed integer width")?
                .with_system_base(flag("system_base")?),
            )
        }
        "complex" => PortableValue::Complex(
            Complex::new(number(j, "real")?, number(j, "imaginary")?).with_presentation(
                match j.get("presentation") {
                    None => ppl_math::ComplexPresentation::Rectangular,
                    Some(value) if value == "sexagesimal" => {
                        ppl_math::ComplexPresentation::Sexagesimal
                    }
                    Some(value) if value == "polar" => ppl_math::ComplexPresentation::Polar,
                    _ => return Err("Unknown complex presentation".into()),
                },
            ),
        ),
        "string" => PortableValue::String(j["value"].as_str().ok_or("Missing string")?.into()),
        "utf16" => {
            let data = j["value"].as_array().ok_or("Missing UTF-16 units")?;
            if data.len() > MAX_CELLS {
                return Err("UTF-16 string exceeds unit limit".into());
            }
            let units = data
                .iter()
                .map(|unit| {
                    unit.as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .ok_or_else(|| "Invalid UTF-16 unit".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if String::from_utf16(&units).is_ok() {
                return Err("UTF-16 storage requires isolated code units".into());
            }
            PortableValue::Utf16(units)
        }
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
        json!({"version":VERSION,"cas":self.cas,"active_app":self.active_app,"app_settings":self.app_settings.iter().map(|(name,s)|(name.clone(),json!({"angle":s.angle,"complex":s.complex,"format":s.format,"digits":s.digits}))).collect::<serde_json::Map<String,Json>>(),"app_plot_settings":self.app_plot_settings.iter().map(|(name,settings)|(name.clone(),Json::Object(crate::AppPlotSettings::NAMES.into_iter().map(|field|(field.into(),encode_value(&PortableValue::from(&settings.get(field).unwrap())))).collect()))).collect::<serde_json::Map<String,Json>>(),"geometry_plot_settings":self.geometry_plot_settings.as_ref().map(|settings|Json::Object(crate::GeometryPlotSettings::NAMES.into_iter().map(|name|(name.into(),encode_value(&PortableValue::from(&settings.get(name).expect("Geometry field"))))).collect())),"app_numeric_settings":self.app_numeric_settings.iter().map(|(name,s)|(name.clone(),json!({"start":encode_value(&PortableValue::Real(s.start)),"step":encode_value(&PortableValue::Real(s.step)),"zoom":encode_value(&PortableValue::Real(s.zoom)),"kind":s.kind,"independent":s.independent.iter().map(|v|encode_value(&PortableValue::Real(*v))).collect::<Vec<_>>()}))).collect::<serde_json::Map<String,Json>>(),"app_files":self.app_files.iter().map(|(k,v)|(k.clone(),encode_map(v))).collect::<serde_json::Map<String,Json>>(),"generation":self.generation,"globals":encode_map(&self.globals),
            "modules":self.modules.iter().map(|(k,m)|(k.clone(),json!({"revision":m.revision,"initialized":m.initialized,"values":encode_map(&m.values)}))).collect::<serde_json::Map<String,Json>>(),
            "notes":self.notes,"note_bullets":self.note_bullets,"programs":self.programs,"last_value":self.last_value.as_ref().map(encode_value),
            "settings":{"angle":self.settings.angle.code(),"format":self.settings.hformat,"digits":self.settings.hdigits,"string_format_version":self.settings.string_format_version.code(),"numeric_version":self.settings.numeric_version.code(),"complex":self.settings.complex_results,"separator":self.settings.digit_grouping.code(),"complex_format":self.settings.complex_display.code(),"base":base_code(self.settings.int_base),"bits":self.settings.int_bits,"signed":self.settings.int_signed,"font_px":self.settings.font_px,"exp_char":self.settings.exp_char.to_string()},
            "rng":self.rng.state(),"terminal":self.terminal,"display":if self.display==DisplayMode::Graphics {"graphics"}else{"terminal"},
            "grobs":self.grobs.iter().map(|g|g.as_ref().map(|g|json!({"width":g.width(),"height":g.height(),"pixels":g.pixels()}))).collect::<Vec<_>>()})
    }
    /// Parse into a fresh context first; malformed/future state never mutates this session.
    pub fn restore_memory(&mut self, j: &Json) -> Result<(), String> {
        let version = j["version"]
            .as_u64()
            .filter(|version| (1..=VERSION).contains(version))
            .ok_or("Unsupported Native memory version")?;
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
        if let Some(value) = j
            .get("geometry_plot_settings")
            .filter(|value| !value.is_null())
        {
            let fields = value.as_object().ok_or("Invalid Geometry Plot settings")?;
            if fields.len() != crate::GeometryPlotSettings::NAMES.len() {
                return Err("Invalid Geometry Plot settings fields".into());
            }
            let mut settings = crate::GeometryPlotSettings::default();
            for field in crate::GeometryPlotSettings::NAMES {
                let number = numeric_value(&value[field], false)?;
                if matches!(
                    field,
                    "Axes" | "GridDots" | "GridLines" | "Labels" | "ScrollText"
                ) && number != Real::ZERO
                    && number != Real::ONE
                {
                    return Err("Invalid Geometry flag".into());
                }
                settings
                    .set(field, &Value::Real(number))
                    .map_err(|e| e.to_string())?;
                if settings
                    .get(field)
                    .map_err(|e| e.to_string())?
                    .expect_real(field)
                    .map_err(|e| e.to_string())?
                    != number
                {
                    return Err("Noncanonical Geometry Plot value".into());
                }
            }
            settings.validate()?;
            next.geometry_plot_settings = Some(settings);
        }
        if let Some(apps) = j.get("app_plot_settings") {
            let apps = apps.as_object().ok_or("Invalid Plot settings map")?;
            if apps.len() > 1024 {
                return Err("Too many Plot settings".into());
            }
            for (name, value) in apps {
                let mut settings = crate::AppPlotSettings::for_app(name)
                    .ok_or_else(|| format!("Unestablished Plot settings app {name}"))?;
                let fields = value.as_object().ok_or("Invalid Plot settings")?;
                if fields.len() != crate::AppPlotSettings::NAMES.len() {
                    return Err("Invalid Plot settings fields".into());
                }
                for field in crate::AppPlotSettings::NAMES {
                    let decoded = decode_value(&value[field])?;
                    let number = match &decoded {
                        PortableValue::Real(number) => *number,
                        PortableValue::Symbolic(ppl_cas::CasValue::Infinity(_))
                            if name == "Statistics 1Var" && matches!(field, "Ymin" | "Ymax") =>
                        {
                            settings
                                .set(field, &decoded.to_value())
                                .map_err(|error| error.to_string())?;
                            continue;
                        }
                        _ => return Err("Numeric setting must be a real".into()),
                    };
                    if matches!(field, "Axes" | "GridDots" | "GridLines" | "Labels")
                        && number != Real::ZERO
                        && number != Real::ONE
                    {
                        return Err("Invalid Plot flag".into());
                    }
                    settings
                        .set(field, &Value::Real(number))
                        .map_err(|error| error.to_string())?;
                }
                settings.validate_for(name)?;
                next.app_plot_settings.insert(name.clone(), settings);
            }
        }
        if let Some(apps) = j.get("app_numeric_settings") {
            let apps = apps.as_object().ok_or("Invalid numeric settings map")?;
            if apps.len() > 1024 {
                return Err("Too many numeric settings".into());
            }
            for (name, value) in apps {
                if name.is_empty() || name.len() > 1024 {
                    return Err("Invalid numeric settings name".into());
                }
                let legacy = version < TYPED_NUMERIC_VERSION;
                let real = |key: &str| numeric_value(&value[key], legacy);
                let settings = crate::AppNumericSettings {
                    start: real("start")?,
                    step: real("step")?,
                    zoom: real("zoom")?,
                    kind: value["kind"].as_i64().ok_or("Invalid NumType")?,
                    independent: {
                        let cells = value["independent"].as_array().ok_or("Invalid NumIndep")?;
                        if cells.len() > MAX_CELLS {
                            return Err("Too many NumIndep rows".into());
                        }
                        cells
                            .iter()
                            .map(|value| numeric_value(value, legacy))
                            .collect::<Result<Vec<_>, String>>()?
                    },
                };
                settings.validate()?;
                next.app_numeric_settings.insert(name.clone(), settings);
            }
        } else if version >= TYPED_NUMERIC_VERSION {
            return Err("Missing numeric settings map".into());
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
        // Older profiles stored active-app bounds in Home globals. Migrate
        // those exact values once, never overwrite an explicit scoped map.
        if let Some(mut settings) = crate::AppPlotSettings::for_app(&next.active_app) {
            let mut found = false;
            for name in crate::AppPlotSettings::NAMES {
                if let Some(value) = next.globals.remove(name) {
                    if !next.app_plot_settings.contains_key(&next.active_app) {
                        settings
                            .set(name, &value.to_value())
                            .map_err(|e| e.to_string())?;
                        found = true;
                    }
                }
            }
            if found {
                next.app_plot_settings
                    .insert(next.active_app.clone(), settings);
            }
        }

        if version < TYPED_NUMERIC_VERSION {
            let mut settings = next.app_numeric_settings();
            let mut changed = false;
            for name in ["NumStart", "NumStep", "NumZoom", "NumType", "NumIndep"] {
                if let Some(value) = next.globals.remove(name) {
                    settings
                        .set(
                            crate::AppNumericSettings::setting(name).unwrap(),
                            &value.to_value(),
                        )
                        .map_err(|e| e.to_string())?;
                    changed = true;
                }
            }
            if changed {
                next.app_numeric_settings
                    .insert(next.active_app.clone(), settings);
            }
        }
        for (name, value) in &mut next.globals {
            if crate::statistics_columns::is_column(name) {
                *value = PortableValue::from(
                    &crate::statistics_columns::coerce(name, value.to_value())
                        .map_err(|error| error.to_string())?,
                );
            }
        }
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
        if let Some(bullets) = j.get("note_bullets") {
            next.note_bullets =
                serde_json::from_value(bullets.clone()).map_err(|e| e.to_string())?;
            next.note_bullets.retain(|name, bullets| {
                next.notes.get(name).is_some_and(|text| {
                    let paragraphs = text.split('\n').count();
                    bullets.retain(|index| *index < paragraphs);
                    !bullets.is_empty()
                })
            });
        }
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
        next.settings.string_format_version = match s.get("string_format_version") {
            None => crate::StringFormatVersion::default(),
            Some(value) => value
                .as_i64()
                .and_then(crate::StringFormatVersion::from_code)
                .ok_or("Invalid STRING format version")?,
        };
        next.settings.numeric_version = match s.get("numeric_version") {
            None if version < TYPED_NUMERIC_VERSION => ppl_math::NumericVersion::Legacy15157,
            Some(value) => value
                .as_i64()
                .and_then(ppl_math::NumericVersion::from_code)
                .ok_or("Invalid numeric version")?,
            None => return Err("Missing numeric version".into()),
        };
        next.settings.int_base = base(s["base"].as_u64().ok_or("Missing base")?)?;
        next.settings.int_bits = s["bits"]
            .as_u64()
            .filter(|v| (1..=64).contains(v))
            .ok_or("Invalid bits")? as u8;
        next.settings
            .set_integer_signed(s["signed"].as_bool().ok_or("Missing signed setting")?)
            .map_err(|error| error.to_string())?;
        next.settings.digit_grouping = match s.get("separator") {
            None => ppl_math::DigitGrouping::default(),
            Some(value) => value
                .as_i64()
                .and_then(ppl_math::DigitGrouping::from_code)
                .ok_or("Invalid digit grouping")?,
        };
        next.settings.complex_display = match s.get("complex_format") {
            None => ppl_math::ComplexFormat::Algebraic,
            Some(value) => value
                .as_i64()
                .and_then(ppl_math::ComplexFormat::from_code)
                .ok_or("Invalid complex display")?,
        };
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
    fn note_bullets_are_optional_metadata_and_follow_the_saved_note() {
        let mut context = SessionContext::default();
        context
            .store_bulleted_note("NOTE", "title\nitem".into(), [1, 8].into())
            .unwrap();
        let mut image = context.memory_json();
        assert_eq!(image["notes"]["NOTE"], "title\nitem");
        assert_eq!(image["note_bullets"]["NOTE"], json!([1]));

        context.restore_memory(&image).unwrap();
        context.rename_note("NOTE", "RENAMED").unwrap();
        assert_eq!(context.snapshot().note_bullets["RENAMED"], [1].into());
        let result = context.evaluate_home(
            r#"Notes("RENAMED")"#,
            Box::new(crate::HeadlessHost::new()),
            crate::Limits::default(),
        );
        assert_eq!(
            PortableValue::from(&result.result.unwrap()),
            PortableValue::String("title\nitem".into())
        );
        assert_eq!(context.snapshot().note_bullets["RENAMED"], [1].into());

        context.delete_note("RENAMED");
        assert!(context.snapshot().note_bullets.is_empty());
        image.as_object_mut().unwrap().remove("note_bullets");
        context.restore_memory(&image).unwrap();
        assert!(context.snapshot().note_bullets.is_empty());
        assert_eq!(context.snapshot().notes["NOTE"], "title\nitem");
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
