//! App-owned Plot Setup values. Bounds are independently assignable in the
//! original firmware: an inverted window is rejected when drawing, not when
//! storing one of its bounds. Ticks and histogram width must be positive.
use crate::{ErrorKind, RtResult, RuntimeError, Value};
use ppl_math::Real;

#[derive(Clone, Debug, PartialEq)]
pub struct AppPlotSettings {
    pub xmin: Real,
    pub xmax: Real,
    pub ymin: Real,
    pub ymax: Real,
    pub xtick: Real,
    pub ytick: Real,
    pub axes: bool,
    pub grid_dots: bool,
    pub grid_lines: bool,
    pub labels: bool,
    pub hmin: Real,
    pub hmax: Real,
    pub hwidth: Real,
}

impl AppPlotSettings {
    pub const NAMES: [&'static str; 13] = [
        "Xmin",
        "Xmax",
        "Ymin",
        "Ymax",
        "Xtick",
        "Ytick",
        "Axes",
        "GridDots",
        "GridLines",
        "Labels",
        "Hmin",
        "Hmax",
        "Hwidth",
    ];
    /// Defaults established by separate original-firmware app selections.
    /// Other applications require their own variable/default contracts.
    pub fn for_app(app: &str) -> Option<Self> {
        let positive = match app {
            "Function" | "Parametric" | "Polar" | "Solve" | "Advanced Graphing" => false,
            "Sequence" | "Statistics 1Var" | "Statistics 2Var" | "Inference" | "Finance" => true,
            _ => return None,
        };
        let r = |text| Real::parse(text).expect("verified decimal Plot default");
        Some(Self {
            xmin: r(if positive { "-1.8" } else { "-15.9" }),
            xmax: r(if positive { "30" } else { "15.9" }),
            ymin: r(if positive { "-1.8" } else { "-10.9" }),
            ymax: r(if positive { "20" } else { "10.9" }),
            xtick: Real::ONE,
            ytick: Real::ONE,
            axes: false,
            grid_dots: false,
            grid_lines: false,
            labels: false,
            hmin: Real::ZERO,
            hmax: Real::from_i64(24),
            hwidth: Real::ONE,
        })
    }
    pub fn canonical_name(name: &str) -> Option<&'static str> {
        Self::NAMES
            .into_iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(name))
    }
    pub fn check_field(app: &str, name: &str) -> RtResult<()> {
        if matches!(Self::canonical_name(name), Some("Hmin" | "Hmax" | "Hwidth"))
            && app != "Statistics 1Var"
        {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                format!("Histogram Plot variable {name} is not established for {app}"),
            ));
        }
        Ok(())
    }
    pub fn get(&self, name: &str) -> Option<Value> {
        Some(match Self::canonical_name(name)? {
            "Xmin" => Value::Real(self.xmin),
            "Xmax" => Value::Real(self.xmax),
            "Ymin" => Value::Real(self.ymin),
            "Ymax" => Value::Real(self.ymax),
            "Xtick" => Value::Real(self.xtick),
            "Ytick" => Value::Real(self.ytick),
            "Axes" => Value::from_bool(self.axes),
            "GridDots" => Value::from_bool(self.grid_dots),
            "GridLines" => Value::from_bool(self.grid_lines),
            "Labels" => Value::from_bool(self.labels),
            "Hmin" => Value::Real(self.hmin),
            "Hmax" => Value::Real(self.hmax),
            "Hwidth" => Value::Real(self.hwidth),
            _ => unreachable!(),
        })
    }
    pub fn set(&mut self, name: &str, value: &Value) -> RtResult<()> {
        let name = Self::canonical_name(name).ok_or_else(|| {
            RuntimeError::new(ErrorKind::Unsupported, "Unknown Plot Setup variable")
        })?;
        // App variable storage is decimal even when the source is an exact
        // Current-model rational. This is observable after subsequent math.
        let number = value.expect_real(name)?.decimal();
        if matches!(name, "Xtick" | "Ytick" | "Hwidth") && number <= Real::ZERO {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                format!("{name} must be positive"),
            ));
        }
        match name {
            "Xmin" => self.xmin = number,
            "Xmax" => self.xmax = number,
            "Ymin" => self.ymin = number,
            "Ymax" => self.ymax = number,
            "Xtick" => self.xtick = number,
            "Ytick" => self.ytick = number,
            "Axes" => self.axes = !number.is_zero(),
            "GridDots" => self.grid_dots = !number.is_zero(),
            "GridLines" => self.grid_lines = !number.is_zero(),
            "Labels" => self.labels = !number.is_zero(),
            "Hmin" => self.hmin = number,
            "Hmax" => self.hmax = number,
            "Hwidth" => self.hwidth = number,
            _ => unreachable!(),
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.xtick <= Real::ZERO || self.ytick <= Real::ZERO || self.hwidth <= Real::ZERO {
            return Err("Plot tick and histogram widths must be positive".into());
        }
        Ok(())
    }
    pub(crate) fn project(
        &self,
        app: &str,
        globals: &mut std::collections::BTreeMap<String, crate::PortableValue>,
    ) {
        for name in Self::NAMES {
            if Self::check_field(app, name).is_ok() {
                globals.insert(
                    name.into(),
                    crate::PortableValue::from(&self.get(name).unwrap()),
                );
            }
        }
    }
}
