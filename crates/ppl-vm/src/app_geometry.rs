//! Geometry owns minima and square-pixel size, rather than four independent
//! bounds. Its canvas spans319×219; Cartesian graphics keep their318×218 map.
use crate::{ErrorKind, RtResult, RuntimeError, Value};
use ppl_math::Real;

#[derive(Clone, Debug, PartialEq)]
pub struct GeometryPlotSettings {
    pub xmin: Real,
    pub ymin: Real,
    pub pix_size: Real,
    pub xtick: Real,
    pub ytick: Real,
    pub axes: bool,
    pub grid_dots: bool,
    pub grid_lines: bool,
    pub labels: bool,
    pub scroll_text: bool,
}
impl Default for GeometryPlotSettings {
    fn default() -> Self {
        Self {
            xmin: Real::from_i64(-16).decimal(),
            ymin: Real::from_i64(-11).decimal(),
            pix_size: Real::parse("0.1").unwrap().decimal(),
            xtick: Real::ONE.decimal(),
            ytick: Real::ONE.decimal(),
            axes: false,
            grid_dots: false,
            grid_lines: false,
            labels: false,
            scroll_text: false,
        }
    }
}
fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, message.into())
}
impl GeometryPlotSettings {
    pub const NAMES: [&'static str; 10] = [
        "Xmin",
        "Ymin",
        "PixSize",
        "Xtick",
        "Ytick",
        "Axes",
        "GridDots",
        "GridLines",
        "Labels",
        "ScrollText",
    ];
    pub fn canonical_name(name: &str) -> Option<&'static str> {
        Self::NAMES
            .into_iter()
            .find(|field| field.eq_ignore_ascii_case(name))
    }
    /// Rejected public maxima still require scoped resolution and must never
    /// fall through to unrelated Home globals or a presentation host.
    pub fn recognizes(name: &str) -> bool {
        Self::canonical_name(name).is_some()
            || name.eq_ignore_ascii_case("Xmax")
            || name.eq_ignore_ascii_case("Ymax")
    }
    pub fn get(&self, name: &str) -> RtResult<Value> {
        Ok(
            match Self::canonical_name(name)
                .ok_or_else(|| invalid(format!("Geometry has no {name} variable")))?
            {
                "Xmin" => Value::Real(self.xmin),
                "Ymin" => Value::Real(self.ymin),
                "PixSize" => Value::Real(self.pix_size),
                "Xtick" => Value::Real(self.xtick),
                "Ytick" => Value::Real(self.ytick),
                "Axes" => Value::from_bool(self.axes),
                "GridDots" => Value::from_bool(self.grid_dots),
                "GridLines" => Value::from_bool(self.grid_lines),
                "Labels" => Value::from_bool(self.labels),
                "ScrollText" => Value::from_bool(self.scroll_text),
                _ => unreachable!(),
            },
        )
    }
    pub fn set(&mut self, name: &str, value: &Value) -> RtResult<()> {
        let name = Self::canonical_name(name)
            .ok_or_else(|| invalid(format!("Geometry has no {name} variable")))?;
        let number = match value {
            Value::Real(v) => *v,
            Value::Int(v) => v.to_real(),
            Value::Complex(v) if v.im.is_zero() => v.re,
            _ => return Err(invalid("Geometry Plot Setup needs a real value")),
        }
        .decimal();
        let limit = Real::from_i64(10000);
        let mut next = self.clone();
        match name {
            "Xmin" => {
                next.xmin = if number < limit.neg() {
                    limit.neg()
                } else if number > limit {
                    limit
                } else {
                    number
                }
            }
            "Ymin" => {
                next.ymin = if number < limit.neg() {
                    limit.neg()
                } else if number > limit {
                    limit
                } else {
                    number
                }
            }
            "PixSize" => {
                if number <= Real::parse("0.001").unwrap() {
                    return Err(invalid("Geometry PixSize must be greater than0.001"));
                }
                next.pix_size = if number > limit { limit } else { number };
            }
            "Xtick" => next.xtick = number,
            "Ytick" => next.ytick = number,
            "Axes" => next.axes = !number.is_zero(),
            "GridDots" => next.grid_dots = !number.is_zero(),
            "GridLines" => next.grid_lines = !number.is_zero(),
            "Labels" => next.labels = !number.is_zero(),
            "ScrollText" => next.scroll_text = !number.is_zero(),
            _ => unreachable!(),
        }
        next.validate().map_err(invalid)?;
        *self = next;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        let limit = Real::from_i64(10000);
        if self.xmin < limit.neg()
            || self.xmin > limit
            || self.ymin < limit.neg()
            || self.ymin > limit
        {
            return Err("Geometry minima must be within−10000to10000".into());
        }
        if self.pix_size <= Real::parse("0.001").unwrap() || self.pix_size > limit {
            return Err("Geometry PixSize must be greater than0.001 and at most10000".into());
        }
        if self.xtick <= Real::ZERO || self.ytick <= Real::ZERO {
            return Err("Geometry ticks must be positive".into());
        }
        Ok(())
    }
    /// Internal upper bounds are needed by ordinary graphics, but are not
    /// calculator variables in Geometry's public namespace.
    pub fn derived_bounds(&self) -> [Real; 4] {
        [
            self.xmin,
            self.xmin
                .add(
                    self.pix_size
                        .mul(Real::from_i64(319))
                        .expect("bounded Geometry width"),
                )
                .expect("bounded Geometry X range"),
            self.ymin,
            self.ymin
                .add(
                    self.pix_size
                        .mul(Real::from_i64(219))
                        .expect("bounded Geometry height"),
                )
                .expect("bounded Geometry Y range"),
        ]
    }
    pub fn zoom(&self, cursor: [Real; 2], factor: Real) -> RtResult<Self> {
        let mut next = self.clone();
        next.set("PixSize", &Value::Real(self.pix_size.mul(factor)?))?;
        next.set(
            "Xmin",
            &Value::Real(cursor[0].sub(next.pix_size.mul(Real::from_i64(160))?)?),
        )?;
        next.set(
            "Ymin",
            &Value::Real(cursor[1].sub(next.pix_size.mul(Real::from_i64(109))?)?),
        )?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(value: &str) -> Real {
        Real::parse(value).unwrap()
    }
    #[test]
    fn measured_admission_canonicalizes_before_float_conversion_and_is_atomic() {
        let mut settings = GeometryPlotSettings::default();
        for invalid in [
            Value::Real(Real::ZERO),
            Value::Real(r("0.001")),
            Value::Real(r("-0.1")),
            Value::str("0.1"),
            Value::list(vec![Value::Real(r("0.1"))]),
            crate::cas::from_cas(ppl_cas::CasValue::Infinity(ppl_cas::InfinitySign::Positive)),
            Value::Complex(ppl_math::Complex::new(r("0.1"), r("0.2"))),
        ] {
            let before = settings.clone();
            let error = settings.set("PixSize", &invalid).unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidInput);
            assert_eq!(settings, before);
        }
        settings
            .set("PixSize", &Value::Real(r("0.001000000001")))
            .unwrap();
        assert_eq!(settings.pix_size, r("0.001000000001"));
        settings.set("PixSize", &Value::Real(r("1E499"))).unwrap();
        assert_eq!(settings.pix_size, Real::from_i64(10000));
        settings.set("Xmin", &Value::Real(r("1E499"))).unwrap();
        assert_eq!(settings.xmin, Real::from_i64(10000));
        settings.set("Ymin", &Value::Real(r("-1E499"))).unwrap();
        assert_eq!(settings.ymin, Real::from_i64(-10000));
        settings
            .set(
                "PixSize",
                &Value::Int(ppl_math::HpInt::new(2, 64, ppl_math::IntBase::Hex)),
            )
            .unwrap();
        assert_eq!(settings.pix_size, Real::from_i64(2));
        settings
            .set(
                "PixSize",
                &Value::Complex(ppl_math::Complex::new(r("0.1"), Real::ZERO)),
            )
            .unwrap();
        assert_eq!(settings.pix_size, r("0.1"));
        assert!(settings.get("Xmax").is_err());
        assert!(settings.set("Ymax", &Value::Real(Real::ONE)).is_err());
    }
    #[test]
    fn minima_translate_derived_ranges_and_zoom_centers_the_live_cursor() {
        let mut settings = GeometryPlotSettings::default();
        settings
            .set("Xmin", &Value::Real(Real::from_i64(100)))
            .unwrap();
        assert_eq!(
            settings.derived_bounds(),
            [r("100"), r("131.9"), r("-11"), r("10.9")]
        );
        settings.set("Xmin", &Value::Real(r("-7"))).unwrap();
        settings.set("Ymin", &Value::Real(r("-3"))).unwrap();
        settings.set("PixSize", &Value::Real(r("0.2"))).unwrap();
        let zoomed = settings.zoom([Real::ZERO; 2], r("0.5")).unwrap();
        assert_eq!(
            (zoomed.xmin, zoomed.ymin, zoomed.pix_size),
            (r("-16"), r("-10.9"), r("0.1"))
        );
        let out = zoomed.zoom([Real::ZERO; 2], r("2")).unwrap();
        assert_eq!(
            (out.xmin, out.ymin, out.pix_size),
            (r("-32"), r("-21.8"), r("0.2"))
        );
        let center = out.zoom([r("-12"), r("8")], r("0.5")).unwrap();
        assert_eq!(
            (center.xmin, center.ymin, center.pix_size),
            (r("-28"), r("-2.9"), r("0.1"))
        );
    }
}
