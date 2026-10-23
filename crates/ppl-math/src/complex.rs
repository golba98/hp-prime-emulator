//! Complex numbers built from two 12-digit reals.

use crate::{FormatOptions, MathError, Real};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    pub re: Real,
    pub im: Real,
}

impl Complex {
    pub fn new(re: Real, im: Real) -> Complex {
        Complex { re, im }
    }

    pub fn add(self, o: Complex) -> Result<Complex, MathError> {
        Ok(Complex::new(self.re.add(o.re)?, self.im.add(o.im)?))
    }

    pub fn sub(self, o: Complex) -> Result<Complex, MathError> {
        Ok(Complex::new(self.re.sub(o.re)?, self.im.sub(o.im)?))
    }

    pub fn mul(self, o: Complex) -> Result<Complex, MathError> {
        let re = self.re.mul(o.re)?.sub(self.im.mul(o.im)?)?;
        let im = self.re.mul(o.im)?.add(self.im.mul(o.re)?)?;
        Ok(Complex::new(re, im))
    }

    pub fn div(self, o: Complex) -> Result<Complex, MathError> {
        if o.re.is_zero() && o.im.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        // Both operands are scaled by exact powers of ten so that their
        // larger part lies in [1, 10). This gives the same digits as the
        // unscaled formula wherever that one stays in range, but `|o|²`
        // and the products can no longer overflow or flush to zero when
        // the quotient itself is representable.
        let j = self.scale_exp();
        let k = o.scale_exp();
        let (a, b) = (self.re.scale10(-j)?, self.im.scale10(-j)?);
        let (c, d) = (o.re.scale10(-k)?, o.im.scale10(-k)?);
        let den = c.mul(c)?.add(d.mul(d)?)?;
        let re = a.mul(c)?.add(b.mul(d)?)?.div(den)?;
        let im = b.mul(c)?.sub(a.mul(d)?)?.div(den)?;
        Ok(Complex::new(re.scale10(j - k)?, im.scale10(j - k)?))
    }

    /// Decimal exponent of the larger part (0 for zero).
    fn scale_exp(self) -> i32 {
        match (self.re.is_zero(), self.im.is_zero()) {
            (true, true) => 0,
            (false, true) => self.re.exp10(),
            (true, false) => self.im.exp10(),
            (false, false) => self.re.exp10().max(self.im.exp10()),
        }
    }

    pub fn neg(self) -> Complex {
        Complex::new(self.re.neg(), self.im.neg())
    }

    pub fn conj(self) -> Complex {
        Complex::new(self.re, self.im.neg())
    }

    pub fn abs(self) -> Result<Real, MathError> {
        // Scaled by an exact power of ten (same digits as the unscaled
        // formula when that stays in range) so `re² + im²` cannot overflow
        // or flush to zero: |3E300+4E300i| = 5E300.
        let k = self.scale_exp();
        let (a, b) = (self.re.scale10(-k)?, self.im.scale10(-k)?);
        a.mul(a)?.add(b.mul(b)?)?.sqrt()?.scale10(k)
    }

    /// Argument in radians.
    pub fn arg_radians(self) -> Result<Real, MathError> {
        Real::from_f64(self.im.to_f64().atan2(self.re.to_f64()))
    }

    pub fn sqrt(self) -> Result<Complex, MathError> {
        // Outside the f64 range, take the root of z / 10^(2h) and multiply
        // by 10^h (exact). Inside it the f64 path is unchanged.
        let e = self.scale_exp();
        if !(-300..=300).contains(&e) {
            let h = e.div_euclid(2);
            let z = Complex::new(self.re.scale10(-2 * h)?, self.im.scale10(-2 * h)?);
            let r = z.sqrt()?;
            return Ok(Complex::new(r.re.scale10(h)?, r.im.scale10(h)?));
        }
        let (re, im) = (self.re.to_f64(), self.im.to_f64());
        let r = re.hypot(im);
        let a = ((r + re) / 2.0).sqrt();
        let b = ((r - re) / 2.0)
            .sqrt()
            .copysign(if im == 0.0 { 1.0 } else { im });
        Ok(Complex::new(Real::from_f64(a)?, Real::from_f64(b)?))
    }

    pub fn is_real(self) -> bool {
        self.im.is_zero()
    }

    /// `a+b*i` as displayed on the Prime (unverified spacing and sign rules).
    pub fn format(self, opts: FormatOptions) -> String {
        if self.im.is_zero() {
            return self.re.format(opts);
        }
        let im_abs = self.im.abs();
        let im_part = if im_abs == Real::ONE {
            "i".to_string()
        } else {
            format!("{}*i", im_abs.format(opts))
        };
        if self.re.is_zero() {
            if self.im.is_negative() {
                format!("-{im_part}")
            } else {
                im_part
            }
        } else {
            let sign = if self.im.is_negative() { '-' } else { '+' };
            format!("{}{sign}{im_part}", self.re.format(opts))
        }
    }
}
