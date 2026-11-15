//! Principal complex functions. Complex trig arguments are always radians on
//! the G1. Decimal scaling keeps exponential/logarithmic magnitudes outside
//! the binary float exponent range usable.
use super::*;

fn exp_times(exponent: Real, factor: f64) -> Result<Real, MathError> {
    if factor == 0.0 {
        return Ok(Real::ZERO);
    }
    let magnitude =
        crate::real::ten_pow(exponent.to_f64() / std::f64::consts::LN_10 + factor.abs().log10())?;
    Ok(if factor.is_sign_negative() {
        magnitude.neg()
    } else {
        magnitude
    })
}

impl Complex {
    pub fn exp(self) -> Result<Self, MathError> {
        if self.im.is_zero() {
            return Ok(Self::new(self.re.exp_e()?, Real::ZERO));
        }
        let angle = self.im.to_f64();
        let re = self.re.to_f64();
        if re.abs() < 700.0 {
            let magnitude = re.exp();
            return Ok(Self::new(
                Real::from_f64(magnitude * angle.cos())?,
                Real::from_f64(magnitude * angle.sin())?,
            ));
        }
        Ok(Self::new(
            exp_times(self.re, angle.cos())?,
            exp_times(self.re, angle.sin())?,
        ))
    }

    pub fn ln(self) -> Result<Self, MathError> {
        if self.re.is_zero() && self.im.is_zero() {
            return Err(MathError::Undefined);
        }
        if self.im.is_zero() {
            return Ok(Self::new(
                self.re.abs().ln()?,
                if self.re.is_negative() {
                    Real::from_f64(std::f64::consts::PI)?
                } else {
                    Real::ZERO
                },
            ));
        }
        let scale = self.scale_exp();
        let re = self.re.scale10(-scale)?.to_f64();
        let im = self.im.scale10(-scale)?.to_f64();
        Ok(Self::new(
            Real::from_f64(re.hypot(im).ln() + scale as f64 * std::f64::consts::LN_10)?,
            Real::from_f64(im.atan2(re))?,
        ))
    }

    fn sin_cos(self, cosine: bool) -> Result<Self, MathError> {
        let re = self.re.to_f64();
        let im = self.im.to_f64();
        if im.abs() < 700.0 {
            // Round once after each complete component, rather than rounding
            // hyperbolic factors before the multiplication.
            let (a, b) = if cosine {
                (re.cos() * im.cosh(), -re.sin() * im.sinh())
            } else {
                (re.sin() * im.cosh(), re.cos() * im.sinh())
            };
            return Ok(Self::new(Real::from_f64(a)?, Real::from_f64(b)?));
        }
        // At |y|>=700, exp(-|y|) cannot affect twelve-digit rounding.
        let (a, b) = if cosine {
            (re.cos(), -re.sin() * im.signum())
        } else {
            (re.sin(), re.cos() * im.signum())
        };
        Ok(Self::new(
            exp_times(self.im.abs(), a / 2.0)?,
            exp_times(self.im.abs(), b / 2.0)?,
        ))
    }
    pub fn sin(self) -> Result<Self, MathError> {
        self.sin_cos(false)
    }
    pub fn cos(self) -> Result<Self, MathError> {
        self.sin_cos(true)
    }
    pub fn tan(self) -> Result<Self, MathError> {
        let re = self.re.to_f64();
        let im = self.im.to_f64();
        if self.im.is_zero() {
            return Ok(Self::new(Real::from_f64(re.tan())?, Real::ZERO));
        }
        // Dividing the double-angle identity by exp(2|y|) avoids overflow.
        let t = (-2.0 * im.abs()).exp();
        let den = 1.0 + 2.0 * (2.0 * re).cos() * t + t * t;
        Ok(Self::new(
            Real::from_f64(2.0 * (2.0 * re).sin() * t / den)?,
            Real::from_f64(im.signum() * (1.0 - t * t) / den)?,
        ))
    }
}
