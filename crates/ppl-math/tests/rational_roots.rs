use num_bigint::BigUint;
use ppl_math::{NumericVersion, Real, RealOrigin};

fn current(text: &str) -> Real {
    Real::parse_versioned(text, NumericVersion::Current15751).unwrap()
}

#[test]
fn original_exact_variance_and_decimal_variance_have_distinct_root_results() {
    let variance = current("14").div(current("9")).unwrap();
    let root = variance.sqrt().unwrap();
    assert_eq!(root.to_string(), "1.24721912892");
    assert_eq!(root.origin(), RealOrigin::Decimal);
    assert_eq!(
        variance.decimal().sqrt().unwrap().to_string(),
        "1.24721912893"
    );
    assert_eq!(
        variance.pow(current("0.5")).unwrap().to_string(),
        "1.24721912892"
    );
    assert_eq!(
        variance.pow(current("-0.5")).unwrap().to_string(),
        "0.801783725737"
    );
    let legacy = Real::parse("14")
        .unwrap()
        .div(Real::parse("9").unwrap())
        .unwrap();
    assert_eq!(legacy.sqrt().unwrap().to_string(), "1.24721912893");
    let exact = current("4").div(current("9")).unwrap().sqrt().unwrap();
    assert_eq!(
        exact.origin(),
        RealOrigin::Rational {
            numerator: 2,
            denominator: 3
        }
    );
    assert_eq!(exact.mul(current("3")).unwrap(), current("2"));
}

fn compare_scaled(a: BigUint, ae: i32, b: BigUint, be: i32) -> std::cmp::Ordering {
    let exp = ae.min(be);
    (a * BigUint::from(10u8).pow((ae - exp) as u32))
        .cmp(&(b * BigUint::from(10u8).pow((be - exp) as u32)))
}

#[test]
fn rational_root_rounding_is_proved_by_exact_midpoint_inequalities() {
    let mut state = 0xfedcba9876543210u64;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        state
    };
    let cases = [
        (1, u64::MAX),
        (u64::MAX, 1),
        (14, 9),
        (1, 3),
        (1, 41),
        (3, 23),
        (9999999999999999999, 1000000000000000000),
    ];
    for (n, d) in cases
        .into_iter()
        .chain((0..2000).map(|_| (next().max(1), next().max(1))))
    {
        let root = Real::exact_rational(false, n, d).unwrap().sqrt().unwrap();
        let digits = root.mantissa_digits();
        let pad = 12 - digits.len() as i32;
        let m = digits.parse::<u128>().unwrap() * 10u128.pow(pad as u32);
        let exp = root.raw_exp() - pad;
        let input = BigUint::from(n) * 4u8;
        let upper = BigUint::from((2 * m + 1).pow(2)) * BigUint::from(d);
        assert!(
            compare_scaled(input.clone(), 0, upper, 2 * exp).is_le(),
            "sqrt({n}/{d})={root}: upper midpoint"
        );
        let (lower_m, lower_exp) = if m == 100000000000 {
            (m * 10, exp - 1)
        } else {
            (m, exp)
        };
        let lower = BigUint::from((2 * lower_m - 1).pow(2)) * BigUint::from(d);
        assert!(
            compare_scaled(input, 0, lower, 2 * lower_exp).is_ge(),
            "sqrt({n}/{d})={root}: lower midpoint"
        );
    }
}
