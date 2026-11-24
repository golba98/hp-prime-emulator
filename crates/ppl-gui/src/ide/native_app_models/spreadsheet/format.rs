//! Persisted worksheet formatting applies to bounded rectangular selections.
use super::{MAX_COLUMNS, MAX_ROWS};
use serde::{Deserialize, Serialize};

/// Calculator Auto color. Other high bytes carry transparency, including the
/// legitimate black 0x01000000 which older Native profiles used as Auto.
pub const AUTO_COLOR: u32 = u32::MAX;
pub(super) const CURRENT_ENCODING: u8 = 1;
pub(super) const LEGACY_AUTO_COLOR: u32 = 0x01000000;
pub fn canonical_color(value: u32) -> u32 {
    if value >> 24 == 0xff {
        AUTO_COLOR
    } else {
        value
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub left: usize,
    pub right: usize,
    pub top: usize,
    pub bottom: usize,
}
impl Block {
    pub fn cell(col: usize, row: usize) -> Self {
        Self {
            left: col,
            right: col,
            top: row,
            bottom: row,
        }
    }
    /// Defaults belong to sheet, rows, then columns; bounded selections are
    /// explicit cell formatting and take precedence over all defaults.
    pub(super) fn scope(&self) -> usize {
        match (
            self.left == 1 && self.right == MAX_COLUMNS,
            self.top == 1 && self.bottom == MAX_ROWS,
        ) {
            (true, true) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 3,
        }
    }
    pub fn contains(&self, col: usize, row: usize) -> bool {
        (self.left..=self.right).contains(&col) && (self.top..=self.bottom).contains(&row)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.left == 0
            || self.top == 0
            || self.left > self.right
            || self.top > self.bottom
            || self.right > MAX_COLUMNS
            || self.bottom > MAX_ROWS
        {
            return Err("Invalid spreadsheet selection".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CellFormat {
    pub number: Option<i64>,
    pub digits: Option<i64>,
    pub font: Option<u8>,
    pub color: Option<u32>,
    pub fill: Option<u32>,
    pub horizontal: Option<u8>,
    pub vertical: Option<u8>,
    pub quotes: Option<u8>,
    /// 0 Auto, 1 Yes (two dimensional objects), 2 No (algebraic).
    pub textbook: Option<u8>,
}
impl CellFormat {
    pub(super) fn decoded(&self, encoding: u8) -> Self {
        let mut result = self.clone();
        for color in [&mut result.color, &mut result.fill] {
            *color = color.map(|value| {
                if encoding == 0 && value == LEGACY_AUTO_COLOR {
                    AUTO_COLOR
                } else {
                    canonical_color(value)
                }
            });
        }
        result
    }
    pub(super) fn defaults() -> Self {
        Self {
            number: Some(-1),
            digits: Some(-1),
            font: Some(0),
            color: Some(0),
            fill: Some(0xffffff),
            horizontal: Some(3),
            vertical: Some(3),
            quotes: Some(2),
            textbook: Some(2),
        }
    }
    pub(super) fn overlay_explicit(&mut self, other: &Self) {
        let mut explicit = other.clone();
        explicit.number = explicit.number.filter(|&v| v != -1);
        explicit.digits = explicit.digits.filter(|&v| v != -1);
        explicit.font = explicit.font.filter(|&v| v != 0);
        explicit.color = explicit.color.filter(|&v| v != AUTO_COLOR);
        explicit.fill = explicit.fill.filter(|&v| v != AUTO_COLOR);
        explicit.horizontal = explicit.horizontal.filter(|&v| v != 3);
        explicit.vertical = explicit.vertical.filter(|&v| v != 3);
        explicit.quotes = explicit.quotes.filter(|&v| v != 0);
        explicit.textbook = explicit.textbook.filter(|&v| v != 0);
        self.overlay(&explicit);
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.number.is_some_and(|v| !(-1..=5).contains(&v))
            || self.digits.is_some_and(|v| !(-1..=11).contains(&v))
            || self
                .font
                .is_some_and(|v| v != 0 && (!(10..=22).contains(&v) || v % 2 != 0))
            || self.horizontal.is_some_and(|v| v > 3)
            || self.vertical.is_some_and(|v| v > 3)
            || self.quotes.is_some_and(|v| v > 2)
            || self.textbook.is_some_and(|v| v > 2)
        {
            return Err("Invalid spreadsheet format".into());
        }
        Ok(())
    }
    pub fn overlay(&mut self, other: &Self) {
        macro_rules! field { ($($f:ident),*) => { $(if other.$f.is_some() { self.$f=other.$f; })* }; }
        field!(
            number, digits, font, color, fill, horizontal, vertical, quotes, textbook
        );
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatRule {
    pub block: Block,
    pub style: CellFormat,
}
