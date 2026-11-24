//! Execution-scoped real overflow. Ordinary numeric construction/parsing stays
//! strict; the current G1 runtime saturates arithmetic at its signed maximum.
use crate::{MathError, Real};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverflowPolicy {
    #[default]
    Error,
    Saturate,
}
thread_local! {
    static OVERFLOW: Cell<OverflowPolicy> = const { Cell::new(OverflowPolicy::Error) };
}
pub fn with_overflow_policy<T>(policy: OverflowPolicy, operation: impl FnOnce() -> T) -> T {
    struct Restore(OverflowPolicy);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERFLOW.with(|current| current.set(self.0));
        }
    }
    let _restore = Restore(OVERFLOW.with(|current| current.replace(policy)));
    operation()
}
pub fn overflow_result(negative: bool) -> Result<Real, MathError> {
    OVERFLOW.with(|policy| match policy.get() {
        OverflowPolicy::Error => Err(MathError::Overflow),
        OverflowPolicy::Saturate => Ok(Real::max_finite(negative)),
    })
}
