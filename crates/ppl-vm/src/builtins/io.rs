//! Terminal, dialogs, keyboard, touch and time.

use ppl_compiler::builtins::Builtin;
use ppl_graphics::{BLACK, WHITE, rgb, text};

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::host::{Dialog, DialogResult, DisplayMode, InputField};
use crate::machine::Machine;
use crate::value::Value;

fn text_of(m: &Machine, v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        other => other.display(m.settings.format()),
    }
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    match b {
        B::Print => {
            match args.first() {
                None => {
                    m.terminal.clear();
                    m.display = DisplayMode::Terminal;
                    m.host.clear_terminal();
                    m.screen_dirty = true;
                }
                Some(v) => {
                    let line = text_of(m, v);
                    m.print_line(line)?;
                }
            }
            Ok(Value::zero())
        }
        B::Msgbox => {
            let text = text_of(m, arg(args, 0, name)?);
            let screen = m.screen().clone();
            let r = m.blocking(|h| h.dialog(&Dialog::Message { text }, &screen))?;
            Ok(Value::from_bool(r != DialogResult::Cancelled))
        }
        B::Getkey => {
            let k = m.blocking(|h| h.poll_key())?;
            Ok(Value::from_i64(k.unwrap_or(-1)))
        }
        B::Iskeydown => {
            let k = arg(args, 0, name)?.expect_i64(name)?;
            let down = m.blocking(|h| h.is_key_down(k))?;
            Ok(Value::from_bool(down))
        }
        B::Wait => {
            let secs = match args.first() {
                Some(v) => v.expect_real(name)?.to_f64(),
                None => 0.0,
            };
            if secs > 0.0 {
                m.blocking(|h| h.sleep(secs))?;
                return Ok(Value::zero());
            }
            let timeout = if secs < 0.0 { Some(60.0) } else { None };
            let k = m.blocking(|h| h.wait_key(timeout))?;
            Ok(Value::from_i64(k.unwrap_or(-1)))
        }
        B::Mouse => {
            // Hardware-compatible shape: with no touch active, MOUSE()
            // returns an empty list — real programs (e.g. the FIREWORKS
            // fixture) guard with `SIZE(m) > 0` before indexing m[1,1],
            // so an idle MOUSE must never look non-empty (regression:
            // it used to return [[],[]], which crashed such programs).
            let t = m.blocking(|h| h.touch())?;
            let Some(t) = t else {
                return Ok(Value::list(Vec::new()));
            };
            let finger = Value::list(vec![
                Value::from_i64(t.x),
                Value::from_i64(t.y),
                Value::from_i64(t.x),
                Value::from_i64(t.y),
                Value::zero(),
            ]);
            let both = Value::list(vec![finger, Value::list(Vec::new())]);
            match args.first() {
                Some(i) => {
                    let idx = i.expect_i64(name)?;
                    let Value::List(l) = &both else {
                        unreachable!()
                    };
                    let Value::List(f) = &l[0] else {
                        unreachable!()
                    };
                    Ok(f.get((idx - 1).max(0) as usize)
                        .cloned()
                        .unwrap_or(Value::from_i64(-1)))
                }
                None => Ok(both),
            }
        }
        B::Disp => {
            // DISP(row, text): rows are 24 px high, 1–9 (approximation).
            let row = arg(args, 0, name)?.expect_i64(name)?.clamp(1, 10);
            let txt = text_of(m, arg(args, 1, name)?);
            let y = (row - 1) * 24;
            let px = m.settings.font_px;
            let g = m.grobs[0].as_mut().unwrap();
            g.fill_rect(0, y, 319, y + 23, WHITE);
            text::draw_text(g, &txt, 0, y + 4, px, BLACK, None, None);
            m.display = DisplayMode::Graphics;
            m.screen_dirty = true;
            Ok(Value::zero())
        }
        B::Freeze => {
            m.display = DisplayMode::Graphics;
            m.present_now();
            Ok(Value::zero())
        }
        B::Drawmenu => {
            // Six 53-pixel soft-key slots on the bottom 20 rows (approximation).
            let labels: Vec<String> = args.iter().map(|v| text_of(m, v)).collect();
            let px = 12.0;
            let g = m.grobs[0].as_mut().unwrap();
            for i in 0..6i64 {
                let x1 = i * 53;
                let x2 = if i == 5 { 319 } else { x1 + 51 };
                g.fill_rect(x1, 220, x2, 239, rgb(0x40, 0x40, 0x40));
                if let Some(l) = labels.get(i as usize) {
                    let w = text::measure(l, px);
                    text::draw_text(
                        g,
                        l,
                        x1 + ((x2 - x1 + 1) - w).max(0) / 2,
                        222,
                        px,
                        WHITE,
                        Some(x2 - x1),
                        None,
                    );
                }
            }
            m.display = DisplayMode::Graphics;
            m.screen_dirty = true;
            Ok(Value::zero())
        }
        B::Ticks => {
            let t = m.host.now_ms();
            Ok(Value::from_i64(t.floor() as i64))
        }
        B::Time => {
            let (_, _, _, h, mi, s) = m.host.local_time();
            let v = h as f64 + mi as f64 / 100.0 + s.floor() / 10000.0;
            Ok(Value::Real(ppl_math::Real::from_f64(v)?))
        }
        B::Date => {
            let (y, mo, d, ..) = m.host.local_time();
            let v = y as f64 + mo as f64 / 100.0 + d as f64 / 10000.0;
            Ok(Value::Real(ppl_math::Real::from_f64(v)?))
        }
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not an I/O command"),
        )),
    }
}

fn str_list(m: &Machine, v: Option<&Value>, n: usize) -> Vec<String> {
    match v {
        Some(Value::List(l)) => (0..n)
            .map(|i| l.get(i).map(|x| text_of(m, x)).unwrap_or_default())
            .collect(),
        Some(other) => {
            let mut out = vec![text_of(m, other)];
            out.resize(n, String::new());
            out
        }
        None => vec![String::new(); n],
    }
}

/// `INPUT(var | {vars}, [title], [labels], [help], [reset], [initial])`.
/// `args[0]` holds the current values of the referenced variables.
/// The reset and initial values (arguments 5 and 6) are accepted but not
/// applied yet: fields always start from the variables' current values.
pub fn input(m: &mut Machine, args: &[Value]) -> RtResult<(Value, Vec<Value>)> {
    let current = args[0].expect_list("INPUT")?;
    let n = current.len();
    let title = args.get(1).map(|v| text_of(m, v)).unwrap_or_default();
    let labels = str_list(m, args.get(2), n);
    let helps = str_list(m, args.get(3), n);
    let fields = (0..n)
        .map(|i| InputField {
            label: labels[i].clone(),
            help: helps[i].clone(),
            current: current[i].display(m.settings.format()),
        })
        .collect();
    let screen = m.screen().clone();
    let r = m.blocking(|h| h.dialog(&Dialog::Input { title, fields }, &screen))?;
    match r {
        DialogResult::Entered(texts) => {
            let mut out = Vec::new();
            for (i, t) in texts.iter().enumerate().take(n) {
                let v = if t.trim().is_empty() {
                    current[i].clone()
                } else {
                    m.eval_text(t)?
                };
                out.push(v);
            }
            while out.len() < n {
                out.push(current[out.len()].clone());
            }
            Ok((Value::from_i64(1), out))
        }
        _ => Ok((Value::zero(), current.iter().cloned().collect())),
    }
}

/// `CHOOSE(var, title, item1, item2, …)` or `CHOOSE(var, title, {items})`.
pub fn choose(m: &mut Machine, args: &[Value]) -> RtResult<(Value, Vec<Value>)> {
    let current = args[0].expect_list("CHOOSE")?;
    let title = args.get(1).map(|v| text_of(m, v)).unwrap_or_default();
    let items: Vec<String> = match &args[2..] {
        [Value::List(l)] => l.iter().map(|v| text_of(m, v)).collect(),
        rest => rest.iter().map(|v| text_of(m, v)).collect(),
    };
    let cur = current
        .first()
        .and_then(|v| v.expect_i64("CHOOSE").ok())
        .unwrap_or(1)
        .max(1) as usize;
    let screen = m.screen().clone();
    let r = m.blocking(|h| {
        h.dialog(
            &Dialog::Choose {
                title,
                items: items.clone(),
                current: cur,
            },
            &screen,
        )
    })?;
    match r {
        DialogResult::Chosen(i) if i >= 1 && i <= items.len() => {
            Ok((Value::from_i64(i as i64), vec![Value::from_i64(i as i64)]))
        }
        DialogResult::Chosen(i) => Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            format!("choice {i} is out of range"),
        )),
        _ => Ok((Value::zero(), current.iter().cloned().collect())),
    }
}
