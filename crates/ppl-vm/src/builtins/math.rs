//! Numeric commands and integer/bit operations.

use ppl_compiler::builtins::Builtin;
use ppl_math::{Complex, HpInt, IntBase, MathError, Real};
use ppl_syntax::ast::BinOp;

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::ops::{self, map_numeric, norm_complex};
use crate::settings::{AngleMode, Settings};
use crate::value::Value;

fn real_map(v: &Value, f: impl Fn(Real) -> RtResult<Real>) -> RtResult<Value> {
    map_numeric(v, &mut |x| Ok(Value::Real(f(x.expect_real("argument")?)?)))
}

fn f64_map(v: &Value, f: impl Fn(f64) -> f64) -> RtResult<Value> {
    real_map(v, |x| Ok(Real::from_f64(f(x.to_f64()))?))
}

pub fn sqrt(m: &Machine, v: &Value) -> RtResult<Value> {
    let complex = m.settings.complex_results;
    map_numeric(v, &mut |x| match x {
        Value::Complex(c) => Ok(norm_complex(c.sqrt()?)),
        other => {
            let r = other.expect_real("√")?;
            match r.sqrt() {
                Ok(s) => Ok(Value::Real(s)),
                Err(MathError::NonReal) if complex => {
                    Ok(Value::Complex(Complex::new(Real::ZERO, r.neg().sqrt()?)))
                }
                Err(e) => Err(e.into()),
            }
        }
    })
}

/// Reduces an angle in the current mode and returns exact results for
/// multiples of a quarter turn, so `SIN(180)` is 0 in degree mode.
fn trig(s: &Settings, x: Real, which: u8) -> RtResult<Real> {
    let turn = match s.angle {
        AngleMode::Degrees => Some(Real::from_i64(360)),
        AngleMode::Gradians => Some(Real::from_i64(400)),
        AngleMode::Radians => None,
    };
    if let Some(turn) = turn {
        let r = ops::real_mod(x, turn)?;
        let quarter = turn.div(Real::from_i64(4))?;
        let q = r.div(quarter)?;
        if q.is_integer() {
            let k = q.to_i64().unwrap_or(0) % 4;
            let (sn, cs): (i64, i64) = [(0, 1), (1, 0), (0, -1), (-1, 0)][k as usize];
            return match which {
                0 => Ok(Real::from_i64(sn)),
                1 => Ok(Real::from_i64(cs)),
                _ => {
                    if cs == 0 {
                        Err(RuntimeError::new(
                            ErrorKind::Undefined,
                            "TAN of an odd multiple of 90°",
                        ))
                    } else {
                        Ok(Real::from_i64(sn * cs))
                    }
                }
            };
        }
    }
    let rad = s.to_radians(x);
    let v = match which {
        0 => rad.sin(),
        1 => rad.cos(),
        _ => rad.tan(),
    };
    Ok(Real::from_f64(v)?)
}

fn inverse_trig(s: &Settings, x: Real, f: fn(f64) -> f64, domain_check: bool) -> RtResult<Real> {
    let v = x.to_f64();
    if domain_check && !(-1.0..=1.0).contains(&v) {
        return Err(RuntimeError::kind(ErrorKind::NonReal));
    }
    Ok(Real::from_f64(s.from_radians(f(v)))?)
}

fn int_value(v: &Value) -> RtResult<(u64, Option<HpInt>)> {
    match v {
        Value::Int(i) => Ok((i.unsigned(), Some(*i))),
        other => {
            let n = other.expect_i64("integer")?;
            Ok((n as u64, None))
        }
    }
}

fn int_result(v: u64, like: Option<HpInt>, s: &Settings) -> Value {
    match like {
        Some(i) => Value::Int(HpInt::new(v as i128, i.bits(), i.base)),
        None => {
            let bits = s.int_bits;
            let masked = if bits >= 64 {
                v
            } else {
                v & ((1u64 << bits) - 1)
            };
            Value::Real(Real::from_u64(masked))
        }
    }
}

fn base_code(b: IntBase) -> i64 {
    match b {
        IntBase::Bin => 1,
        IntBase::Oct => 2,
        IntBase::Dec => 3,
        IntBase::Hex => 4,
    }
}

fn reduce_list(args: &[Value]) -> RtResult<Vec<Real>> {
    let items: Vec<Value> = match args {
        [Value::List(l)] => l.iter().cloned().collect(),
        _ => args.to_vec(),
    };
    items.iter().map(|v| v.expect_real("MAX/MIN")).collect()
}

fn stats_values(args: &[Value], name: &str) -> RtResult<Vec<Real>> {
    let data = arg(args, 0, name)?.expect_list(name)?;
    let freq = match args.get(1) {
        Some(f) => Some(f.expect_list(name)?),
        None => None,
    };
    let mut out = Vec::new();
    for (i, v) in data.iter().enumerate() {
        let r = v.expect_real(name)?;
        let n = match &freq {
            Some(f) => f
                .get(i)
                .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?
                .expect_i64(name)?,
            None => 1,
        };
        for _ in 0..n.max(0) {
            out.push(r);
        }
    }
    if out.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::InvalidDimension,
            format!("{name} of an empty list"),
        ));
    }
    Ok(out)
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let s = m.settings.clone();
    let a0 = || arg(args, 0, name);
    match b {
        B::Abs => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Real(c.abs()?)),
            Value::Int(i) => Ok(Value::Int(*i)),
            other => Ok(Value::Real(other.expect_real("ABS")?.abs())),
        }),
        B::Sqrt => sqrt(m, a0()?),
        B::Sq => map_numeric(a0()?, &mut |x| ops::binary(BinOp::Mul, x, x, &s)),
        B::Exp => real_map(a0()?, |x| Ok(x.exp_e()?)),
        B::Ln => real_map(a0()?, |x| Ok(x.ln()?)),
        B::Log => match args.get(1) {
            None => real_map(a0()?, |x| Ok(x.log10()?)),
            Some(base) => {
                let b = base.expect_real("LOG")?;
                real_map(a0()?, |x| Ok(x.ln()?.div(b.ln()?)?))
            }
        },
        B::Alog => real_map(a0()?, |x| Ok(Real::from_i64(10).pow(x)?)),
        B::Expm1 => f64_map(a0()?, f64::exp_m1),
        B::Lnp1 => real_map(a0()?, |x| {
            if x.to_f64() <= -1.0 {
                return Err(RuntimeError::kind(ErrorKind::NonReal));
            }
            Ok(Real::from_f64(x.to_f64().ln_1p())?)
        }),
        B::Sin => real_map(a0()?, |x| trig(&s, x, 0)),
        B::Cos => real_map(a0()?, |x| trig(&s, x, 1)),
        B::Tan => real_map(a0()?, |x| trig(&s, x, 2)),
        B::Csc => real_map(a0()?, |x| Ok(Real::ONE.div(trig(&s, x, 0)?)?)),
        B::Sec => real_map(a0()?, |x| Ok(Real::ONE.div(trig(&s, x, 1)?)?)),
        B::Cot => real_map(a0()?, |x| Ok(Real::ONE.div(trig(&s, x, 2)?)?)),
        B::Asin => real_map(a0()?, |x| inverse_trig(&s, x, f64::asin, true)),
        B::Acos => real_map(a0()?, |x| inverse_trig(&s, x, f64::acos, true)),
        B::Atan => real_map(a0()?, |x| inverse_trig(&s, x, f64::atan, false)),
        B::Sinh => f64_map(a0()?, f64::sinh),
        B::Cosh => f64_map(a0()?, f64::cosh),
        B::Tanh => f64_map(a0()?, f64::tanh),
        B::Asinh => f64_map(a0()?, f64::asinh),
        B::Acosh => real_map(a0()?, |x| {
            if x.to_f64() < 1.0 {
                return Err(RuntimeError::kind(ErrorKind::NonReal));
            }
            Ok(Real::from_f64(x.to_f64().acosh())?)
        }),
        B::Atanh => real_map(a0()?, |x| {
            let v = x.to_f64();
            if v.abs() >= 1.0 {
                return Err(RuntimeError::kind(if v.abs() == 1.0 {
                    ErrorKind::Undefined
                } else {
                    ErrorKind::NonReal
                }));
            }
            Ok(Real::from_f64(v.atanh())?)
        }),
        B::Floor => real_map(a0()?, |x| Ok(x.floor())),
        B::Ceiling => real_map(a0()?, |x| Ok(x.ceil())),
        B::Ip => real_map(a0()?, |x| Ok(x.trunc())),
        B::Fp => real_map(a0()?, |x| Ok(x.fract())),
        B::Round | B::Truncate => {
            let places = match args.get(1) {
                // Clamp rather than wrap: ROUND(x, 2^32) must not act like
                // ROUND(x, 0).
                Some(p) => p.expect_i64(name)?.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                None => 0,
            };
            let round = b == B::Round;
            real_map(a0()?, |x| {
                Ok(if round {
                    x.round_places(places)
                } else {
                    x.truncate_places(places)
                })
            })
        }
        B::Sign => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => {
                let r = c.abs()?;
                Ok(norm_complex(Complex::new(c.re.div(r)?, c.im.div(r)?)))
            }
            other => {
                let r = other.expect_real("SIGN")?;
                Ok(Value::from_i64(if r.is_zero() {
                    0
                } else if r.is_negative() {
                    -1
                } else {
                    1
                }))
            }
        }),
        B::Max | B::Min => {
            let vals = reduce_list(args)?;
            let first = *vals
                .first()
                .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?;
            let pick = vals.into_iter().fold(first, |acc, v| {
                let better = if b == B::Max { v > acc } else { v < acc };
                if better { v } else { acc }
            });
            Ok(Value::Real(pick))
        }
        B::Comb | B::Perm => {
            let n = a0()?.expect_i64(name)?;
            let k = arg(args, 1, name)?.expect_i64(name)?;
            if n < 0 || k < 0 || k > n {
                return Err(RuntimeError::kind(ErrorKind::BadArgValue));
            }
            let mut acc = Real::ONE;
            for i in 0..k {
                acc = acc.mul(Real::from_i64(n - i))?;
                if b == B::Comb {
                    acc = acc.div(Real::from_i64(i + 1))?;
                }
            }
            Ok(Value::Real(acc.round_places(0)))
        }
        B::Random => {
            if args.len() >= 3 {
                m.check_len(args[0].expect_i64(name)?.max(0) as usize)?;
            }
            let mut draw = |lo: f64, hi: f64| -> RtResult<Value> {
                let u = m.rng.next_f64();
                Ok(Value::Real(Real::from_f64(lo + (hi - lo) * u)?))
            };
            match args.len() {
                0 => draw(0.0, 1.0),
                1 => draw(0.0, args[0].expect_real(name)?.to_f64()),
                2 => draw(
                    args[0].expect_real(name)?.to_f64(),
                    args[1].expect_real(name)?.to_f64(),
                ),
                _ => {
                    let n = args[0].expect_i64(name)?.max(0) as usize;
                    let (lo, hi) = (
                        args[1].expect_real(name)?.to_f64(),
                        args[2].expect_real(name)?.to_f64(),
                    );
                    Ok(Value::list(
                        (0..n).map(|_| draw(lo, hi)).collect::<RtResult<_>>()?,
                    ))
                }
            }
        }
        B::Randint => {
            if args.len() == 3 {
                let n = args[0].expect_i64(name)?.max(0) as usize;
                m.check_len(n)?;
                let (lo, hi) = (args[1].expect_i64(name)?, args[2].expect_i64(name)?);
                return Ok(Value::list(
                    (0..n)
                        .map(|_| Value::from_i64(m.rng.range(lo, hi)))
                        .collect(),
                ));
            }
            // User Guide 3rd ed.: no argument gives 0 or 1, one argument a
            // gives an integer between 0 and a.
            let (lo, hi) = match args.len() {
                0 => (0, 1),
                1 => (0, a0()?.expect_i64(name)?),
                _ => (
                    a0()?.expect_i64(name)?,
                    arg(args, 1, name)?.expect_i64(name)?,
                ),
            };
            Ok(Value::from_i64(m.rng.range(lo, hi)))
        }
        B::Randnorm => {
            let mu = args
                .first()
                .map(|v| v.expect_real(name))
                .transpose()?
                .map(|r| r.to_f64())
                .unwrap_or(0.0);
            let sigma = args
                .get(1)
                .map(|v| v.expect_real(name))
                .transpose()?
                .map(|r| r.to_f64())
                .unwrap_or(1.0);
            Ok(Value::Real(Real::from_f64(mu + sigma * m.rng.normal())?))
        }
        B::Randseed => {
            let seed = match args.first() {
                Some(v) => v.expect_real(name)?.to_f64().to_bits(),
                None => 0x5EED,
            };
            m.set_seed(seed);
            Ok(Value::zero())
        }
        B::Mant => real_map(a0()?, |x| {
            if x.is_zero() {
                return Ok(Real::ZERO);
            }
            let e = x.exp10();
            Ok(x.abs().div(Real::from_i64(10).powi(e as i64)?)?)
        }),
        B::Xpon => real_map(a0()?, |x| Ok(Real::from_i64(x.exp10() as i64))),
        B::Re => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Real(c.re)),
            o => Ok(Value::Real(o.expect_real("RE")?)),
        }),
        B::Im => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Real(c.im)),
            o => {
                o.expect_real("IM")?;
                Ok(Value::zero())
            }
        }),
        B::Conj => map_numeric(a0()?, &mut |x| match x {
            Value::Complex(c) => Ok(Value::Complex(c.conj())),
            o => Ok(Value::Real(o.expect_real("CONJ")?)),
        }),
        B::Arg => map_numeric(a0()?, &mut |x| {
            let c = match x {
                Value::Complex(c) => *c,
                o => Complex::new(o.expect_real("ARG")?, Real::ZERO),
            };
            Ok(Value::Real(Real::from_f64(
                s.from_radians(c.arg_radians()?.to_f64()),
            )?))
        }),
        B::Approx => Ok(a0()?.clone()),
        B::Hms => real_map(a0()?, |x| {
            // decimal hours → H.MMSS
            let neg = x.is_negative();
            let total = x.abs().to_f64() * 3600.0;
            let h = (total / 3600.0).floor();
            let mi = ((total - h * 3600.0) / 60.0).floor();
            let sec = total - h * 3600.0 - mi * 60.0;
            let v = h + mi / 100.0 + sec / 10000.0;
            Ok(Real::from_f64(if neg { -v } else { v })?)
        }),
        B::HmsFrom => real_map(a0()?, |x| {
            let v = x.abs().to_f64();
            let h = v.trunc();
            let rest = (v - h) * 100.0;
            let mi = (rest + 1e-9).trunc();
            let sec = (rest - mi) * 100.0;
            let out = h + mi / 60.0 + sec / 3600.0;
            Ok(Real::from_f64(if x.is_negative() { -out } else { out })?)
        }),
        B::Nthroot => {
            let n = a0()?.expect_real(name)?;
            real_map(arg(args, 1, name)?, |x| {
                if x.is_negative() {
                    let k = n.to_i64().ok_or(RuntimeError::kind(ErrorKind::NonReal))?;
                    if k % 2 == 0 {
                        return Err(RuntimeError::kind(ErrorKind::NonReal));
                    }
                    return Ok(x.neg().pow(Real::ONE.div(n)?)?.neg());
                }
                Ok(x.pow(Real::ONE.div(n)?)?)
            })
        }
        B::Factorial => real_map(a0()?, ops::factorial),
        B::Mean => {
            let v = stats_values(args, name)?;
            let mut sum = Real::ZERO;
            for x in &v {
                sum = sum.add(*x)?;
            }
            Ok(Value::Real(sum.div(Real::from_i64(v.len() as i64))?))
        }
        B::Median => {
            let mut v = stats_values(args, name)?;
            v.sort_by(|a, b| a.cmp_value(*b));
            let n = v.len();
            if n % 2 == 1 {
                Ok(Value::Real(v[n / 2]))
            } else {
                Ok(Value::Real(
                    v[n / 2 - 1].add(v[n / 2])?.div(Real::from_i64(2))?,
                ))
            }
        }
        B::Stddev => {
            // Population standard deviation (unverified: the Prime may use
            // the sample form).
            let v = stats_values(args, name)?;
            let n = v.len() as f64;
            let mean = v.iter().map(|x| x.to_f64()).sum::<f64>() / n;
            let var = v.iter().map(|x| (x.to_f64() - mean).powi(2)).sum::<f64>() / n;
            Ok(Value::Real(Real::from_f64(var.sqrt())?))
        }
        B::Ifte => {
            let c = a0()?.truthy()?;
            Ok(arg(args, if c { 1 } else { 2 }, name)?.clone())
        }
        B::Bitand | B::Bitor | B::Bitxor => {
            let (mut acc, mut like) = int_value(a0()?)?;
            for v in &args[1..] {
                let (x, l) = int_value(v)?;
                like = like.or(l);
                acc = match b {
                    B::Bitand => acc & x,
                    B::Bitor => acc | x,
                    _ => acc ^ x,
                };
            }
            Ok(int_result(acc, like, &s))
        }
        B::Bitnot => {
            let (x, like) = int_value(a0()?)?;
            Ok(int_result(!x, like, &s))
        }
        B::Bitsl | B::Bitsr => {
            let (x, like) = int_value(a0()?)?;
            let n = match args.get(1) {
                Some(v) => v.expect_i64(name)?,
                None => 1,
            };
            let r = if n >= 64 || n <= -64 {
                0
            } else if (b == B::Bitsl) == (n >= 0) {
                x << n.unsigned_abs()
            } else {
                x >> n.unsigned_abs()
            };
            Ok(int_result(r, like, &s))
        }
        B::RtoB => {
            let n = a0()?.expect_i64(name)?;
            Ok(Value::Int(HpInt::new(n as i128, s.int_bits, s.int_base)))
        }
        B::BtoR => match a0()? {
            Value::Int(i) => Ok(Value::Real(if s.int_signed {
                Real::from_i64(i.signed())
            } else {
                Real::from_u64(i.unsigned())
            })),
            other => Ok(Value::Real(other.expect_real(name)?)),
        },
        B::Setbase => {
            let (x, like) = int_value(a0()?)?;
            let code = args
                .get(1)
                .map(|v| v.expect_i64(name))
                .transpose()?
                .unwrap_or(base_code(s.int_base));
            let base = match code {
                1 => IntBase::Bin,
                2 => IntBase::Oct,
                3 => IntBase::Dec,
                4 => IntBase::Hex,
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "base must be 1 (bin) … 4 (hex)",
                    ));
                }
            };
            let bits = like.map(|i| i.bits()).unwrap_or(s.int_bits);
            Ok(Value::Int(HpInt::new(x as i128, bits, base)))
        }
        B::Getbase => match a0()? {
            Value::Int(i) => Ok(Value::from_i64(base_code(i.base))),
            _ => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "GETBASE needs an integer (#…)",
            )),
        },
        B::Setbits => {
            let (x, like) = int_value(a0()?)?;
            let bits = args
                .get(1)
                .map(|v| v.expect_i64(name))
                .transpose()?
                .unwrap_or(s.int_bits as i64)
                .clamp(1, 64) as u8;
            let base = like.map(|i| i.base).unwrap_or(s.int_base);
            Ok(Value::Int(HpInt::new(x as i128, bits, base)))
        }
        B::Getbits => match a0()? {
            Value::Int(i) => Ok(Value::from_i64(i.bits() as i64)),
            _ => Err(RuntimeError::new(
                ErrorKind::BadArgType,
                "GETBITS needs an integer (#…)",
            )),
        },
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a math command"),
        )),
    }
}
