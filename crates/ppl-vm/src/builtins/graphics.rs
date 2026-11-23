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
mod triangle;

/// Plot settings are the same calculator variables used by the Function app.
struct PlotWindow {
    xmin: f64,
    xmax: f64,
    ymin: f64,
    ymax: f64,
}
impl PlotWindow {
    fn from_machine(machine: &Machine) -> RtResult<Self> {
        let window = if machine.active_app == "Geometry" {
            let [xmin, xmax, ymin, ymax] = machine.geometry_plot_settings().derived_bounds();
            Self {
                xmin: xmin.to_f64(),
                xmax: xmax.to_f64(),
                ymin: ymin.to_f64(),
                ymax: ymax.to_f64(),
            }
        } else if crate::AppPlotSettings::for_app(&machine.active_app).is_some() {
            let settings = machine.plot_settings(&machine.active_app)?;
            Self {
                xmin: settings.xmin.to_f64(),
                xmax: settings.xmax.to_f64(),
                ymin: settings.ymin.to_f64(),
                ymax: settings.ymax.to_f64(),
            }
        } else {
            // Preserve existing graphics for apps whose own Plot Setup
            // variable contracts have not been established yet.
            let value = |name: &str, default: f64| -> RtResult<f64> {
                machine
                    .calculator_global(name)
                    .map(|v| v.expect_real(name).map(|n| n.to_f64()))
                    .transpose()
                    .map(|n| n.unwrap_or(default))
            };
            Self {
                xmin: value("Xmin", -10.)?,
                xmax: value("Xmax", 10.)?,
                ymin: value("Ymin", -10.)?,
                ymax: value("Ymax", 10.)?,
            }
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
    coordinate_number(
        v.expect_real("coordinate")?.to_f64(),
        cartesian,
        axis_x,
        window,
    )
}

fn coordinate_number(r: f64, cartesian: bool, axis_x: bool, window: &PlotWindow) -> RtResult<i64> {
    if !r.is_finite() {
        return Err(RuntimeError::new(
            ErrorKind::BadArgValue,
            "Coordinate exceeds drawing range",
        ));
    }
    if !cartesian {
        // Calculator pixel commands tested here floor the supplied coordinate.
        return Ok(r.floor() as i64);
    }
    Ok(cartesian_pixel(r, axis_x, window) as i64)
}

fn cartesian_pixel(value: f64, axis_x: bool, window: &PlotWindow) -> f64 {
    // Genuine G1 projects the plot rectangle to (1,0)–(319,218),
    // independently of the target GROB. Half pixels round toward positive infinity.
    let pixel = if axis_x {
        1.0 + (value - window.xmin) / (window.xmax - window.xmin) * (SCREEN_W as f64 - 2.0)
    } else {
        (window.ymax - value) / (window.ymax - window.ymin) * (SCREEN_H as f64 - 22.0)
    };
    (pixel + 0.5).floor()
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
        B::CtoPx
            | B::PxToC
            | B::Rect
            | B::Line
            | B::Pixon
            | B::Pixoff
            | B::Getpix
            | B::Circle
            | B::Arc
            | B::Textout
            | B::Blit
            | B::Triangle
            | B::Fillpoly
            | B::Subgrob
            | B::Grobw
            | B::Grobh
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
        B::CtoPx | B::PxToC => {
            let real = |value: &Value| match value {
                Value::Real(v) => Some(*v),
                Value::Int(v) => Some(v.to_real()),
                Value::Complex(v) if v.im.is_zero() => Some(v.re),
                _ => None,
            };
            let coordinates = match args {
                [Value::Complex(value)] => Some((value.re, value.im)),
                [Value::List(values)] => match values.as_slice() {
                    [x, y] => real(x).zip(real(y)),
                    _ => None,
                },
                [x, y] => real(x).zip(real(y)),
                _ => None,
            };
            if b == B::CtoPx {
                let Some((x, y)) = coordinates else {
                    return Ok(Value::list(vec![Value::zero(), Value::zero()]));
                };
                let project = |v: ppl_math::Real, x: bool| {
                    let pixel = cartesian_pixel(v.to_f64(), x, &window);
                    Value::from_i64(pixel.clamp(i32::MIN as f64, i32::MAX as f64) as i64)
                };
                return Ok(Value::list(vec![project(x, true), project(y, false)]));
            }
            let (x, y) = coordinates.unwrap_or((ppl_math::Real::ZERO, ppl_math::Real::ZERO));
            let pixel = |v: ppl_math::Real| {
                let v = v.to_f64().floor();
                if v.is_finite() && v >= i32::MIN as f64 && v <= i32::MAX as f64 {
                    Some(v)
                } else {
                    None
                }
            };
            // The original converts the pair in order: an invalid first
            // component leaves both zero; an invalid second retains the first.
            let (x, y) = match pixel(x) {
                Some(x) => (x, pixel(y).unwrap_or(0.)),
                None => (0., 0.),
            };
            let x = window.xmin + (x - 1.) * (window.xmax - window.xmin) / 318.;
            let y = window.ymax - y * (window.ymax - window.ymin) / 218.;
            Ok(Value::list(vec![
                Value::Real(ppl_math::Real::from_f64(x)?),
                Value::Real(ppl_math::Real::from_f64(y)?),
            ]))
        }
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
            let dst = *dst as usize;
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
                    let extent = |start: i64, end: i64| {
                        end.checked_sub(start)
                            .and_then(|length| usize::try_from(length).ok())
                            .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))
                    };
                    let width = extent(x1, x2)?;
                    let height = extent(y1, y2)?;
                    let cells = width
                        .checked_mul(height)
                        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidInput))?;
                    // Zero-area crops still carry dimensions, which must
                    // remain bounded even when the pixel count is zero.
                    m.check_len(cells.max(width).max(height))?;

                    // Uncovered cells retain the original destination buffer's
                    // prefix, even when the row stride changes. Aliased crops
                    // read live pixels only after resizing that buffer.
                    let mut target = m.grobs[dst]
                        .take()
                        .unwrap_or_else(|| Grob::new(0, 0, WHITE));
                    target.resize_preserving_pixels(width, height);
                    let output = (0, 0, width as i64, height as i64);
                    let input = (x1, y1, x2, y2);
                    if src == dst {
                        target.blit_in_place(output, input, None, 255);
                    } else {
                        let source = m.grobs[src].as_ref().unwrap();
                        target.blit(output, source, input, None, 255);
                    }
                    target
                }
                _ => {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        "SUBGROB needs zero, two or four coordinates",
                    ));
                }
            };
            m.grobs[dst] = Some(sub);
            Ok(Value::from_i64(1))
        }
        B::InvertP => {
            let (g, r) = split_grob(args);
            if !matches!(r.len(), 0 | 2 | 4) {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "INVERT_P requires zero, two or four coordinates",
                ));
            }
            let grob = grob_mut(m, g)?;
            let (w, h) = (grob.width() as i64, grob.height() as i64);
            let c = |i: usize, d: i64| -> RtResult<i64> {
                r.get(i)
                    .map(|v| coord(v, false, i % 2 == 0, &window))
                    .transpose()
                    .map(|x| x.unwrap_or(d))
            };
            let (x1, y1, x2, y2) = (c(0, 0)?, c(1, 0)?, c(2, w - 1)?, c(3, h - 1)?);
            grob.invert(x1, y1, x2, y2);
            Ok(Value::from_i64(1))
        }
        B::FillpolyP | B::Fillpoly => {
            let (g, r) = split_grob(args);
            if r.len() > 3 {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    format!("{name} requires points with optional color and transparency"),
                ));
            }
            let points = polygon_points(arg(r, 0, name)?, cart, &window)?;
            let mut color = opt_color(r, 1, BLACK)?;
            if let Some(alpha) = r.get(2) {
                let alpha = alpha.expect_real(name)?;
                if alpha.is_negative() {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgValue,
                        format!("{name} transparency must be nonnegative"),
                    ));
                }
                color = color & 0xff_ff_ff | (alpha.to_f64().min(255.) as u32) << 24;
            }
            let interrupt = m.host.interrupt_flag();
            if !grob_mut(m, g)?.calculator_polygon(&points, color, || {
                interrupt
                    .as_ref()
                    .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            }) {
                return Err(RuntimeError::kind(ErrorKind::Interrupted));
            }
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
                    let alpha = r
                        .get(3)
                        .map(|value| value.expect_real(name).map(|number| number.to_f64()))
                        .transpose()?;
                    let vertices = [
                        triangle_vertex(&r[0])?,
                        triangle_vertex(&r[1])?,
                        triangle_vertex(&r[2])?,
                    ];
                    (
                        [vertices[0].0, vertices[1].0, vertices[2].0],
                        [vertices[0].1, vertices[1].1, vertices[2].1],
                        alpha,
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
                        8 => Some(r[7].expect_real(name)?.to_f64()),
                        10 => Some(r[9].expect_real(name)?.to_f64()),
                        _ => None,
                    };
                    (vertices, colors, alpha)
                };
            let project = |(x, y)| -> RtResult<_> {
                Ok((
                    coordinate_number(x, cart, true, &window)?,
                    coordinate_number(y, cart, false, &window)?,
                ))
            };
            let [a, b, c] = vertices;
            let vertices = [project(a)?, project(b)?, project(c)?];
            let transparency = alpha.map_or(0, |value| value.clamp(0., 255.) as u8);
            let interrupt = m.host.interrupt_flag();
            let grob = grob_mut(m, g)?;
            triangle::draw(grob, vertices, colors, transparency, interrupt.as_deref())?;
            Ok(Value::from_i64(1))
        }
        B::GrobwP | B::GrobhP | B::Grobw | B::Grobh => {
            let index = match args.first() {
                None => 0,
                Some(Value::Grob(index)) => usize::from(*index),
                Some(_) => {
                    return Err(RuntimeError::new(
                        ErrorKind::BadArgType,
                        format!("{name} expects a graphic G0–G9"),
                    ));
                }
            };
            let width = matches!(b, B::GrobwP | B::Grobw);
            let size = m
                .grobs
                .get(index)
                .and_then(Option::as_ref)
                .map_or(0, |grob| if width { grob.width() } else { grob.height() });
            if cart {
                let (span, pixels) = if width {
                    (window.xmax - window.xmin, SCREEN_W - 2)
                } else {
                    (window.ymax - window.ymin, SCREEN_H - 22)
                };
                Ok(Value::Real(ppl_math::Real::from_f64(
                    size as f64 * span / pixels as f64,
                )?))
            } else {
                Ok(Value::from_i64(size as i64))
            }
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

fn polygon_points(v: &Value, cartesian: bool, window: &PlotWindow) -> RtResult<Vec<(i64, i64)>> {
    let point = |x: &Value, y: &Value| -> RtResult<_> {
        Ok((
            coord(x, cartesian, true, window)?,
            coord(y, cartesian, false, window)?,
        ))
    };
    let flat = |values: &[Value]| -> RtResult<_> {
        if values.len() % 2 != 0 {
            return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
        }
        values
            .chunks_exact(2)
            .map(|pair| point(&pair[0], &pair[1]))
            .collect()
    };
    let complex_point = |value: &Value| match value {
        Value::Complex(value) => point(&Value::Real(value.re), &Value::Real(value.im)),
        value => point(value, &Value::zero()),
    };
    match v {
        // Matrix storage normalizes zero-imaginary complex values to reals.
        Value::Matrix(matrix) if matrix.data.iter().any(|v| matches!(v, Value::Complex(_))) => {
            matrix.data.iter().map(complex_point).collect()
        }
        // Real vectors and matrices supply consecutive x, y pairs in storage order.
        Value::Matrix(matrix) => flat(&matrix.data),
        Value::List(list) => {
            let mut values = list.iter();
            let mut points = Vec::new();
            while let Some(value) = values.next() {
                points.push(match value {
                    Value::List(coords) if coords.len() == 2 => point(&coords[0], &coords[1])?,
                    Value::List(_) => return Err(RuntimeError::kind(ErrorKind::InvalidDimension)),
                    Value::Complex(_) => complex_point(value)?,
                    x => {
                        let y = values
                            .next()
                            .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidDimension))?;
                        point(x, y)?
                    }
                });
            }
            Ok(points)
        }
        _ => Err(RuntimeError::new(
            ErrorKind::BadArgType,
            "Polygon requires a list or matrix of points",
        )),
    }
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
    let src = m.grobs[src_n].as_ref().unwrap();
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
            // Cartesian BLIT normalizes the key to RGB. BLIT_P compares an
            // explicit key as a full pixel word, independently of opacity.
            color_of(value).map(|color| {
                if cartesian {
                    color & 0x00FF_FFFF
                } else {
                    color
                }
            })
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
    let destination = (dx1, dy1, dx2, dy2);
    let source = (sx1, sy1, sx2, sy2);
    if src_n == target {
        grob_mut(m, target)?.blit_in_place(destination, source, transparent, alpha);
    } else {
        let src = src.clone();
        grob_mut(m, target)?.blit(destination, &src, source, transparent, alpha);
    }
    Ok(Value::from_i64(1))
}
