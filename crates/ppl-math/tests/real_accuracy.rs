//! Accuracy tests for `Real` at the edges of its 12-digit, ±499 range.
//!
//! Expected vectors were computed independently with Python's `decimal`
//! module (50-digit precision, then `ROUND_HALF_UP` to 12 digits), e.g.
//! `Context(prec=12, rounding=ROUND_HALF_UP).plus(Decimal('5.12345678901').sqrt())`.

use std::cmp::Ordering;

use ppl_math::{FormatOptions, MathError, NumFormat, Real};

fn r(s: &str) -> Real {
    Real::parse(s).unwrap_or_else(|| panic!("literal {s} should parse"))
}

/// `(coefficient, exponent)` with the coefficient as a decimal string.
fn parts(x: Real) -> (u128, i32) {
    (x.mantissa_digits().parse().unwrap(), x.raw_exp())
}

/// Compares `a × 10^ea` with `b × 10^eb` exactly, using decimal strings so
/// nothing can overflow.
fn cmp_scaled(a: u128, ea: i32, b: u128, eb: i32) -> Ordering {
    let e = ea.min(eb);
    let mut sa = a.to_string();
    let mut sb = b.to_string();
    sa.push_str(&"0".repeat((ea - e) as usize));
    sb.push_str(&"0".repeat((eb - e) as usize));
    let sa = sa.trim_start_matches('0');
    let sb = sb.trim_start_matches('0');
    sa.len().cmp(&sb.len()).then_with(|| sa.cmp(sb))
}

/// Asserts `root` is `sqrt(x)` correctly rounded to 12 significant digits:
/// `(root - ½ulp)² <= x <= (root + ½ulp)²`, checked with exact integers.
fn assert_correct_sqrt(x: Real, root: Real) {
    if x.is_zero() {
        assert!(root.is_zero());
        return;
    }
    let (xm, xe) = parts(x);
    let (rm, re) = parts(root);
    // Spread the root's coefficient to exactly 12 digits.
    let digits = rm.to_string().len() as i32;
    let pad = 12 - digits;
    let r12 = rm * 10u128.pow(pad as u32);
    let ulp_exp = re - pad; // root = r12 × 10^ulp_exp
    // x × 4 against (2·r12 ± 1)² × 10^(2·ulp_exp).
    let x4 = xm * 4;
    let hi = (2 * r12 + 1) * (2 * r12 + 1);
    assert_ne!(
        cmp_scaled(x4, xe, hi, 2 * ulp_exp),
        Ordering::Greater,
        "sqrt({x}) = {root} is too small"
    );
    // Below a power of ten the spacing of 12-digit values is ten times finer.
    let (lo_m, lo_e) = if r12 == 100_000_000_000 {
        (r12 * 10, ulp_exp - 1)
    } else {
        (r12, ulp_exp)
    };
    let lo = (2 * lo_m - 1) * (2 * lo_m - 1);
    assert_ne!(
        cmp_scaled(lo, 2 * lo_e, x4, xe),
        Ordering::Greater,
        "sqrt({x}) = {root} is too large"
    );
}

#[test]
fn sqrt_fixed_vectors() {
    let cases = [
        ("2", "1.41421356237"),
        ("3", "1.73205080757"),
        ("10", "3.16227766017"),
        ("0.5", "0.707106781187"),
        // Odd exponent with a large coefficient: overflowed u128 before.
        ("5.12345678901", "2.26350542058"),
        ("9999999.99999", "3162.27766017"),
        ("999999999999", "999999.999999"),
        ("123456789012", "351364.182882"),
        ("1.23456789012E-301", "3.51364182882E-151"),
        ("7.7777777777E77", "8.81917103684E38"),
        ("1E-499", "3.16227766017E-250"),
        ("1E-498", "1E-249"),
        ("1E499", "3.16227766017E249"),
        ("9.99999999999E499", "9.99999999999E249"),
        ("0", "0"),
        ("144", "12"),
        ("1.44", "1.2"),
        ("1E-498", "1E-249"),
        ("16", "4"),
    ];
    for (x, want) in cases {
        let got = r(x).sqrt().unwrap();
        assert_eq!(got.to_string(), want, "sqrt({x})");
        assert_correct_sqrt(r(x), got);
    }
}

#[test]
fn sqrt_negative_is_non_real() {
    assert_eq!(r("-4").sqrt(), Err(MathError::NonReal));
}

#[test]
fn sqrt_sweep_is_correctly_rounded() {
    // Deterministic LCG over 12-digit coefficients and both exponent
    // parities across the whole range.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 11
    };
    let mut checked = 0;
    for exp in (-499..=499).step_by(7) {
        for _ in 0..40 {
            let digits = 1 + (next() % 12) as u32;
            let lo = 10u64.pow(digits - 1);
            let mant = lo + next() % (lo * 9);
            let x = r(&format!("{mant}E{}", exp - digits as i32 + 1));
            let root = x.sqrt().unwrap();
            assert_correct_sqrt(x, root);
            checked += 1;
        }
    }
    // Coefficients right below a power of ten and the largest 12-digit ones.
    for exp in [-499, -250, -1, 0, 1, 2, 3, 250, 498, 499] {
        for mant in [
            999_999_999_999u64,
            999_999_999_998,
            100_000_000_001,
            341_000_000_000,
            512_345_678_901,
        ] {
            let x = r(&format!("{mant}E{}", exp - 11));
            assert_correct_sqrt(x, x.sqrt().unwrap());
            checked += 1;
        }
    }
    assert!(checked > 5_000);
}

#[test]
fn sqrt_of_perfect_squares_is_exact() {
    for n in 1u64..2_000 {
        let sq = Real::from_u64(n * n);
        assert_eq!(sq.sqrt().unwrap(), Real::from_u64(n), "sqrt({})", n * n);
    }
    // 999999^2 = 999998000001 (12 digits).
    assert_eq!(r("999998000001").sqrt().unwrap().to_string(), "999999");
    assert_eq!(r("0.0144").sqrt().unwrap().to_string(), "0.12");
}

// ---------------------------------------------------------------------------
// Basic operations at the extremes.
// ---------------------------------------------------------------------------

#[test]
fn multiply_at_range_edges() {
    // 9.99999999999E499 × 9.99999999999 overflows.
    assert_eq!(
        r("9.99999999999E499").mul(r("9.99999999999")),
        Err(MathError::Overflow)
    );
    // Largest coefficient squared: 999999999999² = 999999999998000000000001.
    assert_eq!(
        r("999999999999")
            .mul(r("999999999999"))
            .unwrap()
            .to_string(),
        "9.99999999998E23"
    );
    assert_eq!(
        r("9.99999999999E249")
            .mul(r("9.99999999999E249"))
            .unwrap()
            .to_string(),
        "9.99999999998E499"
    );
    // 1E-250 × 1E-250 = 1E-500 underflows to zero.
    assert!(r("1E-250").mul(r("1E-250")).unwrap().is_zero());
    assert_eq!(r("1E-250").mul(r("1E-249")).unwrap().to_string(), "1E-499");
    // Rounding carries into a new digit.
    assert_eq!(
        r("9.99999999999")
            .mul(r("1.00000000001"))
            .unwrap()
            .to_string(),
        "10.0000000001"
    );
    assert_eq!(
        r("9.99999999999")
            .mul(r("1.00000000002"))
            .unwrap()
            .to_string(),
        "10.0000000002"
    );
}

#[test]
fn divide_with_extreme_coefficients() {
    // Python decimal reference values.
    assert_eq!(r("1").div(r("999999999999")).unwrap(), r("1E-12"));
    assert_eq!(
        r("999999999999").div(r("1")).unwrap().to_string(),
        "999999999999"
    );
    assert_eq!(
        r("999999999999").div(r("7")).unwrap().to_string(),
        "142857142857"
    );
    assert_eq!(r("1").div(r("7")).unwrap().to_string(), "0.142857142857");
    assert_eq!(
        r("123456789012")
            .div(r("0.000000000003"))
            .unwrap()
            .to_string(),
        "4.1152263004E22"
    );
    assert_eq!(r("1E499").div(r("1E-499")), Err(MathError::Overflow));
    assert!(r("1E-499").div(r("1E499")).unwrap().is_zero());
    assert_eq!(r("5E-499").div(r("5")).unwrap().to_string(), "1E-499");
}

#[test]
fn add_and_subtract_cancellation() {
    assert_eq!(
        r("1").sub(r("0.000000000001")).unwrap().to_string(),
        "0.999999999999"
    );
    assert_eq!(r("1").sub(r("1E-13")).unwrap().to_string(), "1");
    assert_eq!(r("1").sub(r("5E-13")).unwrap().to_string(), "1");
    assert_eq!(
        r("1").sub(r("6E-13")).unwrap().to_string(),
        "0.999999999999"
    );
    assert_eq!(
        r("9.99999999999E499").add(r("9.99999999999E499")),
        Err(MathError::Overflow)
    );
    assert_eq!(
        r("123456789012")
            .sub(r("123456789011"))
            .unwrap()
            .to_string(),
        "1"
    );
    // 9E-500 is below the range and flushes to zero as a literal.
    assert!(r("9E-500").is_zero());
    assert_eq!(r("1E-499").sub(r("9E-500")).unwrap().to_string(), "1E-499");
    assert!(r("2E-499").sub(r("1.5E-499")).unwrap().is_zero());
}

#[test]
fn literals_with_more_than_twelve_digits() {
    assert_eq!(r("1234567890125").to_string(), "1.23456789012E12");
    assert_eq!(r("1234567890124999").to_string(), "1.23456789012E15");
    assert_eq!(r("0.1234567890125").to_string(), "0.123456789012");
    assert_eq!(r("9.999999999995").to_string(), "10");
    assert_eq!(r("9.999999999994999").to_string(), "9.99999999999");
    assert!(Real::parse("9.999999999995E499").is_none());
    assert_eq!(r("-0.000").to_string(), "0");
}

#[test]
fn literals_with_extreme_exponents_do_not_panic() {
    // Exponents far outside ±499 must overflow (None) or flush to zero,
    // never wrap the i32 exponent.
    assert!(Real::parse("1E2147483647").is_none());
    assert!(Real::parse("10E2147483647").is_none());
    assert!(Real::parse("1234567890123E2147483647").is_none());
    assert!(Real::parse("1E99999999999").is_none());
    assert!(r("1E-2147483648").is_zero());
    assert!(r("1234567890123E-2147483648").is_zero());
    assert!(r("0.001E-2147483646").is_zero());
}

#[test]
fn round_with_negative_and_extreme_places() {
    assert_eq!(r("123456").round_places(-2).to_string(), "120000");
    assert_eq!(r("-123456").round_places(-3).to_string(), "-123000");
    assert_eq!(r("0.00123456").round_places(-2).to_string(), "0.0012");
    assert_eq!(r("2.5").round_places(0).to_string(), "3");
    assert_eq!(r("0.5").round_places(0).to_string(), "1");
    assert_eq!(r("999999999999").round_places(-1).to_string(), "1E12");
    assert_eq!(
        r("1.23456789012").round_places(20).to_string(),
        "1.23456789012"
    );
    // Extreme place counts must not overflow the exponent arithmetic.
    assert_eq!(r("0.01").round_places(i32::MIN).to_string(), "0.01");
    assert_eq!(r("0.01").round_places(i32::MAX).to_string(), "0.01");
    assert_eq!(r("123").round_places(i32::MIN).to_string(), "123");
    assert_eq!(r("0.01").truncate_places(i32::MIN).to_string(), "0.01");
    assert_eq!(r("0.01").truncate_places(i32::MAX).to_string(), "0.01");
    assert!(r("1E-300").round_places(5).is_zero());
}

#[test]
fn integer_powers() {
    // 2.2E-250 ^ -2 = 2.06611570248E499: x² underflows but the reciprocal
    // is representable.
    assert_eq!(
        r("2.2E-250").powi(-2).unwrap().to_string(),
        "2.06611570248E499"
    );
    assert_eq!(r("1E-300").powi(-2), Err(MathError::Overflow));
    assert_eq!(r("1E300").powi(2), Err(MathError::Overflow));
    assert!(r("1E300").powi(-2).unwrap().is_zero());
    assert_eq!(r("2").powi(-1).unwrap().to_string(), "0.5");
    // Negative powers are rounded once (Python decimal reference values);
    // rounding x^n to 12 digits before taking the reciprocal gave
    // 0.914339824244, 8.77278002592E-17 and 0.190879420786.
    assert_eq!(r("1.01").powi(-9).unwrap().to_string(), "0.91433982424");
    assert_eq!(r("7").powi(-19).unwrap().to_string(), "8.77278002594E-17");
    assert_eq!(r("1.23").powi(-8).unwrap().to_string(), "0.190879420787");
    assert_eq!(
        r("123.456").powi(-3).unwrap().to_string(),
        "5.31451203798E-7"
    );
    assert_eq!(r("-2").powi(-3).unwrap().to_string(), "-0.125");
    assert_eq!(r("10").powi(-499).unwrap().to_string(), "1E-499");
    assert!(r("10").powi(-500).unwrap().is_zero());
    assert_eq!(r("-2").powi(3).unwrap().to_string(), "-8");
    // Huge exponents must not overflow the digit-count estimate.
    assert_eq!(
        r("999").powi(9_000_000_000_000_000_000),
        Err(MathError::Overflow)
    );
    assert!(
        r("0.999")
            .powi(9_000_000_000_000_000_000)
            .unwrap()
            .is_zero()
    );
    assert_eq!(r("1").powi(i64::MAX).unwrap().to_string(), "1");
    assert_eq!(r("-1").powi(i64::MAX).unwrap().to_string(), "-1");
}

#[test]
fn negative_base_with_huge_even_integer_exponent() {
    // Integers beyond i64 are even (coefficient × 10^k, k ≥ 7), so the
    // result is real.
    assert_eq!(r("-1").pow(r("1E20")).unwrap().to_string(), "1");
    assert_eq!(r("-2").pow(r("1E20")), Err(MathError::Overflow));
    assert!(r("-0.5").pow(r("1E20")).unwrap().is_zero());
    // Non-integers stay non-real.
    assert_eq!(r("-2").pow(r("0.5")), Err(MathError::NonReal));
}

#[test]
fn display_formats_at_extremes() {
    let o = |f| FormatOptions {
        format: f,
        exp_char: 'E',
    };
    assert_eq!(r("9.99999999999E499").to_string(), "9.99999999999E499");
    assert_eq!(r("1E-499").to_string(), "1E-499");
    assert_eq!(r("-1.5E-499").to_string(), "-1.5E-499");
    assert_eq!(
        r("123456789012").format(o(NumFormat::Fixed(11))),
        "123456789012.00000000000"
    );
    assert_eq!(
        r("0.999999999999").format(o(NumFormat::Fixed(11))),
        "1.00000000000"
    );
    assert_eq!(
        r("9.99999999999E499").format(o(NumFormat::Fixed(2))),
        "9.99999999999E499"
    );
    // Rounding the displayed digits may carry past the largest exponent.
    assert_eq!(
        r("9.99999999999E499").format(o(NumFormat::Scientific(3))),
        "1.000E500"
    );
    assert_eq!(
        r("9.99999999999E499").format(o(NumFormat::Engineering(2))),
        "100.00E498"
    );
    assert_eq!(r("1E-499").format(o(NumFormat::Fixed(4))), "0.0000");
    // Wide Fixed widths (not reachable through HFormat, which clamps to
    // 11) must not overflow or index past the power table.
    assert_eq!(
        r("123456789012").format(o(NumFormat::Fixed(40))),
        format!("123456789012.{}", "0".repeat(40))
    );
    assert_eq!(
        r("1.5").format(o(NumFormat::Fixed(60))),
        format!("1.5{}", "0".repeat(59))
    );
}

#[test]
fn from_f64_rounds_the_exact_binary_value() {
    // The double nearest 2.552272097695 is 2.55227209769499996383..., which
    // `{:.16e}` prints as 2.5522720976950000 — a false tie. The correctly
    // rounded 12-digit value is 2.55227209769.
    let cases = [
        (2.552_272_097_695_f64, "2.55227209769"),
        (5.658_909_859_025, "5.65890985902"),
        (1.159_964_182_105, "1.1599641821"),
        (-6.959_036_012_965, "-6.95903601296"),
        // The double nearest 1.0000000000005 is 1.00000000000050004...
        (1.000_000_000_000_5, "1.00000000000"),
        // These values are exactly retained at twelve significant digits.
        (1.25, "1.25"),
        (0.000_000_000_000_5, "5E-13"),
    ];
    for (v, want) in cases {
        let want = Real::parse(want).unwrap().to_string();
        assert_eq!(Real::from_f64(v).unwrap().to_string(), want, "{v:e}");
    }
}

#[test]
fn g1_15157_stored_literal_and_addition_rounding() {
    // Physical controls: ZZT_CD534F, tag 0bec061e, HAngle/HFormat/HDigits
    // 0|0|0; stored-value controls ZZT_241F07, tag af198c96. The latter
    // distinguish value equality from a displayed-string coincidence.
    for (input, expected) in [
        ("1.234567890125", "1.23456789012"),
        ("1.234567890135", "1.23456789014"),
        ("-1.234567890125", "-1.23456789012"),
        ("-1.234567890135", "-1.23456789014"),
        ("1234567890125", "1.23456789012E12"),
        ("1.234567890125E20", "1.23456789012E20"),
        ("1.2345678901251", "1.23456789013"),
        ("1.2345678901249", "1.23456789012"),
        ("1.234567890124999999", "1.23456789012"),
    ] {
        assert_eq!(r(input).to_string(), expected, "{input}");
    }
    let low = r("1.23456789012");
    assert_eq!(r("1.234567890125"), low);
    assert_eq!(low.add(r("5E-12")).unwrap(), low);
    assert_eq!(low.neg().sub(r("5E-12")).unwrap(), low.neg());
    assert_eq!(r("1.234567890125").mantissa_digits(), "123456789012");
    assert_eq!(r("1.234567890125").raw_exp(), -11);
    // Explicit ROUND is separate from rounding a stored twelve-digit real.
    assert_eq!(r("2.5").round_places(0), r("3"));
    assert_eq!(r("-2.5").round_places(0), r("-3"));
}

#[test]
fn nearest_even_keeps_nonzero_tails_beyond_the_guard_digit() {
    assert_eq!(r("1.23456789012500000000000000000000"), r("1.23456789012"));
    assert_eq!(r("1.23456789012500000000000000000001"), r("1.23456789013"));
    assert_eq!(
        r("-1.23456789012500000000000000000001"),
        r("-1.23456789013")
    );
    assert_eq!(r("1.23456789013500000000000000000000"), r("1.23456789014"));
    // The truncated quotient is exactly a halfway coefficient with even
    // retained digit, but the exact rational has a nonzero remainder above
    // that boundary: 10^25 = 13135513548850*d + 159691933950.
    let divisor = r("761294940073");
    assert_eq!(r("1").div(divisor).unwrap(), r("1.31355135489E-12"));
    assert_eq!(divisor.powi(-1).unwrap(), r("1.31355135489E-12"));
    // Exact arithmetic ties use the same rounder, including both parities.
    assert_eq!(
        r("246913578025").mul(r("5")).unwrap(),
        r("1.23456789012E12")
    );
    assert_eq!(
        r("1.23456789013").add(r("5E-12")).unwrap(),
        r("1.23456789014")
    );
}

#[test]
fn binary_conversion_distinguishes_exact_ties_and_their_neighbours() {
    let tie = 1_234_567_890_125_f64;
    assert_eq!(Real::from_f64(tie).unwrap(), r("1.23456789012E12"));
    assert_eq!(Real::from_f64(-tie).unwrap(), r("-1.23456789012E12"));
    assert_eq!(
        Real::from_f64(1_234_567_890_135_f64).unwrap(),
        r("1.23456789014E12")
    );
    assert_eq!(
        Real::from_f64(f64::from_bits(tie.to_bits() - 1)).unwrap(),
        r("1.23456789012E12")
    );
    assert_eq!(
        Real::from_f64(f64::from_bits(tie.to_bits() + 1)).unwrap(),
        r("1.23456789013E12")
    );
}

// ---------------------------------------------------------------------------
// Complex numbers near the range limits.
// ---------------------------------------------------------------------------

fn c(re: &str, im: &str) -> ppl_math::Complex {
    ppl_math::Complex::new(r(re), r(im))
}

#[test]
fn complex_abs_does_not_overflow_or_flush() {
    assert_eq!(c("3", "4").abs().unwrap().to_string(), "5");
    assert_eq!(c("3E300", "4E300").abs().unwrap().to_string(), "5E300");
    assert_eq!(c("3E-300", "4E-300").abs().unwrap().to_string(), "5E-300");
    assert_eq!(c("1E-499", "0").abs().unwrap().to_string(), "1E-499");
    assert_eq!(c("0", "0").abs().unwrap().to_string(), "0");
    assert_eq!(c("9E499", "9E499").abs(), Err(MathError::Overflow));
    // Unchanged digits in the ordinary range: sqrt(1.1² + 2.2²).
    assert_eq!(c("1.1", "2.2").abs().unwrap().to_string(), "2.45967477525");
}

#[test]
fn complex_division_with_tiny_and_huge_divisors() {
    let q = c("1", "1").div(c("1E-300", "1E-300")).unwrap();
    assert_eq!(q.re.to_string(), "1E300");
    assert!(q.im.is_zero());
    let q = c("2E300", "0").div(c("0", "1E300")).unwrap();
    assert!(q.re.is_zero());
    assert_eq!(q.im.to_string(), "-2");
    let q = c("1E499", "0").div(c("5", "0")).unwrap();
    assert_eq!(q.re.to_string(), "2E498");
    // Ordinary range: (1+2i)/(3+4i) = 0.44 + 0.08i.
    let q = c("1", "2").div(c("3", "4")).unwrap();
    assert_eq!(
        (q.re.to_string(), q.im.to_string()),
        ("0.44".into(), "0.08".into())
    );
    assert_eq!(c("1", "1").div(c("0", "0")), Err(MathError::DivisionByZero));
}

#[test]
fn complex_sqrt_outside_f64_range() {
    // sqrt(-4E400) = 2E200 i.
    let s = c("-4E400", "0").sqrt().unwrap();
    assert!(s.re.is_zero());
    assert_eq!(s.im.to_string(), "2E200");
    let s = c("0", "2E-400").sqrt().unwrap();
    assert_eq!(
        (s.re.to_string(), s.im.to_string()),
        ("1E-200".into(), "1E-200".into())
    );
    let s = c("-4", "0").sqrt().unwrap();
    assert_eq!(s.im.to_string(), "2");
}
