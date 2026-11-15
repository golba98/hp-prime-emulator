//! STARTVIEW numbers follow each app's Views menu, not the Function menu.
//! HP Prime User Guide 813269-002, p.603; order cross-checked against the
//! legitimately installed G1 firmware menus (firmware provenance retained with test evidence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialView {
    PlotDetail,
    PlotTable,
    Autoscale,
    Decimal,
    Integer,
    Trig,
}
impl SpecialView {
    pub fn label(self) -> &'static str {
        match self {
            Self::PlotDetail => "Split Screen: Plot Detail",
            Self::PlotTable => "Split Screen: Plot Table",
            Self::Autoscale => "Autoscale",
            Self::Decimal => "Decimal",
            Self::Integer => "Integer",
            Self::Trig => "Trig",
        }
    }
}
pub fn views(app: &str) -> &'static [SpecialView] {
    use SpecialView::*;
    match app {
        "Function" | "Parametric" => &[PlotDetail, PlotTable, Autoscale, Decimal, Integer, Trig],
        "Sequence" => &[PlotDetail, PlotTable, Autoscale],
        "Polar" => &[PlotDetail, PlotTable, Autoscale, Decimal],
        "Advanced Graphing" => &[PlotDetail, Decimal, Integer, Trig],
        "Solve" => &[PlotDetail, Autoscale, Decimal, Integer, Trig],
        "Statistics 1Var" | "Statistics 2Var" => &[PlotDetail, Autoscale],
        _ => &[],
    }
}
pub fn view(app: &str, number: i64) -> Option<SpecialView> {
    number
        .checked_sub(8)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| views(app).get(i))
        .copied()
}
