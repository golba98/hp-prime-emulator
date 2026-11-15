mod common;
use common::Run;
use ppl_vm::Value;

#[test]
fn rgb_transparency_is_continuous_rather_than_an_alpha_threshold_flag() {
    // Actual G1 build15751 RGBFLAG_PROBE returns these booleans. The old User
    // Guide's 'A>128 is transparent' sentence contradicts observed firmware.
    let mut run = Run::new(
        "RGBTRANS",
        r#"EXPORT MAIN() BEGIN
      DIMGROB_P(G1,8,4,RGB(0,0,255));
      PIXON_P(G1,0,0,RGB(255,0,0,0));
      PIXON_P(G1,1,0,RGB(255,0,0,64));
      PIXON_P(G1,2,0,RGB(255,0,0,128));
      PIXON_P(G1,3,0,RGB(255,0,0,129));
      PIXON_P(G1,4,0,RGB(255,0,0,255));
      RETURN {GETPIX_P(G1,0,0)==RGB(255,0,0),GETPIX_P(G1,1,0)==RGB(255,0,0),
        GETPIX_P(G1,2,0)==RGB(255,0,0),GETPIX_P(G1,3,0)==RGB(0,0,255),
        GETPIX_P(G1,4,0)==RGB(0,0,255)};
    END;"#,
    );
    let Value::List(values) = run.call("MAIN", &[]).unwrap() else {
        panic!("expected boolean observations");
    };
    assert_eq!(
        values
            .iter()
            .map(|value| value.expect_i64("transparency observation").unwrap())
            .collect::<Vec<_>>(),
        [1, 0, 0, 0, 1]
    );
}
