//! Graph 3D's Numeric table uses independently configured X and Y coordinates.
use super::{AppNumericSettings, ErrorKind, Real, RtResult, RuntimeError, Setting, Value};
use crate::value::Matrix;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct Graph3dNumericSettings {
    pub y_start: Real,
    pub y_step: Real,
    pub y_zoom: Real,
    pub points: Vec<[Real; 2]>,
}

impl Default for Graph3dNumericSettings {
    fn default() -> Self {
        Self {
            y_start: Real::ZERO,
            y_step: Real::parse("0.1").unwrap(),
            y_zoom: Real::from_i64(2),
            points: Vec::new(),
        }
    }
}

impl Graph3dNumericSettings {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.y_step <= Real::ZERO || self.y_zoom <= Real::ONE {
            return Err("Graph 3D Y step must be positive and Y zoom must exceed one".into());
        }
        if self.points.len() > 500_000 {
            return Err("Too many Graph 3D Numeric rows".into());
        }
        Ok(())
    }
}

impl AppNumericSettings {
    pub(crate) fn graph3d_field(name: &str) -> bool {
        matches!(
            name,
            "NumXStart"
                | "NumYStart"
                | "NumXStep"
                | "NumYStep"
                | "NumXZoom"
                | "NumYZoom"
                | "NumType"
                | "NumIndep"
        )
    }

    pub fn get_graph3d(&self, field: &str) -> RtResult<Value> {
        let Some(y) = &self.graph3d else {
            return Err(invalid("Numeric settings do not belong to Graph 3D"));
        };
        Ok(match field {
            "NumXStart" => Value::Real(self.start),
            "NumYStart" => Value::Real(y.y_start),
            "NumXStep" => Value::Real(self.step),
            "NumYStep" => Value::Real(y.y_step),
            "NumXZoom" => Value::Real(self.zoom),
            "NumYZoom" => Value::Real(y.y_zoom),
            "NumType" => Value::from_i64(self.kind),
            "NumIndep" => Value::list(
                y.points
                    .iter()
                    .map(|point| {
                        Value::Matrix(Rc::new(Matrix {
                            rows: 1,
                            cols: 2,
                            data: point.iter().copied().map(Value::Real).collect(),
                            vector: true,
                        }))
                    })
                    .collect(),
            ),
            _ => return Err(invalid("Unknown Graph 3D Numeric Setup field")),
        })
    }

    pub fn set_graph3d(&mut self, field: &str, value: &Value) -> RtResult<()> {
        let mut next = self.clone();
        let Some(y) = &mut next.graph3d else {
            return Err(invalid("Numeric settings do not belong to Graph 3D"));
        };
        if field == "NumIndep" {
            let Value::List(rows) = value else {
                return Err(invalid(
                    "NumIndep requires a list of two-coordinate vectors",
                ));
            };
            if rows.len() > 500_000 {
                return Err(invalid("Too many Graph 3D Numeric rows"));
            }
            y.points = rows
                .iter()
                .map(|value| {
                    let Value::Matrix(vector) = value else {
                        return Err(invalid(
                            "NumIndep requires a list of two-coordinate vectors",
                        ));
                    };
                    if !vector.vector {
                        return Err(invalid(
                            "NumIndep requires a list of two-coordinate vectors",
                        ));
                    }
                    let [x, y] = vector.data.as_slice() else {
                        return Err(invalid(
                            "NumIndep requires a list of two-coordinate vectors",
                        ));
                    };
                    Ok([x.expect_real(field)?, y.expect_real(field)?])
                })
                .collect::<RtResult<_>>()?;
        } else {
            let number = value.expect_real(field)?;
            match field {
                "NumXStart" => next.start = number,
                "NumYStart" => y.y_start = number,
                "NumXStep" => next.step = number,
                "NumYStep" => y.y_step = number,
                "NumXZoom" => next.zoom = number,
                "NumYZoom" => y.y_zoom = number,
                "NumType" if number == Real::ZERO || number == Real::ONE => {
                    next.set(Setting::NumType, value)?;
                }
                "NumType" => return Err(invalid("NumType must be 0 or 1")),
                _ => return Err(invalid("Unknown Graph 3D Numeric Setup field")),
            }
        }
        next.validate().map_err(invalid)?;
        *self = next;
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::BadArgValue, message)
}
