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
    match b {
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
        | B::TriangleP
        | B::GrobwP
        | B::GrobhP
        | B::Rgb => graphics::call(m, b, name, &args),
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
        B::Type => Ok(Value::from_i64(arg(&args, 0, name)?.type_code())),
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
        B::Exact
        | B::Editmat
        | B::Editlist
        | B::Eval
        | B::Startapp
        | B::Startview
        | B::Recall
        | B::Afiles
        | B::Hvars
        | B::CasCall => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{name} is not implemented by this emulator"),
        )),
    }
}

/// Commands that write back into variables: returns the result plus the
/// new values of the referenced variables, in order.
pub fn call_ref(m: &mut Machine, b: Builtin, args: Vec<Value>) -> RtResult<(Value, Vec<Value>)> {
    match b {
        Builtin::Input => io::input(m, &args),
        Builtin::Choose => io::choose(m, &args),
        other => Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            format!("{} does not take variable arguments", spec(other).name),
        )),
    }
}
