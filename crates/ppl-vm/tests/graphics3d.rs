//! Native graphics execution, including genuine local firmware controls.
mod common;
use common::{Run, text};
fn run(body: &str) -> Result<String, ppl_vm::RuntimeError> {
    let mut r = Run::new("<graphics3d>", &format!("EXPORT TEST() BEGIN {body} END;"));
    r.call("TEST", &[]).map(|v| text(&v))
}
#[test]
fn packed_graphics_fourth_argument_matches_local_firmware_controls() {
    assert_eq!(run("DIMGROB_P(G1,2,2,{#000003E07C00001F:64h}); RETURN {GETPIX_P(G1,0,0),GETPIX_P(G1,1,0),GETPIX_P(G1,0,1),GETPIX_P(G1,1,1)};").unwrap(),"{#FFh,#FF0000h,#FF00h,#0h}");
    assert_eq!(run("DIMGROB_P(G1,2,2,{#1122334455667788:64h,#99AABBCCDDEEFF00:64h}); RETURN {GETPIX_P(G1,0,0),GETPIX_P(G1,1,0),GETPIX_P(G1,0,1),GETPIX_P(G1,1,1)};").unwrap(),"{#55667788h,#11223344h,#DDEEFF00h,#99AABBCCh}");
    assert_eq!(run("DIMGROB_P(G1,2,2,{#80007FFF801F4210:64h}); RETURN {GETPIX_P(G1,0,0),GETPIX_P(G1,1,0),GETPIX_P(G1,0,1),GETPIX_P(G1,1,1)};").unwrap(),"{#848484h,#FFFFFFh,#FFFFFFh,#FFFFFFh}");
    assert_eq!(run("DIMGROB_P(G1,6,1,{#1F:16h}); RETURN {GETPIX_P(G1,0,0),GETPIX_P(G1,3,0),GETPIX_P(G1,4,0)};").unwrap(),"{#FFh,#0h,#FFFFFFh}");
    assert_eq!(
        run("DIMGROB_P(G1,2,1,{}); RETURN GETPIX_P(G1,0,0);").unwrap(),
        "#FFFFFFh"
    );
}
#[test]
fn line_clips_in_original_space_before_transform_matching_firmware() {
    assert_eq!(run("DIMGROB_P(G1,150,50); LOCAL c:=LINE_P(G1,{{10,10,1},{20,10,1}},{{1,2,#FF0000h}},{[[1,0,0,100],[0,1,0,0],[0,0,1,0]],\"\",-1,{0,30,0,30,0,30}}); RETURN {GETPIX_P(G1,110,10),GETPIX_P(G1,15,10),TYPE(c)};").unwrap(),"{#FF0000h,#FFFFFFh,2}");
}
#[test]
fn normalized_depth_is_separate_from_projection_matching_firmware() {
    assert_eq!(run("DIMGROB_P(G1,100,60); LINE_P(G1,{{10,10,2},{20,10,4}},{{1,2,#FF0000h}},{-1,\"N\",{-60,-20,2}}); RETURN {GETPIX_P(G1,70,30),GETPIX_P(G1,70,25)};").unwrap(),"{#FF0000h,#FF0000h}");
}
#[test]
fn cached_points_are_reusable_by_both_line_and_triangle() {
    assert_eq!(run("LOCAL c:=LINE_P({{0,0,#FF0000h},{10,0,#FF00h},{0,10,#FFh}},{{1,2}},{-1}); RECT_P(); TRIANGLE_P(c,{{1,2,3}}); RETURN GETPIX_P(1,1);").unwrap(),"#B32626h");
    assert_eq!(run("LOCAL c:=TRIANGLE_P({{0,0},{10,0},{0,10}},{{1,2,3,#FF0000h}},{-1}); RECT_P(); LINE_P(c,{{1,2,#FFh}}); RETURN {GETPIX_P(5,0),GETPIX_P(1,1)};").unwrap(),"{#FFh,#FFFFFFh}");
}
#[test]
fn triangles_clip_to_source_box_then_translate() {
    assert_eq!(run("TRIANGLE_P({{-10,0,1},{10,0,1},{10,20,1}},{{1,2,3,#FFh}},{[[1,0,0,100],[0,1,0,0],[0,0,1,0]],\"\",-1,{0,10,0,20,0,2}}); RETURN {GETPIX_P(105,3),GETPIX_P(95,3)};").unwrap(),"{#FFh,#FFFFFFh}");
}
#[test]
fn per_primitive_colors_alpha_and_complex_points_are_real_graphics() {
    assert_eq!(
        run("RECT_P(0); LINE_P({(0,0),(10,0)},{#FF0000h,128,{1,2}},{-1}); RETURN GETPIX_P(5,0);")
            .unwrap(),
        "#800000h"
    );
    // Original15751 firmware normalizes numeric vector entries to real.
    // These third components are depth coordinates, not word colors.
    assert_eq!(
        run("LINE_P({[0,0,#FF0000h],[10,0,#FFh]},{{1,2}},[[1,0],[0,1]]); RETURN GETPIX_P(5,0);")
            .unwrap(),
        "#0h"
    );
}
#[test]
fn direct_vertex_zbuffer_mutates_local_and_global_strings() {
    assert_eq!(run("DIMGROB_P(G1,8,8); LOCAL z:=TRIANGLE_P(G1); TRIANGLE_P(G1,{0,0,#FFh,10},{8,0,#FFh,10},{0,8,#FFh,10},z); TRIANGLE_P(G1,{0,0,#FF0000h,100},{8,0,#FF0000h,100},{0,8,#FF0000h,100},z); RETURN {GETPIX_P(G1,1,1),DIM(z),ASC(LEFT(z,1))};").unwrap(),"{#FFh,32,{2570}}");
    assert_eq!(run("DIMGROB_P(G1,8,8); Z0:=TRIANGLE_P(G1); TRIANGLE_P(G1,0,0,8,0,0,8,#FF00h,Z0,5,5,5); RETURN {GETPIX_P(G1,1,1),ASC(LEFT(Z0,1))};").unwrap(),"{#FF00h,{1285}}");
}
#[test]
fn cached_lines_read_and_write_the_same_depth_buffer() {
    assert_eq!(run("DIMGROB_P(G1,8,8); LOCAL z:=TRIANGLE_P(G1); TRIANGLE_P(G1,{{0,0,10,#FFh},{8,0,10,#FFh},{0,8,10,#FFh}},{{1,2,3}},[[1,0,0],[0,1,0],[0,0,1]],z); RETURN {GETPIX_P(G1,1,1),ASC(LEFT(z,1))};").unwrap(),"{#FFh,{2570}}");
    assert_eq!(run("DIMGROB_P(G1,8,8); LOCAL z:=TRIANGLE_P(G1); LOCAL c:=LINE_P(G1,{{0,1,10},{7,1,10}},{{1,2,#FFh}},{-1},z); LINE_P(G1,c,{{1,2,#FF0000h}},z); RETURN {GETPIX_P(G1,3,1),ASC(MID(z,5,1))};").unwrap(),"{#FF0000h,{2570}}");
}
#[test]
fn zbuffer_surrogate_units_use_the_exact_packed_utf16_layout() {
    assert_eq!(run("DIMGROB_P(G1,8,8); LOCAL z:=TRIANGLE_P(G1); TRIANGLE_P(G1,0,0,8,0,0,8,#FFh,z,220,220,220); TRIANGLE_P(G1,0,0,8,0,0,8,#FF0000h,z,230,230,230); RETURN {GETPIX_P(G1,1,1),DIM(z),ASC(LEFT(z,1))};").unwrap(),"{#FFh,32,{56540}}");
}
#[test]
fn invalid_geometry_and_nonwritable_zstrings_do_not_draw_partial_results() {
    assert_eq!(
        run(
            "IFERR LINE_P({{0,0},{10,0}},{{1,2,#FFh},{1,99}},{-1}); THEN END; RETURN GETPIX_P(5,0);"
        )
        .unwrap(),
        "#FFFFFFh"
    );
    assert_eq!(
        run("IFERR LINE_P(\"firmware opaque cache\",{{1,2}}); THEN END; RETURN GETPIX_P(0,0);")
            .unwrap(),
        "#FFFFFFh"
    );
    assert_eq!(run("LOCAL z:=TRIANGLE_P(); IFERR LINE_P({{0,0},{10,0}},{{1,2,#FFh}},{-1},z+\"\"); THEN END; RETURN GETPIX_P(5,0);").unwrap(),"#FFFFFFh");
    assert!(run("LINE_P({{0,0},{10,0}},{{1,2}},[[1,0,0,0,0],[0,1,0,0,0]]);").is_err());
}

#[test]
fn cartesian_triangle_and_line_share_plot_variables_and_cached_points() {
    assert_eq!(run("Xmin:=0; Xmax:=10; Ymin:=0; Ymax:=10; LOCAL c:=LINE({{0,0},{10,0},{0,10}},{{1,2}},{-1}); RECT_P(); TRIANGLE(c,{{1,2,3,#FFh}}); RETURN {GETPIX_P(10,220),GETPIX_P(310,10)};").unwrap(),"{#FFh,#FFFFFFh}");
    assert_eq!(run("Xmin:=0; Xmax:=10; Ymin:=0; Ymax:=10; TRIANGLE(0,0,10,0,0,10,#FFh); RETURN GETPIX_P(10,220);").unwrap(),"#FFh");
    assert_eq!(
        run("TRIANGLE_P((0,0),(10,0),(0,10)); RETURN GETPIX_P(1,1);").unwrap(),
        "#0h"
    );
}

#[test]
fn rectangular_matrix_multiplies_z_and_real_axis_points_follow_numeric_row_semantics() {
    assert_eq!(run("LINE_P({{10,10,5},{20,10,5}},{{1,2,#FF0000h}},[[1,0,2],[0,1,3]]); RETURN {GETPIX_P(12,13),GETPIX_P(20,25)};").unwrap(),"{#FFFFFFh,#FF0000h}");
    assert_eq!(
        run("LINE_P({{(0,0),#FF0000h},{(10,0),#FFh}},{{1,2}},{-1}); RETURN GETPIX_P(5,0);")
            .unwrap(),
        "#FFFFFFh"
    );
}
#[test]
fn full_scalar_depth_form_writes_only_z_and_preserves_string_copies() {
    assert_eq!(run("DIMGROB_P(G1,8,8,0); LOCAL z:=TRIANGLE_P(G1), old:=z, depth:=10, alpha:=128; TRIANGLE_P(G1,0,0,8,0,0,8,#FF0000h,#FF0000h,#FF0000h,alpha,z,depth,depth,depth); RETURN {GETPIX_P(G1,1,1),ASC(LEFT(z,1)),ASC(LEFT(old,1)),alpha,depth};").unwrap(),"{#800000h,{2570},{65535},128,10}");
    assert_eq!(
        run("LOCAL color:=#FFh; LINE_P(0,0,10,0,color); RETURN {GETPIX_P(5,0),color};").unwrap(),
        "{#FFh,#FFh}"
    );
}
