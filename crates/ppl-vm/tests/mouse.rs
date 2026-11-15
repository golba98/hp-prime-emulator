//! VM/Host transport of independently observed G1 pointer packets. Actual GUI
//! input delivery is validated separately, not supplied by this packet host.
use ppl_vm::host::{Dialog, DialogResult, MouseContact, MouseKind, MouseState, Touch};
use ppl_vm::{HeadlessHost, Host, Limits, RuntimeError, SessionContext, Value};
use std::{cell::Cell, rc::Rc};
struct PacketHost {
    inner: HeadlessHost,
    packet: MouseState,
    polls: Rc<Cell<usize>>,
}
impl Host for PacketHost {
    fn mouse_state(&mut self) -> Result<MouseState, RuntimeError> {
        self.polls.set(self.polls.get() + 1);
        Ok(self.packet)
    }
    fn print(&mut self, text: &str) {
        self.inner.print(text)
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.inner.poll_key()
    }
    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.inner.is_key_down(key)
    }
    fn wait_key(&mut self, timeout: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.inner.wait_key(timeout)
    }
    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.inner.sleep(seconds)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.inner.touch()
    }
    fn dialog(
        &mut self,
        dialog: &Dialog,
        screen: &ppl_graphics::Grob,
    ) -> Result<DialogResult, RuntimeError> {
        self.inner.dialog(dialog, screen)
    }
    fn now_ms(&mut self) -> f64 {
        self.inner.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.inner.local_time()
    }
}
fn evaluate(source: &str, packet: MouseState) -> (Result<Value, RuntimeError>, usize) {
    let polls = Rc::new(Cell::new(0));
    let mut session = SessionContext::default();
    let result = session
        .evaluate_home(
            source,
            Box::new(PacketHost {
                inner: HeadlessHost::new(),
                packet,
                polls: polls.clone(),
            }),
            Limits::default(),
        )
        .result;
    (result, polls.get())
}
#[test]
fn original_press_drag_and_idle_packets_keep_words_origins_and_zero_based_indices() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-mouse-controls.json"
    ))
    .unwrap();
    for sample in fixture["samples"].as_array().unwrap() {
        let packet = MouseState {
            pointers: std::array::from_fn(|i| {
                let p = &sample["pointers"][i];
                if p.is_null() {
                    return None;
                }
                Some(MouseContact {
                    x: p["x"].as_i64().unwrap() as i16,
                    y: p["y"].as_i64().unwrap() as i16,
                    origin_x: p["origin_x"].as_i64().unwrap() as i16,
                    origin_y: p["origin_y"].as_i64().unwrap() as i16,
                    kind: match p["kind"].as_u64().unwrap() {
                        0 => MouseKind::New,
                        2 => MouseKind::Drag,
                        _ => panic!("unobserved kind"),
                    },
                })
            }),
        };
        let (value, polls) = evaluate("MOUSE", packet);
        assert_eq!(polls, 1);
        let Value::List(pointers) = value.unwrap() else {
            panic!("two lists")
        };
        assert_eq!(pointers.len(), 2);
        for (i, pointer) in pointers.iter().enumerate() {
            let Value::List(cells) = pointer else {
                panic!("pointer list")
            };
            let Some(expected) = packet.pointers[i] else {
                assert!(cells.is_empty());
                continue;
            };
            let expected = [
                expected.x,
                expected.y,
                expected.origin_x,
                expected.origin_y,
                expected.kind as i16,
            ];
            assert_eq!(cells.len(), 5);
            for (k, (cell, expected)) in cells.iter().zip(expected).enumerate() {
                let Value::Int(word) = cell else {
                    panic!("original pointer cells are words")
                };
                assert_eq!(word.numeric_value(), i128::from(expected));
                assert_eq!(word.width_code(), if k < 4 { -16 } else { 16 });
                assert!(word.is_system_base());
            }
        }
        for i in 0..10 {
            let (value, polls) = evaluate(&format!("MOUSE({i})"), packet);
            assert_eq!(polls, 1);
            let value = value.unwrap();
            let expected = if let Value::List(cells) = &pointers[i / 5] {
                cells.get(i % 5)
            } else {
                unreachable!()
            };
            if let Some(expected) = expected {
                assert_eq!(
                    ppl_vm::PortableValue::from(&value),
                    ppl_vm::PortableValue::from(expected)
                );
            } else {
                assert!(matches!(value,Value::Real(n)if n==ppl_math::Real::from_i64(-1)));
            }
        }
    }
}
#[test]
fn invalid_mouse_indices_fail_before_consuming_a_host_packet() {
    for argument in [
        "-2", "-1", "10", "12", "-0.5", "0.5", "1.9", "9.9", "10.1", "\"2\"", "{2}", "(1,2)",
    ] {
        let (value, polls) = evaluate(&format!("MOUSE({argument})"), MouseState::default());
        assert_eq!(
            value.unwrap_err().kind,
            ppl_vm::ErrorKind::InvalidInput,
            "{argument}"
        );
        assert_eq!(polls, 0, "{argument}");
    }
    let (value, polls) = evaluate("MOUSE(#2h)", MouseState::default());
    assert_eq!(polls, 1);
    assert_eq!(value.unwrap().expect_i64("idle").unwrap(), -1);
}
#[test]
fn both_slots_are_preserved_and_second_pointer_indices_are_independent() {
    let first = MouseContact {
        x: 10,
        y: 20,
        origin_x: 1,
        origin_y: 2,
        kind: MouseKind::Drag,
    };
    let second = MouseContact {
        x: 30,
        y: 40,
        origin_x: 3,
        origin_y: 4,
        kind: MouseKind::New,
    };
    let (value, _) = evaluate(
        "{MOUSE(0),MOUSE(5),MOUSE(7),MOUSE(9)}",
        MouseState {
            pointers: [Some(first), Some(second)],
        },
    );
    let Value::List(cells) = value.unwrap() else {
        panic!("list")
    };
    assert_eq!(
        cells
            .iter()
            .map(|v| v.expect_i64("packet").unwrap())
            .collect::<Vec<_>>(),
        [10, 30, 3, 0]
    );
}

#[test]
fn real_headless_touch_events_are_explicit_single_contacts_and_do_not_consume_keys() {
    use ppl_vm::host::ScriptEvent;
    let mut host = HeadlessHost::new();
    host.keys
        .extend([ScriptEvent::Key(30), ScriptEvent::Touch(100, 50)]);
    assert_eq!(host.mouse_state().unwrap(), MouseState::default());
    assert_eq!(host.poll_key().unwrap(), Some(30));
    let state = host.mouse_state().unwrap();
    assert_eq!(
        state.pointers[0],
        Some(MouseContact {
            x: 100,
            y: 50,
            origin_x: 100,
            origin_y: 50,
            kind: MouseKind::New
        })
    );
    assert_eq!(state.pointers[1], None);
    assert_eq!(host.mouse_state().unwrap(), MouseState::default());
    host.keys.push_back(ScriptEvent::Touch(32768, 0));
    assert_eq!(
        host.mouse_state().unwrap_err().kind,
        ppl_vm::ErrorKind::InvalidInput
    );
    assert_eq!(
        host.keys.len(),
        1,
        "rejected coordinates do not consume input"
    );
}
