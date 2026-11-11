//! Native EDITMAT contracts observed in local G1 firmware (not physical hardware).
mod common;
use common::{Run, text};
use ppl_vm::ErrorKind;

fn run(body: &str, inputs: &[&str]) -> Result<ppl_vm::Value, ppl_vm::RuntimeError> {
    let mut machine = Run::new(
        "<editmat>",
        &format!("EXPORT MATRIX_TEST() BEGIN {body} END;"),
    );
    machine.call("MATRIX_TEST", inputs)
}

#[test]
fn calculator_matrix_variable_writes_back_and_returns_edited_matrix() {
    let result = run(
        "M1:=[[1,2],[3,4]]; LOCAL a:=EDITMAT(M1); RETURN {M1,a};",
        &["9", "2", "3", "4"],
    )
    .unwrap();
    assert_eq!(text(&result), "{[[9,2],[3,4]],[[9,2],[3,4]]}");
}

#[test]
fn local_matrix_named_m1_keeps_calculator_writeback_semantics() {
    let result = run(
        "LOCAL M1:=[[1,2],[3,4]]; LOCAL b:=EDITMAT(M1); RETURN {M1,b};",
        &["9", "2", "3", "4"],
    )
    .unwrap();
    assert_eq!(text(&result), "{[[9,2],[3,4]],[[9,2],[3,4]]}");
}

#[test]
fn local_matrix_is_a_value_not_a_writeback_reference() {
    let result = run(
        "LOCAL a:=[[1,2],[3,4]]; LOCAL b:=EDITMAT(a); RETURN {a,b};",
        &["9", "2", "3", "4"],
    )
    .unwrap();
    assert_eq!(text(&result), "{[[1,2],[3,4]],[[9,2],[3,4]]}");
}

#[test]
fn matrix_expressions_and_vectors_are_accepted() {
    assert_eq!(
        text(&run("RETURN EDITMAT([[1,2],[3,4]]);", &["9", "2", "3", "4"]).unwrap()),
        "[[9,2],[3,4]]"
    );
    assert_eq!(
        text(&run("RETURN EDITMAT([1,2]);", &["3", "4"]).unwrap()),
        "[3,4]"
    );
}

#[test]
fn cancel_returns_original_and_preserves_calculator_variable() {
    assert_eq!(
        text(
            &run(
                "M1:=[[1,2],[3,4]]; LOCAL a:=EDITMAT(M1); RETURN {M1,a};",
                &["9", "<ESC>"]
            )
            .unwrap()
        ),
        "{[[1,2],[3,4]],[[1,2],[3,4]]}"
    );
}

#[test]
fn readonly_returns_original_without_requiring_edits() {
    assert_eq!(text(&run("M1:=[[1,2],[3,4]]; LOCAL a:=EDITMAT(M1,{\"View\",{\"Row A\",\"Row B\"},{\"X\",\"Y\"}},1); RETURN {M1,a};", &[]).unwrap()), "{[[1,2],[3,4]],[[1,2],[3,4]]}");
}

#[test]
fn invalid_cells_reopen_without_partial_writeback() {
    assert_eq!(
        text(&run("RETURN EDITMAT([[1]]);", &["", "<ESC>"]).unwrap()),
        "[[1]]"
    );
    assert_eq!(
        text(
            &run(
                "M1:=[[1,2]]; LOCAL a:=EDITMAT(M1); RETURN {M1,a};",
                &["9", "\"not a number\"", "<ESC>"]
            )
            .unwrap()
        ),
        "{[[1,2]],[[1,2]]}"
    );
    assert_eq!(
        text(&run("RETURN EDITMAT([[1,2]]);", &["9", "unknown", "9", "2^3"]).unwrap()),
        "[[9,8]]"
    );
}

#[test]
fn invalid_arguments_are_explicit_errors() {
    for body in [
        "RETURN EDITMAT(42);",
        "RETURN EDITMAT([[1]],2);",
        "RETURN EDITMAT([[1]],{\"T\",42});",
    ] {
        assert_eq!(run(body, &[]).unwrap_err().kind, ErrorKind::BadArgType);
    }
}

#[test]
fn cancelling_does_not_consume_following_program_input() {
    assert_eq!(
        text(
            &run(
                "LOCAL a:=EDITMAT([[1]]),b; INPUT(b); RETURN {a,b};",
                &["<ESC>", "7"]
            )
            .unwrap()
        ),
        "{[[1]],7}"
    );
}

#[test]
fn editlist_calculator_variable_writes_back_and_cancel_preserves_list() {
    assert_eq!(
        text(
            &run(
                "L1:={1,2}; LOCAL result:=EDITLIST(L1); RETURN {L1,result};",
                &["9", "3"]
            )
            .unwrap()
        ),
        "{{9,3},{9,3}}"
    );
    assert_eq!(
        text(
            &run(
                "L1:={1,2}; LOCAL result:=EDITLIST(L1); RETURN {L1,result};",
                &["9", "<ESC>"]
            )
            .unwrap()
        ),
        "{{1,2},{1,2}}"
    );
    assert_eq!(
        text(&run("RETURN EDITLIST({1,2});", &["9", "3"]).unwrap()),
        "{9,3}"
    );
}

#[test]
fn editlist_invalid_expression_reopens_with_error_and_preserves_original_on_cancel() {
    let mut run = common::Run::new(
        "<list validation>",
        "EXPORT TEST() BEGIN L1:={1,2}; LOCAL result:=EDITLIST(L1); RETURN {L1,result}; END;",
    );
    let result = run
        .call("TEST", &["unknown_variable", "2", "<ESC>"])
        .unwrap();
    assert_eq!(text(&result), "{{1,2},{1,2}}");
    assert!(
        run.dialogs()
            .iter()
            .any(|text| text.starts_with("EDITLIST-ERROR:"))
    );
}
