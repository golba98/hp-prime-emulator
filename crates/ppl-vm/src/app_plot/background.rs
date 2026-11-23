use crate::{ErrorKind, RtResult, RuntimeError, Value};
use ppl_math::Real;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Placement {
    #[default]
    None,
    Centered,
    Stretched,
    BestFit,
    Range,
}

impl Placement {
    pub const CHOICES: [Self; 5] = [
        Self::None,
        Self::Centered,
        Self::Stretched,
        Self::BestFit,
        Self::Range,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Opacity,
    XMin,
    XMax,
    YMin,
    YMax,
}

impl Field {
    fn bound(self) -> Option<usize> {
        Some(match self {
            Self::Opacity => return None,
            Self::XMin => 0,
            Self::XMax => 1,
            Self::YMin => 2,
            Self::YMax => 3,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Picture {
    BuiltIn(u8),
    AppFile(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Background {
    pub placement: Placement,
    pub picture: Option<Picture>,
    opacity: u8,
    bounds: [Real; 4],
}

impl Default for Background {
    fn default() -> Self {
        Self {
            placement: Placement::None,
            picture: None,
            opacity: 75,
            bounds: [
                Real::ZERO,
                Real::from_i64(30),
                Real::ZERO,
                Real::from_i64(20),
            ]
            .map(Real::decimal),
        }
    }
}

impl Background {
    pub const fn opacity(&self) -> u8 {
        self.opacity
    }

    pub const fn bounds(&self) -> [Real; 4] {
        self.bounds
    }

    pub fn value(&self, field: Field) -> Real {
        field.bound().map_or_else(
            || Real::from_i64(i64::from(self.opacity)),
            |index| self.bounds[index],
        )
    }

    pub fn set(&mut self, field: Field, value: &Value) -> RtResult<()> {
        let number = value.expect_real("Plot background value")?.decimal();
        if let Some(index) = field.bound() {
            self.bounds[index] = number;
            return Ok(());
        }
        self.opacity = number
            .to_i64()
            .and_then(|value| u8::try_from(value).ok())
            .filter(|&value| value <= 100)
            .ok_or_else(|| {
                RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "Image opacity must be between 0 and 100",
                )
            })?;
        Ok(())
    }
}
