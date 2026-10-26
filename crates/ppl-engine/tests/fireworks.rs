//! Preservation of the intended native animation when invalid presentation-only
//! calls are removed. This is not physical display or firmware-runtime evidence.
use ppl_graphics::Grob;
use ppl_vm::{
    Dialog, DialogResult, HeadlessHost, Host, Limits, Machine, RuntimeError, Settings,
    host::{ScriptEvent, Touch},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};
type Frames = Rc<RefCell<Vec<(u64, Vec<u32>)>>>;
struct Recording {
    inner: HeadlessHost,
    frames: Frames,
}
impl Host for Recording {
    fn print(&mut self, text: &str) {
        self.inner.print(text)
    }
    fn present(&mut self, s: &Grob, t: Option<&[String]>) {
        self.inner.present(s, t)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.inner.poll_key()
    }
    fn is_key_down(&mut self, k: i64) -> Result<bool, RuntimeError> {
        self.inner.is_key_down(k)
    }
    fn wait_key(&mut self, t: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.inner.wait_key(t)
    }
    fn sleep(&mut self, s: f64) -> Result<(), RuntimeError> {
        self.frames.borrow_mut().push((
            s.to_bits(),
            self.inner.last_screen.as_ref().unwrap().pixels().to_vec(),
        ));
        self.inner.sleep(s)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.inner.touch()
    }
    fn dialog(&mut self, d: &Dialog, s: &Grob) -> Result<DialogResult, RuntimeError> {
        self.inner.dialog(d, s)
    }
    fn now_ms(&mut self) -> f64 {
        self.inner.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.inner.local_time()
    }
    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        self.inner.interrupt_flag()
    }
}
fn frames(source: &str) -> (Vec<(u64, Vec<u32>)>, Vec<u32>) {
    let file = ppl_syntax::SourceFile::new("FIREWORKS", source);
    let (program, diagnostics) = ppl_compiler::compile_source(&file, &Default::default());
    assert!(!diagnostics.has_errors());
    let frames = Rc::new(RefCell::new(Vec::new()));
    let mut host = HeadlessHost::new();
    for i in 0..60 {
        host.keys.push_back(ScriptEvent::Key(-1));
        if i == 2 {
            host.keys.push_back(ScriptEvent::Touch(120, 40));
        }
    }
    host.keys.push_back(ScriptEvent::Key(4));
    let mut m = Machine::new(
        Rc::new(program.unwrap()),
        Box::new(Recording {
            inner: host,
            frames: frames.clone(),
        }),
        Settings::default(),
        Limits {
            max_steps: Some(5_000_000),
            ..Default::default()
        },
    );
    m.set_seed(0x5eed);
    m.call_function("FIREWORKS", vec![]).unwrap();
    let final_pixels = m.screen().pixels().to_vec();
    let recorded = frames.borrow().clone();
    (recorded, final_pixels)
}
#[test]
fn removal_of_invalid_blits_preserves_every_wait_frame_and_exit_screen() {
    let corrected = include_str!("../../../examples/FIREWORKS.hpprgm");
    let marker = "// G0 already displays drawing; no blit is required.";
    assert_eq!(corrected.matches(marker).count(), 2);
    let prior_native = corrected.replace(marker, "BLIT_P();");
    let (before, final_before) = frames(&prior_native);
    let (after, final_after) = frames(corrected);
    assert_eq!(before.len(), 61);
    assert!(
        before.windows(2).filter(|w| w[0].1 != w[1].1).count() > 10,
        "simulation must actually animate"
    );
    assert_eq!(before, after);
    assert_eq!(final_before, final_after);
}
