//! Native graphics commands must finish even when most geometry is offscreen.
mod common;
use common::{Run, text};

#[test]
fn circles_with_extreme_finite_geometry_complete_and_preserve_visible_pixels() {
    let mut run = Run::new(
        "circle clipping",
        r#"
EXPORT CLIPPEDCIRCLES()
BEGIN
  DIMGROB_P(G1,8,8,RGB(255,255,255));
  FILLCIRCLE_P(G1,0,0,1E18,0);
  LOCAL filled:=GETPIX_P(G1,7,7);
  CIRCLE_P(G1,0,0,1E18,RGB(255,255,255));
  LOCAL outside:=GETPIX_P(G1,0,0);
  CIRCLE_P(G1,1E18,0,1E18,RGB(255,255,255));
  RETURN {filled,outside,GETPIX_P(G1,0,0)};
END;
"#,
    );
    let value = run.call("CLIPPEDCIRCLES", &[]).unwrap();
    assert_eq!(text(&value), "{#0h,#0h,#FFFFFFh}");
}
