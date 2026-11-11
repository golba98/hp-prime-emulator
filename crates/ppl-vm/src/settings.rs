//! Home settings that change how programs compute and display values.

use ppl_math::{FormatOptions, IntBase, NumFormat, Real};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleMode {
    Radians,
    Degrees,
    Gradians,
}

impl AngleMode {
    /// `HAngle` codes: 0 radians (factory default), 1 degrees, 2 gradians.
    /// Source: calculator help text as quoted on the HP Museum forum;
    /// **unverified** on the G1.
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

#[derive(Clone, Debug)]
pub struct Settings {
    pub angle: AngleMode,
    pub number_format: NumFormat,
    /// `HFormat` code as last assigned (kept for read-back).
    pub hformat: i64,
    pub hdigits: i64,
    /// `HComplex`: allow complex results from real arguments.
    pub complex_results: bool,
    pub int_base: IntBase,
    pub int_bits: u8,
    pub int_signed: bool,
    /// Pixel size of the system font (TEXTOUT font 0).
    pub font_px: f32,
    pub exp_char: char,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            angle: AngleMode::Radians,
            number_format: NumFormat::Standard,
            hformat: 0,
            hdigits: 4,
            complex_results: false,
            int_base: IntBase::Hex,
            int_bits: 32,
            int_signed: false,
            font_px: ppl_graphics::text::DEFAULT_PX,
            exp_char: 'E',
        }
    }
}

impl Settings {
    pub fn format(&self) -> FormatOptions {
        FormatOptions {
            format: self.number_format,
            exp_char: self.exp_char,
        }
    }

    /// Applies `HFormat` / `HDigits`: 0 Standard, 1 Fixed, 2 Scientific,
    /// 3 Engineering (4 Floating and 5 Rounded are mapped to Standard and
    /// are approximations).
    pub fn set_format(&mut self, hformat: i64, digits: i64) {
        self.hformat = hformat;
        self.hdigits = digits.clamp(0, 11);
        let d = self.hdigits as u8;
        self.number_format = match hformat {
            1 => NumFormat::Fixed(d),
            2 => NumFormat::Scientific(d),
            3 => NumFormat::Engineering(d),
            _ => NumFormat::Standard,
        };
    }

    pub fn to_radians(&self, x: Real) -> f64 {
        let v = x.to_f64();
        match self.angle {
            AngleMode::Radians => v,
            AngleMode::Degrees => v.to_radians(),
            AngleMode::Gradians => v * std::f64::consts::PI / 200.0,
        }
    }

    pub fn from_radians(&self, r: f64) -> f64 {
        match self.angle {
            AngleMode::Radians => r,
            AngleMode::Degrees => r.to_degrees(),
            AngleMode::Gradians => r * 200.0 / std::f64::consts::PI,
        }
    }
}

/// Per-application Symbolic Setup overrides. HP Prime User Guide, chapter 28,
/// pages 637–638. Zero means System for angle, complex and format; ADigits is
/// independent and used when an app selects its own numeric format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSettings {
    /// 0 System, 1 Radians, 2 Degrees, 3 Gradians.
    pub angle: i64,
    /// 0 System, 1 ON, 2 OFF.
    pub complex: i64,
    /// 0 System, 1 Standard, 2 Fixed, 3 Scientific, 4 Engineering.
    pub format: i64,
    /// ADigits: documented integer range 0 < n < 11.
    pub digits: i64,
}
impl Default for AppSettings {
    fn default() -> Self {
        Self {
            angle: 0,
            complex: 0,
            format: 0,
            digits: 4,
        }
    }
}
impl AppSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(0..=3).contains(&self.angle) {
            return Err("AAngle must be 0, 1, 2 or 3".into());
        }
        if !(0..=2).contains(&self.complex) {
            return Err("AComplex must be 0, 1 or 2".into());
        }
        if !(0..=4).contains(&self.format) {
            return Err("AFormat must be 0, 1, 2, 3 or 4".into());
        }
        if !(1..=10).contains(&self.digits) {
            return Err("ADigits must be between 1 and 10".into());
        }
        Ok(())
    }
    pub(crate) fn set_code(&mut self, name: &str, code: i64) -> crate::RtResult<()> {
        let mut next = self.clone();
        match name {
            "aangle" => next.angle = code,
            "acomplex" => next.complex = code,
            "aformat" => next.format = code,
            "adigits" => next.digits = code,
            _ => {
                return Err(crate::RuntimeError::new(
                    crate::ErrorKind::BadArgValue,
                    "Unknown app setting",
                ));
            }
        }
        next.validate()
            .map_err(|error| crate::RuntimeError::new(crate::ErrorKind::BadArgValue, error))?;
        *self = next;
        Ok(())
    }
    /// A graph/numeric worker receives this clone; Home memory is unchanged.
    pub fn effective(&self, home: &Settings) -> Settings {
        let mut effective = home.clone();
        effective.angle = match self.angle {
            1 => AngleMode::Radians,
            2 => AngleMode::Degrees,
            3 => AngleMode::Gradians,
            _ => home.angle,
        };
        effective.complex_results = match self.complex {
            1 => true,
            2 => false,
            _ => home.complex_results,
        };
        if (1..=4).contains(&self.format) {
            effective.set_format(self.format - 1, self.digits);
        }
        effective
    }
}
