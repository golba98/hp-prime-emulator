//! Shared calculator angle mode for numeric computation and presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleMode {
    Radians,
    Degrees,
    Gradians,
}

impl AngleMode {
    /// `HAngle` codes: 0 radians (factory default), 1 degrees, 2 gradians.
    pub fn from_code(code: i64) -> Option<AngleMode> {
        match code {
            0 => Some(AngleMode::Radians),
            1 => Some(AngleMode::Degrees),
            2 => Some(AngleMode::Gradians),
            _ => None,
        }
    }

    pub fn code(self) -> i64 {
        match self {
            AngleMode::Radians => 0,
            AngleMode::Degrees => 1,
            AngleMode::Gradians => 2,
        }
    }

    /// Units in a full turn.
    pub fn full_turn(self) -> f64 {
        match self {
            AngleMode::Radians => std::f64::consts::TAU,
            AngleMode::Degrees => 360.0,
            AngleMode::Gradians => 400.0,
        }
    }
}
