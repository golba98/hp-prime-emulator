//! Display projection for a history entry; typed calculator values remain intact.
use ppl_math::{Complex, Real};
use ppl_vm::{AngleMode, Settings};
pub fn value(value: &serde_json::Value) -> Option<Complex> {
    match value["type"].as_str()? {
        "real" => Some(Complex::new(
            Real::parse(value["value"].as_str()?)?,
            Real::ZERO,
        )),
        "complex" => Some(Complex::new(
            Real::parse(value["real"].as_str()?)?,
            Real::parse(value["imaginary"].as_str()?)?,
        )),
        _ => None,
    }
}
pub fn format(value: Complex, settings: &Settings) -> Result<String, String> {
    let result = (|| -> Result<String, ppl_math::MathError> {
        let magnitude = value.abs()?;
        let angle = if value.im.is_zero() && !value.re.is_negative() {
            Real::ZERO
        } else if settings.angle != AngleMode::Radians && (value.re.is_zero() || value.im.is_zero())
        {
            let quarter = settings.angle.full_turn() / 4.;
            Real::from_f64(if value.im.is_zero() {
                2. * quarter
            } else if value.im.is_negative() {
                -quarter
            } else {
                quarter
            })?
        } else {
            Real::from_f64(settings.from_radians(value.arg_radians()?.to_f64()))?
        };
        Ok(format!(
            "{}∠{}",
            magnitude.format(settings.format()),
            angle.format(settings.format())
        ))
    })();
    result.map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn angle_mode_axes_and_bcd_scale_have_correct_display() {
        for (mode, expected) in [(AngleMode::Degrees, "90"), (AngleMode::Gradians, "100")] {
            let settings = Settings {
                angle: mode,
                ..Settings::default()
            };
            for magnitude in ["2", "1E400", "1E-400"] {
                let complex = Complex::new(Real::ZERO, Real::parse(magnitude).unwrap());
                assert_eq!(
                    format(complex, &settings).unwrap(),
                    format!("{magnitude}∠{expected}")
                );
            }
        }
    }
}
