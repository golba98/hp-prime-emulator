//! Display projection for a history entry; typed calculator values remain intact.
use ppl_math::{Complex, Real};
use ppl_vm::Settings;
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
    value
        .format_polar(settings.format())
        .map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use ppl_vm::AngleMode;
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
                    format!("{magnitude}∡{expected}")
                );
            }
        }
    }
}
