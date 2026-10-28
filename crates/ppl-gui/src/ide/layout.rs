//! Pure, DPI-independent workspace constraints; preferred sizes are never clamped in storage.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WorkspaceLayout {
    #[default]
    Split,
    Editor,
    Calculator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CalculatorMode {
    #[default]
    Full,
    ScreenOnly,
}

pub const ACTIVITY_WIDTH: f32 = 44.0;
pub const EDITOR_MIN: f32 = 480.0;
pub const DEVICE_MIN_SCALE: f32 = 0.80;

pub fn valid_size(value: f32, default: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PanelLimits {
    pub explorer_max: f32,
    pub calculator_max: f32,
    pub explorer_visible: bool,
}

impl PanelLimits {
    pub fn for_width(width: f32, explorer: bool) -> Self {
        let available = (width - ACTIVITY_WIDTH).max(0.0);
        let explorer_visible = explorer && available >= EDITOR_MIN + 320.0 + 160.0;
        let explorer_max = (available - EDITOR_MIN - 320.0).clamp(160.0, 400.0);
        let calculator_max = (available - EDITOR_MIN - if explorer_visible { 160.0 } else { 0.0 })
            .clamp(240.0, 700.0);
        Self {
            explorer_max,
            calculator_max,
            explorer_visible,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn narrow_windows_reserve_editor_space() {
        let l = PanelLimits::for_width(900.0, true);
        assert!(!l.explorer_visible);
        assert!(l.calculator_max + EDITOR_MIN + ACTIVITY_WIDTH <= 900.0);
        assert!(PanelLimits::for_width(1920.0, true).explorer_visible);
    }
    #[test]
    fn preferences_reject_nonfinite_sizes() {
        assert_eq!(valid_size(f32::NAN, 400.0, 280.0, 900.0), 400.0);
        assert_eq!(valid_size(-1.0, 400.0, 280.0, 900.0), 280.0);
    }
}
