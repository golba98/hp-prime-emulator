//! Hexagesimal presentation for calculator real values. The installed G1
//! renders two minute/second digits and at most five fractional second digits,
//! independently of STRING's decimal/scientific formatting parameters.
use crate::{Real, RealPresentation};

pub(crate) fn format(value: Real) -> String {
    let value = value.with_presentation(RealPresentation::Decimal);
    let sign = if value.is_negative() { "−" } else { "" };
    let value = value.abs();
    let mut degrees = value.floor();
    // Each intermediate is in [0,60); there is no exponent overflow even
    // for a real near the calculator's maximum magnitude.
    let minutes = value.sub(degrees).unwrap().mul(Real::from_i64(60)).unwrap();
    let mut whole_minutes = minutes.floor();
    let mut seconds = minutes
        .sub(whole_minutes)
        .unwrap()
        .mul(Real::from_i64(60))
        .unwrap()
        .round_places(5);
    if seconds >= Real::from_i64(60) {
        seconds = Real::ZERO;
        whole_minutes = whole_minutes.add(Real::ONE).unwrap();
    }
    if whole_minutes >= Real::from_i64(60) {
        whole_minutes = Real::ZERO;
        degrees = degrees.add(Real::ONE).unwrap();
    }
    let seconds_text = seconds.to_string();
    let seconds_text = if seconds < Real::from_i64(10) {
        format!("0{seconds_text}")
    } else {
        seconds_text
    };
    format!(
        "{sign}{degrees}°{:02}′{seconds_text}″",
        whole_minutes.to_i64().unwrap()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FormatOptions, NumFormat};

    #[test]
    fn exact_carry_padding_and_original_decimal_are_preserved() {
        for (decimal, expected) in [
            ("1.375", "1°22′30″"),
            ("-1.5", "−1°30′00″"),
            ("2.2360679775", "2°14′09.84472″"),
            ("1.00003429355", "1°00′00.12346″"),
            ("1.01666666667", "1°01′00″"),
            ("0", "0°00′00″"),
        ] {
            let number = Real::parse(decimal).unwrap();
            let formatted = number.with_presentation(RealPresentation::Sexagesimal);
            assert_eq!(formatted.format(FormatOptions::default()), expected);
            assert_eq!(formatted, number);
            assert_eq!(
                formatted.to_string(),
                decimal,
                "canonical transport remains decimal"
            );
            assert_eq!(
                formatted.format(FormatOptions {
                    format: NumFormat::Scientific(4),
                    ..Default::default()
                }),
                expected
            );
        }
    }
}
