//! Persisted worksheet formatting applies to bounded rectangular selections.
use super::{MAX_COLUMNS, MAX_ROWS};
use serde::{Deserialize, Serialize};
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
}
impl CellFormat {
    pub fn validate(&self) -> Result<(), String> {
        if self.number.is_some_and(|v| !(-1..=5).contains(&v))
            || self.digits.is_some_and(|v| !(-1..=11).contains(&v))
            || self
                .font
                .is_some_and(|v| v != 0 && (!(10..=22).contains(&v) || v % 2 != 0))
            || self.color.is_some_and(|v| v > 0x1000000)
            || self.fill.is_some_and(|v| v > 0x1000000)
            || self.horizontal.is_some_and(|v| v > 3)
            || self.vertical.is_some_and(|v| v > 3)
            || self.quotes.is_some_and(|v| v > 2)
        {
            return Err("Invalid spreadsheet format".into());
        }
        Ok(())
    }
    pub fn overlay(&mut self, other: &Self) {
        macro_rules! field { ($($f:ident),*) => { $(if other.$f.is_some() { self.$f=other.$f; })* }; }
        field!(
            number, digits, font, color, fill, horizontal, vertical, quotes
        );
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatRule {
    pub block: Block,
    pub style: CellFormat,
}
