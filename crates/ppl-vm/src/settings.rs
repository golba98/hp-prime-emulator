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
