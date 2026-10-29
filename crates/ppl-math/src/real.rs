//! Decimal floating-point reals matching the HP Prime Home number model.
//!
//! The Prime's Home/PPL reals are decimal with 12 significant digits and a
//! decimal exponent of at most ±499 (documented in the HP Prime User Guide,
//! "Numbers"). Each value is kept as `mant × 10^exp` with `mant < 10^12`.
//!
//! The four basic operations are computed exactly in 128-bit integers and
//! rounded once to 12 digits, nearest with ties to even (observed for stored
//! literals and addition on physical G1 build 15157). Explicit `ROUND` uses
//! its separate half-away-from-zero rule. Transcendental functions go
//! through `f64` and are then rounded to 12 digits. That is at most one unit in
//! the last place away from a correctly rounded result and is tracked as an
//! approximation.

use std::cmp::Ordering;
use std::fmt;

use crate::MathError;

/// Significant decimal digits kept by a real.
pub const DIGITS: u32 = 12;
/// Largest decimal exponent of the leading digit.
pub const MAX_EXP10: i32 = 499;
/// Smallest decimal exponent of the leading digit.
pub const MIN_EXP10: i32 = -499;

const MANT_LIMIT: u64 = 1_000_000_000_000; // 10^DIGITS

const POW10_U128: [u128; 39] = {
    let mut t = [1u128; 39];
    let mut i = 1;
    while i < 39 {
        t[i] = t[i - 1] * 10;
        i += 1;
    }
    t
};

const POW10_F64: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

fn digits_u128(v: u128) -> u32 {
    if v == 0 {
        return 1;
    }
    let mut d = 0;
    while d < 39 && POW10_U128[d as usize] <= v {
        d += 1;
    }
    d
}

/// A 12-significant-digit decimal real.
#[derive(Clone, Copy, Debug)]
pub struct Real {
    neg: bool,
    /// Coefficient without trailing zeros; zero only for the value zero.
    mant: u64,
    exp: i32,
}

impl Real {
    pub const ZERO: Real = Real {
        neg: false,
        mant: 0,
        exp: 0,
    };
    pub const ONE: Real = Real {
        neg: false,
        mant: 1,
        exp: 0,
    };

    /// Builds a real from an exact coefficient, rounding to 12 digits.
    pub fn from_parts(neg: bool, coeff: u128, exp: i32) -> Result<Real, MathError> {
        Real::from_parts_wide(neg, coeff, exp as i64)
    }

    /// [`Real::from_parts`] with a 64-bit exponent, so callers can pass
    /// exponents far outside the range without wrapping `i32`.
    fn from_parts_wide(neg: bool, coeff: u128, exp: i64) -> Result<Real, MathError> {
        Self::from_parts_wide_sticky(neg, coeff, exp, false)
    }

    /// `sticky` records any nonzero tail discarded below `coeff`'s last digit.
    /// It distinguishes an exact halfway coefficient from a value above it.
    fn from_parts_wide_sticky(
        neg: bool,
        coeff: u128,
        exp: i64,
        sticky: bool,
    ) -> Result<Real, MathError> {
        if coeff == 0 {
            return Ok(Real::ZERO);
        }
        let (mant, exp) = round_coeff(coeff, exp, sticky);
        let e10 = exp + digits_u128(mant as u128) as i64 - 1;
        if e10 > MAX_EXP10 as i64 {
            return Err(MathError::Overflow);
        }
        if e10 < MIN_EXP10 as i64 {
            // Underflow flushes to zero (unverified against the G1).
            return Ok(Real::ZERO);
        }
        Ok(Real {
            neg,
            mant,
            exp: exp as i32,
        })
    }

    /// Rounds like [`Real::from_parts`] but without the range check. Only
    /// for intermediate values and display; `exp` must be within a few
    /// thousand of zero.
    fn unchecked(neg: bool, coeff: u128, exp: i64) -> Real {
        if coeff == 0 {
            return Real::ZERO;
        }
        let (mant, exp) = round_coeff(coeff, exp, false);
        Real {
            neg,
            mant,
            exp: exp as i32,
        }
    }

    /// Multiplies by `10^k` exactly (only the range can fail).
    pub fn scale10(self, k: i32) -> Result<Real, MathError> {
        if self.mant == 0 {
            return Ok(self);
        }
        Real::from_parts_wide(self.neg, self.mant as u128, self.exp as i64 + k as i64)
    }

    pub fn from_i64(v: i64) -> Real {
        Real::from_parts(v < 0, v.unsigned_abs() as u128, 0).expect("i64 always fits")
    }

    pub fn from_u64(v: u64) -> Real {
        Real::from_parts(false, v as u128, 0).expect("u64 always fits")
    }

    /// Rounds an `f64` to 12 significant digits.
    pub fn from_f64(v: f64) -> Result<Real, MathError> {
        if v.is_nan() {
            return Err(MathError::Undefined);
        }
        if v.is_infinite() {
            return Err(MathError::Overflow);
        }
        if v == 0.0 {
            return Ok(Real::ZERO);
        }
        // `{:.16e}` prints the binary value correctly rounded to 17 digits.
        // `parse` keeps 13 of them and rounds on the 13th, which is exact for
        // nearest/ties-to-even rounding unless the 17-digit rounding itself
        // produced a halfway pattern (`…d50000`) from a value beside it.
        // Then the exact expansion (at most 767 significant digits for an
        // f64) decides.
        let s = format!("{:.16e}", v);
        let digits: String = s
            .split(['e', 'E'])
            .next()
            .unwrap_or("")
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if digits.len() == 17 && &digits[12..] == "50000" {
            let exact = format!("{:.800e}", v);
            return Real::parse(&exact).ok_or(MathError::Undefined);
        }
        Real::parse(&s).ok_or(MathError::Undefined)
    }

    pub fn to_f64(self) -> f64 {
        if self.mant == 0 {
            return 0.0;
        }
        let m = self.mant as f64;
        let v = if (0..=22).contains(&self.exp) {
            m * POW10_F64[self.exp as usize]
        } else if (-22..0).contains(&self.exp) {
            m / POW10_F64[(-self.exp) as usize]
        } else {
            format!("{}e{}", self.mant, self.exp)
                .parse::<f64>()
                .unwrap_or(f64::INFINITY)
        };
        if self.neg { -v } else { v }
    }

    /// Parses a decimal literal: `123`, `1.5`, `.5`, `1E-3`, `1ᴇ5`, `2.e3`.
    pub fn parse(s: &str) -> Option<Real> {
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-').or_else(|| s.strip_prefix('−')) {
            Some(rest) => (true, rest),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (body, exp_part) = match s.find(['e', 'E', 'ᴇ']) {
            Some(i) => {
                let ch_len = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
                (&s[..i], Some(&s[i + ch_len..]))
            }
            None => (s, None),
        };
        let mut coeff: u128 = 0;
        let mut sig = 0u32;
        let mut exp: i64 = 0;
        let mut seen_digit = false;
        let mut seen_dot = false;
        let mut sticky = false;
        for c in body.chars() {
            match c {
                '0'..='9' => {
                    seen_digit = true;
                    let d = c as u8 - b'0';
                    if coeff == 0 && d == 0 {
                        if seen_dot {
                            exp -= 1;
                        }
                        continue;
                    }
                    if sig < DIGITS + 1 {
                        coeff = coeff * 10 + d as u128;
                        sig += 1;
                        if seen_dot {
                            exp -= 1;
                        }
                    } else {
                        sticky |= d != 0;
                        if !seen_dot {
                            exp += 1;
                        }
                    }
                }
                '.' if !seen_dot => seen_dot = true,
                _ => return None,
            }
        }
        if !seen_digit {
            return None;
        }
        if let Some(e) = exp_part {
            exp += parse_exponent(e)?;
        }
        // Keep the guard digit and whether later digits are nonzero. Dropping
        // that tail would turn a just-above-half literal into an exact tie.
        Real::from_parts_wide_sticky(neg, coeff, exp, sticky).ok()
    }

    pub fn is_zero(self) -> bool {
        self.mant == 0
    }

    pub fn is_negative(self) -> bool {
        self.neg && self.mant != 0
    }

    /// Decimal exponent of the leading digit.
    pub fn exp10(self) -> i32 {
        if self.mant == 0 {
            return 0;
        }
        self.exp + digits_u128(self.mant as u128) as i32 - 1
    }

    pub fn mantissa_digits(self) -> String {
        self.mant.to_string()
    }

    pub fn raw_exp(self) -> i32 {
        self.exp
    }

    pub fn neg(self) -> Real {
        if self.mant == 0 {
            self
        } else {
            Real {
                neg: !self.neg,
                ..self
            }
        }
    }

    pub fn abs(self) -> Real {
        Real { neg: false, ..self }
    }

    pub fn is_integer(self) -> bool {
        self.mant == 0 || self.exp >= 0
    }

    pub fn to_i64(self) -> Option<i64> {
        if !self.is_integer() || self.exp10() > 18 {
            return None;
        }
        let v = (self.mant as i128) * POW10_U128[self.exp.max(0) as usize] as i128;
        let v = if self.neg { -v } else { v };
        i64::try_from(v).ok()
    }

    pub fn add(self, o: Real) -> Result<Real, MathError> {
        if self.mant == 0 {
            return Ok(o);
        }
        if o.mant == 0 {
            return Ok(self);
        }
        let (big, small) = if self.exp10() >= o.exp10() {
            (self, o)
        } else {
            (o, self)
        };
        if big.exp10() - small.exp10() > (DIGITS as i32 + 2) {
            return Ok(big);
        }
        let emin = big.exp.min(small.exp);
        let a = big.mant as u128 * POW10_U128[(big.exp - emin) as usize];
        let b = small.mant as u128 * POW10_U128[(small.exp - emin) as usize];
        if big.neg == small.neg {
            Real::from_parts(big.neg, a + b, emin)
        } else if a >= b {
            Real::from_parts(big.neg, a - b, emin)
        } else {
            Real::from_parts(small.neg, b - a, emin)
        }
    }

    pub fn sub(self, o: Real) -> Result<Real, MathError> {
        self.add(o.neg())
    }

    pub fn mul(self, o: Real) -> Result<Real, MathError> {
        if self.mant == 0 || o.mant == 0 {
            return Ok(Real::ZERO);
        }
        Real::from_parts(
            self.neg != o.neg,
            self.mant as u128 * o.mant as u128,
            self.exp + o.exp,
        )
    }

    pub fn div(self, o: Real) -> Result<Real, MathError> {
        if o.mant == 0 {
            return Err(MathError::DivisionByZero);
        }
        if self.mant == 0 {
            return Ok(Real::ZERO);
        }
        // 25 extra digits leave at least 14 quotient digits, enough to round.
        const SHIFT: u32 = 25;
        let num = self.mant as u128 * POW10_U128[SHIFT as usize];
        let q = num / o.mant as u128;
        Real::from_parts_wide_sticky(
            self.neg != o.neg,
            q,
            self.exp as i64 - o.exp as i64 - SHIFT as i64,
            num % o.mant as u128 != 0,
        )
    }

    /// Total order on values (zero has no sign).
    pub fn cmp_value(self, o: Real) -> Ordering {
        let sa = if self.mant == 0 {
            0
        } else if self.neg {
            -1
        } else {
            1
        };
        let sb = if o.mant == 0 {
            0
        } else if o.neg {
            -1
        } else {
            1
        };
        if sa != sb {
            return sa.cmp(&sb);
        }
        if sa == 0 {
            return Ordering::Equal;
        }
        let mag = match self.exp10().cmp(&o.exp10()) {
            Ordering::Equal => {
                let da = digits_u128(self.mant as u128);
                let db = digits_u128(o.mant as u128);
                let a = self.mant as u128 * POW10_U128[(DIGITS - da) as usize];
                let b = o.mant as u128 * POW10_U128[(DIGITS - db) as usize];
                a.cmp(&b)
            }
            other => other,
        };
        if sa < 0 { mag.reverse() } else { mag }
    }

    /// Integer part toward negative infinity.
    pub fn floor(self) -> Real {
        let t = self.trunc();
        if self.neg && t.cmp_value(self) != Ordering::Equal {
            t.sub(Real::ONE).unwrap_or(t)
        } else {
            t
        }
    }

    pub fn ceil(self) -> Real {
        let t = self.trunc();
        if !self.neg && t.cmp_value(self) != Ordering::Equal {
            t.add(Real::ONE).unwrap_or(t)
        } else {
            t
        }
    }

    /// Integer part toward zero (`IP`).
    pub fn trunc(self) -> Real {
        if self.is_integer() {
            return self;
        }
        let drop = (-self.exp) as u32;
        if drop >= 19 {
            return Real::ZERO;
        }
        let q = self.mant as u128 / POW10_U128[drop as usize];
        Real::from_parts(self.neg, q, 0).unwrap_or(Real::ZERO)
    }

    /// Fractional part (`FP`), keeping the sign.
    pub fn fract(self) -> Real {
        self.sub(self.trunc()).unwrap_or(Real::ZERO)
    }

    /// Rounds to `places` decimal places, half away from zero. Negative
    /// `places` rounds to significant digits, as `ROUND` does on the Prime.
    pub fn round_places(self, places: i32) -> Real {
        match self.quantize(places, true) {
            Some((q, e)) => Real::from_parts_wide(self.neg, q, e).unwrap_or(self),
            None => self,
        }
    }

    /// Truncates to `places` decimal places (`TRUNCATE`).
    pub fn truncate_places(self, places: i32) -> Real {
        match self.quantize(places, false) {
            Some((q, e)) => Real::from_parts_wide(self.neg, q, e).unwrap_or(self),
            None => self,
        }
    }

    /// Coefficient and exponent of `self` rounded (or truncated) to
    /// `places`, or `None` when nothing changes. Exponents are computed in
    /// 64 bits so extreme `places` cannot overflow.
    fn quantize(self, places: i32, round: bool) -> Option<(u128, i64)> {
        if self.mant == 0 {
            return None;
        }
        let target_exp = if places >= 0 {
            -(places as i64)
        } else {
            self.exp10() as i64 + places as i64 + 1
        };
        if self.exp as i64 >= target_exp {
            return None;
        }
        let drop = target_exp - self.exp as i64;
        if drop > 18 {
            // The coefficient has at most 12 digits, so it rounds to zero.
            return Some((0, 0));
        }
        let p = POW10_U128[drop as usize];
        let m = self.mant as u128;
        let q = m / p;
        let q = if round && (m % p) * 2 >= p { q + 1 } else { q };
        Some((q, target_exp))
    }

    /// Raises to an integer power exactly when the coefficient fits in 128
    /// bits, otherwise through logarithms.
    pub fn powi(self, n: i64) -> Result<Real, MathError> {
        if n == 0 {
            if self.mant == 0 {
                return Err(MathError::Undefined);
            }
            return Ok(Real::ONE);
        }
        if self.mant == 0 {
            return if n < 0 {
                Err(MathError::DivisionByZero)
            } else {
                Ok(Real::ZERO)
            };
        }
        let un = n.unsigned_abs();
        let d = digits_u128(self.mant as u128) as u64;
        let neg = self.neg && un % 2 == 1;
        if d.saturating_mul(un) <= 38 {
            // Exact coefficient (< 10^38) and exponent (|exp| <= 38 * 520).
            let mut acc: u128 = 1;
            for _ in 0..un {
                acc *= self.mant as u128;
            }
            let exp = self.exp as i64 * un as i64;
            if n > 0 {
                return Real::from_parts_wide(neg, acc, exp);
            }
            // 1 / (acc × 10^exp), rounded once: x^n may underflow while its
            // reciprocal is still representable, and rounding x^n first
            // would round twice.
            let k = digits_u128(acc) + DIGITS + 1;
            let (coefficient, sticky) = recip_scaled(acc, k);
            return Real::from_parts_wide_sticky(neg, coefficient, -(k as i64) - exp, sticky);
        }
        let r = pow_via_log10(self.abs(), n as f64)?;
        Ok(if neg { r.neg() } else { r })
    }

    pub fn pow(self, y: Real) -> Result<Real, MathError> {
        if let Some(n) = y.to_i64() {
            return self.powi(n);
        }
        if self.mant == 0 {
            return if y.is_negative() {
                Err(MathError::DivisionByZero)
            } else {
                Ok(Real::ZERO)
            };
        }
        if self.neg {
            // An integer too large for i64 has a coefficient of at most 12
            // digits times 10^k with k >= 7, so it is even.
            if y.is_integer() {
                return pow_via_log10(self.abs(), y.to_f64());
            }
            return Err(MathError::NonReal);
        }
        pow_via_log10(self, y.to_f64())
    }

    pub fn sqrt(self) -> Result<Real, MathError> {
        if self.neg && self.mant != 0 {
            return Err(MathError::NonReal);
        }
        if self.mant == 0 {
            return Ok(self);
        }
        // Scale the coefficient to a 36- or 37-digit radicand with an even
        // exponent. The radicand stays below 10^37 < 2^127, and its integer
        // square root has 18 or 19 digits. Preserve a nonzero square-root
        // remainder as a sticky tail when rounding that truncated root.
        let d = digits_u128(self.mant as u128) as i32;
        let mut shift = 36 - d;
        if (self.exp - shift) % 2 != 0 {
            shift += 1;
        }
        let m = self.mant as u128 * POW10_U128[shift as usize];
        let exp = self.exp - shift;
        let root = isqrt(m);
        Real::from_parts_wide_sticky(false, root, exp as i64 / 2, root * root != m)
    }

    pub fn exp_e(self) -> Result<Real, MathError> {
        let x = self.to_f64();
        if x.abs() < 700.0 {
            return Real::from_f64(x.exp());
        }
        // e^x = 10^(x / ln 10), split so f64 does not overflow.
        ten_pow(x / std::f64::consts::LN_10)
    }

    pub fn ln(self) -> Result<Real, MathError> {
        if self.mant == 0 {
            return Err(MathError::Undefined);
        }
        if self.neg {
            return Err(MathError::NonReal);
        }
        Real::from_f64(self.ln_f64())
    }

    pub fn log10(self) -> Result<Real, MathError> {
        if self.mant == 0 {
            return Err(MathError::Undefined);
        }
        if self.neg {
            return Err(MathError::NonReal);
        }
        if self.mant == 1 {
            return Ok(Real::from_i64(self.exp as i64));
        }
        Real::from_f64((self.mant as f64).log10() + self.exp as f64)
    }

    fn ln_f64(self) -> f64 {
        (self.mant as f64).ln() + self.exp as f64 * std::f64::consts::LN_10
    }
}

/// `floor(sqrt(n))` for any `n` below 2^127.
fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    // One Newton step from the f64 estimate lands at or above the floor
    // root (AM-GM), after which Newton decreases monotonically to it.
    let est = ((n as f64).sqrt() as u128).max(1);
    let mut x = (est + n / est) / 2;
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// `floor(10^k / d)` by long division, for `1 <= d < 2^127` and quotients
/// that fit in `u128`. Each step multiplies the remainder by ten through
/// repeated addition, so nothing overflows.
fn recip_scaled(d: u128, k: u32) -> (u128, bool) {
    if d == 1 {
        return (POW10_U128[k as usize], false);
    }
    let mut rem: u128 = 1;
    let mut q: u128 = 0;
    for _ in 0..k {
        let mut s: u128 = 0;
        let mut digit = 0;
        for _ in 0..10 {
            s += rem;
            if s >= d {
                s -= d;
                digit += 1;
            }
        }
        q = q * 10 + digit;
        rem = s;
    }
    (q, rem != 0)
}

/// Rounds a non-zero coefficient to at most 12 digits (nearest/ties-to-even)
/// and strips trailing zeros. A nonzero discarded tail makes a tie above-half.
fn round_coeff(coeff: u128, exp: i64, sticky: bool) -> (u64, i64) {
    let mut coeff = coeff;
    let mut exp = exp;
    let d = digits_u128(coeff);
    if d > DIGITS {
        let drop = d - DIGITS;
        let p = POW10_U128[drop as usize];
        let q = coeff / p;
        let r = coeff % p;
        let round_up = r > p - r || (r == p - r && (sticky || q % 2 != 0));
        coeff = if round_up { q + 1 } else { q };
        exp += drop as i64;
        if coeff == MANT_LIMIT as u128 {
            coeff /= 10;
            exp += 1;
        }
    }
    let mut mant = coeff as u64;
    while mant % 10 == 0 {
        mant /= 10;
        exp += 1;
    }
    (mant, exp)
}

/// Parses an exponent (`-12`, `+3`, `−4`), saturating far outside the
/// representable range instead of failing or wrapping.
fn parse_exponent(e: &str) -> Option<i64> {
    let (neg, digits) = if let Some(rest) = e.strip_prefix('-').or_else(|| e.strip_prefix('−')) {
        (true, rest)
    } else {
        (false, e.strip_prefix('+').unwrap_or(e))
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v: i64 = 0;
    for b in digits.bytes() {
        v = (v * 10 + (b - b'0') as i64).min(1_000_000_000_000);
    }
    Some(if neg { -v } else { v })
}

fn ten_pow(l: f64) -> Result<Real, MathError> {
    if !l.is_finite() {
        return Err(MathError::Overflow);
    }
    let int = l.floor();
    let frac = l - int;
    if int > MAX_EXP10 as f64 + 1.0 {
        return Err(MathError::Overflow);
    }
    if int < MIN_EXP10 as f64 - 2.0 {
        return Ok(Real::ZERO);
    }
    let m = Real::from_f64(10f64.powf(frac))?;
    Real::from_parts(m.neg, m.mant as u128, m.exp + int as i32)
}

fn pow_via_log10(x: Real, y: f64) -> Result<Real, MathError> {
    let l = ((x.mant as f64).log10() + x.exp as f64) * y;
    ten_pow(l)
}

impl PartialEq for Real {
    fn eq(&self, other: &Self) -> bool {
        self.cmp_value(*other) == Ordering::Equal
    }
}

impl PartialOrd for Real {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp_value(*other))
    }
}

/// Number display formats (`HFormat`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NumFormat {
    #[default]
    Standard,
    Fixed(u8),
    Scientific(u8),
    Engineering(u8),
}

/// Formatting options shared by every real formatter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    pub format: NumFormat,
    /// Exponent marker. The calculator shows `ᴇ` (U+1D07); ASCII `E` is the
    /// host default (unverified which one STRING() produces).
    pub exp_char: char,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            format: NumFormat::Standard,
            exp_char: 'E',
        }
    }
}

impl Real {
    pub fn format(self, opts: FormatOptions) -> String {
        match opts.format {
            NumFormat::Standard => self.format_standard(opts.exp_char),
            NumFormat::Fixed(n) => self.format_fixed(n as i32, opts.exp_char),
            NumFormat::Scientific(n) => self.format_sci(n as i32, opts.exp_char, false),
            NumFormat::Engineering(n) => self.format_sci(n as i32, opts.exp_char, true),
        }
    }

    fn format_standard(self, e: char) -> String {
        if self.mant == 0 {
            return "0".to_string();
        }
        let s = self.mant.to_string();
        let n = s.len() as i32;
        let e10 = self.exp10();
        let sign = if self.neg { "-" } else { "" };
        let body = if (0..DIGITS as i32).contains(&e10) {
            let int_len = e10 + 1;
            if n <= int_len {
                format!("{}{}", s, "0".repeat((int_len - n) as usize))
            } else {
                format!("{}.{}", &s[..int_len as usize], &s[int_len as usize..])
            }
        } else if e10 < 0 && (-e10 - 1) + n <= DIGITS as i32 {
            format!("0.{}{}", "0".repeat((-e10 - 1) as usize), s)
        } else {
            sci_body(&s, e10, e)
        };
        format!("{sign}{body}")
    }

    fn format_fixed(self, places: i32, e: char) -> String {
        let r = self.round_places(places);
        if r.mant != 0 && r.exp10() >= DIGITS as i32 {
            return self.format_sci(DIGITS as i32 - 1, e, false);
        }
        let sign = if r.neg && r.mant != 0 { "-" } else { "" };
        // `r` is a multiple of 10^-places, so `r × 10^places` is the
        // coefficient followed by zeros; built as text so wide formats
        // cannot overflow.
        let scaled = if r.mant == 0 {
            "0".to_string()
        } else {
            let shift = (r.exp + places).max(0) as usize;
            format!("{}{}", r.mant, "0".repeat(shift))
        };
        let digits = format!("{:0>width$}", scaled, width = places as usize + 1);
        let split = digits.len() - places as usize;
        if places == 0 {
            format!("{sign}{}", &digits[..split])
        } else {
            format!("{sign}{}.{}", &digits[..split], &digits[split..])
        }
    }

    fn format_sci(self, places: i32, e: char, eng: bool) -> String {
        if self.mant == 0 {
            return if places > 0 {
                format!("0.{}{}0", "0".repeat(places as usize), e)
            } else {
                format!("0{e}0")
            };
        }
        // Round to `places + 1` significant digits without the range check:
        // 9.99999999999E499 shown with 3 places is 1.000E500.
        let r = match self.quantize(-(places + 1), true) {
            Some((q, ex)) => Real::unchecked(self.neg, q, ex),
            None => self,
        };
        let mut e10 = r.exp10();
        let mut int_digits = 1;
        if eng {
            let shift = e10.rem_euclid(3);
            e10 -= shift;
            int_digits += shift;
        }
        let mut s = r.mant.to_string();
        let need = (int_digits + places) as usize;
        while s.len() < need {
            s.push('0');
        }
        let sign = if r.neg { "-" } else { "" };
        let (a, b) = s.split_at(int_digits as usize);
        let b = &b[..(places as usize).min(b.len())];
        if b.is_empty() {
            format!("{sign}{a}{e}{e10}")
        } else {
            format!("{sign}{a}.{b}{e}{e10}")
        }
    }
}

fn sci_body(s: &str, e10: i32, e: char) -> String {
    if s.len() == 1 {
        format!("{s}{e}{e10}")
    } else {
        format!("{}.{}{e}{e10}", &s[..1], &s[1..])
    }
}

impl fmt::Display for Real {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format(FormatOptions::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> Real {
        Real::parse(s).unwrap()
    }

    #[test]
    fn parse_and_display() {
        assert_eq!(r("123").to_string(), "123");
        assert_eq!(r("1.50").to_string(), "1.5");
        assert_eq!(r(".5").to_string(), "0.5");
        assert_eq!(r("1E12").to_string(), "1E12");
        assert_eq!(r("999999999999").to_string(), "999999999999");
        assert_eq!(r("0.0001").to_string(), "0.0001");
        assert_eq!(r("1ᴇ-20").to_string(), "1E-20");
        assert_eq!(r("-2.5").to_string(), "-2.5");
        assert_eq!(r("3.141592653589793").to_string(), "3.14159265359");
    }

    #[test]
    fn twelve_digit_arithmetic() {
        let third = Real::ONE.div(r("3")).unwrap();
        assert_eq!(third.to_string(), "0.333333333333");
        assert_eq!(third.mul(r("3")).unwrap().to_string(), "0.999999999999");
        assert_eq!(r("2").div(r("3")).unwrap().to_string(), "0.666666666667");
        assert_eq!(r("0.1").add(r("0.2")).unwrap().to_string(), "0.3");
        assert_eq!(r("1E12").add(r("1")).unwrap().to_string(), "1E12");
        assert_eq!(r("10").sub(r("10")).unwrap().to_string(), "0");
    }

    #[test]
    fn rounding_functions() {
        assert_eq!(r("2.5").round_places(0).to_string(), "3");
        assert_eq!(r("-2.5").round_places(0).to_string(), "-3");
        assert_eq!(r("3.789").truncate_places(2).to_string(), "3.78");
        assert_eq!(r("-3.9").floor().to_string(), "-4");
        assert_eq!(r("3.2").ceil().to_string(), "4");
        assert_eq!(r("-3.9").trunc().to_string(), "-3");
        assert_eq!(r("123456").round_places(-2).to_string(), "120000");
    }

    #[test]
    fn range_limits() {
        assert!(r("9.99999999999E499").mul(r("10")).is_err());
        assert!(r("1E-499").div(r("10")).unwrap().is_zero());
        assert!(Real::parse("1E500").is_none());
    }

    #[test]
    fn powers_and_roots() {
        assert_eq!(r("2").pow(r("10")).unwrap().to_string(), "1024");
        assert_eq!(r("2").pow(r("0.5")).unwrap().to_string(), "1.41421356237");
        assert_eq!(r("16").sqrt().unwrap().to_string(), "4");
        assert_eq!(r("2").sqrt().unwrap().to_string(), "1.41421356237");
        assert_eq!(r("1000").exp_e().unwrap().to_string(), "1.97007111402E434");
        assert_eq!(r("100").log10().unwrap().to_string(), "2");
    }

    #[test]
    fn formats() {
        let o = |f| FormatOptions {
            format: f,
            exp_char: 'E',
        };
        assert_eq!(r("3.14159").format(o(NumFormat::Fixed(2))), "3.14");
        assert_eq!(r("5").format(o(NumFormat::Fixed(3))), "5.000");
        assert_eq!(r("12345").format(o(NumFormat::Scientific(2))), "1.23E4");
        assert_eq!(r("12345").format(o(NumFormat::Engineering(2))), "12.30E3");
    }

    #[test]
    fn to_i64_roundtrip() {
        assert_eq!(r("42").to_i64(), Some(42));
        assert_eq!(r("4.2").to_i64(), None);
        assert_eq!(r("-1E15").to_i64(), Some(-1_000_000_000_000_000));
    }
}
