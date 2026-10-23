//! Pixel (`_P`) and Cartesian graphics commands.
//!
//! An optional first argument naming G0–G9 selects the target graphic.
//! Default colours: lines, pixels and circles are black; `RECT`/`RECT_P`
//! without colours fills with white (User Guide). Cartesian variants map
//! through the Function app's default plot window (approximation).

use ppl_compiler::builtins::Builtin;
use ppl_graphics::{BLACK, Color, Grob, SCREEN_H, SCREEN_W, WHITE, text};

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::value::Value;

/// Default plot window of the Function app (factory settings).
const XMIN: f64 = -15.9;
const XMAX: f64 = 15.9;
const YMIN: f64 = -10.9;
const YMAX: f64 = 10.9;

pub fn color_of(v: &Value) -> RtResult<Color> {
    match v {
        Value::Int(i) => Ok(i.unsigned() as u32),
        other => {
            let n = other.expect_real("color")?.trunc().to_i64().unwrap_or(0);
            Ok(n as u32)
        }
    }
}

fn split_grob(args: &[Value]) -> (usize, &[Value]) {
    match args.first() {
        Some(Value::Grob(n)) => (*n as usize, &args[1..]),
        _ => (0, args),
    }
}

fn coord(v: &Value, cartesian: bool, axis_x: bool) -> RtResult<i64> {
    let r = v.expect_real("coordinate")?.to_f64();
    if !cartesian {
        // Pixel coordinates are truncated (unverified: the G1 may round).
        return Ok(r.floor() as i64);
    }
    Ok(if axis_x {
        ((r - XMIN) / (XMAX - XMIN) * (SCREEN_W as f64 - 1.0)).round() as i64
    } else {
        ((YMAX - r) / (YMAX - YMIN) * (SCREEN_H as f64 - 1.0)).round() as i64
    })
}

fn grob_mut(m: &mut Machine, n: usize) -> RtResult<&mut Grob> {
    if n == 0 {
        m.screen_dirty = true;
    }
    m.grobs.get_mut(n).and_then(|g| g.as_mut()).ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::InvalidDimension,
            format!("G{n} has not been created (use DIMGROB_P)"),
        )
    })
}

fn grob_ref(m: &Machine, v: &Value) -> RtResult<usize> {
    match v {
        Value::Grob(n) if m.grobs.get(*n as usize).is_some_and(Option::is_some) => Ok(*n as usize),
        Value::Grob(n) => Err(RuntimeError::new(
            ErrorKind::InvalidDimension,
            format!("G{n} has not been created (use DIMGROB_P)"),
        )),
        other => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            format!("expected a graphic G0–G9, got {}", other.type_name()),
        )),
    }
}

fn opt_color(args: &[Value], i: usize, default: Color) -> RtResult<Color> {
    args.get(i)
        .map(color_of)
        .transpose()
        .map(|c| c.unwrap_or(default))
}

pub fn call(m: &mut Machine, b: Builtin, name: &str, args: &[Value]) -> RtResult<Value> {
    use Builtin as B;
    let cart = matches!(
        b,
        B::Rect
            | B::Line
            | B::Pixon
            | B::Pixoff
            | B::Getpix
            | B::Circle
            | B::Arc
            | B::Textout
            | B::Blit
    );
    match b {
        B::Rgb => {
            let c = |i: usize| -> RtResult<u32> {
                Ok(arg(args, i, name)?.expect_i64(name)?.clamp(0, 255) as u32)
            };
            let a = match args.get(3) {
                Some(v) => v.expect_i64(name)?.clamp(0, 255) as u32,
                None => 0,
            };
            Ok(Value::from_i64(
                (a << 24 | c(0)? << 16 | c(1)? << 8 | c(2)?) as i64,
            ))
        }
        B::RectP | B::Rect => {
            let (g, rest) = split_grob(args);
            let grob = grob_mut(m, g)?;
            let (w, h) = (grob.width() as i64, grob.height() as i64);
            match rest.len() {
                0 => grob.fill(WHITE),
                1 => grob.fill(color_of(&rest[0])?),
                2 => {
                    let (e, f) = (color_of(&rest[0])?, color_of(&rest[1])?);
                    grob.rect(0, 0, w - 1, h - 1, e, f);
                }
                n if n >= 4 => {
                    let x1 = coord(&rest[0], cart, true)?;
                    let y1 = coord(&rest[1], cart, false)?;
                    let x2 = coord(&rest[2], cart, true)?;
                    let y2 = coord(&rest[3], cart, false)?;
                    let edge = opt_color(rest, 4, WHITE)?;
                    let fill = opt_color(rest, 5, edge)?;
                    grob.rect(x1, y1, x2, y2, edge, fill);
                }
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        format!("{name}: 3 arguments is not a valid form"),
                    ));
                }
            }
            Ok(Value::zero())
        }
        B::LineP | B::Line => {
            let (g, r) = split_grob(args);
            if matches!(r.first(), Some(Value::List(_) | Value::Matrix(_))) {
                return Err(RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!("{name}: the 3D points/lines form is not emulated"),
                ));
            }
            let c = [
                coord(arg(r, 0, name)?, cart, true)?,
                coord(arg(r, 1, name)?, cart, false)?,
                coord(arg(r, 2, name)?, cart, true)?,
                coord(arg(r, 3, name)?, cart, false)?,
            ];
            let color = opt_color(r, 4, BLACK)?;
            grob_mut(m, g)?.line(c[0], c[1], c[2], c[3], color);
            Ok(Value::zero())
        }
        B::PixonP | B::Pixon | B::PixoffP | B::Pixoff => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true)?,
                coord(arg(r, 1, name)?, cart, false)?,
            );
            let color = if matches!(b, B::PixoffP | B::Pixoff) {
                WHITE
            } else {
                opt_color(r, 2, BLACK)?
            };
            grob_mut(m, g)?.set(x, y, color);
            Ok(Value::zero())
        }
        B::GetpixP | B::Getpix => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true)?,
                coord(arg(r, 1, name)?, cart, false)?,
            );
            let grob = m
                .grobs
                .get(g)
                .and_then(|g| g.as_ref())
                .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?;
            Ok(Value::from_i64(grob.get(x, y).unwrap_or(0) as i64))
        }
        B::CircleP | B::Circle | B::FillcircleP => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true)?,
                coord(arg(r, 1, name)?, cart, false)?,
            );
            let rad_v = arg(r, 2, name)?.expect_real(name)?.to_f64();
            let rad = if cart {
                (rad_v / (XMAX - XMIN) * (SCREEN_W as f64 - 1.0)).round() as i64
            } else {
                rad_v.floor() as i64
            };
            let color = opt_color(r, 3, BLACK)?;
            let grob = grob_mut(m, g)?;
            if b == B::FillcircleP {
                let fill = opt_color(r, 4, color)?;
                grob.fill_circle(x, y, rad, fill);
                grob.circle(x, y, rad, color);
            } else {
                grob.circle(x, y, rad, color);
            }
            Ok(Value::zero())
        }
        B::ArcP | B::Arc => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true)?,
                coord(arg(r, 1, name)?, cart, false)?,
            );
            let rad = arg(r, 2, name)?.expect_real(name)?.to_f64().floor() as i64;
            let (a1, a2, color) = match r.len() {
                3 => (0.0, std::f64::consts::TAU, BLACK),
                4 => (0.0, std::f64::consts::TAU, color_of(&r[3])?),
                _ => {
                    let a1 = m.settings.to_radians(r[3].expect_real(name)?);
                    let a2 = m.settings.to_radians(arg(r, 4, name)?.expect_real(name)?);
                    (a1, a2, opt_color(r, 5, BLACK)?)
                }
            };
            grob_mut(m, g)?.arc(x, y, rad, a1, a2, color);
            Ok(Value::zero())
        }
        B::TextoutP | B::Textout => {
            let txt = match arg(args, 0, name)? {
                Value::Str(s) => s.to_string(),
                other => other.display(m.settings.format()),
            };
            let (g, r) = split_grob(&args[1..]);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true)?,
                coord(arg(r, 1, name)?, cart, false)?,
            );
            let font = r
                .get(2)
                .map(|v| v.expect_i64(name))
                .transpose()?
                .unwrap_or(0);
            let color = opt_color(r, 3, BLACK)?;
            let width = r.get(4).map(|v| v.expect_i64(name)).transpose()?;
            let bg = r.get(5).map(color_of).transpose()?;
            let px = text::font_px(font, m.settings.font_px);
            let end = text::draw_text(grob_mut(m, g)?, &txt, x, y, px, color, width, bg);
            Ok(Value::from_i64(end))
        }
        B::BlitP | B::Blit => blit(m, args, name),
        B::DimgrobP | B::Dimgrob => {
            let Value::Grob(n) = arg(args, 0, name)? else {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "DIMGROB_P needs G0–G9 as first argument",
                ));
            };
            if args.len() == 2 {
                return Err(RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!("{name}(G, list): the pixel-data form is not emulated"),
                ));
            }
            let w = arg(args, 1, name)?.expect_i64(name)?;
            let h = arg(args, 2, name)?.expect_i64(name)?;
            if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    format!("graphic size {w}×{h}"),
                ));
            }
            m.check_len((w * h) as usize)?;
            let color = opt_color(args, 3, WHITE)?;
            m.grobs[*n as usize] = Some(Grob::new(w as usize, h as usize, color));
            if *n == 0 {
                m.screen_dirty = true;
            }
            Ok(Value::zero())
        }
        B::SubgrobP | B::Subgrob => {
            let src = grob_ref(m, arg(args, 0, name)?)?;
            if args.len() == 2 {
                // SUBGROB_P(srcGRB, trgtGRB): copy the whole graphic.
                let Value::Grob(dst) = &args[1] else {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        "SUBGROB_P target must be G0–G9",
                    ));
                };
                let copy = m.grobs[src].clone();
                m.grobs[*dst as usize] = copy;
                if *dst == 0 {
                    m.screen_dirty = true;
                }
                return Ok(Value::zero());
            }
            let c: Vec<i64> = (1..5)
                .map(|i| arg(args, i, name).and_then(|v| v.expect_i64(name)))
                .collect::<RtResult<_>>()?;
            let Value::Grob(dst) = arg(args, 5, name)? else {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "SUBGROB_P target must be G0–G9",
                ));
            };
            let sub = m.grobs[src].as_ref().unwrap().sub(c[0], c[1], c[2], c[3]);
            m.grobs[*dst as usize] = Some(sub);
            Ok(Value::zero())
        }
        B::InvertP => {
            let (g, r) = split_grob(args);
            let grob = grob_mut(m, g)?;
            let (w, h) = (grob.width() as i64, grob.height() as i64);
            let c = |i: usize, d: i64| -> RtResult<i64> {
                r.get(i)
                    .map(|v| v.expect_i64(name))
                    .transpose()
                    .map(|x| x.unwrap_or(d))
            };
            let (x1, y1, x2, y2) = (c(0, 0)?, c(1, 0)?, c(2, w - 1)?, c(3, h - 1)?);
            grob.invert(x1, y1, x2, y2);
            Ok(Value::zero())
        }
        B::FillpolyP => {
            let (g, r) = split_grob(args);
            let pts = points(arg(r, 0, name)?)?;
            let color = opt_color(r, 1, BLACK)?;
            grob_mut(m, g)?.fill_polygon(&pts, color);
            Ok(Value::zero())
        }
        B::TriangleP => {
            let (g, r) = split_grob(args);
            if r.len() < 6 || matches!(r.first(), Some(Value::List(_) | Value::Matrix(_))) {
                return Err(RuntimeError::new(
                    ErrorKind::Unsupported,
                    format!("{name}: only the x1, y1, x2, y2, x3, y3, color form is emulated"),
                ));
            }
            let mut pts = Vec::new();
            for i in 0..3 {
                pts.push((
                    arg(r, 2 * i, name)?.expect_real(name)?.to_f64(),
                    arg(r, 2 * i + 1, name)?.expect_real(name)?.to_f64(),
                ));
            }
            let color = opt_color(r, 6, BLACK)?;
            grob_mut(m, g)?.fill_polygon(&pts, color);
            Ok(Value::zero())
        }
        B::GrobwP | B::GrobhP => {
            let n = grob_ref(m, arg(args, 0, name)?)?;
            let g = m.grobs[n].as_ref().unwrap();
            Ok(Value::from_i64(if b == B::GrobwP {
                g.width()
            } else {
                g.height()
            } as i64))
        }
        other => Err(RuntimeError::new(
            ErrorKind::Unsupported,
            format!("{other:?} is not a graphics command"),
        )),
    }
}

fn points(v: &Value) -> RtResult<Vec<(f64, f64)>> {
    let l = v.expect_list("FILLPOLY_P")?;
    let mut out = Vec::new();
    if l.iter().all(|p| matches!(p, Value::List(_))) {
        for p in l.iter() {
            let pl = p.expect_list("point")?;
            if pl.len() != 2 {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            out.push((
                pl[0].expect_real("x")?.to_f64(),
                pl[1].expect_real("y")?.to_f64(),
            ));
        }
    } else if l.iter().all(|p| matches!(p, Value::Complex(_))) {
        for p in l.iter() {
            let Value::Complex(c) = p else { unreachable!() };
            out.push((c.re.to_f64(), c.im.to_f64()));
        }
    } else {
        // Flat {x1,y1,x2,y2,…} (Python-era form, kept for compatibility).
        if l.len() % 2 != 0 {
            return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
        }
        for p in l.chunks(2) {
            out.push((
                p[0].expect_real("x")?.to_f64(),
                p[1].expect_real("y")?.to_f64(),
            ));
        }
    }
    Ok(out)
}

/// `BLIT_P([trgtG], [dx1, dy1, [dx2, dy2]], srcG, [sx1, sy1, [sx2, sy2]], [c], [a])`.
fn blit(m: &mut Machine, args: &[Value], name: &str) -> RtResult<Value> {
    if args.is_empty() {
        // BLIT_P() without arguments: treated as a no-op (unverified).
        m.screen_dirty = true;
        return Ok(Value::zero());
    }
    let grob_positions: Vec<usize> = args
        .iter()
        .enumerate()
        .filter(|(_, v)| matches!(v, Value::Grob(_)))
        .map(|(i, _)| i)
        .collect();
    let (target, src_pos) = match grob_positions.as_slice() {
        [s] => (0usize, *s),
        [t, s, ..] if *t == 0 => (grob_ref(m, &args[*t])?, *s),
        [s, ..] => (0usize, *s),
        [] => {
            return Err(RuntimeError::new(
                ErrorKind::BadArgType,
                format!("{name}: no source graphic"),
            ));
        }
    };
    let dest_nums = &args[if grob_positions.len() >= 2 && grob_positions[0] == 0 {
        1
    } else {
        0
    }..src_pos];
    let after = &args[src_pos + 1..];
    let src_n = grob_ref(m, &args[src_pos])?;
    let num = |v: &Value| v.expect_real(name).map(|r| r.to_f64().floor() as i64);
    let src = m.grobs[src_n].as_ref().unwrap().clone();
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    let (sx1, sy1, sx2, sy2, rest) = match after.len() {
        n if n >= 4 => (
            num(&after[0])?,
            num(&after[1])?,
            num(&after[2])?,
            num(&after[3])?,
            &after[4..],
        ),
        n if n >= 2 => (num(&after[0])?, num(&after[1])?, sw, sh, &after[2..]),
        _ => (0, 0, sw, sh, after),
    };
    let (dx1, dy1) = match dest_nums.len() {
        0 => (0, 0),
        _ => (num(&dest_nums[0])?, num(arg(dest_nums, 1, name)?)?),
    };
    let (dx2, dy2) = if dest_nums.len() >= 4 {
        (num(&dest_nums[2])?, num(&dest_nums[3])?)
    } else {
        (dx1 + (sx2 - sx1), dy1 + (sy2 - sy1))
    };
    let transparent = rest.first().map(color_of).transpose()?;
    let alpha = rest
        .get(1)
        .map(|v| v.expect_i64(name))
        .transpose()?
        .unwrap_or(255)
        .clamp(0, 255) as u8;
    grob_mut(m, target)?.blit(
        (dx1, dy1, dx2, dy2),
        &src,
        (sx1, sy1, sx2, sy2),
        transparent,
        alpha,
    );
    Ok(Value::zero())
}
