//! The boundary between the VM and the outside world (terminal, window,
//! keyboard, clock). The VM never touches the host OS directly.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ppl_graphics::Grob;

use crate::error::{ErrorKind, RuntimeError};

/// Modal interactions drawn by the calculator.
#[derive(Clone, Debug)]
pub enum Dialog {
    Message {
        text: String,
    },
    Choose {
        title: String,
        items: Vec<String>,
        current: usize,
    },
    Input {
        title: String,
        fields: Vec<InputField>,
    },
}

#[derive(Clone, Debug)]
pub struct InputField {
    pub label: String,
    pub help: String,
    /// Current value, formatted.
    pub current: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DialogResult {
    /// OK pressed (message box) or nothing to report.
    Ok,
    Cancelled,
    /// 1-based choice index.
    Chosen(usize),
    /// Text typed for each input field.
    Entered(Vec<String>),
}

/// Touch/mouse state in pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub x: i64,
    pub y: i64,
}

/// What a host shows on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Graphics,
    Terminal,
}

pub trait Host {
    /// One `PRINT` line.
    fn print(&mut self, text: &str);
    fn clear_terminal(&mut self) {}
    /// The screen changed; `terminal` is set when the terminal view is shown.
    fn present(&mut self, _screen: &Grob, _terminal: Option<&[String]>) {}
    /// `GETKEY`: next key code, or `None` (-1) when no key is pressed.
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError>;
    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError>;
    /// `WAIT(0)` / `WAIT(-1)`: block until a key (or timeout).
    fn wait_key(&mut self, timeout_s: Option<f64>) -> Result<Option<i64>, RuntimeError>;
    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError>;
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError>;
    fn dialog(&mut self, dialog: &Dialog, screen: &Grob) -> Result<DialogResult, RuntimeError>;
    /// Milliseconds since an arbitrary epoch (for `TICKS`).
    fn now_ms(&mut self) -> f64;
    /// Wall-clock date and time `(y, m, d, h, mi, s)`.
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64);
    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

/// One scripted event for headless runs.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptEvent {
    /// GETKEY returns this code (`-1` = no key this poll).
    Key(i64),
    Touch(i64, i64),
}

/// Deterministic host for tests and CI: scripted input, virtual clock,
/// captured output.
#[derive(Debug, Default)]
pub struct HeadlessHost {
    pub output: Vec<String>,
    pub echo: bool,
    pub keys: VecDeque<ScriptEvent>,
    pub inputs: VecDeque<String>,
    pub choices: VecDeque<usize>,
    pub dialogs: Vec<String>,
    pub clock_ms: f64,
    pub key_down: Option<i64>,
    pub last_screen: Option<Grob>,
    pub interrupt: Option<Arc<AtomicBool>>,
}

impl HeadlessHost {
    pub fn new() -> HeadlessHost {
        HeadlessHost::default()
    }

    fn exhausted(&self, what: &str) -> RuntimeError {
        RuntimeError::new(
            ErrorKind::ScriptExhausted,
            format!(
                "{what} needs input but the headless script is empty (pass --keys / --input / --choose)"
            ),
        )
    }

    fn next_key_event(&mut self, what: &str) -> Result<ScriptEvent, RuntimeError> {
        if self
            .interrupt
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
        {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        self.keys.pop_front().ok_or_else(|| self.exhausted(what))
    }
}

impl Host for HeadlessHost {
    fn print(&mut self, text: &str) {
        if self.echo {
            println!("{text}");
        }
        self.output.push(text.to_string());
    }

    fn present(&mut self, screen: &Grob, _terminal: Option<&[String]>) {
        self.last_screen = Some(screen.clone());
    }

    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.clock_ms += 1.0;
        match self.next_key_event("GETKEY")? {
            ScriptEvent::Key(-1) => {
                self.key_down = None;
                Ok(None)
            }
            ScriptEvent::Key(k) => {
                self.key_down = Some(k);
                Ok(Some(k))
            }
            ScriptEvent::Touch(..) => Ok(None),
        }
    }

    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.clock_ms += 1.0;
        match self.next_key_event("ISKEYDOWN")? {
            ScriptEvent::Key(k) => Ok(k == key),
            ScriptEvent::Touch(..) => Ok(false),
        }
    }

    fn wait_key(&mut self, _timeout_s: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        loop {
            match self.next_key_event("WAIT")? {
                ScriptEvent::Key(-1) => continue,
                ScriptEvent::Key(k) => return Ok(Some(k)),
                ScriptEvent::Touch(..) => return Ok(None),
            }
        }
    }

    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        if self
            .interrupt
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
        {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        self.clock_ms += seconds.max(0.0) * 1000.0;
        Ok(())
    }

    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        match self.keys.front() {
            Some(ScriptEvent::Touch(x, y)) => {
                let t = Touch { x: *x, y: *y };
                self.keys.pop_front();
                Ok(Some(t))
            }
            _ => Ok(None),
        }
    }

    fn dialog(&mut self, dialog: &Dialog, _screen: &Grob) -> Result<DialogResult, RuntimeError> {
        match dialog {
            Dialog::Message { text } => {
                self.dialogs.push(format!("MSGBOX: {text}"));
                Ok(DialogResult::Ok)
            }
            Dialog::Choose { title, items, .. } => {
                let pick = self
                    .choices
                    .pop_front()
                    .ok_or_else(|| self.exhausted("CHOOSE"))?;
                self.dialogs
                    .push(format!("CHOOSE: {title} {items:?} -> {pick}"));
                if pick == 0 {
                    Ok(DialogResult::Cancelled)
                } else {
                    Ok(DialogResult::Chosen(pick))
                }
            }
            Dialog::Input { title, fields } => {
                let mut out = Vec::new();
                for f in fields {
                    let v = self
                        .inputs
                        .pop_front()
                        .ok_or_else(|| self.exhausted("INPUT"))?;
                    out.push(v);
                    let _ = f;
                }
                self.dialogs.push(format!("INPUT: {title} -> {out:?}"));
                Ok(DialogResult::Entered(out))
            }
        }
    }

    fn now_ms(&mut self) -> f64 {
        self.clock_ms
    }

    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        // Fixed date keeps headless runs reproducible.
        (2026, 1, 1, 0, 0, self.clock_ms / 1000.0)
    }

    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        self.interrupt.clone()
    }
}

/// Parses a key script: `4`, `-1*30,4`, `touch:10:20`.
pub fn parse_key_script(s: &str) -> Result<Vec<ScriptEvent>, String> {
    let mut out = Vec::new();
    for item in s.split(',').map(str::trim).filter(|x| !x.is_empty()) {
        if let Some(rest) = item.strip_prefix("touch:") {
            let mut parts = rest.split(':');
            let x = parts
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or(format!("bad touch event `{item}`"))?;
            let y = parts
                .next()
                .and_then(|v| v.parse().ok())
                .ok_or(format!("bad touch event `{item}`"))?;
            out.push(ScriptEvent::Touch(x, y));
            continue;
        }
        let (code, count) = match item.split_once('*') {
            Some((c, n)) => (
                c,
                n.parse::<usize>()
                    .map_err(|_| format!("bad repeat count in `{item}`"))?,
            ),
            None => (item, 1),
        };
        let code: i64 = code.parse().map_err(|_| format!("bad key code `{code}`"))?;
        if !(-1..=50).contains(&code) {
            return Err(format!("key code {code} is outside the Prime's 0–50 range"));
        }
        out.extend(std::iter::repeat_n(ScriptEvent::Key(code), count));
    }
    Ok(out)
}
