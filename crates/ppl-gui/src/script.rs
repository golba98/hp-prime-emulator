//! Scripted input for the GUI (`ppl gui --keys SCRIPT`).
//!
//! A key script is fed into the same queues the window's keyboard and mouse
//! handlers use, from a small feeder thread, so a GUI run can be driven end
//! to end without an X input tool. Syntax (comma separated):
//!
//! | Item | Meaning |
//! |---|---|
//! | `N`, `N*K` | press HP key `N` (0–50), `K` times |
//! | `-1`, `-1*K` | idle for one interval (`K` times) |
//! | `wait:MS` | pause `MS` milliseconds |
//! | `text:CHARS` | type `CHARS` (no commas) as keyboard characters, e.g. into INPUT |
//! | `touch:X:Y[:MS]` | touch screen pixel (X,Y) for `MS` ms (default: one interval) |
//!
//! Every key/character is held down (for `ISKEYDOWN`) for half an interval
//! and followed by a full interval before the next item.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ppl_vm::host::Touch;

use crate::dialog::KeyPress;
use crate::host::GuiSharedState;
use crate::keymap::map_char;

/// Default pause after each scripted event.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(100);

/// One scripted GUI input event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptItem {
    /// Press and release an HP key.
    Key(i64),
    /// Do nothing for one interval.
    Idle,
    /// Pause.
    Wait(Duration),
    /// Type one character (with its HP key code when it has one).
    Char(char),
    /// Touch a screen pixel for a duration (`None`: one interval).
    Touch(i64, i64, Option<Duration>),
}

/// A parsed key script plus its timing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyScript {
    pub items: Vec<ScriptItem>,
    pub interval: Duration,
}

impl KeyScript {
    /// Parses the `--keys` syntax described in the module docs.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut items = Vec::new();
        for item in s.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            if let Some(ms) = item.strip_prefix("wait:") {
                let ms: u64 = ms
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad wait `{item}` (expected wait:MILLISECONDS)"))?;
                items.push(ScriptItem::Wait(Duration::from_millis(ms)));
                continue;
            }
            if let Some(text) = item.strip_prefix("text:") {
                items.extend(text.chars().map(ScriptItem::Char));
                continue;
            }
            if let Some(rest) = item.strip_prefix("touch:") {
                let bad = || format!("bad touch `{item}` (expected touch:X:Y or touch:X:Y:MS)");
                let parts: Vec<&str> = rest.split(':').map(str::trim).collect();
                if !(2..=3).contains(&parts.len()) {
                    return Err(bad());
                }
                let x: i64 = parts[0].parse().map_err(|_| bad())?;
                let y: i64 = parts[1].parse().map_err(|_| bad())?;
                let hold = match parts.get(2) {
                    Some(ms) => Some(Duration::from_millis(ms.parse().map_err(|_| bad())?)),
                    None => None,
                };
                items.push(ScriptItem::Touch(x, y, hold));
                continue;
            }
            let (code, count) = match item.split_once('*') {
                Some((c, n)) => (
                    c.trim(),
                    n.trim()
                        .parse::<usize>()
                        .map_err(|_| format!("bad repeat count in `{item}`"))?,
                ),
                None => (item, 1),
            };
            let code: i64 = code
                .parse()
                .map_err(|_| format!("bad key script item `{item}`"))?;
            let ev = match code {
                -1 => ScriptItem::Idle,
                0..=50 => ScriptItem::Key(code),
                _ => return Err(format!("key code {code} is outside the Prime's 0–50 range")),
            };
            items.extend(std::iter::repeat_n(ev, count));
        }
        Ok(Self {
            items,
            interval: DEFAULT_INTERVAL,
        })
    }
}

/// Sleeps until `until`, returning `false` early if the feeder should stop.
fn pause(state: &GuiSharedState, stop: &AtomicBool, d: Duration) -> bool {
    let until = Instant::now() + d;
    loop {
        if stop.load(Ordering::Acquire) || state.is_interrupted() {
            return false;
        }
        let now = Instant::now();
        if now >= until {
            return true;
        }
        std::thread::sleep((until - now).min(Duration::from_millis(10)));
    }
}

/// Feeds `script` into `state` on the current thread; stops early when
/// `stop` is set or the program is interrupted.
pub fn feed(script: &KeyScript, state: &GuiSharedState, stop: &AtomicBool) {
    let hold = script.interval / 2;
    let rest = script.interval - hold;
    for item in &script.items {
        let ok = match *item {
            ScriptItem::Key(code) => {
                state.set_key_down(code, true);
                state.push_key_press(KeyPress::code(code));
                let held = pause(state, stop, hold);
                state.set_key_down(code, false);
                held && pause(state, stop, rest)
            }
            ScriptItem::Char(c) => {
                let code = map_char(c.encode_utf8(&mut [0; 4]));
                if let Some(code) = code {
                    state.set_key_down(code, true);
                }
                state.push_key_press(KeyPress::with_char(code, c));
                let held = pause(state, stop, hold);
                if let Some(code) = code {
                    state.set_key_down(code, false);
                }
                held && pause(state, stop, rest)
            }
            ScriptItem::Idle => pause(state, stop, script.interval),
            ScriptItem::Wait(d) => pause(state, stop, d),
            ScriptItem::Touch(x, y, d) => {
                state.set_touch(Some(Touch { x, y }));
                let held = pause(state, stop, d.unwrap_or(script.interval));
                state.set_touch(None);
                held && pause(state, stop, rest)
            }
        };
        if !ok {
            return;
        }
    }
}

/// A background thread feeding a key script into the GUI queues.
pub struct KeyFeeder {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl KeyFeeder {
    pub fn spawn(script: KeyScript, state: Arc<GuiSharedState>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = stop.clone();
        let thread = std::thread::Builder::new()
            .name("ppl-key-script".into())
            .spawn(move || feed(&script, &state, &stop_t))
            .expect("failed to spawn key script thread");
        Self {
            stop,
            thread: Some(thread),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Stops feeding and waits for the thread (it polls every 10 ms).
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for KeyFeeder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::GuiHost;
    use crate::keymap::{KEY_1, KEY_ENTER, KEY_ESC};
    use ppl_vm::host::{Dialog, DialogResult, Host, InputField};

    #[test]
    fn parse_all_item_kinds() {
        let s =
            KeyScript::parse("30, -1*2 ,wait:250,text:4.5,touch:10:20,touch:1:2:300,4*2").unwrap();
        assert_eq!(
            s.items,
            vec![
                ScriptItem::Key(30),
                ScriptItem::Idle,
                ScriptItem::Idle,
                ScriptItem::Wait(Duration::from_millis(250)),
                ScriptItem::Char('4'),
                ScriptItem::Char('.'),
                ScriptItem::Char('5'),
                ScriptItem::Touch(10, 20, None),
                ScriptItem::Touch(1, 2, Some(Duration::from_millis(300))),
                ScriptItem::Key(4),
                ScriptItem::Key(4),
            ]
        );
        assert_eq!(s.interval, DEFAULT_INTERVAL);
        assert_eq!(KeyScript::parse("").unwrap().items, vec![]);
    }

    #[test]
    fn parse_errors() {
        for bad in [
            "51",
            "-2",
            "x",
            "4*y",
            "wait:",
            "wait:-5",
            "touch:1",
            "touch:a:b",
        ] {
            assert!(KeyScript::parse(bad).is_err(), "{bad} should not parse");
        }
    }

    fn fast(s: &str) -> KeyScript {
        KeyScript {
            interval: Duration::from_millis(4),
            ..KeyScript::parse(s).unwrap()
        }
    }

    #[test]
    fn feed_queues_keys_chars_and_touch() {
        let state = Arc::new(GuiSharedState::new());
        feed(&fast("30,text:a1,4"), &state, &AtomicBool::new(false));
        let q: Vec<KeyPress> = state.key_queue.lock().unwrap().drain(..).collect();
        assert_eq!(
            q,
            vec![
                KeyPress::code(KEY_ENTER),
                KeyPress::with_char(None, 'a'),
                KeyPress::with_char(Some(KEY_1), '1'),
                KeyPress::code(KEY_ESC),
            ]
        );
        // Everything is released afterwards.
        assert!(state.held_keys.lock().unwrap().is_empty());
        assert_eq!(*state.touch_pos.lock().unwrap(), None);
    }

    #[test]
    fn touch_is_held_then_released() {
        let state = Arc::new(GuiSharedState::new());
        let mut f = KeyFeeder::spawn(
            KeyScript {
                interval: Duration::from_millis(10),
                ..KeyScript::parse("touch:12:34:300").unwrap()
            },
            state.clone(),
        );
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            GuiHost::new(state.clone()).touch().unwrap(),
            Some(Touch { x: 12, y: 34 })
        );
        f.stop();
        assert_eq!(*state.touch_pos.lock().unwrap(), None);
    }

    #[test]
    fn stop_and_interrupt_end_the_feeder_promptly() {
        let state = Arc::new(GuiSharedState::new());
        let mut f = KeyFeeder::spawn(KeyScript::parse("wait:60000,30").unwrap(), state.clone());
        let t0 = Instant::now();
        f.stop();
        assert!(t0.elapsed() < Duration::from_millis(500));
        assert!(state.key_queue.lock().unwrap().is_empty());

        let state = Arc::new(GuiSharedState::new());
        let f = KeyFeeder::spawn(KeyScript::parse("wait:60000").unwrap(), state.clone());
        state.interrupt();
        let t0 = Instant::now();
        while !f.is_finished() {
            assert!(t0.elapsed() < Duration::from_millis(500), "feeder hung");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn script_drives_input_dialog_through_host() {
        let state = Arc::new(GuiSharedState::new());
        let s2 = state.clone();
        let worker = std::thread::spawn(move || {
            GuiHost::new(s2).dialog(
                &Dialog::Input {
                    title: "T".into(),
                    fields: vec![
                        InputField {
                            label: "A".into(),
                            help: String::new(),
                            current: "1".into(),
                            string_only: false,
                        },
                        InputField {
                            label: "B".into(),
                            help: String::new(),
                            current: "2".into(),
                            string_only: false,
                        },
                    ],
                    error: None,
                    focus: 0,
                },
                &ppl_graphics::Grob::screen(),
            )
        });
        let _f = KeyFeeder::spawn(fast("text:42,12,text:7.5,30"), state);
        assert_eq!(
            worker.join().unwrap().unwrap(),
            DialogResult::Entered(vec!["42".into(), "7.5".into()])
        );
    }
}
