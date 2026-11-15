//! Build15751's bounded exact real representations. The cached decimal remains
//! available for display and for operations which require a decimal result.
use super::*;
use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer;
use num_traits::{One, ToPrimitive, Zero};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NumericVersion {
    #[default]
    Legacy15157,
    Current15751,
}
impl NumericVersion {
    pub fn code(self) -> i64 {
        match self {
            Self::Legacy15157 => 15157,
            Self::Current15751 => 15751,
        }
    }
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            15157 => Some(Self::Legacy15157),
            15751 => Some(Self::Current15751),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RealOrigin {
    #[default]
    LegacyBcd,
    Decimal,
    Rational {
        numerator: u64,
        denominator: u64,
    },
    PiMultiple {
        numerator: u64,
        denominator: u64,
    },
}

#[derive(Clone, Copy)]
pub(super) enum Binary {
    Add,
    Mul,
    Div,
}

impl Real {
    /// Typed CAS coefficients contain integer text, never executable source.
    pub fn from_fraction_strings(
        numerator: &str,
        denominator: &str,
        version: NumericVersion,
    ) -> Option<Self> {
        if version == NumericVersion::Legacy15157 {
            return Self::parse(numerator)?.div(Self::parse(denominator)?).ok();
        }
        let (n, d) = (
            BigInt::parse_bytes(numerator.as_bytes(), 10)?,
            BigInt::parse_bytes(denominator.as_bytes(), 10)?,
        );
        Self::from_ratio_big(
            n.sign() != d.sign(),
            n.magnitude().clone(),
            d.magnitude().clone(),
            false,
        )
        .ok()
    }
    /// Executable source for recalling a real without losing its arithmetic
    /// representation. Numeric `approx` is evaluated locally by the VM.
    pub fn ppl_source(self) -> String {
        match self.origin {
            RealOrigin::LegacyBcd => self.to_string(),
            RealOrigin::Decimal => format!("approx({self})"),
            RealOrigin::Rational {
                numerator,
                denominator,
            } => self.exact_decimal().unwrap_or_else(|| {
                format!(
                    "({}{numerator}/{denominator})",
                    if self.neg { "-" } else { "" }
                )
            }),
            RealOrigin::PiMultiple {
                numerator,
                denominator,
            } => {
                let sign = if self.neg { "-" } else { "" };
                if numerator == 1 && denominator == 1 {
                    format!("{sign}π")
                } else {
                    format!("({sign}{numerator}*π/{denominator})")
                }
            }
        }
    }
    pub(super) fn origin_pow(self, exponent: Self) -> Option<Result<Self, MathError>> {
        if self.origin == RealOrigin::LegacyBcd && exponent.origin == RealOrigin::LegacyBcd {
            return None;
        }
        if let Some(power) = exponent.to_i64() {
            if let Some(result) = self.origin_powi(power) {
                return Some(result);
            }
        }
        if matches!(
            exponent.origin,
            RealOrigin::Rational {
                numerator: 1,
                denominator: 2
            }
        ) && !self.neg
        {
            match self.origin {
                RealOrigin::Rational {
                    numerator,
                    denominator,
                } => {
                    if exponent.neg {
                        if numerator == 0 {
                            return Some(Err(MathError::DivisionByZero));
                        }
                        return Some(
                            Self::from_ratio_big(
                                false,
                                denominator.into(),
                                numerator.into(),
                                false,
                            )
                            .and_then(Self::sqrt),
                        );
                    }
                    return self.origin_sqrt();
                }
                RealOrigin::PiMultiple {
                    numerator,
                    denominator,
                } => {
                    return Some(pi_sqrt_ratio_decimal(
                        &numerator.into(),
                        &denominator.into(),
                        exponent.neg,
                    ));
                }
                _ => {}
            }
        }
        if let (
            RealOrigin::Rational {
                numerator: n,
                denominator: d,
            },
            RealOrigin::Rational {
                numerator: p,
                denominator: q,
            },
        ) = (self.origin, exponent.origin)
        {
            if n == 0 {
                return Some(if exponent.is_zero() {
                    Err(MathError::Undefined)
                } else if exponent.neg {
                    Err(MathError::DivisionByZero)
                } else {
                    Self::exact_rational(false, 0, 1)
                });
            }
            if n == d && !self.neg {
                return Some(Self::exact_rational(false, 1, 1));
            }
            if !self.neg && q <= 64 {
                let (n, d) = (BigUint::from(n), BigUint::from(d));
                let (rn, rd) = (n.nth_root(q as u32), d.nth_root(q as u32));
                if rn.pow(q as u32) == n && rd.pow(q as u32) == d && p <= i64::MAX as u64 {
                    let root =
                        Self::from_ratio_big(false, rn, rd, false).expect("bounded rational root");
                    return Some(root.powi(if exponent.neg { -(p as i64) } else { p as i64 }));
                }
            }
        }
        Some(self.bcd().pow(exponent.bcd()).map(Self::decimal))
    }
    /// Lossless terminating decimal for an exact rational; irrational and
    /// nonterminating values keep their structural representation instead.
    pub fn exact_decimal(self) -> Option<String> {
        let RealOrigin::Rational {
            numerator,
            denominator,
        } = self.origin
        else {
            return None;
        };
        let (n, d) = (BigUint::from(numerator), BigUint::from(denominator));
        let (whole, mut remainder) = n.div_rem(&d);
        let mut text = format!("{}{}", if self.neg { "-" } else { "" }, whole);
        if !remainder.is_zero() {
            text.push('.');
        }
        for _ in 0..64 {
            if remainder.is_zero() {
                return Some(canonical_decimal_text(&text));
            }
            remainder *= 10u8;
            let (digit, rest) = remainder.div_rem(&d);
            text.push(char::from(b'0' + digit.to_u8().unwrap()));
            remainder = rest;
        }
        remainder.is_zero().then(|| canonical_decimal_text(&text))
    }
    pub(super) fn origin_integer(self, mode: i8) -> Option<Self> {
        if self.origin == RealOrigin::LegacyBcd {
            return None;
        }
        if self.origin == RealOrigin::Decimal {
            let value = match mode {
                -1 => self.bcd().floor(),
                1 => self.bcd().ceil(),
                _ => self.bcd().trunc(),
            };
            return Some(value.decimal());
        }
        let (n, d, pi) = self.coefficient().unwrap();
        let (mut whole, fractional) = if pi {
            (pi_integer(&n, &d, false), true)
        } else {
            let (q, r) = n.div_rem(&d);
            (q, !r.is_zero())
        };
        if fractional && ((mode < 0 && self.neg) || (mode > 0 && !self.neg)) {
            whole += 1u8;
        }
        Some(
            Self::from_ratio_big(self.neg, whole, BigUint::one(), false)
                .expect("bounded integer part"),
        )
    }
    pub(super) fn origin_round(self, places: i32, round: bool) -> Option<Self> {
        if self.origin == RealOrigin::LegacyBcd {
            return None;
        }
        if self.origin == RealOrigin::Decimal {
            let value = if round {
                self.bcd().round_places(places)
            } else {
                self.bcd().truncate_places(places)
            };
            return Some(value.decimal());
        }
        let (mut n, mut d, pi) = self.coefficient().unwrap();
        let exponent = if places >= 0 {
            -i64::from(places)
        } else {
            i64::from(self.exp10()) + i64::from(places) + 1
        };
        if exponent < -1024 {
            return Some(self.decimal());
        }
        if exponent > 1024 {
            return Some(Self::ZERO.decimal());
        }
        let scale = BigUint::from(10u8).pow(exponent.unsigned_abs() as u32);
        if exponent < 0 {
            n *= &scale;
        } else {
            d *= &scale;
        }
        let whole = if pi {
            pi_integer(&n, &d, round)
        } else {
            let (mut q, r) = n.div_rem(&d);
            if round && r * 2u8 >= d {
                q += 1u8;
            }
            q
        };
        let value = if exponent < 0 {
            Self::from_ratio_big(self.neg, whole, scale, false)
        } else {
            Self::from_ratio_big(self.neg, whole * scale, BigUint::one(), false)
        }
        .expect("bounded rounded value");
        Some(if pi { value.decimal() } else { value })
    }
    pub(super) fn origin_sqrt(self) -> Option<Result<Self, MathError>> {
        if self.origin == RealOrigin::LegacyBcd {
            return None;
        }
        if self.neg && !self.is_zero() {
            return Some(Err(MathError::NonReal));
        }
        if let RealOrigin::Rational {
            numerator,
            denominator,
        } = self.origin
        {
            let (n, d) = (BigUint::from(numerator), BigUint::from(denominator));
            let (rn, rd) = (n.sqrt(), d.sqrt());
            if &rn * &rn == n && &rd * &rd == d {
                return Some(Self::from_ratio_big(false, rn, rd, false));
            }
            return Some(sqrt_ratio_decimal(&n, &d));
        }
        if let RealOrigin::PiMultiple {
            numerator,
            denominator,
        } = self.origin
        {
            return Some(pi_sqrt_ratio_decimal(
                &numerator.into(),
                &denominator.into(),
                false,
            ));
        }
        Some(self.bcd().sqrt().map(Self::decimal))
    }
    pub(super) fn origin_powi(self, power: i64) -> Option<Result<Self, MathError>> {
        if self.origin == RealOrigin::LegacyBcd {
            return None;
        }
        if self.origin == RealOrigin::Decimal {
            return Some(self.bcd().powi(power).map(Self::decimal));
        }
        if power == 0 {
            return Some(if self.is_zero() {
                Err(MathError::Undefined)
            } else {
                Self::exact_rational(false, 1, 1)
            });
        }
        if power == 1 {
            return Some(Ok(self));
        }
        let (n, d, pi) = self.coefficient().unwrap();
        if n.is_zero() {
            return Some(if power < 0 {
                Err(MathError::DivisionByZero)
            } else {
                Self::exact_rational(false, 0, 1)
            });
        }
        let magnitude = power.unsigned_abs();
        if pi || magnitude > 4096 {
            return Some(self.bcd().powi(power).map(Self::decimal));
        }
        let (n, d) = (n.pow(magnitude as u32), d.pow(magnitude as u32));
        let (n, d) = if power < 0 { (d, n) } else { (n, d) };
        Some(Self::from_ratio_big(
            self.neg && magnitude % 2 == 1,
            n,
            d,
            false,
        ))
    }
    pub(super) fn origin_scale10(self, power: i32) -> Option<Result<Self, MathError>> {
        if self.origin == RealOrigin::LegacyBcd {
            return None;
        }
        if self.origin == RealOrigin::Decimal || !(-1024..=1024).contains(&power) {
            return Some(self.bcd().scale10(power).map(Self::decimal));
        }
        let (mut n, mut d, pi) = self.coefficient().unwrap();
        let scale = BigUint::from(10u8).pow(power.unsigned_abs());
        if power < 0 {
            d *= scale;
        } else {
            n *= scale;
        }
        Some(Self::from_ratio_big(self.neg, n, d, pi))
    }
    pub fn origin(self) -> RealOrigin {
        self.origin
    }
    /// Explicit decimal conversion drops exact provenance, retaining the
    /// actual twelve-digit value rather than guessing a rational from f64.
    pub fn decimal(self) -> Self {
        Self {
            origin: RealOrigin::Decimal,
            ..self
        }
    }
    pub(super) fn bcd(self) -> Self {
        Self {
            origin: RealOrigin::LegacyBcd,
            ..self
        }
    }

    pub fn parse_versioned(source: &str, version: NumericVersion) -> Option<Self> {
        let cached = Self::parse(source)?;
        if version == NumericVersion::Legacy15157 {
            return Some(cached);
        }
        let source = source.trim().trim_start_matches(['+', '-', '−']);
        let (body, power) = match source.find(['e', 'E', 'ᴇ']) {
            Some(index) => {
                let width = source[index..].chars().next()?.len_utf8();
                (&source[..index], parse_exponent(&source[index + width..])?)
            }
            None => (source, 0),
        };
        let fractional = body.find('.').map_or(0, |index| body.len() - index - 1);
        let mut digits: String = body.chars().filter(char::is_ascii_digit).collect();
        let zeros = digits.len() - digits.trim_end_matches('0').len();
        digits.truncate(digits.len() - zeros);
        let digits = digits.trim_start_matches('0');
        if digits.is_empty() {
            return Self::exact_rational(false, 0, 1).ok();
        }
        let exponent = power
            .checked_sub(fractional as i64)?
            .checked_add(zeros as i64)?;
        // A reduced finite decimal with a u64 denominator has at most63
        // decimal places. Avoid constructing enormous integers for literals
        // already known to require the decimal representation.
        if !(-63..=19).contains(&exponent) || digits.len() > 84 {
            return Some(cached.decimal());
        }
        let mut numerator = BigUint::parse_bytes(digits.as_bytes(), 10)?;
        let mut denominator = BigUint::one();
        if exponent >= 0 {
            numerator *= BigUint::from(10u8).pow(exponent as u32);
        } else {
            denominator = BigUint::from(10u8).pow((-exponent) as u32);
        }
        let negative = cached.neg;
        Self::from_ratio_big(negative, numerator, denominator, false).ok()
    }

    pub fn exact_rational(
        negative: bool,
        numerator: u64,
        denominator: u64,
    ) -> Result<Self, MathError> {
        Self::from_ratio_big(negative, numerator.into(), denominator.into(), false)
    }
    pub fn exact_pi_multiple(
        negative: bool,
        numerator: u64,
        denominator: u64,
    ) -> Result<Self, MathError> {
        Self::from_ratio_big(negative, numerator.into(), denominator.into(), true)
    }

    fn from_ratio_big(
        negative: bool,
        numerator: BigUint,
        denominator: BigUint,
        pi: bool,
    ) -> Result<Self, MathError> {
        if denominator.is_zero() {
            return Err(MathError::DivisionByZero);
        }
        if numerator.is_zero() {
            return Ok(Self {
                origin: RealOrigin::Rational {
                    numerator: 0,
                    denominator: 1,
                },
                ..Self::ZERO
            });
        }
        let gcd = numerator.gcd(&denominator);
        let n = numerator / &gcd;
        let d = denominator / gcd;
        let mut cached = if pi {
            pi_ratio_decimal(negative, &n, &d)?
        } else {
            ratio_decimal(negative, &n, &d)?
        };
        cached.origin = match (n.to_u64(), d.to_u64(), pi) {
            (Some(numerator), Some(denominator), false) => RealOrigin::Rational {
                numerator,
                denominator,
            },
            (Some(numerator), Some(denominator), true) => RealOrigin::PiMultiple {
                numerator,
                denominator,
            },
            _ => RealOrigin::Decimal,
        };
        Ok(cached)
    }

    fn decimal_ratio(self) -> (BigUint, BigUint) {
        let mut n = BigUint::from(self.mant);
        let mut d = BigUint::one();
        if self.exp >= 0 {
            n *= BigUint::from(10u8).pow(self.exp as u32);
        } else {
            d = BigUint::from(10u8).pow(self.exp.unsigned_abs());
        }
        (n, d)
    }
    fn coefficient(self) -> Option<(BigUint, BigUint, bool)> {
        match self.origin {
            RealOrigin::Rational {
                numerator,
                denominator,
            } => Some((numerator.into(), denominator.into(), false)),
            RealOrigin::PiMultiple {
                numerator,
                denominator,
            } => Some((numerator.into(), denominator.into(), true)),
            RealOrigin::LegacyBcd => {
                let (n, d) = self.decimal_ratio();
                let gcd = n.gcd(&d);
                let (n, d) = (n / &gcd, d / gcd);
                (n.to_u64().is_some() && d.to_u64().is_some()).then_some((n, d, false))
            }
            RealOrigin::Decimal => None,
        }
    }

    fn decimal_binary(self, other: Self, op: Binary) -> Result<Self, MathError> {
        let (a, b) = (self.bcd(), other.bcd());
        match op {
            Binary::Add => a.add(b),
            Binary::Mul => a.mul(b),
            Binary::Div => a.div(b),
        }
        .map(Self::decimal)
    }
    pub(super) fn origin_binary(self, other: Self, op: Binary) -> Option<Result<Self, MathError>> {
        if self.origin == RealOrigin::LegacyBcd && other.origin == RealOrigin::LegacyBcd {
            return None;
        }
        let Some((a, b, ap)) = self.coefficient() else {
            return Some(self.decimal_binary(other, op));
        };
        let Some((c, d, cp)) = other.coefficient() else {
            return Some(self.decimal_binary(other, op));
        };
        let result = match op {
            Binary::Add => {
                if ap != cp && !a.is_zero() && !c.is_zero() {
                    return Some(self.decimal_binary(other, op));
                }
                let sign = |negative| if negative { Sign::Minus } else { Sign::Plus };
                let n = BigInt::from_biguint(sign(self.neg), &a * &d)
                    + BigInt::from_biguint(sign(other.neg), &c * &b);
                Self::from_ratio_big(
                    n.sign() == Sign::Minus,
                    n.magnitude().clone(),
                    b * d,
                    ap || cp,
                )
            }
            Binary::Mul => {
                if ap && cp && !a.is_zero() && !c.is_zero() {
                    return Some(self.decimal_binary(other, op));
                }
                Self::from_ratio_big(self.neg != other.neg, a * c, b * d, ap || cp)
            }
            Binary::Div => {
                if c.is_zero() {
                    return Some(Err(MathError::DivisionByZero));
                }
                if !ap && cp && !a.is_zero() {
                    return Some(self.decimal_binary(other, op));
                }
                Self::from_ratio_big(self.neg != other.neg, a * d, b * c, ap && !cp)
            }
        };
        Some(result)
    }

    pub(super) fn origin_cmp(self, other: Self) -> Option<Ordering> {
        if self.origin == RealOrigin::LegacyBcd && other.origin == RealOrigin::LegacyBcd {
            return None;
        }
        let cached = self.bcd().cmp_value(other.bcd());
        if cached != Ordering::Equal {
            return Some(cached);
        }
        let fraction = |v: Self| match v.origin {
            RealOrigin::Rational {
                numerator,
                denominator,
            } => (BigUint::from(numerator), BigUint::from(denominator), false),
            RealOrigin::PiMultiple {
                numerator,
                denominator,
            } => (BigUint::from(numerator), BigUint::from(denominator), true),
            _ => {
                let (n, d) = v.decimal_ratio();
                (n, d, false)
            }
        };
        let (a, b, ap) = fraction(self);
        let (c, d, cp) = fraction(other);
        let magnitude = if ap == cp {
            (a * d).cmp(&(c * b))
        } else if ap {
            compare_pi_ratio(&(a * d), &(c * b))
        } else {
            compare_pi_ratio(&(c * b), &(a * d)).reverse()
        };
        Some(if self.neg {
            magnitude.reverse()
        } else {
            magnitude
        })
    }
}

fn canonical_decimal_text(text: &str) -> String {
    let (sign, text) = text
        .strip_prefix('-')
        .map_or(("", text), |text| ("-", text));
    let point = text.find('.').unwrap_or(text.len());
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let leading_zeros = digits.len() - digits.trim_start_matches('0').len();
    if leading_zeros == digits.len() {
        return "0".into();
    }
    let exponent = point as i32 - leading_zeros as i32 - 1;
    if (-6..=15).contains(&exponent) {
        return format!("{sign}{text}");
    }
    let digits = digits.trim_start_matches('0').trim_end_matches('0');
    if digits.len() == 1 {
        format!("{sign}{digits}e{exponent}")
    } else {
        format!("{sign}{}.{}e{exponent}", &digits[..1], &digits[1..])
    }
}

/// Round an exact nonnegative rational square root once. A cached12-digit
/// input loses the guard information before Newton's method even begins.
/// Integer square-root division supplies one guard digit plus exact sticky
/// information to the same rounding boundary used by all other real results.
fn sqrt_ratio_decimal(n: &BigUint, d: &BigUint) -> Result<Real, MathError> {
    if n.is_zero() {
        return Ok(Real::ZERO.decimal());
    }
    let difference = n.to_str_radix(10).len() as i32 - d.to_str_radix(10).len() as i32;
    let mut exponent = difference.div_euclid(2);
    let squared_exponent = 2 * exponent;
    let below = if squared_exponent >= 0 {
        n < &(d * BigUint::from(10u8).pow(squared_exponent as u32))
    } else {
        n * BigUint::from(10u8).pow(squared_exponent.unsigned_abs()) < *d
    };
    if below {
        exponent -= 1;
    }
    let places = DIGITS as i32 - exponent;
    let scale = BigUint::from(10u8).pow(2 * places.unsigned_abs());
    let (numerator, denominator) = if places >= 0 {
        (n * scale, d.clone())
    } else {
        (n.clone(), d * scale)
    };
    let root = (&numerator / &denominator).sqrt();
    let sticky = &root * &root * &denominator != numerator;
    Real::from_parts_wide_sticky(
        false,
        root.to_u128().expect("13-digit square root"),
        -i64::from(places),
        sticky,
    )
    .map(Real::decimal)
}

fn ratio_decimal(negative: bool, n: &BigUint, d: &BigUint) -> Result<Real, MathError> {
    let (whole, mut remainder) = n.div_rem(d);
    let mut text = whole.to_str_radix(10);
    let mut significant = text.trim_start_matches('0').len();
    if negative && !n.is_zero() {
        text.insert(0, '-');
    }
    if !remainder.is_zero() {
        text.push('.');
    }
    while significant < 13 && !remainder.is_zero() {
        remainder *= 10u8;
        let (digit, rest) = remainder.div_rem(d);
        let digit = digit.to_u8().expect("decimal division digit");
        text.push(char::from(b'0' + digit));
        if significant != 0 || digit != 0 {
            significant += 1;
        }
        remainder = rest;
    }
    // The existing BCD parser needs only a guard digit and nonzero sticky tail.
    if !remainder.is_zero() {
        text.push('1');
    }
    match Real::parse(&text) {
        Some(value) => Ok(value.decimal()),
        None => crate::overflow_result(negative),
    }
}

struct PiBounds {
    lower: BigUint,
    upper: BigUint,
    scale: BigUint,
}
fn pi_bounds(precision: u32) -> PiBounds {
    // Machin's identity, with outward integer bounds on every truncated term:
    // https://personal.math.ubc.ca/~feldman/m121/pi.pdf
    let scale = BigUint::from(10u8).pow(precision);
    let atan = |divisor: u32| {
        let mut power = BigUint::from(divisor);
        let square = divisor * divisor;
        let mut sum = BigInt::zero();
        let mut terms = 0u32;
        loop {
            let term = &scale / (&power * (2 * terms + 1));
            if term.is_zero() {
                break;
            }
            let term = BigInt::from(term);
            if terms % 2 == 0 {
                sum += term;
            } else {
                sum -= term;
            }
            terms += 1;
            power *= square;
        }
        let error = BigInt::from(terms + 1);
        (
            (&sum - &error).to_biguint().unwrap(),
            (sum + error).to_biguint().unwrap(),
        )
    };
    let (al, au) = atan(5);
    let (bl, bu) = atan(239);
    PiBounds {
        lower: al * 16u8 - bu * 4u8,
        upper: au * 16u8 - bl * 4u8,
        scale,
    }
}
fn initial_pi() -> &'static PiBounds {
    static PI: OnceLock<PiBounds> = OnceLock::new();
    PI.get_or_init(|| pi_bounds(128))
}
fn compare_pi_ratio(a: &BigUint, b: &BigUint) -> Ordering {
    let compare = |bounds: &PiBounds| {
        let rhs = b * &bounds.scale;
        if a * &bounds.lower > rhs {
            Some(Ordering::Greater)
        } else if a * &bounds.upper < rhs {
            Some(Ordering::Less)
        } else {
            None
        }
    };
    if let Some(ordering) = compare(initial_pi()) {
        return ordering;
    }
    // Rational coefficients are finite and π is irrational: tightening the
    // proven interval eventually separates them, without an f64 tie fallback.
    let mut precision = 256;
    loop {
        if let Some(ordering) = compare(&pi_bounds(precision)) {
            return ordering;
        }
        precision *= 2;
    }
}
fn pi_ratio_decimal(negative: bool, n: &BigUint, d: &BigUint) -> Result<Real, MathError> {
    let round = |bounds: &PiBounds| {
        let denominator = d * &bounds.scale;
        let lower = ratio_decimal(negative, &(n * &bounds.lower), &denominator)?;
        let upper = ratio_decimal(negative, &(n * &bounds.upper), &denominator)?;
        Ok::<_, MathError>((lower, upper))
    };
    let (lower, upper) = round(initial_pi())?;
    if lower == upper {
        return Ok(lower);
    }
    let mut precision = 256;
    loop {
        let (lower, upper) = round(&pi_bounds(precision))?;
        if lower == upper {
            return Ok(lower);
        }
        precision *= 2;
    }
}

fn pi_sqrt_ratio_decimal(n: &BigUint, d: &BigUint, inverse: bool) -> Result<Real, MathError> {
    let round = |bounds: &PiBounds| {
        if inverse {
            Ok::<_, MathError>((
                sqrt_ratio_decimal(&(d * &bounds.scale), &(n * &bounds.upper))?,
                sqrt_ratio_decimal(&(d * &bounds.scale), &(n * &bounds.lower))?,
            ))
        } else {
            let denominator = d * &bounds.scale;
            Ok::<_, MathError>((
                sqrt_ratio_decimal(&(n * &bounds.lower), &denominator)?,
                sqrt_ratio_decimal(&(n * &bounds.upper), &denominator)?,
            ))
        }
    };
    let (lower, upper) = round(initial_pi())?;
    if lower == upper {
        return Ok(lower);
    }
    let mut precision = 256;
    loop {
        let (lower, upper) = round(&pi_bounds(precision))?;
        if lower == upper {
            return Ok(lower);
        }
        precision *= 2;
    }
}

fn pi_integer(n: &BigUint, d: &BigUint, round: bool) -> BigUint {
    let quantize = |bounds: &PiBounds| {
        let denominator = d * &bounds.scale;
        let q = |bound: &BigUint| {
            let (mut q, r) = (n * bound).div_rem(&denominator);
            if round && r * 2u8 >= denominator {
                q += 1u8;
            }
            q
        };
        (q(&bounds.lower), q(&bounds.upper))
    };
    let (lower, upper) = quantize(initial_pi());
    if lower == upper {
        return lower;
    }
    let mut precision = 256;
    loop {
        let (lower, upper) = quantize(&pi_bounds(precision));
        if lower == upper {
            return lower;
        }
        precision *= 2;
    }
}
