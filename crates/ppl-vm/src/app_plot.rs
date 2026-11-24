//! App-owned Plot Setup values. Bounds are independently assignable in the
//! original firmware: an inverted window is rejected when drawing, not when
//! storing one of its bounds. Ticks and histogram width must be positive.
pub mod background;

use crate::{ErrorKind, RtResult, RuntimeError, Value};
use ppl_cas::{CasValue, InfinitySign};
use ppl_math::Real;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlotBound {
    Finite(Real),
    NegativeInfinity,
    PositiveInfinity,
}

impl PlotBound {
    pub fn to_f64(self) -> f64 {
        match self {
            Self::Finite(value) => value.to_f64(),
            Self::NegativeInfinity => f64::NEG_INFINITY,
            Self::PositiveInfinity => f64::INFINITY,
        }
    }

    pub fn to_value(self) -> Value {
        match self {
            Self::Finite(value) => Value::Real(value),
            Self::NegativeInfinity => {
                crate::cas::from_cas(CasValue::Infinity(InfinitySign::Negative))
            }
            Self::PositiveInfinity => {
                crate::cas::from_cas(CasValue::Infinity(InfinitySign::Positive))
            }
        }
    }

    fn signed_infinity(value: &Value) -> Option<Self> {
        let Value::Symbolic(value) = value else {
            return None;
        };
        match value.as_ref() {
            CasValue::Infinity(InfinitySign::Negative) => Some(Self::NegativeInfinity),
            CasValue::Infinity(InfinitySign::Positive) => Some(Self::PositiveInfinity),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PlotCursorMode {
    Standard,
    Inverting,
    Blinking,
}

impl PlotCursorMode {
    pub const CHOICES: [Self; 3] = [Self::Standard, Self::Inverting, Self::Blinking];

    pub fn value(self) -> Value {
        Value::Real(Real::from_i64(self as i64))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AppPlotSettings {
    pub xmin: Real,
    pub xmax: Real,
    pub ymin: PlotBound,
    pub ymax: PlotBound,
    pub xtick: Real,
    pub ytick: Real,
    pub axes: bool,
    pub grid_dots: bool,
    pub grid_lines: bool,
    pub labels: bool,
    pub hmin: Real,
    pub hmax: Real,
    pub hwidth: Real,
    depth: Option<Box<[Real; 3]>>,
    zoom: Option<Box<StatisticsZoom>>,
    cursor_mode: Option<PlotCursorMode>,
    statistics_background: Option<Box<background::Background>>,
}

#[derive(Clone, Debug, PartialEq)]
struct StatisticsZoom {
    factors: [Real; 2],
    recenter: bool,
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
    const GRAPH_3D_NAMES: [&'static str; 9] = [
        "Xmin", "Xmax", "Ymin", "Ymax", "Zmin", "Zmax", "Xtick", "Ytick", "Ztick",
    ];
    pub(crate) const CURSOR_NAME: &'static str = "Cursor";
    pub(crate) const ZOOM_NAMES: [&'static str; 2] = ["Xzoom", "Yzoom"];

    pub const fn fields(app: &str) -> &'static [&'static str] {
        if matches!(app.as_bytes(), b"Graph 3D") {
            &Self::GRAPH_3D_NAMES
        } else {
            &Self::NAMES
        }
    }

    /// Defaults established by separate original-firmware app selections.
    /// Other applications require their own variable/default contracts.
    pub fn for_app(app: &str) -> Option<Self> {
        let positive = match app {
            "Function" | "Parametric" | "Polar" | "Graph 3D" => false,
            "Sequence" | "Statistics 1Var" | "Statistics 2Var" => true,
            _ => return None,
        };
        let r = |text| Real::parse(text).expect("verified decimal Plot default");
        if app == "Graph 3D" {
            let low = Real::from_i64(-4);
            let high = Real::from_i64(4);
            return Some(Self {
                xmin: low,
                xmax: high,
                ymin: PlotBound::Finite(low),
                ymax: PlotBound::Finite(high),
                depth: Some(Box::new([low, high, Real::ONE])),
                cursor_mode: None,
                ..Self::for_app("Function").unwrap()
            });
        }
        Some(Self {
            xmin: r(if positive { "-1.8" } else { "-15.9" }),
            xmax: r(if positive { "30" } else { "15.9" }),
            ymin: PlotBound::Finite(r(if positive { "-1.8" } else { "-10.9" })),
            ymax: PlotBound::Finite(r(if positive { "20" } else { "10.9" })),
            xtick: Real::ONE,
            ytick: Real::ONE,
            axes: false,
            grid_dots: false,
            grid_lines: false,
            labels: false,
            hmin: Real::ZERO,
            hmax: Real::from_i64(24),
            hwidth: Real::ONE,
            depth: None,
            cursor_mode: Self::has_cursor(app).then_some(PlotCursorMode::Standard),
            statistics_background: (app == "Statistics 1Var").then(Box::default),
            zoom: (app == "Statistics 1Var").then(|| {
                Box::new(StatisticsZoom {
                    factors: [Real::from_i64(2); 2],
                    recenter: true,
                })
            }),
        })
    }
    pub fn canonical_name(name: &str) -> Option<&'static str> {
        Self::NAMES
            .into_iter()
            .chain(["Zmin", "Zmax", "Ztick"])
            .find(|candidate| candidate.eq_ignore_ascii_case(name))
    }

    fn zoom_axis(name: &str) -> Option<usize> {
        Self::ZOOM_NAMES
            .iter()
            .position(|field| field.eq_ignore_ascii_case(name))
    }

    fn has_cursor(app: &str) -> bool {
        matches!(app, "Function" | "Statistics 1Var")
    }

    pub fn statistics_background(&self) -> Option<&background::Background> {
        self.statistics_background.as_deref()
    }

    pub fn statistics_background_mut(&mut self) -> Option<&mut background::Background> {
        self.statistics_background.as_deref_mut()
    }

    pub fn cursor_mode(&self) -> Option<PlotCursorMode> {
        self.cursor_mode
    }

    pub(crate) fn recognizes(app: &str, name: &str) -> bool {
        Self::canonical_name(name).is_some()
            || (app == "Statistics 1Var" && Self::zoom_axis(name).is_some())
            || (Self::has_cursor(app) && name.eq_ignore_ascii_case(Self::CURSOR_NAME))
    }

    pub(crate) fn stored_zoom_fields(&self) -> &'static [&'static str] {
        match &self.zoom {
            Some(zoom) if zoom.factors.iter().any(|value| *value != Real::from_i64(2)) => {
                &Self::ZOOM_NAMES
            }
            _ => &[],
        }
    }

    pub fn statistics_zoom(&self) -> Option<([Real; 2], bool)> {
        self.zoom.as_ref().map(|zoom| (zoom.factors, zoom.recenter))
    }

    pub fn set_statistics_zoom(&mut self, factors: [Real; 2], recenter: bool) -> RtResult<()> {
        let zoom = self.zoom.as_mut().ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::Unsupported,
                "This app has no stored zoom factors",
            )
        })?;
        let factors = factors.map(Real::decimal);
        for (axis, factor) in factors.iter().enumerate() {
            Self::validate_zoom_factor(axis, *factor)?;
        }
        **zoom = StatisticsZoom { factors, recenter };
        Ok(())
    }

    fn validate_zoom_factor(axis: usize, factor: Real) -> RtResult<()> {
        if factor <= Real::ZERO {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                format!("{} must be positive", Self::ZOOM_NAMES[axis]),
            ));
        }
        Ok(())
    }

    pub fn check_field(app: &str, name: &str) -> RtResult<()> {
        if let Some(field) = Self::canonical_name(name)
            && (app == "Graph 3D" && !Self::GRAPH_3D_NAMES.contains(&field)
                || app != "Graph 3D" && matches!(field, "Zmin" | "Zmax" | "Ztick"))
        {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                format!("Plot variable {field} is unavailable for {app}"),
            ));
        }
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
        if name.eq_ignore_ascii_case(Self::CURSOR_NAME) {
            return self.cursor_mode.map(PlotCursorMode::value);
        }
        if let Some(axis) = Self::zoom_axis(name) {
            return self
                .zoom
                .as_ref()
                .map(|zoom| Value::Real(zoom.factors[axis]));
        }
        Some(match Self::canonical_name(name)? {
            "Xmin" => Value::Real(self.xmin),
            "Xmax" => Value::Real(self.xmax),
            "Ymin" => self.ymin.to_value(),
            "Ymax" => self.ymax.to_value(),
            "Xtick" => Value::Real(self.xtick),
            "Ytick" => Value::Real(self.ytick),
            "Zmin" => Value::Real(self.depth.as_ref()?[0]),
            "Zmax" => Value::Real(self.depth.as_ref()?[1]),
            "Ztick" => Value::Real(self.depth.as_ref()?[2]),
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
        if name.eq_ignore_ascii_case(Self::CURSOR_NAME) {
            let current = self.cursor_mode.as_mut().ok_or_else(|| {
                RuntimeError::new(ErrorKind::Unsupported, "This app has no stored Cursor mode")
            })?;
            let number = value.expect_real(name)?;
            if !number.is_integer() {
                return Err(RuntimeError::kind(ErrorKind::InvalidInput));
            }
            *current = number
                .to_i64()
                .and_then(|value| usize::try_from(value).ok())
                .and_then(|index| PlotCursorMode::CHOICES.get(index))
                .copied()
                .ok_or_else(|| {
                    RuntimeError::new(ErrorKind::BadArgValue, "Cursor is out of bounds")
                })?;
            return Ok(());
        }
        if let Some(axis) = Self::zoom_axis(name) {
            let zoom = self.zoom.as_mut().ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::Unsupported,
                    "This app has no stored zoom factors",
                )
            })?;
            let factor = value.expect_real(name)?.decimal();
            Self::validate_zoom_factor(axis, factor)?;
            zoom.factors[axis] = factor;
            return Ok(());
        }
        let name = Self::canonical_name(name).ok_or_else(|| {
            RuntimeError::new(ErrorKind::Unsupported, "Unknown Plot Setup variable")
        })?;
        if let Some(bound) = PlotBound::signed_infinity(value) {
            match name {
                "Ymin" => self.ymin = bound,
                "Ymax" => self.ymax = bound,
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        format!("{name} must be a finite number"),
                    ));
                }
            }
            return Ok(());
        }
        // App variable storage is decimal even when the source is an exact
        // Current-model rational. This is observable after subsequent math.
        let number = value.expect_real(name)?.decimal();
        if matches!(name, "Xtick" | "Ytick" | "Ztick" | "Hwidth") && number <= Real::ZERO {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                format!("{name} must be positive"),
            ));
        }
        match name {
            "Xmin" => self.xmin = number,
            "Xmax" => self.xmax = number,
            "Ymin" => self.ymin = PlotBound::Finite(number),
            "Ymax" => self.ymax = PlotBound::Finite(number),
            "Xtick" => self.xtick = number,
            "Ytick" => self.ytick = number,
            "Zmin" | "Zmax" | "Ztick" => {
                let depth = self.depth.as_mut().ok_or_else(|| {
                    RuntimeError::new(ErrorKind::Unsupported, "This app has no depth axis")
                })?;
                let index = match name {
                    "Zmin" => 0,
                    "Zmax" => 1,
                    _ => 2,
                };
                depth[index] = number;
            }
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

    pub fn has_infinite_y(&self) -> bool {
        !matches!(self.ymin, PlotBound::Finite(_)) || !matches!(self.ymax, PlotBound::Finite(_))
    }

    pub fn validate_for(&self, app: &str) -> Result<(), String> {
        self.validate()?;
        if (app == "Graph 3D") != self.depth.is_some() {
            return Err("Graph 3D depth settings must belong to their app".into());
        }
        if (app == "Statistics 1Var") != self.zoom.is_some() {
            return Err("Statistics 1Var zoom factors must belong to their app".into());
        }
        if (app == "Statistics 1Var") != self.statistics_background.is_some() {
            return Err("Plot background settings must belong to Statistics 1Var".into());
        }
        if Self::has_cursor(app) != self.cursor_mode.is_some() {
            return Err("Cursor mode must belong to its app".into());
        }
        if app != "Statistics 1Var" && self.has_infinite_y() {
            return Err("Plot bounds must be finite".into());
        }
        Ok(())
    }

    pub(crate) fn set_for(&mut self, app: &str, name: &str, value: &Value) -> RtResult<()> {
        let mut next = self.clone();
        next.set(name, value)?;
        next.validate_for(app)
            .map_err(|error| RuntimeError::new(ErrorKind::BadArgValue, error))?;
        *self = next;
        Ok(())
    }
}
