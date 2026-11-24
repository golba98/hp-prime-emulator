//! Terminal, dialogs, keyboard, touch and time.

use ppl_compiler::builtins::Builtin;
use ppl_graphics::{BLACK, WHITE, text};

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::host::{
    Dialog, DialogResult, DisplayMode, INPUT_LINES_PER_PAGE, InputControl, InputField,
    InputPosition, MatrixDialog,
};
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
            let ok_cancel = args
                .get(1)
                .map(|v| v.expect_real(name).map(|n| !n.is_zero()))
                .transpose()?
                .unwrap_or(false);
            let r =
                m.blocking(|h| h.dialog(&Dialog::MessageChoice { text, ok_cancel }, &screen))?;
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
            let index = args
                .first()
                .map(|value| {
                    if !matches!(value, Value::Real(_) | Value::Int(_)) {
                        return Err(RuntimeError::kind(ErrorKind::InvalidInput));
                    }
                    let number = value.expect_real(name)?;
                    if !number.is_integer() {
                        return Err(RuntimeError::kind(ErrorKind::InvalidInput));
                    }
                    let index = number
                        .to_i64()
                        .filter(|index| (0..10).contains(index))
                        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
                    Ok(index as usize)
                })
                .transpose()?;
            let state = m.blocking(|host| host.mouse_state())?;
            let word = |value: i16, signed| {
                Value::Int(
                    ppl_math::HpInt::new_with_width(
                        i128::from(value),
                        if signed { -16 } else { 16 },
                        m.settings.int_base,
                    )
                    .expect("bounded pointer word")
                    .with_system_base(true),
                )
            };
            let pointers = state.pointers.map(|pointer| {
                pointer.map(|p| {
                    [
                        word(p.x, true),
                        word(p.y, true),
                        word(p.origin_x, true),
                        word(p.origin_y, true),
                        word(p.kind as i16, false),
                    ]
                })
            });
            if let Some(index) = index {
                return Ok(pointers[index / 5]
                    .as_ref()
                    .map(|pointer| pointer[index % 5].clone())
                    .unwrap_or_else(|| Value::from_i64(-1)));
            }
            Ok(Value::list(
                pointers
                    .into_iter()
                    .map(|pointer| Value::list(pointer.map(Vec::from).unwrap_or_default()))
                    .collect(),
            ))
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
            m.blocking(|host| host.wait_key(None))?;
            Ok(Value::zero())
        }
        B::Drawmenu => {
            let values = match args {
                [Value::List(items)] => items.as_slice(),
                _ => args,
            };
            let labels = values
                .iter()
                .take(6)
                .map(|value| {
                    if value.is_string() {
                        return Ok(value.expect_string_units("DRAWMENU label")?.into_owned());
                    }
                    super::strings::menu_label(m, value)?
                        .expect_string_units("DRAWMENU label")
                        .map(|units| units.into_owned())
                })
                .collect::<RtResult<Vec<_>>>()?;
            let borrowed = labels.iter().map(Vec::as_slice).collect::<Vec<_>>();
            ppl_graphics::softkeys::draw_units(m.grobs[0].as_mut().unwrap(), &borrowed)
                .map_err(|error| RuntimeError::new(ErrorKind::Unsupported, error.to_string()))?;
            m.display = DisplayMode::Graphics;
            m.screen_dirty = true;
            Ok(Value::from_i64(1))
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

/// Parse an explicitly supplied edit-field descriptor. Scalar and nonnumeric
/// list descriptors can denote other firmware controls; never turn them into
/// unrestricted text inputs. Omitted descriptors are handled separately.
fn allowed_types(spec: &Value) -> RtResult<Vec<i64>> {
    let unsupported = || {
        RuntimeError::new(
            ErrorKind::Unsupported,
            "Native INPUT supports edit fields with a vector/list of TYPE codes; checkbox, radio and choice descriptors require G1 Firmware mode",
        )
    };
    let values = match spec {
        Value::Matrix(mx) if mx.vector => mx.data.as_slice(),
        Value::Matrix(_) => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "INPUT TYPE codes require a one-dimensional vector, not a matrix",
            ));
        }
        Value::List(values) => values.as_slice(),
        _ => return Err(unsupported()),
    };
    let mut codes = Vec::with_capacity(values.len());
    for value in values {
        let number = value
            .expect_real("INPUT TYPE code")
            .map_err(|_| unsupported())?;
        if number != number.trunc() {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "INPUT TYPE codes must be integers; fractional codes are not rounded",
            ));
        }
        let code = number.to_i64().ok_or_else(|| {
            RuntimeError::new(ErrorKind::BadArgValue, "INPUT TYPE code is out of range")
        })?;
        if !matches!(code, -1..=9) {
            return Err(RuntimeError::new(
                ErrorKind::Unsupported,
                format!("Native INPUT does not support TYPE code {code}; use G1 Firmware mode"),
            ));
        }
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    if codes.contains(&-1) {
        codes.clear();
    }
    Ok(codes)
}

/// Preserve omitted edit descriptors, typed vectors, checkbox groups and choice lists.
fn input_specs(
    m: &Machine,
    value: &Value,
    count: usize,
) -> RtResult<(Vec<Vec<i64>>, Vec<InputControl>)> {
    let fields = value.expect_list("INPUT field descriptors")?;
    if fields.len() != count {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "INPUT field descriptor count does not match its variables",
        ));
    }
    let mut allowed = Vec::with_capacity(count);
    let mut controls = Vec::with_capacity(count);
    let mut radio_end = 0;
    let mut radio_start = 0;
    for (index, field) in fields.iter().enumerate() {
        let optional = field.expect_list("INPUT optional field descriptor")?;
        let (types, mut control) = match optional.as_slice() {
            [] => (Vec::new(), InputControl::Edit),
            [Value::Real(number)] => {
                if *number > ppl_math::Real::ONE {
                    let n = number
                        .to_i64()
                        .filter(|_| *number == number.trunc())
                        .and_then(|n| usize::try_from(n).ok())
                        .filter(|n| index.checked_add(*n).is_some_and(|end| end <= count))
                        .ok_or_else(|| {
                            RuntimeError::new(
                                ErrorKind::BadArgValue,
                                "INPUT radio group exceeds its fields or has a fractional size",
                            )
                        })?;
                    if index < radio_end {
                        return Err(RuntimeError::new(
                            ErrorKind::BadArgValue,
                            "INPUT radio groups overlap",
                        ));
                    }
                    radio_start = index;
                    radio_end = index + n;
                }
                (Vec::new(), InputControl::Checkbox { group: None })
            }
            [Value::List(items)] => {
                if items.is_empty() {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "INPUT choose field needs at least one item",
                    ));
                }
                (
                    Vec::new(),
                    InputControl::Choice {
                        items: items.iter().map(|v| text_of(m, v)).collect(),
                    },
                )
            }
            [spec] => (allowed_types(spec)?, InputControl::Edit),
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "INPUT optional field descriptor is malformed",
                ));
            }
        };
        if index < radio_end {
            if !matches!(control, InputControl::Checkbox { .. }) {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "INPUT radio group requires consecutive checkbox controls",
                ));
            }
            control = InputControl::Checkbox {
                group: Some(radio_start),
            };
        }
        allowed.push(types);
        controls.push(control);
    }
    Ok((allowed, controls))
}

fn control_value(
    m: &mut Machine,
    text: &str,
    current: &Value,
    allowed: &[i64],
    control: &InputControl,
) -> RtResult<Value> {
    match control {
        InputControl::Edit => field_value(m, text, current, allowed),
        InputControl::Checkbox { .. } => {
            let value = m
                .eval_text(text)
                .map_err(|error| field_error(error, "Checkbox requires 0 or 1"))?;
            let value = value
                .expect_real("INPUT checkbox")
                .map_err(|error| field_error(error, "Checkbox requires 0 or 1"))?;
            if value != ppl_math::Real::ZERO && value != ppl_math::Real::ONE {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "Checkbox requires 0 or 1",
                ));
            }
            Ok(Value::real(value))
        }
        InputControl::Choice { items } => {
            let value = m
                .eval_text(text)
                .map_err(|error| field_error(error, "Choose a valid item"))?;
            let index = value
                .expect_i64("INPUT choice")
                .map_err(|error| field_error(error, "Choose a valid item"))?;
            if index < 1 || index as usize > items.len() {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "Choose a valid item",
                ));
            }
            Ok(Value::from_i64(index))
        }
    }
}

/// The value typed into one field, or the message the dialog shows when
/// the entry is rejected.
fn field_value(m: &mut Machine, text: &str, current: &Value, allowed: &[i64]) -> RtResult<Value> {
    // A string-only field stores the text verbatim and never evaluates it
    // (HP: "the input will be shown WITHOUT the quotes and save directly as
    // a string"). UNVERIFIED on G1: an empty string-only entry yields "".
    if allowed == [2] {
        return Ok(Value::str(text));
    }
    if text.trim().is_empty() {
        return Ok(current.clone());
    }
    let v = if text == current.string_form(m.settings.format()) {
        current.clone()
    } else {
        m.eval_text(text).map_err(|e| {
            // The field already shows the text: keep only the reason.
            let prefix = format!("cannot evaluate `{text}`: ");
            let reason = e.detail.strip_prefix(&prefix).unwrap_or(&e.detail);
            let message = format!("Syntax error: {reason}");
            field_error(e, &message)
        })?
    };
    if !allowed.is_empty() && !allowed.contains(&crate::cas::type_code(m, &v)?) {
        return Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("Invalid input: a {} is not allowed here", v.type_name()),
        ));
    }
    Ok(v)
}

/// Host stops retain their kind and trace, and must never become a re-prompt.
fn field_error(mut error: RuntimeError, message: &str) -> RuntimeError {
    if error.kind.catchable() {
        error.detail = message.to_owned();
    }
    error
}

/// Validate the documented on-screen layout subset before opening a dialog.
/// Out-of-screen/fractional-line behavior is not hardware verified.
fn input_positions(value: Option<&Value>, count: usize) -> RtResult<Vec<Option<InputPosition>>> {
    let Some(Value::List(values)) = value else {
        return Ok(vec![None; count]);
    };
    let invalid = || {
        RuntimeError::new(
            ErrorKind::Unsupported,
            "Native INPUT positions require {start%,width%,line}: on-screen percentages, positive width and a nonnegative integer line; other layouts require Firmware mode",
        )
    };
    let mut positions = Vec::with_capacity(count);
    for value in values.iter() {
        let Value::List(optional) = value else {
            return Err(invalid());
        };
        if optional.is_empty() {
            positions.push(None);
            continue;
        }
        let [Value::List(parts)] = optional.as_slice() else {
            return Err(invalid());
        };
        if parts.len() != 3 {
            return Err(invalid());
        }
        let start_value = parts[0].expect_real("INPUT position")?;
        let start = start_value.to_f64();
        let width = parts[1].expect_real("INPUT position")?.to_f64();
        let line = parts[2].expect_real("INPUT position")?;
        if start_value.is_negative()
            || !(0.0..100.0).contains(&start)
            || !(0.0..=100.0).contains(&width)
            || width == 0.0
            || start + width > 100.0
            || line != line.trunc()
        {
            return Err(invalid());
        }
        let line = line
            .to_i64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(invalid)?;
        positions.push(Some(InputPosition {
            start_percent: start,
            width_percent: width,
            line,
        }));
    }
    if positions.len() != count {
        return Err(invalid());
    }
    if positions.iter().any(Option::is_some) && positions.iter().any(Option::is_none) {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "INPUT positions must be specified for either all fields or none",
        ));
    }
    Ok(positions)
}

/// `INPUT(var | {vars}, [title], [labels], [help], [reset], [initial])`.
/// `args[0]` holds the current values of the referenced variables and
/// `args[1]` (added by the compiler) preserves each optional field descriptor.
/// A rejected entry reopens the dialog with an error instead of stopping
/// the program, the way the calculator keeps its form open.
/// Hidden arguments 2 and 3 preserve scalar/list form and field positions.
pub fn input(m: &mut Machine, args: &[Value]) -> RtResult<(Value, Vec<Value>)> {
    let current = args[0].expect_list("INPUT")?;
    let n = current.len();
    let (allowed, controls) = input_specs(m, arg(args, 1, "INPUT")?, n)?;
    let scalar = args
        .get(2)
        .is_some_and(|v| v.expect_i64("INPUT").ok() == Some(1));
    let positions = input_positions(args.get(3), n)?;
    let titles: Vec<String> = match args.get(4) {
        Some(Value::List(values)) => values.iter().map(|v| text_of(m, v)).collect(),
        Some(value) => vec![text_of(m, value)],
        None => Vec::new(),
    };
    let title = titles.first().cloned().unwrap_or_default();
    let labels = str_list(m, args.get(5), n);
    let helps = str_list(m, args.get(6), n);
    let defaults = |value: Option<&Value>| -> RtResult<Option<Vec<Value>>> {
        match value {
            None => Ok(None),
            Some(v) if scalar => Ok(Some(vec![v.clone()])),
            Some(Value::List(values)) if values.len() <= n => {
                let mut defaults = current.to_vec();
                for (target, value) in defaults.iter_mut().zip(values.iter()) {
                    *target = value.clone();
                }
                Ok(Some(defaults))
            }
            Some(Value::List(_)) => Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "INPUT reset/initial list exceeds the number of fields",
            )),
            Some(value) => {
                let mut defaults = current.to_vec();
                if let Some(first) = defaults.first_mut() {
                    *first = value.clone();
                }
                Ok(Some(defaults))
            }
        }
    };
    let reset = defaults(args.get(7))?;
    let initial = defaults(args.get(8))?;
    let opts = m.settings.format();
    let mut texts: Vec<String> = (0..n)
        .map(|i| {
            if allowed[i] == [2] {
                initial
                    .as_ref()
                    .map_or(&current[i], |v| &v[i])
                    .display(opts)
            } else {
                initial
                    .as_ref()
                    .map_or(&current[i], |v| &v[i])
                    .string_form(opts)
            }
        })
        .collect();
    let mut error = None;
    let mut focus = 0;
    let mut edited = vec![false; n];
    let mut validated: Vec<Option<Value>> = vec![None; n];
    loop {
        let fields = (0..n)
            .map(|i| InputField {
                label: labels[i].clone(),
                help: helps[i].clone(),
                current: texts[i].clone(),
                string_only: allowed[i] == [2],
                control: controls[i].clone(),
                position: positions[i],
                page_title: titles
                    .get(positions[i].map_or(i, |p| p.line) / INPUT_LINES_PER_PAGE)
                    .cloned(),
                reset_text: reset.as_ref().map(|v| {
                    if allowed[i] == [2] {
                        v[i].display(opts)
                    } else {
                        v[i].string_form(opts)
                    }
                }),
            })
            .collect();
        let dialog = Dialog::Input {
            report_edits: true,
            title: title.clone(),
            fields,
            error: error.take(),
            focus,
        };
        let screen = m.screen().clone();
        let entered = match m.blocking(|h| h.dialog(&dialog, &screen))? {
            DialogResult::Entered(values) => values.into_iter().map(Some).collect(),
            DialogResult::InputEntered(values) if values.len() == n => values,
            DialogResult::InputEntered(_) => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "INPUT host returned incomplete field metadata",
                ));
            }
            _ => return Ok((Value::zero(), current.iter().cloned().collect())),
        };
        // Preserve every edit before validating any field. A failure in the
        // first field must not discard text entered in later fields.
        for (i, value) in entered.iter().enumerate().take(n) {
            if let Some(typed) = value {
                texts[i] = typed.clone();
                edited[i] = true;
                validated[i] = None;
            }
        }
        let mut out = Vec::with_capacity(n);
        for (i, text) in texts.iter().enumerate() {
            if let Some(value) = &validated[i] {
                out.push(value.clone());
                continue;
            } else if !edited[i] {
                out.push(
                    initial
                        .as_ref()
                        .map_or(&current[i], |values| &values[i])
                        .clone(),
                );
                continue;
            }
            match control_value(m, text, &current[i], &allowed[i], &controls[i]) {
                Ok(v) => {
                    validated[i] = Some(v.clone());
                    out.push(v);
                }
                Err(failure) => {
                    if !failure.kind.catchable() {
                        return Err(failure);
                    }
                    error = Some(failure.detail);
                    focus = i;
                    break;
                }
            }
        }
        if out.len() == n {
            let mut selected = std::collections::BTreeSet::new();
            for (i, control) in controls.iter().enumerate() {
                if let InputControl::Checkbox { group: Some(group) } = control {
                    if !out[i].expect_real("INPUT checkbox")?.is_zero() && !selected.insert(*group)
                    {
                        error = Some("Only one radio option may be selected".into());
                        focus = i;
                        break;
                    }
                }
            }
            if error.is_none() {
                return Ok((Value::from_i64(1), out));
            }
        }
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

/// EDITMAT returns the original matrix on Cancel and the edited matrix on OK.
/// Reference writeback is handled by the compiler/VM, so expressions also work.
pub fn editmat(m: &mut Machine, args: &[Value]) -> RtResult<Value> {
    let original = arg(args, 0, "EDITMAT")?;
    let Value::Matrix(matrix) = original else {
        return Err(RuntimeError::new(
            ErrorKind::BadArgType,
            "EDITMAT expects a matrix or vector",
        ));
    };
    let header_list = |value: Option<&Value>| -> RtResult<Vec<String>> {
        match value {
            None => Ok(Vec::new()),
            Some(Value::List(items)) => items
                .iter()
                .map(|v| v.expect_str("EDITMAT header").map(|s| s.to_string()))
                .collect(),
            _ => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "EDITMAT row/column headers must be lists of strings",
            )),
        }
    };
    let (title, row_headers, column_headers) = match args.get(1) {
        None => ("Matrix".into(), Vec::new(), Vec::new()),
        Some(Value::Str(title)) => (title.to_string(), Vec::new(), Vec::new()),
        Some(Value::List(items)) if (1..=3).contains(&items.len()) => (
            items[0].expect_str("EDITMAT title")?.to_string(),
            header_list(items.get(1))?,
            header_list(items.get(2))?,
        ),
        _ => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "EDITMAT title must be a string or {title,row_headers,column_headers}",
            ));
        }
    };
    let read_only = args
        .get(2)
        .map(|v| v.expect_real("EDITMAT read only").map(|n| !n.is_zero()))
        .transpose()?
        .unwrap_or(false);
    let mut dialog = MatrixDialog {
        title,
        row_headers,
        column_headers,
        rows: matrix.rows,
        cols: matrix.cols,
        vector: matrix.vector,
        cells: matrix
            .data
            .iter()
            .map(|v| v.string_form(m.settings.format()))
            .collect(),
        read_only,
        error: None,
        focus: 0,
    };
    let screen = m.screen().clone();
    loop {
        let response = m.blocking(|h| h.dialog(&Dialog::Matrix(dialog.clone()), &screen))?;
        if read_only {
            return Ok(original.clone());
        }
        let (rows, cols, entered) = match response {
            DialogResult::Cancelled => return Ok(original.clone()),
            DialogResult::Entered(entered) if entered.len() == matrix.data.len() => {
                (matrix.rows, matrix.cols, entered)
            }
            DialogResult::MatrixEdited { rows, cols, cells }
                if rows > 0
                    && cols > 0
                    && rows.checked_mul(cols) == Some(cells.len())
                    && cells.len() <= 1_000_000
                    && (!matrix.vector || rows == 1) =>
            {
                (rows, cols, cells)
            }
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "EDITMAT host returned an incomplete matrix",
                ));
            }
        };
        dialog.rows = rows;
        dialog.cols = cols;
        dialog.cells = entered;
        dialog.error = None;
        let mut edited = crate::value::Matrix::zeros(rows, cols);
        edited.vector = matrix.vector;
        for (index, cell) in dialog.cells.iter().enumerate() {
            let value = if cell.trim().is_empty() {
                Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "Enter a numeric expression",
                ))
            } else {
                field_value(m, cell, &Value::zero(), &[0, 1, 3])
            };
            match value {
                Ok(Value::Int(value)) => edited.data[index] = Value::real(value.to_real()),
                Ok(value) => edited.data[index] = value,
                Err(failure) => {
                    if !failure.kind.catchable() {
                        return Err(failure);
                    }
                    dialog.error = Some(failure.detail);
                    dialog.focus = index;
                    break;
                }
            }
        }
        if dialog.error.is_none() {
            return Ok(Value::Matrix(std::rc::Rc::new(edited)));
        }
    }
}

/// Interactive list editor; the original list survives cancellation without partial writes.
pub fn editlist(m: &mut Machine, args: &[Value]) -> RtResult<Value> {
    let original = arg(args, 0, "EDITLIST")?;
    let values = original.expect_list("EDITLIST")?;
    let mut items: Vec<String> = values
        .iter()
        .map(|v| v.string_form(m.settings.format()))
        .collect();
    let screen = m.screen().clone();
    let mut error = None;
    let mut focus = 0;
    loop {
        let dialog = Dialog::List {
            title: "List".into(),
            items: items.clone(),
            error: error.take(),
            focus,
        };
        match m.blocking(|h| h.dialog(&dialog, &screen))? {
            DialogResult::Cancelled => return Ok(original.clone()),
            DialogResult::Entered(entered) => {
                items = entered;
                let mut result = Vec::with_capacity(items.len());
                let mut valid = true;
                for (index, item) in items.iter().enumerate() {
                    match field_value(m, item, &Value::zero(), &[]) {
                        Ok(value) => result.push(value),
                        Err(failure) => {
                            if !failure.kind.catchable() {
                                return Err(failure);
                            }
                            error = Some(failure.detail);
                            focus = index;
                            valid = false;
                            break;
                        }
                    }
                }
                if valid {
                    return Ok(Value::list(result));
                }
            }
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "EDITLIST host returned an invalid result",
                ));
            }
        }
    }
}
