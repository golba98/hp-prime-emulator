//! Documented I/O forms and blocking behavior (HP User Guide pp.593,609).
mod common;
use common::{Run, text};
use ppl_vm::ScriptEvent;

#[test]
fn freeze_consumes_a_key_before_following_execution() {
    let mut run = Run::new(
        "<freeze>",
        "EXPORT TEST() BEGIN FREEZE; RETURN GETKEY; END;",
    );
    run.host
        .borrow_mut()
        .keys
        .extend([ScriptEvent::Key(30), ScriptEvent::Key(12)]);
    assert_eq!(text(&run.call("TEST", &[]).unwrap()), "12");
}
#[test]
fn freeze_reports_script_exhaustion_instead_of_fabricating_a_key() {
    let mut run = Run::new("<freeze>", "EXPORT TEST() BEGIN FREEZE; RETURN 7; END;");
    assert_eq!(
        run.call("TEST", &[]).unwrap_err().kind,
        ppl_vm::ErrorKind::ScriptExhausted
    );
}
#[test]
fn drawmenu_list_and_positional_labels_render_identically() {
    let mut list = Run::new(
        "<menu>",
        "EXPORT TEST() BEGIN DRAWMENU({\"ABC\",\"\",\"DEF\"}); RETURN 0; END;",
    );
    list.call("TEST", &[]).unwrap();
    let mut positional = Run::new(
        "<menu>",
        "EXPORT TEST() BEGIN DRAWMENU(\"ABC\",\"\",\"DEF\"); RETURN 0; END;",
    );
    positional.call("TEST", &[]).unwrap();
    assert_eq!(
        list.machine.screen().pixels(),
        positional.machine.screen().pixels()
    );
}
#[test]
fn drawmenu_rejects_more_than_six_list_items() {
    let mut run = Run::new(
        "<menu>",
        "EXPORT TEST() BEGIN DRAWMENU({1,2,3,4,5,6,7}); END;",
    );
    assert_eq!(
        run.call("TEST", &[]).unwrap_err().kind,
        ppl_vm::ErrorKind::BadArgValue
    );
}
#[test]
fn mouse_index_is_minus_one_when_no_pointer_is_active() {
    let mut run = Run::new(
        "<mouse>",
        "EXPORT TEST() BEGIN RETURN {MOUSE(),MOUSE(1),MOUSE(5)}; END;",
    );
    assert_eq!(text(&run.call("TEST", &[]).unwrap()), "{{},-1,-1}");
}
