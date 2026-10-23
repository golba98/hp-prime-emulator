//! HP Prime numeric model used by the PPL virtual machine.
//!
//! * [`Real`]: 12 significant decimal digits, exponent range ±499.
//! * [`HpInt`]: word-sized integers with a display base (`#FFh`).
//! * [`Complex`]: pairs of reals.

// Fallible arithmetic methods (add, sub, mul, div) return Result<..., MathError>
// to reflect calculator limits without panicking, so standard std::ops arithmetic
// traits are intentionally not used for binary operations.
#![allow(clippy::should_implement_trait)]

mod complex;
mod int;
mod real;

pub use complex::Complex;
pub use int::{DEFAULT_BITS, HpInt, IntBase};
pub use real::{DIGITS, FormatOptions, MAX_EXP10, MIN_EXP10, NumFormat, Real};

/// Numeric failures, mapped to PPL runtime errors by the VM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathError {
    Overflow,
    DivisionByZero,
    /// Result would be complex in a real-only context.
    NonReal,
    Undefined,
}

impl std::fmt::Display for MathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MathError::Overflow => "Overflow",
            MathError::DivisionByZero => "Division by 0",
            MathError::NonReal => "Non-real result",
            MathError::Undefined => "Undefined result",
        })
    }
}

impl std::error::Error for MathError {}
