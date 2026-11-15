//! Pixel (`_P`) and Cartesian graphics commands.
//!
//! An optional first argument naming G0–G9 selects the target graphic.
//! Default colours: lines, pixels and circles are black; `RECT`/`RECT_P`
//! without colours fills with white (User Guide). Cartesian variants map
//! through the calculator's current plot window. Numeric 3D batches share the
//! versioned Native cache and depth renderer in `mesh`.

use ppl_compiler::builtins::Builtin;
use ppl_graphics::{BLACK, Color, Grob, SCREEN_H, SCREEN_W, WHITE, text};

use super::arg;
use crate::error::{ErrorKind, RtResult, RuntimeError};
use crate::machine::Machine;
use crate::value::Value;

mod mesh;

/// Plot settings are the same calculator variables used by the Function app.
struct PlotWindow {
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
}
impl PlotWindow {
    fn from_machine(machine: &Machine) -> RtResult<Self> {
        let value = |name: &str, default: f64| -> RtResult<f64> {
            machine
                .calculator_global(name)
                .map(|v| v.expect_real(name).map(|n| n.to_f64()))
                .transpose()
                .map(|n| n.unwrap_or(default))
        };
        let window = Self {
            xmin: value("Xmin", -10.0)?,
            xmax: value("Xmax", 10.0)?,
            ymin: value("Ymin", -10.0)?,
            ymax: value("Ymax", 10.0)?,
        };
        if ![window.xmin, window.xmax, window.ymin, window.ymax]
            .iter()
            .all(|n| n.is_finite())
            || window.xmin >= window.xmax
            || window.ymin >= window.ymax
        {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "Plot bounds must be finite with min < max",
            ));
        }
        Ok(window)
    }
}

pub fn color_of(v: &Value) -> RtResult<Color> {
    match v {
        Value::Int(i) => Ok(i.numeric_value().unsigned_abs() as u32),
        other => {
            let number = other.expect_real("color")?.abs();
            let value = number
                .to_i64()
                .filter(|_| number.is_integer())
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::InvalidInput,
                        "Color must have an integral32-bit magnitude",
                    )
                })?;
            Ok(value)
        }
    }
}

fn split_grob(args: &[Value]) -> (usize, &[Value]) {
    match args.first() {
        Some(Value::Grob(n)) => (*n as usize, &args[1..]),
        _ => (0, args),
    }
}

fn coord(v: &Value, cartesian: bool, axis_x: bool, window: &PlotWindow) -> RtResult<i64> {
    let r = v.expect_real("coordinate")?.to_f64();
    if !r.is_finite() {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "Coordinate exceeds drawing range",
        ));
    }
    if !cartesian {
        // Pixel coordinates are truncated (unverified: the G1 may round).
        return Ok(r.floor() as i64);
    }
    Ok(if axis_x {
        // Genuine G1 C→PX: (Xmin,Ymax)=(1,0), (Xmax,Ymin)=(319,218).
        // The projection is independent of the target GROB dimensions.
        (1.0 + (r - window.xmin) / (window.xmax - window.xmin) * (SCREEN_W as f64 - 2.0)).round()
            as i64
    } else {
        ((window.ymax - r) / (window.ymax - window.ymin) * (SCREEN_H as f64 - 22.0)).round() as i64
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
    call_inner(m, b, name, args, &[], &mut None)
}

/// Compiler-supplied optional argument references, selected by runtime overload.
/// Unused candidates retain their input value; aliases receive the same update.
pub(super) fn call_ref(
    m: &mut Machine,
    b: Builtin,
    mut args: Vec<Value>,
) -> RtResult<(Value, Vec<Value>)> {
    let metadata = args
        .pop()
        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
    let metadata = metadata.expect_list("graphics references")?;
    let mut references = Vec::new();
    for entry in metadata.iter() {
        let entry = entry.expect_list("graphics reference")?;
        if entry.len() != 2 {
            return Err(RuntimeError::kind(ErrorKind::InvalidInput));
        }
        let position = entry[0].expect_i64("graphics reference position")?;
        if position < 0 || position as usize >= args.len() {
            return Err(RuntimeError::kind(ErrorKind::InvalidInput));
        }
        references.push((
            position as usize,
            entry[1].expect_str("graphics reference identity")?,
        ));
    }
    let positions = references
        .iter()
        .map(|(position, _)| *position)
        .collect::<Vec<_>>();
    let mut update = None;
    let result = call_inner(m, b, super::spec(b).name, &args, &positions, &mut update)?;
    let identity = update.as_ref().and_then(|(position, _)| {
        references
            .iter()
            .find(|(candidate, _)| candidate == position)
            .map(|(_, identity)| identity)
    });
    let values = references
        .iter()
        .map(|(position, candidate)| {
            if identity == Some(candidate) {
                update.as_ref().unwrap().1.clone()
            } else {
                args[*position].clone()
            }
        })
        .collect();
    Ok((result, values))
}

fn call_inner(
    m: &mut Machine,
    b: Builtin,
    name: &str,
    args: &[Value],
    writable: &[usize],
    update: &mut Option<(usize, Value)>,
) -> RtResult<Value> {
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
            | B::Triangle
            | B::Subgrob
    );
    let window = if cart {
        PlotWindow::from_machine(m)?
    } else {
        PlotWindow {
            xmin: -10.0,
            xmax: 10.0,
            ymin: -10.0,
            ymax: 10.0,
        }
    };
    match b {
        B::Rgb => {
            let c = |i: usize| -> RtResult<u32> {
                Ok(arg(args, i, name)?.expect_i64(name)?.clamp(0, 255) as u32)
            };
            let a = match args.get(3) {
                Some(v) => v.expect_i64(name)?.clamp(0, 255) as u32,
                None => 0,
            };
            Ok(Value::Int(ppl_math::HpInt::new(
                i128::from(a << 24 | c(0)? << 16 | c(1)? << 8 | c(2)?),
                32,
                ppl_math::IntBase::Hex,
            )))
        }
        B::RectP | B::Rect => {
            let (g, rest) = split_grob(args);
            let grob = grob_mut(m, g)?;
            let (w, h) = (grob.width() as i64, grob.height() as i64);
            match rest.len() {
                0 => grob.fill(WHITE),
                1 => {
                    let color = color_of(&rest[0])?;
                    grob.calculator_rect((0, 0), (w - 1, h - 1), color, color);
                }
                2 => {
                    let (e, f) = (color_of(&rest[0])?, color_of(&rest[1])?);
                    grob.calculator_rect((0, 0), (w - 1, h - 1), e, f);
                }
                n if n >= 4 => {
                    let x1 = coord(&rest[0], cart, true, &window)?;
                    let y1 = coord(&rest[1], cart, false, &window)?;
                    let x2 = coord(&rest[2], cart, true, &window)?;
                    let y2 = coord(&rest[3], cart, false, &window)?;
                    let edge = opt_color(rest, 4, WHITE)?;
                    let fill = opt_color(rest, 5, edge)?;
                    grob.calculator_rect((x1, y1), (x2, y2), edge, fill);
                }
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        format!("{name}: 3 arguments is not a valid form"),
                    ));
                }
            }
            Ok(Value::from_i64(1))
        }
        B::LineP | B::Line => {
            let (g, r) = split_grob(args);
            if matches!(
                r.first(),
                Some(Value::List(_) | Value::Matrix(_) | Value::Str(_))
            ) {
                let offset = args.len() - r.len();
                let position = mesh::z_index(r, false).map(|index| index + offset);
                let (result, depth) = mesh::render(
                    m,
                    g,
                    r,
                    cart,
                    &window,
                    false,
                    position.is_some_and(|index| writable.contains(&index)),
                )?;
                if let (Some(position), Some(depth)) = (position, depth) {
                    *update = Some((position, depth));
                }
                return Ok(result);
            }
            if !(4..=5).contains(&r.len()) {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "Line expects four coordinates and optional color",
                ));
            }
            let c = [
                coord(arg(r, 0, name)?, cart, true, &window)?,
                coord(arg(r, 1, name)?, cart, false, &window)?,
                coord(arg(r, 2, name)?, cart, true, &window)?,
                coord(arg(r, 3, name)?, cart, false, &window)?,
            ];
            let color = opt_color(r, 4, BLACK)?;
            grob_mut(m, g)?.calculator_line((c[0], c[1]), (c[2], c[3]), color);
            Ok(Value::from_i64(1))
        }
        B::PixonP | B::Pixon | B::PixoffP | B::Pixoff => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true, &window)?,
                coord(arg(r, 1, name)?, cart, false, &window)?,
            );
            let color = if matches!(b, B::PixoffP | B::Pixoff) {
                WHITE
            } else {
                opt_color(r, 2, BLACK)?
            };
            grob_mut(m, g)?.set(x, y, color);
            Ok(Value::from_i64(1))
        }
        B::GetpixP | B::Getpix => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true, &window)?,
                coord(arg(r, 1, name)?, cart, false, &window)?,
            );
            let grob = m
                .grobs
                .get(g)
                .and_then(|g| g.as_ref())
                .ok_or(RuntimeError::kind(ErrorKind::InvalidDimension))?;
            Ok(Value::Int(ppl_math::HpInt::new(
                i128::from(grob.get(x, y).unwrap_or(0)),
                32,
                ppl_math::IntBase::Hex,
            )))
        }
        B::CircleP | B::Circle | B::FillcircleP => {
            let (g, r) = split_grob(args);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true, &window)?,
                coord(arg(r, 1, name)?, cart, false, &window)?,
            );
            let rad_v = arg(r, 2, name)?.expect_real(name)?.to_f64();
            let rad = if cart {
                (rad_v / (window.xmax - window.xmin) * (SCREEN_W as f64 - 1.0)).round() as i64
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
            Ok(Value::from_i64(1))
        }
        B::ArcP | B::Arc => {
            let (g, r) = split_grob(args);
            if !(3..=6).contains(&r.len()) {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "ARC expects a center, pixel radius or radius pair, optional angles and color",
                ));
            }
            let center = (
                coord(&r[0], cart, true, &window)?,
                coord(&r[1], cart, false, &window)?,
            );
            let radius = |v: &Value| v.expect_real(name).map(|v| v.to_f64().trunc() as i64);
            let radii = match &r[2] {
                Value::List(values) if values.len() == 2 => {
                    (radius(&values[0])?, radius(&values[1])?)
                }
                Value::List(_) => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "ARC radius pair needs two values",
                    ));
                }
                value => {
                    let r = radius(value)?;
                    (r, r)
                }
            };
            let (angles, color) = if r.len() <= 4 {
                ((0., std::f64::consts::TAU), r.get(3))
            } else {
                (
                    (
                        m.settings.to_radians(r[3].expect_real(name)?),
                        m.settings.to_radians(r[4].expect_real(name)?),
                    ),
                    r.get(5),
                )
            };
            let (edge, fill) = match color {
                None => (BLACK, None),
                Some(Value::List(values)) if values.len() == 2 => {
                    (color_of(&values[0])?, Some(color_of(&values[1])?))
                }
                Some(Value::List(_)) => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "ARC color pair needs edge and fill colors",
                    ));
                }
                Some(value) => (color_of(value)?, None),
            };
            let grob = grob_mut(m, g)?;
            if radii.0 == radii.1 && fill.is_none_or(|color| color >> 24 == 255) {
                grob.arc(center.0, center.1, radii.0, angles.0, angles.1, edge);
            } else {
                grob.ellipse_arc(center, radii, angles, edge, fill)
                    .map_err(|detail| {
                        RuntimeError::new(
                            ErrorKind::ResourceLimit(crate::error::ResourceKind::Memory),
                            detail,
                        )
                    })?;
            }
            Ok(Value::from_i64(1))
        }
        B::TextoutP | B::Textout => {
            let value = arg(args, 0, name)?;
            let units = if value.is_string() {
                value.expect_string_units(name)?.into_owned()
            } else {
                super::strings::menu_label(m, value)?
                    .expect_string_units(name)?
                    .into_owned()
            };
            let (g, r) = split_grob(&args[1..]);
            let (x, y) = (
                coord(arg(r, 0, name)?, cart, true, &window)?,
                coord(arg(r, 1, name)?, cart, false, &window)?,
            );
            let font = r
                .get(2)
                .map(|v| v.expect_i64(name))
                .transpose()?
                .unwrap_or(0);
            let color = opt_color(r, 3, BLACK)?;
            let width = r
                .get(4)
                .map(|v| {
                    let value = v.expect_real(name)?;
                    value.to_i64().filter(|width| *width >= 0).ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::BadArgValue,
                            "Text width must be a nonnegative integer",
                        )
                    })
                })
                .transpose()?;
            let bg = r.get(5).map(color_of).transpose()?;
            // The installed G1 ignores LF in TEXTOUT strings (it does not
            // start a new line). PRINT/dialog text retains multiline layout.
            let units = units
                .into_iter()
                .filter(|u| !matches!(u, 0 | 10 | 13))
                .collect::<Vec<_>>();
            let px = text::font_px(font, m.settings.font_px);
            let end = text::draw_utf16_units(grob_mut(m, g)?, &units, x, y, px, color, width, bg);
            Ok(Value::from_i64(end))
        }
        B::BlitP | B::Blit => blit(m, args, name, cart, &window),
        B::DimgrobP | B::Dimgrob => {
            let Value::Grob(n) = arg(args, 0, name)? else {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "DIMGROB_P needs G0–G9 as first argument",
                ));
            };
            let dimensions;
            let arguments = if args.len() == 2 {
                let list = args[1].expect_list(name)?;
                if list.len() != 2 {
                    return Err(RuntimeError::new(
                        ErrorKind::Unsupported,
                        format!(
                            "{name}: packed graphics data representation is not verified; the dimension pair {{width,height}} is supported"
                        ),
                    ));
                }
                dimensions = list.iter().cloned().collect::<Vec<_>>();
                dimensions.as_slice()
            } else {
                &args[1..]
            };
            let w = arg(arguments, 0, name)?.expect_i64(name)?;
            let h = arg(arguments, 1, name)?.expect_i64(name)?;
            if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    format!("graphic size {w}×{h}"),
                ));
            }
            m.check_len((w * h) as usize)?;
            let graphic = if let Some(Value::List(words)) = arguments.get(2) {
                // Local firmware controls PACKED/GEOM/PACKFMT establish that
                // this is a fourth-argument raw-data overload. Each entry
                // contributes one 64-bit word, least-significant pixel first.
                // A full 32-bit plane selects ARGB32; shorter data uses RGB555
                // with bit 15 transparent. Missing pixels retain white.
                let count = (w * h) as usize;
                let wide = words.len() >= count.div_ceil(2);
                let per_word = if wide { 2 } else { 4 };
                let mut graphic = Grob::new(w as usize, h as usize, WHITE);
                let packed = words
                    .iter()
                    .map(|value| match value {
                        Value::Int(value) => Ok(value.unsigned()),
                        value => value.expect_i64(name).map(|value| value as u64),
                    })
                    .collect::<RtResult<Vec<_>>>()?;
                for (index, pixel) in graphic.pixels_mut().iter_mut().enumerate() {
                    if let Some(word) = packed.get(index / per_word) {
                        *pixel = if wide {
                            (word >> (32 * (index % 2))) as u32
                        } else {
                            let color = (word >> (16 * (index % 4))) as u16;
                            if color & 0x8000 != 0 {
                                WHITE
                            } else {
                                ppl_graphics::from_rgb555(color)
                            }
                        };
                    }
                }
                graphic
            } else {
                Grob::new(w as usize, h as usize, opt_color(arguments, 2, WHITE)?)
            };
            m.grobs[*n as usize] = Some(graphic);
            if *n == 0 {
                m.screen_dirty = true;
            }
            Ok(Value::from_i64(1))
        }
        B::SubgrobP | B::Subgrob => {
            if args.len() < 2 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "SUBGROB requires a source and target, or coordinates and target",
                ));
            }
            let Some(Value::Grob(dst)) = args.last() else {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "SUBGROB target must be G1–G9",
                ));
            };
            if *dst == 0 {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "SUBGROB target cannot be G0",
                ));
            }
            let (src, coordinates) =
                if args.len() >= 2 && matches!(args.first(), Some(Value::Grob(_))) {
                    (grob_ref(m, &args[0])?, &args[1..args.len() - 1])
                } else {
                    (grob_ref(m, &Value::Grob(0))?, &args[..args.len() - 1])
                };
            let source = m.grobs[src].as_ref().unwrap();
            let sub = match coordinates.len() {
                0 => source.clone(),
                2 | 4 => {
                    let x1 = coord(&coordinates[0], cart, true, &window)?;
                    let y1 = coord(&coordinates[1], cart, false, &window)?;
                    let (x2, y2) = if coordinates.len() == 4 {
                        (
                            coord(&coordinates[2], cart, true, &window)?,
                            coord(&coordinates[3], cart, false, &window)?,
                        )
                    } else {
                        (source.width() as i64, source.height() as i64)
                    };
                    let width = x2
                        .checked_sub(x1)
                        .filter(|n| *n > 0)
                        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
                    let height = y2
                        .checked_sub(y1)
                        .filter(|n| *n > 0)
                        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
                    let cells = width
                        .checked_mul(height)
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
                    m.check_len(cells)?;
                    source.sub(x1, y1, x2, y2)
                }
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        "SUBGROB needs zero, two or four coordinates",
                    ));
                }
            };
            m.grobs[*dst as usize] = Some(sub);
            Ok(Value::from_i64(1))
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
            Ok(Value::from_i64(1))
        }
        B::FillpolyP => {
            let (g, r) = split_grob(args);
            let pts = points(arg(r, 0, name)?)?;
            let color = opt_color(r, 1, BLACK)?;
            grob_mut(m, g)?.fill_polygon(&pts, color);
            Ok(Value::from_i64(1))
        }
        B::TriangleP | B::Triangle => {
            let (g, r) = split_grob(args);
            if r.is_empty() {
                let grob = m
                    .grobs
                    .get(g)
                    .and_then(|g| g.as_ref())
                    .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidDimension))?;
                let units = (grob.width() * grob.height()).div_ceil(2);
                m.check_len(units)?;
                return Ok(Value::str("\u{ffff}".repeat(units)));
            }
            let has_depth = mesh::z_index(r, true);
            let advanced = matches!(
                r.first(),
                Some(Value::List(_) | Value::Matrix(_) | Value::Str(_))
            ) && !((3..=4).contains(&r.len()) && r.iter().take(3).all(is_vertex));
            if advanced || has_depth.is_some() {
                let offset = args.len() - r.len();
                let position = has_depth.map(|index| index + offset);
                let (result, depth) = mesh::render(
                    m,
                    g,
                    r,
                    cart,
                    &window,
                    true,
                    position.is_some_and(|index| writable.contains(&index)),
                )?;
                if let (Some(position), Some(depth)) = (position, depth) {
                    *update = Some((position, depth));
                }
                return Ok(result);
            }
            let (vertices, colors, alpha) =
                if (3..=4).contains(&r.len()) && r.iter().take(3).all(is_vertex) {
                    if r.len() > 3 {
                        return Err(RuntimeError::new(
                            ErrorKind::Unsupported,
                            "TRIANGLE_P z-buffer writeback requires a mutable string reference",
                        ));
                    }
                    let vertices = [
                        triangle_vertex(&r[0])?,
                        triangle_vertex(&r[1])?,
                        triangle_vertex(&r[2])?,
                    ];
                    (
                        [vertices[0].0, vertices[1].0, vertices[2].0],
                        [vertices[0].1, vertices[1].1, vertices[2].1],
                        None,
                    )
                } else {
                    if r.len() > 10 {
                        return Err(RuntimeError::new(
                            ErrorKind::Unsupported,
                            "TRIANGLE_P z-buffer writeback requires a mutable string reference",
                        ));
                    }
                    let vertices = [
                        (
                            arg(r, 0, name)?.expect_real(name)?.to_f64(),
                            arg(r, 1, name)?.expect_real(name)?.to_f64(),
                        ),
                        (
                            arg(r, 2, name)?.expect_real(name)?.to_f64(),
                            arg(r, 3, name)?.expect_real(name)?.to_f64(),
                        ),
                        (
                            arg(r, 4, name)?.expect_real(name)?.to_f64(),
                            arg(r, 5, name)?.expect_real(name)?.to_f64(),
                        ),
                    ];
                    let first = opt_color(r, 6, BLACK)?;
                    let colors = if r.len() >= 9 {
                        [first, color_of(&r[7])?, color_of(&r[8])?]
                    } else {
                        [first; 3]
                    };
                    let alpha = match r.len() {
                        8 => Some(r[7].expect_i64(name)?),
                        10 => Some(r[9].expect_i64(name)?),
                        _ => None,
                    };
                    (vertices, colors, alpha)
                };
            let vertices = if cart {
                vertices.map(|(x, y)| {
                    (
                        (x - window.xmin) / (window.xmax - window.xmin) * (SCREEN_W - 1) as f64,
                        (window.ymax - y) / (window.ymax - window.ymin) * (SCREEN_H - 1) as f64,
                    )
                })
            } else {
                vertices
            };
            if alpha.is_some_and(|alpha| !(0..=255).contains(&alpha)) {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgValue,
                    "TRIANGLE_P alpha must be 0 through 255",
                ));
            }
            let interrupt = m.host.interrupt_flag();
            let grob = grob_mut(m, g)?;
            if colors[0] == colors[1] && colors[1] == colors[2] {
                let color = alpha.map_or(colors[0], |alpha| {
                    (colors[0] & 0xffffff) | ((255 - alpha) as u32) << 24
                });
                grob.fill_polygon(&vertices, color);
            } else {
                triangle_gradient(grob, vertices, colors, alpha, interrupt.as_deref())?;
            }
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

fn is_vertex(value: &Value) -> bool {
    match value {
        Value::Complex(_) | Value::Real(_) | Value::Int(_) => true,
        Value::List(values) => {
            (2..=4).contains(&values.len()) && values.first().is_some_and(|v| v.as_real().is_some())
        }
        Value::Matrix(matrix) => matrix.vector && (2..=4).contains(&matrix.data.len()),
        _ => false,
    }
}
fn triangle_vertex(value: &Value) -> RtResult<((f64, f64), Color)> {
    let data = match value {
        Value::Complex(value) => return Ok(((value.re.to_f64(), value.im.to_f64()), BLACK)),
        value if value.as_real().is_some() => {
            return Ok(((value.expect_real("triangle x")?.to_f64(), 0.0), BLACK));
        }
        Value::List(values) => values.as_slice(),
        Value::Matrix(matrix) if matrix.vector => matrix.data.as_slice(),
        _ => return Err(RuntimeError::kind(ErrorKind::BadArgType)),
    };
    Ok((
        (
            data[0].expect_real("triangle x")?.to_f64(),
            data[1].expect_real("triangle y")?.to_f64(),
        ),
        opt_color(data, 2, BLACK)?,
    ))
}
fn triangle_gradient(
    grob: &mut Grob,
    points: [(f64, f64); 3],
    colors: [Color; 3],
    alpha: Option<i64>,
    interrupt: Option<&std::sync::atomic::AtomicBool>,
) -> RtResult<()> {
    let cross = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let area = cross(points[0], points[1], points[2]);
    if area == 0.0 || !area.is_finite() {
        return Ok(());
    }
    let minx = points
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as usize;
    let maxx = points
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min(grob.width() as f64 - 1.0)
        .max(-1.0) as i64;
    let miny = points
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as usize;
    let maxy = points
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min(grob.height() as f64 - 1.0)
        .max(-1.0) as i64;
    if maxx < 0 || maxy < 0 {
        return Ok(());
    }
    for y in miny..=maxy as usize {
        if interrupt.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        for x in minx..=maxx as usize {
            let point = (x as f64 + 0.5, y as f64 + 0.5);
            let weights = [
                cross(points[1], points[2], point) / area,
                cross(points[2], points[0], point) / area,
                cross(points[0], points[1], point) / area,
            ];
            if weights.iter().any(|w| *w < -1e-12) {
                continue;
            }
            let mut color = 0;
            for shift in [0, 8, 16, 24] {
                let value = (0..3)
                    .map(|i| ((colors[i] >> shift) & 255) as f64 * weights[i])
                    .sum::<f64>()
                    .round()
                    .clamp(0.0, 255.0) as u32;
                color |= value << shift;
            }
            if let Some(alpha) = alpha {
                color = (color & 0xffffff) | ((255 - alpha) as u32) << 24;
            }
            grob.set(x as i64, y as i64, color);
        }
    }
    Ok(())
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
fn blit(
    m: &mut Machine,
    args: &[Value],
    name: &str,
    cartesian: bool,
    window: &PlotWindow,
) -> RtResult<Value> {
    if args.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            "BLIT requires a source graphic",
        ));
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
    if !matches!(dest_nums.len(), 0 | 2 | 4) {
        return Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            "BLIT needs zero, two or four destination coordinates",
        ));
    }
    let after = &args[src_pos + 1..];
    let src_n = grob_ref(m, &args[src_pos])?;
    let num = |v: &Value, axis_x: bool| {
        let coordinate = coord(v, cartesian, axis_x, window)?;
        // Original pixel arguments admit signed32-bit coordinates. Cartesian
        // arguments are projected/clipped separately and accept larger values.
        if !cartesian && i32::try_from(coordinate).is_err() {
            return Err(RuntimeError::new(
                ErrorKind::BadArgValue,
                "BLIT pixel coordinate exceeds signed32-bit range",
            ));
        }
        Ok(coordinate)
    };
    let src = m.grobs[src_n].as_ref().unwrap().clone();
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    let (sx1, sy1, sx2, sy2, rest) = match after.len() {
        n if n >= 4 => (
            num(&after[0], true)?,
            num(&after[1], false)?,
            num(&after[2], true)?,
            num(&after[3], false)?,
            &after[4..],
        ),
        n if n >= 2 => (
            num(&after[0], true)?,
            num(&after[1], false)?,
            sw,
            sh,
            &after[2..],
        ),
        _ => (0, 0, sw, sh, after),
    };
    let (dx1, dy1) = match dest_nums.len() {
        0 => (0, 0),
        _ => (
            num(&dest_nums[0], true)?,
            num(arg(dest_nums, 1, name)?, false)?,
        ),
    };
    let (dx2, dy2) = if dest_nums.len() >= 4 {
        (num(&dest_nums[2], true)?, num(&dest_nums[3], false)?)
    } else {
        // Compute the affine endpoint before bounding it to the host clipping
        // envelope. Intermediate subtraction must not overflow on valid,
        // mostly offscreen Cartesian rectangles.
        let end = |start: i64, first: i64, last: i64| {
            let value = i128::from(start) + i128::from(last) - i128::from(first);
            value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
        };
        (end(dx1, sx1, sx2), end(dy1, sy1, sy2))
    };
    let transparent = rest
        .first()
        .map(|value| {
            if matches!(value,Value::Real(number) if number.is_negative()) {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "BLIT transparency color must be nonnegative",
                ));
            }
            color_of(value)
        })
        .transpose()?;
    let alpha = rest
        .get(1)
        .map(|v| {
            v.expect_real(name)?
                .to_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::BadArgValue,
                        "BLIT alpha must be a signed32-bit integer",
                    )
                })
        })
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
    Ok(Value::from_i64(1))
}
