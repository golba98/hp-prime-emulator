//! App view availability from HP Prime User Guide pp.65–69.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppView {
    Symbolic,
    Plot,
    Numeric,
    PlotSetup,
    NumericSetup,
}
pub fn supports(name: &str, view: AppView) -> bool {
    use AppView::*;
    match name {
        "Function" | "Advanced Graphing" | "Parametric" | "Polar" | "Sequence" => true,
        "Linear Explorer" | "Quadratic Explorer" | "Trig Explorer" => view == Plot,
        "Finance" => matches!(view, Plot | Numeric),
        "Linear Solver" | "Triangle Solver" | "Spreadsheet" => view == Numeric,
        "Geometry" | "Statistics 1Var" | "Statistics 2Var" => view != NumericSetup,
        "Inference" => matches!(view, Symbolic | Plot | Numeric),
        "Solve" => view != NumericSetup,
        _ => false,
    }
}
pub fn entry(name: &str) -> AppView {
    match name {
        "Linear Explorer" | "Quadratic Explorer" | "Trig Explorer" | "Geometry" => AppView::Plot,
        "Finance" | "Linear Solver" | "Triangle Solver" | "Spreadsheet" | "Statistics 1Var"
        | "Statistics 2Var" => AppView::Numeric,
        _ => AppView::Symbolic,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn documented_apps_do_not_offer_unrelated_views() {
        assert!(!supports("Finance", AppView::Symbolic));
        assert!(supports("Finance", AppView::Plot));
        assert!(!supports("Spreadsheet", AppView::Plot));
        assert_eq!(entry("Spreadsheet"), AppView::Numeric);
        assert!(!supports("Linear Explorer", AppView::Numeric));
        assert!(supports("Inference", AppView::Plot));
        assert!(supports("Geometry", AppView::Numeric));
    }
}
