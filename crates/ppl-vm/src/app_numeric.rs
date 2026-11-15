//! VM-owned Numeric Setup. Values retain decimal range; rendering workers may
//! project them to f64 without changing the calculator's saved settings.
use crate::{ErrorKind, RtResult, RuntimeError, Value};
use ppl_compiler::Setting;
use ppl_math::Real;

#[derive(Clone, Debug, PartialEq)]
pub struct AppNumericSettings {
    pub start: Real,
    pub step: Real,
    pub zoom: Real,
    /// 0 Automatic, 1 Build Your Own.
    pub kind: i64,
    /// NumIndep: custom independent-variable rows, in their entered order.
    pub independent: Vec<Real>,
}
impl AppNumericSettings {
    pub fn for_app(app: &str) -> Self {
        Self {
            start: if app == "Sequence" {
                Real::ONE
            } else {
                Real::ZERO
            },
            step: if app == "Sequence" {
                Real::ONE
            } else {
                Real::parse("0.1").unwrap()
            },
            zoom: Real::from_i64(2),
            kind: 0,
            independent: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.step <= Real::ZERO {
            return Err("NumStep must be positive".into());
        }
        if self.zoom <= Real::ONE {
            return Err("NumZoom must be greater than 1".into());
        }
        if !(0..=1).contains(&self.kind) {
            return Err("NumType must be 0 or 1".into());
        }
        if self.independent.len() > 1_000_000 {
            return Err("Too many NumIndep rows".into());
        }
        Ok(())
    }
    pub(crate) fn setting(name: &str) -> Option<Setting> {
        Some(match name.to_ascii_lowercase().as_str() {
            "numstart" => Setting::NumStart,
            "numstep" => Setting::NumStep,
            "numzoom" => Setting::NumZoom,
            "numtype" => Setting::NumType,
            "numindep" => Setting::NumIndep,
            _ => return None,
        })
    }
    pub(crate) fn get(&self, setting: Setting) -> Value {
        match setting {
            Setting::NumStart => Value::Real(self.start),
            Setting::NumStep => Value::Real(self.step),
            Setting::NumZoom => Value::Real(self.zoom),
            Setting::NumType => Value::from_i64(self.kind),
            Setting::NumIndep => {
                Value::list(self.independent.iter().copied().map(Value::Real).collect())
            }
            _ => unreachable!("Numeric Setup setting"),
        }
    }
    pub(crate) fn set(&mut self, setting: Setting, value: &Value) -> RtResult<()> {
        if setting == Setting::NumIndep {
            let Value::List(cells) = value else {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "NumIndep expects a list of real numbers",
                ));
            };
            let independent = cells
                .iter()
                .map(|value| match value {
                    Value::Real(value) => Ok(*value),
                    Value::Int(value) => Ok(value.to_real()),
                    Value::Complex(value) if value.im.is_zero() => Ok(value.re),
                    _ => Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        "NumIndep expects a list of real numbers",
                    )),
                })
                .collect::<RtResult<Vec<_>>>()?;
            let mut next = self.clone();
            next.independent = independent;
            next.validate()
                .map_err(|message| RuntimeError::new(ErrorKind::BadArgValue, message))?;
            *self = next;
            return Ok(());
        }
        let number = match value {
            Value::Real(value) => *value,
            Value::Int(value) => value.to_real(),
            Value::Complex(value) if value.im.is_zero() => value.re,
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "Numeric Setup expects a real number",
                ));
            }
        };
        let mut next = self.clone();
        match setting {
            Setting::NumStart => next.start = number,
            Setting::NumStep => next.step = number,
            Setting::NumZoom => next.zoom = number,
            Setting::NumType => {
                next.kind = number.to_i64().ok_or_else(|| {
                    RuntimeError::new(ErrorKind::BadArgValue, "NumType must be 0 or 1")
                })?;
            }
            _ => unreachable!("Numeric Setup setting"),
        }
        next.validate()
            .map_err(|message| RuntimeError::new(ErrorKind::BadArgValue, message))?;
        *self = next;
        Ok(())
    }
}
