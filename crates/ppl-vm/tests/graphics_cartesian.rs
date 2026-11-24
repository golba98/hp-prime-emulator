mod common;
use common::Run;
use ppl_graphics::WHITE;
use ppl_vm::ErrorKind;

fn drawing(body: &str) -> Run {
    let mut run = Run::new(
        "CARTESIAN",
        &format!("EXPORT MAIN() BEGIN Xmin:=0;Xmax:=10;Ymin:=0;Ymax:=10;{body} RETURN 0;END;"),
    );
    run.call("MAIN", &[]).unwrap();
    run
}
#[test]
fn firmware_plot_projection_is_fixed_independent_of_target_size() {
    let run = drawing(
        "DIMGROB_P(G1,641,481); PIXON(G1,0,10,#00FF00h); PIXON(G1,5,5,#FF0000h); PIXON(G1,10,0,#0000FFh); PIXON(G1,2.5,7.5,#FF00FFh); PIXON(G1,7.5,2.5,#FFFF00h);",
    );
    let g = run.machine.grob(1).unwrap();
    for (x, y, color) in [
        (1, 0, 0x00FF00),
        (160, 109, 0xFF0000),
        (319, 218, 0x0000FF),
        (81, 55, 0xFF00FF),
        (240, 164, 0xFFFF00),
    ] {
        assert_eq!(g.get(x, y), Some(color), "({x},{y})");
    }
    assert_eq!(g.get(320, 240), Some(WHITE));
    assert_eq!(g.get(160, 120), Some(WHITE));
}
#[test]
fn arc_uses_cartesian_center_and_pixel_radius_in_rectangular_windows() {
    let run =
        drawing("HAngle:=1; Ymax:=100; DIMGROB_P(G1,641,481); ARC(G1,5,50,10,0,180,#00FF00h);");
    let g = run.machine.grob(1).unwrap();
    for (x, y) in [(170, 109), (160, 99), (150, 109)] {
        assert_eq!(g.get(x, y), Some(0x00FF00));
    }
    assert_eq!(g.get(160, 119), Some(WHITE));
    assert_eq!(g.get(160, 89), Some(WHITE));
}
#[test]
fn cartesian_text_uses_projected_start_but_pixel_width_and_return_coordinate() {
    let run = drawing(
        "DIMGROB_P(G1,320,240); DIMGROB_P(G2,320,240); LOCAL a,b; a:=TEXTOUT(\"Cartesian\",G1,5,5,0,#000000h,20,#FF0000h); b:=TEXTOUT_P(\"Cartesian\",G2,160,109,0,#000000h,20,#FF0000h); IF a!=b THEN RETURN 1/0; END;",
    );
    assert_eq!(
        run.machine.grob(1).unwrap().pixels(),
        run.machine.grob(2).unwrap().pixels()
    );
}
#[test]
fn subgrob_handles_cartesian_regions_pixel_regions_and_omitted_source() {
    let run = drawing(
        "DIMGROB_P(G1,320,240); SUBGROB(G1,0,10,10,0,G2); SUBGROB(G1,5,5,G3); SUBGROB_P(G1,16,24,G4); PIXON_P(17,25,#FF0000h); SUBGROB_P(16,24,18,26,G5); SUBGROB_P(G1,G6);",
    );
    for (n, w, h) in [
        (2, 318, 218),
        (3, 160, 131),
        (4, 304, 216),
        (5, 2, 2),
        (6, 320, 240),
    ] {
        let g = run.machine.grob(n).unwrap();
        assert_eq!((g.width(), g.height()), (w, h), "G{n}");
    }
    assert_eq!(run.machine.grob(5).unwrap().get(1, 1), Some(0xFF0000));
    assert_eq!(
        run.machine.grob(1).unwrap().pixels(),
        run.machine.grob(6).unwrap().pixels()
    );
}
#[test]
fn subgrob_rejects_reversed_regions_and_g0_destination() {
    for body in [
        "DIMGROB_P(G1,320,240); SUBGROB(G1,10,0,0,10,G2);",
        "DIMGROB_P(G1,3,3); SUBGROB_P(G1,G0);",
    ] {
        let mut run = Run::new("BADREGION", &format!("EXPORT MAIN() BEGIN {body} END;"));
        let error = run.call("MAIN", &[]).unwrap_err();
        assert!(matches!(
            error.kind,
            ErrorKind::InvalidInput | ErrorKind::BadArgValue
        ));
    }
}
#[test]
fn blit_pixel_source_region_keeps_zero_based_exclusive_edges() {
    let run = drawing(
        "DIMGROB_P(G1,20,20); PIXON_P(G1,16,10,#FF0000h); DIMGROB_P(G2,20,20); BLIT_P(G2,G1,16,10,17,11);",
    );
    let g = run.machine.grob(2).unwrap();
    assert_eq!(g.get(0, 0), Some(0xFF0000));
    assert_eq!(g.get(1, 0), Some(WHITE));
}

#[test]
fn cartesian_blit_projects_positions_and_explicit_source_rectangles() {
    // Genuine local G1 BLIT3_PROBE: whole-source copies place the red pixel
    // at(9,9), explicit Cartesian source bounds at(8,9), pixel bounds at(8,9).
    let run = drawing(
        "Xmax:=318;Ymax:=218;DIMGROB_P(G1,20,20); PIXON_P(G1,3,4,#FF0000h); DIMGROB_P(G2,40,40); DIMGROB_P(G3,40,40); DIMGROB_P(G4,40,40); DIMGROB_P(G5,40,40); BLIT(G2,5,213,G1); BLIT(G3,5,213,25,193,G1,0,218,20,198); BLIT_P(G4,5,5,25,25,G1,0,0,20,20); BLIT(G5,5,213,25,193,G1);",
    );
    for (n, x) in [(2, 9), (3, 8), (4, 8), (5, 9)] {
        let g = run.machine.grob(n).unwrap();
        assert_eq!(g.get(x, 9), Some(0xFF0000), "G{n}");
        assert_eq!(g.get(x - 1, 9), Some(WHITE), "G{n}");
        assert_eq!(g.get(x + 1, 9), Some(WHITE), "G{n}");
    }
}
