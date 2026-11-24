//! Exact positive integer-base powers entered directly in CAS view.

use ppl_cas::CasValue;
use ppl_syntax::ast::{BinOp, Expr, ExprKind, UnOp};

pub(super) fn positive_integer_power(expr: &Expr) -> Option<CasValue> {
    let ExprKind::Binary(BinOp::Pow, base, exponent) = &expr.kind else {
        return None;
    };
    let ExprKind::Number(base) = &base.kind else {
        return None;
    };
    let base = base.parse::<u128>().ok().filter(|value| *value > 0)?;
    let (inverse, numerator, denominator) = fractional_exponent(exponent)?;
    let common = gcd(numerator, denominator);
    let degree = denominator / common;
    if degree <= 1 {
        return None;
    }
    let magnitude = exact_root(base, degree)?.checked_pow(numerator / common)?;
    if inverse && magnitude != 1 {
        Some(CasValue::Rational {
            numerator: "1".into(),
            denominator: magnitude.to_string(),
        })
    } else {
        Some(CasValue::Integer(magnitude.to_string()))
    }
}

fn fractional_exponent(expr: &Expr) -> Option<(bool, u32, u32)> {
    fn sign(expr: &Expr) -> (bool, &Expr) {
        match &expr.kind {
            ExprKind::Unary(UnOp::Neg, operand) => (true, operand),
            _ => (false, expr),
        }
    }
    let (outer_negative, expr) = sign(expr);
    let ExprKind::Binary(BinOp::Div, numerator, denominator) = &expr.kind else {
        return None;
    };
    let (numerator_negative, numerator) = sign(numerator);
    let (ExprKind::Number(numerator), ExprKind::Number(denominator)) =
        (&numerator.kind, &denominator.kind)
    else {
        return None;
    };
    let numerator = numerator.parse::<u32>().ok().filter(|value| *value > 0)?;
    let denominator = denominator.parse::<u32>().ok().filter(|value| *value > 0)?;
    Some((outer_negative != numerator_negative, numerator, denominator))
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn exact_root(value: u128, degree: u32) -> Option<u128> {
    use std::cmp::Ordering;

    if value == 1 {
        return Some(1);
    }
    let bits = u128::BITS - value.leading_zeros();
    if degree >= bits {
        return None;
    }
    let mut low = 2;
    let mut high = 1u128 << bits.div_ceil(degree);
    while low <= high {
        let middle = low + (high - low) / 2;
        match middle.checked_pow(degree).map(|power| power.cmp(&value)) {
            Some(Ordering::Less) => low = middle + 1,
            Some(Ordering::Equal) => return Some(middle),
            Some(Ordering::Greater) | None => high = middle - 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prove(source: &str) -> Option<CasValue> {
        let (expr, diagnostics) = ppl_syntax::parse_expression(source);
        assert!(!diagnostics.has_errors(), "{source}: {diagnostics:?}");
        positive_integer_power(&expr)
    }

    #[test]
    fn proves_the_original_direct_integer_base_values() {
        for (source, expected) in [
            ("(81)^((3)/(4))", "27"),
            ("16^(3/4)", "8"),
            ("125^(2/3)", "25"),
            ("81^(1/4)", "3"),
        ] {
            assert_eq!(prove(source), Some(CasValue::Integer(expected.into())));
        }
        assert_eq!(
            prove("81^(-3/4)"),
            Some(CasValue::Rational {
                numerator: "1".into(),
                denominator: "27".into(),
            })
        );
    }

    #[test]
    fn leaves_other_domains_and_contexts_to_the_helper() {
        for source in [
            "(16/81)^(3/4)",
            "(9/16)^(1/2)",
            "2^(3/4)",
            "(-8)^(1/3)",
            "(-81)^(3/4)",
            "(1+i)^2",
            "0^(3/4)",
            "x^(3/4)",
            "2+81^(3/4)",
            "'81^(3/4)'",
            "A:=81^(3/4)",
            "81^(1000/4)",
            "81^(1001/4)",
        ] {
            assert_eq!(prove(source), None, "{source}");
        }
    }
}
