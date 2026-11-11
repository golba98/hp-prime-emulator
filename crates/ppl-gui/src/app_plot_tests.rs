//! Real LCD integer controls must update the same drafts as keyboard edits.
use crate::ide::{
    native_integer::IntegerEdit,
    native_tools::{NativeTools, Page},
};
struct IntegerScreen {
    ctx: egui::Context,
    tools: NativeTools,
    time: f64,
}
impl IntegerScreen {
    fn new() -> Self {
        let word = IntegerEdit {
            raw: 3,
            bits: 4,
            base: 16,
            signed: false,
        };
        let mut tools = NativeTools::default();
        tools.open(Page::Base);
        tools.integer_original = Some(word.clone());
        tools.integer_edit = Some(word);
        tools.integer_decimal_draft = "3".into();
        tools.integer_hex_draft = "3".into();
        Self {
            ctx: egui::Context::default(),
            tools,
            time: 0.,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.025;
        let snapshot = ppl_vm::SessionContext::default().snapshot();
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                time: Some(self.time),
                ..Default::default()
            },
            |ctx| {
                self.tools.show(
                    ctx,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320., 240.)),
                    &snapshot,
                );
            },
        )
    }
    fn click(&mut self, label: &str) {
        let mut output = self.frame(vec![]);
        let mut found = None;
        for _ in 0..5 {
            found = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    let point = text.pos + text.galley.size() / 2.;
                    shape.clip_rect.contains(point).then_some(point)
                }
                _ => None,
            });
            if found.is_some() {
                break;
            }
            output = self.frame(vec![
                egui::Event::PointerMoved(egui::pos2(160., 180.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -70.),
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let pos = found.unwrap_or_else(|| {
            panic!("actual button {label} was not reachable through LCD scrolling")
        });
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
}
#[test]
fn integer_buttons_and_signed_checkbox_refresh_both_decimal_and_hex_drafts() {
    let mut screen = IntegerScreen::new();
    screen.frame(vec![]);
    screen.frame(vec![]);
    for (label, raw, decimal, hex) in [
        ("Shift ←", 6, "6", "6"),
        ("NOT", 9, "9", "9"),
        ("Signed", 9, "-7", "9"),
        ("Neg", 7, "7", "7"),
        ("Shift →", 3, "3", "3"),
        ("Reset", 3, "3", "3"),
        ("0", 11, "11", "B"),
        ("Base", 11, "11", "B"),
        ("Reset", 3, "3", "3"),
    ] {
        screen.click(label);
        assert_eq!(
            screen.tools.integer_edit.as_ref().unwrap().raw,
            raw,
            "{label} must execute its actual integer action"
        );
        assert_eq!(
            screen.tools.integer_decimal_draft, decimal,
            "{label} decimal draft"
        );
        assert_eq!(screen.tools.integer_hex_draft, hex, "{label} hex draft");
    }
    assert_eq!(screen.tools.integer_edit, screen.tools.integer_original);
}
