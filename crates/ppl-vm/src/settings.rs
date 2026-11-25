//! Home settings that change how programs compute and display values.

use ppl_math::{FormatOptions, IntBase, NumFormat, Real};

pub use ppl_math::AngleMode;

/// STRING's fraction conversion changed between original G1 builds. This
/// selects formatting only; it does not claim newer exact arithmetic support.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StringFormatVersion {
    Legacy15157,
    #[default]
    Current15751,
}
impl StringFormatVersion {
    pub fn code(self) -> i64 {
        match self {
            Self::Legacy15157 => 15157,
            Self::Current15751 => 15751,
        }
    }
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            15157 => Some(Self::Legacy15157),
            15751 => Some(Self::Current15751),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub numeric_version: ppl_math::NumericVersion,
    pub angle: AngleMode,
    pub number_format: NumFormat,
    /// `HFormat` code as last assigned (kept for read-back).
    pub hformat: i64,
    pub hdigits: i64,
    pub string_format_version: StringFormatVersion,
    /// `HComplex`: allow complex results from real arguments.
    pub complex_results: bool,
    pub digit_grouping: ppl_math::DigitGrouping,
    pub complex_display: ppl_math::ComplexFormat,
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
            numeric_version: ppl_math::NumericVersion::Legacy15157,
            angle: AngleMode::Radians,
            number_format: NumFormat::Standard,
            hformat: 0,
            hdigits: 4,
            string_format_version: StringFormatVersion::default(),
            complex_results: false,
            digit_grouping: ppl_math::DigitGrouping::default(),
            complex_display: ppl_math::ComplexFormat::Algebraic,
            int_base: IntBase::Hex,
            int_bits: 32,
            int_signed: false,
            font_px: 14.0,
            exp_char: 'E',
        }
    }
}

impl Settings {
    pub fn set_integer_base(&mut self, code: i64) -> crate::RtResult<()> {
        self.int_base = match code {
            0 => IntBase::Bin,
            1 => IntBase::Oct,
            2 => IntBase::Dec,
            3 => IntBase::Hex,
            _ => {
                return Err(crate::RuntimeError::new(
                    crate::ErrorKind::InvalidInput,
                    "Base must be0 through3",
                ));
            }
        };
        Ok(())
    }
    pub fn set_integer_bits(&mut self, bits: i64) -> crate::RtResult<()> {
        if !(1..=64).contains(&bits) || (bits == 64 && self.int_signed) {
            return Err(crate::RuntimeError::new(
                crate::ErrorKind::InvalidInput,
                "Bits must be1..64 unsigned or1..63 signed",
            ));
        }
        self.int_bits = bits as u8;
        Ok(())
    }
    pub fn set_integer_signed(&mut self, signed: bool) -> crate::RtResult<()> {
        if signed && self.int_bits == 64 {
            return Err(crate::RuntimeError::new(
                crate::ErrorKind::InvalidInput,
                "Signed words support at most63 magnitude bits",
            ));
        }
        self.int_signed = signed;
        Ok(())
    }
    pub fn integer_width_code(&self) -> i64 {
        if self.int_signed {
            -(self.int_bits as i64)
        } else {
            self.int_bits as i64
        }
    }
    pub fn format(&self) -> FormatOptions {
        FormatOptions {
            format: self.number_format,
            exp_char: self.exp_char,
            angle: self.angle,
            complex_format: self.complex_display,
            int_bits: self.int_bits,
            int_signed: self.int_signed,
            int_base: self.int_base,
            ..Default::default()
        }
    }

    /// Home paint formatting; canonical source/transport continues to use
    /// `format()`. This prevents digit separators from becoming argument syntax.
    pub fn home_format(&self) -> FormatOptions {
        FormatOptions {
            digit_grouping: Some(self.digit_grouping),
            complex_format: self.complex_display,
            exp_char: if self.exp_char == 'E' {
                'ᴇ'
            } else {
                self.exp_char
            },
            ..self.format()
        }
    }
    pub(crate) fn set_angle_code(&mut self, code: i64) -> crate::RtResult<()> {
        self.angle = AngleMode::from_code(code).ok_or_else(|| {
            crate::RuntimeError::new(crate::ErrorKind::BadArgValue, "HAngle must be 0, 1 or 2")
        })?;
        Ok(())
    }

    pub fn set_separator(&mut self, code: i64) -> crate::RtResult<()> {
        self.digit_grouping = ppl_math::DigitGrouping::from_code(code).ok_or_else(|| {
            crate::RuntimeError::new(
                crate::ErrorKind::InvalidInput,
                "HSeparator must be0 through10",
            )
        })?;
        Ok(())
    }
    pub fn complex_code(&self) -> i64 {
        i64::from(self.complex_results) + 10 * self.complex_display.code()
    }
    pub fn set_complex_code(&mut self, code: i64) -> crate::RtResult<()> {
        if !matches!(code, 0 | 1 | 10 | 11) {
            return Err(crate::RuntimeError::new(
                crate::ErrorKind::InvalidInput,
                "HComplex must be0,1,10 or11",
            ));
        }
        self.complex_results = code % 10 == 1;
        self.complex_display = ppl_math::ComplexFormat::from_code(code / 10).unwrap();
        Ok(())
    }
    pub fn try_set_format(&mut self, format: i64, digits: i64) -> crate::RtResult<()> {
        if !(0..=5).contains(&format) || !(0..=11).contains(&digits) {
            return Err(crate::RuntimeError::new(
                crate::ErrorKind::InvalidInput,
                "HFormat must be0 through5; HDigits must be0 through11",
            ));
        }
        self.set_format(format, digits);
        Ok(())
    }
    /// Applies `HFormat` / `HDigits`: 0 Standard, 1 Fixed, 2 Scientific,
    /// 3 Engineering, 4 Floating and 5 Rounded.
    pub fn set_format(&mut self, hformat: i64, digits: i64) {
        self.hformat = hformat;
        self.hdigits = digits.clamp(0, 11);
        let d = self.hdigits as u8;
        self.number_format = match hformat {
            1 => NumFormat::Fixed(d),
            2 => NumFormat::Scientific(d),
            3 => NumFormat::Engineering(d),
            4 => NumFormat::Floating(d),
            5 => NumFormat::Rounded(d),
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
/// pages 637–638, extended by controls from original G1 build15751. Zero means
/// System for angle, complex and format; ADigits is
/// independent and used when an app selects its own numeric format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSettings {
    /// 0 System, 1 Radians, 2 Degrees, 3 Gradians.
    pub angle: i64,
    /// 0 System, 1 ON, 2 OFF.
    pub complex: i64,
    /// 0 System, 1 Standard, 2 Fixed, 3 Scientific, 4 Engineering, 5 Floating, 6 Rounded.
    pub format: i64,
    /// ADigits: build15751 accepts integer codes 0 through 11.
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
        if !(0..=6).contains(&self.format) {
            return Err("AFormat must be 0 through 6".into());
        }
        if !(0..=11).contains(&self.digits) {
            return Err("ADigits must be between 0 and 11".into());
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
        if (1..=6).contains(&self.format) {
            effective.set_format(self.format - 1, self.digits);
        }
        effective
    }

    /// Arithmetic follows the active app even from Home or stored PPL. The
    /// installed G1's AComplex codes have the inverse computational effect of
    /// the labels shown in Symbolic Setup; keep that measured behavior separate
    /// from presentation and the stored setting code.
    pub fn arithmetic(&self, home: &Settings) -> Settings {
        let mut settings = self.effective(home);
        settings.complex_results = match self.complex {
            1 => false,
            2 => true,
            _ => home.complex_results,
        };
        settings
    }
}
