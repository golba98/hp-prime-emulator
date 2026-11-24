//! Legacy G1 fraction selection and scalar formatting.
use ppl_math::{NumFormat, Real};

// The fraction routine's scalar fallback trims insignificant fractional zeros.
pub(super) fn decimal_places(value: Real, places: u8, trim: bool) -> String {
    let rounded = value.round_places(i32::from(places));
    let digits = format!(
        "{}{}",
        rounded.mantissa_digits(),
        "0".repeat((rounded.raw_exp() + i32::from(places)).max(0) as usize)
    );
    let padded = format!("{:0>width$}", digits, width = usize::from(places) + 1);
    let split = padded.len() - usize::from(places);
    let mut out = format!(
        "{}{}.{}",
        if rounded.is_negative() { "-" } else { "" },
        &padded[..split],
        &padded[split..]
    );
    while trim && out.ends_with('0') {
        out.pop();
    }
    if trim && out.ends_with('.') {
        out.pop();
    }
    out
}

/// Legacy G1 (15157) fraction conversion uses the same BCD arithmetic as
/// Native execution. Search convergents, then the first admissible intermediate
/// coefficient. An f64 approximation with a decimal-place epsilon changes both
/// the stopping point and the selected denominator.
pub(super) fn fraction(value: Real, format: NumFormat) -> Option<(u128, u128)> {
    let x = value.abs();
    if x.is_zero() {
        return None;
    }
    let e = x.exp10();
    let power = match format {
        NumFormat::Standard if (-11..11).contains(&e) => (e - 10).max(-11),
        NumFormat::Fixed(p) | NumFormat::Rounded(p) if e >= -i32::from(p) => {
            if e >= 0 {
                -i32::from(p).min(11 - e)
            } else {
                -i32::from(p)
            }
        }
        NumFormat::Floating(p) if e >= -i32::from(p) => {
            if e >= 0 {
                e - i32::from(p) + 1
            } else {
                -i32::from(p)
            }
        }
        NumFormat::Scientific(p) if p > 0 => -i32::from(p) - 1,
        NumFormat::Engineering(p) if p > 0 => -i32::from(p) - e.rem_euclid(3) - 1,
        _ => return None,
    };
    if !(-11..9).contains(&e) {
        return None;
    }
    // The original approximation routine interprets tolerances >=1 as a
    // significant-digit count. Preserve that distinction, including precision0.
    let exponent = if power >= 0 {
        if power >= 3 {
            -499
        } else {
            (e - 10i32.pow(power as u32) + 1).max(-499)
        }
    } else {
        power.max(-499)
    };
    let tolerance = Real::from_parts(false, 1, exponent).ok()?;
    let coefficient = x.mantissa_digits().parse::<u128>().ok()?;
    let (mut n, mut d) = if x.raw_exp() < 0 {
        (coefficient, 10u128.checked_pow(x.raw_exp().unsigned_abs())?)
    } else {
        (
            coefficient.checked_mul(10u128.checked_pow(x.raw_exp() as u32)?)?,
            1,
        )
    };
    let (mut p0, mut p1, mut q0, mut q1) = (0u128, 1u128, 1u128, 0u128);
    for _ in 0..128 {
        let a = n / d;
        let candidate = |k: u128| -> Option<(u128, u128)> {
            Some((
                k.checked_mul(p1)?.checked_add(p0)?,
                k.checked_mul(q1)?.checked_add(q0)?,
            ))
        };
        let (p, q) = candidate(a)?;
        let remainder = n % d;
        let fits =
            |p, q| -> Option<bool> { Some(x.sub(ratio_real(p, q)?).ok()?.abs() < tolerance) };
        if fits(p, q)? || remainder == 0 {
            let (mut low, mut high) = (1, a.max(1));
            while high - low > 1 {
                let mid = low + (high - low) / 2;
                let (p, q) = candidate(mid)?;
                if fits(p, q)? {
                    high = mid;
                } else {
                    low = mid;
                }
            }
            return if a > 1 { candidate(high) } else { Some((p, q)) };
        }
        (p0, p1, q0, q1) = (p1, p, q1, q);
        (n, d) = (d, remainder);
    }
    None
}

/// Divide the wider integer search coefficients before rounding their quotient
/// to the calculator's 12-digit comparison precision. Rounding each integer
/// first loses near-integer fractions. The search range bounds both operands
/// below10^23; long division needs at most35 digits including leading zeros.
fn ratio_real(n: u128, d: u128) -> Option<Real> {
    if d == 0 {
        return None;
    }
    let (mut coefficient, mut remainder) = (n / d, n % d);
    let mut exponent = 0;
    let mut digits = if coefficient == 0 {
        0
    } else {
        coefficient.ilog10() + 1
    };
    while digits < 13 && remainder != 0 {
        remainder = remainder.checked_mul(10)?;
        coefficient = coefficient.checked_mul(10)?.checked_add(remainder / d)?;
        remainder %= d;
        exponent -= 1;
        if coefficient != 0 {
            digits += 1;
        }
    }
    // Retain a sticky tail at halfway; Real then applies its existing single
    // ties-to-even rounding rather than introducing a second division rounding.
    if digits == 13 && coefficient % 10 == 5 && remainder != 0 {
        coefficient += 1;
    }
    Real::from_parts(false, coefficient, exponent).ok()
}

// Existing newer-build component style. Exact rational provenance remains
// a separate compatibility requirement; this does not reconstruct it.
pub(super) fn current_fraction(value: Real, precision: u8) -> Option<(u128, u128)> {
    let x = value.abs().to_f64();
    if !x.is_finite() || x > 1e15 {
        return None;
    }
    let tolerance = 10f64.powi(-(precision.max(1) as i32));
    let mut y = x;
    let (mut h0, mut h1, mut k0, mut k1) = (0u64, 1u64, 1u64, 0u64);
    let mut result = None;
    for _ in 0..48 {
        let a = y.floor() as u64;
        let h = a.checked_mul(h1)?.checked_add(h0)?;
        let k = a.checked_mul(k1)?.checked_add(k0)?;
        if k > 1_000_000_000_000 {
            break;
        }
        result = Some((h, k));
        if (h as f64 / k as f64 - x).abs() <= tolerance {
            break;
        }
        let remainder = y - a as f64;
        if remainder == 0.0 {
            break;
        }
        (h0, h1, k0, k1) = (h1, h, k1, k);
        y = 1.0 / remainder;
    }
    result.map(|(p, q)| (u128::from(p), u128::from(q)))
}
