//! Built-in commands, grouped by category.

pub mod graphics;
pub mod io;
pub mod lists;
pub mod math;
pub mod matrices;
pub mod strings;

use ppl_compiler::builtins::{Builtin, spec};

use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::value::Value;

pub(crate) fn arg<'a>(args: &'a [Value], i: usize, cmd: &str) -> RtResult<&'a Value> {
    args.get(i).ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::InvalidInput,
            format!("{cmd}: missing argument {}", i + 1),
        )
    })
}

pub fn call(m: &mut Machine, b: Builtin, args: Vec<Value>) -> RtResult<Value> {
    use Builtin as B;
    let name = spec(b).name;
    if args.iter().any(|v| matches!(v, Value::Symbolic(_)))
        && spec(b).category == ppl_compiler::builtins::Category::Math
    {
        if let Some(result) = crate::ops::infinity::builtin(b, &args) {
            return result;
        }
        let function = if b == B::Approx {
            "evalf"
        } else if b == B::Exact {
            "exact"
        } else {
            spec(b).name
        };
        return crate::cas::evaluate_math(
            m,
            &format!(
                "{}({})",
                function.to_ascii_lowercase(),
                args.iter()
                    .map(crate::cas::expression)
                    .collect::<RtResult<Vec<_>>>()?
                    .join(",")
            ),
        );
    }
    match b {
        B::SheetSum | B::SheetAverage if m.active_app == "Spreadsheet" => {
            math::sheet_aggregate(&args, &m.settings.clone(), b == B::SheetAverage, || {
                m.check_limits()
            })
        }
        B::PredX | B::PredY if m.active_app == "Spreadsheet" => math::call(m, b, name, &args),
        B::SheetSum | B::SheetAverage | B::PredX | B::PredY => Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            format!("{name} requires the Spreadsheet app"),
        )),
        B::Do1VStats
        | B::Do2VStats
        | B::SetSample
        | B::SetFreq
        | B::Check
        | B::Uncheck
        | B::Ischeck => {
            let request = crate::host::AppVariableRequest {
                app: m.active_app.clone(),
                name: name.to_owned(),
                indices: args.iter().map(crate::PortableValue::from).collect(),
                value: None,
            };
            let context = m.presentation_snapshot();
            m.host
                .app_variable_with_context(request, context)
                .map(|value| value.to_value())
        }
        B::Abs
        | B::Sqrt
        | B::Sq
        | B::Exp
        | B::Ln
        | B::Log
        | B::Alog
        | B::Expm1
        | B::Lnp1
        | B::Sin
        | B::Cos
        | B::Tan
        | B::Asin
        | B::Acos
        | B::Atan
        | B::Csc
        | B::Sec
        | B::Cot
        | B::Sinh
        | B::Cosh
        | B::Tanh
        | B::Asinh
        | B::Acosh
        | B::Atanh
        | B::Floor
        | B::Ceiling
        | B::Round
        | B::Truncate
        | B::Ip
        | B::Fp
        | B::Sign
        | B::Max
        | B::Min
        | B::Comb
        | B::Perm
        | B::Random
        | B::Randint
        | B::Randnorm
        | B::Randseed
        | B::Mant
        | B::Xpon
        | B::Re
        | B::Im
        | B::Conj
        | B::Arg
        | B::Approx
        | B::Hms
        | B::HmsFrom
        | B::Nthroot
        | B::Factorial
        | B::Mean
        | B::Median
        | B::Stddev
        | B::Ifte
        | B::Bitand
        | B::Bitor
        | B::Bitxor
        | B::Bitnot
        | B::Bitsl
        | B::Bitsr
        | B::RtoB
        | B::BtoR
        | B::Setbase
        | B::Getbase
        | B::Setbits
        | B::Getbits => math::call(m, b, name, &args),
        B::Asc
        | B::Char
        | B::Dim
        | B::Instring
        | B::Left
        | B::Right
        | B::Mid
        | B::Rotate
        | B::StringFn
        | B::Upper
        | B::Lower
        | B::Replace
        | B::Expr => strings::call(m, b, name, &args),
        B::Concat
        | B::Makelist
        | B::Size
        | B::Sort
        | B::Reverse
        | B::Pos
        | B::SumList
        | B::ProdList
        | B::DeltaList
        | B::Equal
        | B::Head
        | B::Tail
        | B::Append => lists::call(m, b, name, &args),
        B::Makemat
        | B::Det
        | B::Trn
        | B::Idenmat
        | B::Rank
        | B::Colnorm
        | B::Rownorm
        | B::Inverse
        | B::Addrow
        | B::Addcol
        | B::Delrow
        | B::Delcol
        | B::Redim
        | B::Dot
        | B::Cross => matrices::call(m, b, name, &args),
        B::RectP
        | B::Rect
        | B::LineP
        | B::Line
        | B::PixonP
        | B::Pixon
        | B::PixoffP
        | B::Pixoff
        | B::GetpixP
        | B::Getpix
        | B::CircleP
        | B::Circle
        | B::FillcircleP
        | B::ArcP
        | B::Arc
        | B::TextoutP
        | B::Textout
        | B::BlitP
        | B::Blit
        | B::DimgrobP
        | B::Dimgrob
        | B::SubgrobP
        | B::Subgrob
        | B::InvertP
        | B::FillpolyP
        | B::Fillpoly
        | B::TriangleP
        | B::Triangle
        | B::GrobwP
        | B::GrobhP
        | B::Grobw
        | B::Grobh
        | B::Rgb
        | B::CtoPx
        | B::PxToC => graphics::call(m, b, name, &args),
        B::Print
        | B::Msgbox
        | B::Getkey
        | B::Iskeydown
        | B::Wait
        | B::Mouse
        | B::Disp
        | B::Freeze
        | B::Drawmenu
        | B::Ticks
        | B::Time
        | B::Date => io::call(m, b, name, &args),
        B::Type => Ok(Value::from_i64(crate::cas::type_code(
            m,
            arg(&args, 0, name)?,
        )?)),
        // `Notes` / `Programs` system variables: with no argument, the list
        // of object names; with one, the content (empty when absent — the
        // Prime does not raise for a missing object).
        B::Notes | B::Programs => {
            let store = if b == B::Notes { &m.notes } else { &m.programs };
            if args.is_empty() {
                let names = store.keys().map(|k| Value::str(k.clone())).collect();
                Ok(Value::list(names))
            } else {
                let n = match arg(&args, 0, name)? {
                    Value::Str(s) => s.to_string(),
                    other => other.display(m.settings.format()),
                };
                // HARDWARE-VERIFIED (2026-10-23, G1 build 15157, probe
                // ZZT_PROBE2): reading an object that does not exist raises
                // a catchable runtime error (IFERR caught it and the probe
                // printed `readback:ERR`).
                match store.get(&n) {
                    Some(content) => Ok(Value::str(content.clone())),
                    None => Err(RuntimeError::new(
                        ErrorKind::Undefined,
                        format!("{name}: no object named {n}"),
                    )),
                }
            }
        }
        B::Input | B::Choose => Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            format!("{name} needs a variable argument"),
        )),
        B::Eval => {
            let value = arg(&args, 0, name)?;
            // EVAL evaluates algebra values. Strings are data (EXPR and
            // CAS(string) are the explicit text-evaluation operations).
            if matches!(value, Value::Symbolic(_)) {
                crate::cas::evaluate(m, &crate::cas::expression(value)?)
            } else if let Value::List(items) = value {
                let mut result = Vec::with_capacity(items.len());
                for item in items.iter() {
                    result.push(call(m, B::Eval, vec![item.clone()])?);
                }
                Ok(Value::list(result))
            } else {
                Ok(value.clone())
            }
        }
        B::Exact => {
            let value = arg(&args, 0, name)?;
            // Scalar strings are the explicit exact-conversion source form.
            // A string inside a list/vector remains data, as the typed
            // expression renderer below preserves its quotation.
            let source = if value.is_string() {
                value.expect_str(name)?.to_string()
            } else {
                crate::cas::expression(value)?
            };
            crate::cas::evaluate_exact(m, &format!("exact(({source}))"))
        }
        B::CasCall => {
            if args.len() != 1 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "CAS expects one symbolic expression or source string",
                ));
            }
            let source = crate::cas::expression(&args[0])?;
            crate::cas::evaluate_ppl_cas(m, &source)
        }
        B::Startapp => {
            let Value::Str(app) = arg(&args, 0, name)? else {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "STARTAPP needs an app name",
                ));
            };
            let context = m.presentation_snapshot();
            let outcome = m.host.calculator_view_with_context(
                crate::host::CalculatorView::StartApp(app.to_string()),
                context,
            )?;
            if outcome.changed {
                m.active_app = app.to_string();
                for (name, value) in outcome.globals {
                    m.store_home_value(&name, value.to_value())?;
                }
            }
            Ok(Value::from_i64(i64::from(outcome.changed)))
        }
        B::Startview => {
            let number = arg(&args, 0, name)?.expect_i64(name)?;
            let draw = args
                .get(1)
                .map(|v| v.expect_i64(name))
                .transpose()?
                .unwrap_or(0)
                != 0;
            let context = m.presentation_snapshot();
            let outcome = m.host.calculator_view_with_context(
                crate::host::CalculatorView::StartView { number, draw },
                context,
            )?;
            if outcome.changed {
                for (name, value) in outcome.globals {
                    m.store_home_value(&name, value.to_value())?;
                }
            }
            Ok(Value::from_i64(i64::from(outcome.changed)))
        }
        B::Editmat => io::editmat(m, &args),
        B::Editlist => io::editlist(m, &args),
        B::Hvars => {
            let values = m
                .home_variables
                .iter()
                .filter(|(name, _)| !ppl_compiler::is_system_variable(name))
                .collect::<Vec<_>>();
            if args.is_empty() {
                return Ok(Value::list(
                    values
                        .iter()
                        .map(|(name, _)| Value::str(name.as_str()))
                        .collect(),
                ));
            }
            let value = match &args[0] {
                Value::Str(name) => m.home_variables.get(name.as_ref()),
                other => {
                    let index = other.expect_i64("HVars")?;
                    if index < 1 {
                        None
                    } else {
                        values.get((index - 1) as usize).map(|(_, v)| *v)
                    }
                }
            }
            .ok_or_else(|| {
                RuntimeError::new(ErrorKind::Undefined, "HVars: variable does not exist")
            })?;
            if args.len() == 2 {
                if args[1].expect_i64("HVars")? != 2 {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "HVars metadata selector must be 2",
                    ));
                }
                Ok(crate::cas::function_parameters(value)
                    .map(|names| Value::list(names.into_iter().map(Value::str).collect()))
                    .unwrap_or_else(Value::zero))
            } else {
                Ok(value.clone())
            }
        }
        B::Afiles => {
            let files = m.app_files.get(&m.active_app);
            if args.is_empty() {
                Ok(Value::list(
                    files
                        .into_iter()
                        .flat_map(|f| f.keys().map(|name| Value::str(name.as_str())))
                        .collect(),
                ))
            } else {
                let Value::Str(name) = &args[0] else {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        "AFiles needs a file name",
                    ));
                };
                files
                    .and_then(|f| f.get(name.as_ref()))
                    .map(|v| v.to_value())
                    .ok_or_else(|| {
                        RuntimeError::new(ErrorKind::Undefined, "AFiles: file does not exist")
                    })
            }
        }
    }
}

/// Commands that write back into variables: returns the result plus the
/// new values of the referenced variables, in order.
pub fn call_ref(m: &mut Machine, b: Builtin, args: Vec<Value>) -> RtResult<(Value, Vec<Value>)> {
    match b {
        Builtin::Input => io::input(m, &args),
        Builtin::Choose => io::choose(m, &args),
        Builtin::LineP | Builtin::Line | Builtin::TriangleP | Builtin::Triangle => {
            graphics::call_ref(m, b, args)
        }
        Builtin::Editmat | Builtin::Editlist => {
            let values = args[0].expect_list("EDITMAT")?;
            let mut direct = vec![values[0].clone()];
            direct.extend_from_slice(&args[1..]);
            let result = if b == Builtin::Editmat {
                io::editmat(m, &direct)?
            } else {
                io::editlist(m, &direct)?
            };
            Ok((result.clone(), vec![result]))
        }
        other => Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            format!("{} does not take variable arguments", spec(other).name),
        )),
    }
}
