//! Documented extended forms; local firmware controls are in the remediation evidence.
mod common;
use common::{Run, text};
fn run(body: &str) -> Result<String, ppl_vm::RuntimeError> {
    let mut run = Run::new("<overloads>", &format!("EXPORT TEST() BEGIN {body} END;"));
    run.call("TEST", &[]).map(|v| text(&v))
}
#[test]
fn append_preserves_vector_and_does_not_mutate_original() {
    assert_eq!(
        run("LOCAL a:=[1,2]; LOCAL b:=APPEND(a,3); RETURN {a,b,SIZE(b)};").unwrap(),
        "{[1,2],[1,2,3],{3}}"
    );
    assert_eq!(
        run("RETURN APPEND([1,2],\"bad\");").unwrap_err().kind,
        ppl_vm::ErrorKind::BadArgType
    );
}
#[test]
fn population_stddev_supports_real_weights_vectors_and_matrix_columns() {
    assert_eq!(
        run("RETURN ROUND(STDDEV([1,2,3]),10);").unwrap(),
        "0.8164965809"
    );
    assert_eq!(
        run("RETURN ROUND(STDDEV({1,3},{0.5,1.5}),10);").unwrap(),
        "0.8660254038"
    );
    assert_eq!(run("RETURN STDDEV([[1,2],[3,4]]);").unwrap(), "[1,1]");
    assert_eq!(run("RETURN STDDEV({1,3},{1E100,1E100});").unwrap(), "1");
    assert_eq!(run("RETURN STDDEV({1E-400,3E-400});").unwrap(), "1E-400");
    assert_eq!(run("RETURN STDDEV({1E400,3E400});").unwrap(), "1E400");
    for weights in ["{1}", "{1,-1}", "{0,0}"] {
        assert!(run(&format!("RETURN STDDEV({{1,2}},{weights});")).is_err());
    }
}
#[test]
fn string_separator_symbols_and_dotzero_follow_firmware_controls() {
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,\"123,456\");").unwrap(),
        "1234,567.89"
    );
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,\"123,456.789\");").unwrap(),
        "1,234.567.89"
    );
    assert_eq!(
        run("RETURN STRING(-0.125,2,3,{-1,\",e~\",1});").unwrap(),
        "~0,125"
    );
    assert_eq!(
        run("RETURN STRING(0.125,2,3,{-1,\"\",0});").unwrap(),
        ".125"
    );
    assert_eq!(
        run("RETURN STRING(0.125,2,3,{-1,\"\",1});").unwrap(),
        "0.125"
    );
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,10);").unwrap(),
        "12,34,567.89"
    );
    assert_eq!(
        run("RETURN STRING(1234567.89,2,2,0);").unwrap(),
        "1﹐234﹐567.89"
    );
}
#[test]
fn string_fractions_floating_rounded_and_current_settings() {
    assert_eq!(run("RETURN STRING(1/3,8,3);").unwrap(), "1/3");
    assert_eq!(run("RETURN STRING(-7/3,15,3);").unwrap(), "−2 1/3");
    assert_eq!(run("RETURN STRING(12345.6789,5,3);").unwrap(), "1.235ᴇ4");
    assert_eq!(run("RETURN STRING(12345.6789,6,3);").unwrap(), "12345.679");
    assert_eq!(run("RETURN STRING(0.001234567,5,3);").unwrap(), "0.0012");
    assert_eq!(run("RETURN STRING(0.001234567,6,3);").unwrap(), "0.001");
    assert_eq!(
        run("HFormat:=1; HDigits:=2; RETURN STRING(1.2345,0);").unwrap(),
        "1.23"
    );
    assert_eq!(run("RETURN STRING(1,2,3,\",\");").unwrap(), "1.000");
}
#[test]
fn string_formatting_does_not_rewrite_string_contents_in_collections() {
    assert_eq!(
        run("RETURN STRING({1234.5,\"1234.5\"},2,1,\"123,456\");").unwrap(),
        "{1,234.5,\"1234.5\"}"
    );
}
#[test]
fn dimgrob_dimension_pair_and_depth_string_match_local_firmware() {
    assert_eq!(run("DIMGROB_P(G1,{1,2}); RETURN {GROBW_P(G1),GROBH_P(G1),GETPIX_P(G1,0,0),GETPIX_P(G1,0,1)};").unwrap(),"{1,2,16777215,16777215}");
    assert_eq!(
        run("LOCAL z:=TRIANGLE_P(); RETURN {DIM(z),ASC(LEFT(z,2))};").unwrap(),
        "{38400,{65535,65535}}"
    );
    assert_eq!(
        run("DIMGROB_P(G1,3,3); RETURN DIM(TRIANGLE_P(G1));").unwrap(),
        "5"
    );
}
#[test]
fn triangle_vertex_lists_gradient_and_alpha_have_actual_pixels() {
    assert_eq!(run("RECT_P(0); TRIANGLE_P({0,0,#FF0000h},{10,0,#00FF00h},{0,10,#0000FFh}); RETURN GETPIX_P(1,1);").unwrap(),"11740710");
    assert_eq!(
        run("RECT_P(0); TRIANGLE_P(0,0,10,0,0,10,#FF0000h,128); RETURN GETPIX_P(1,1);").unwrap(),
        "8388608"
    );
}
#[test]
fn cartesian_graphics_use_the_current_calculator_plot_window() {
    assert_eq!(run("Xmin:=0; Xmax:=10; Ymin:=0; Ymax:=10; RECT_P(0); PIXON(5,5,#FF0000h); RETURN {GETPIX_P(160,109),Xmin,Xmax,NumStart,NumStep};").unwrap(),"{16711680,0,10,0,1}");
    assert_eq!(
        run("Xmin:=1; Xmax:=1; PIXON(0,0);").unwrap_err().kind,
        ppl_vm::ErrorKind::BadArgValue
    );
    assert_eq!(
        run("Xmin:=1; Xmax:=1; PIXON_P(0,0); RETURN 1;").unwrap(),
        "1"
    );
}
