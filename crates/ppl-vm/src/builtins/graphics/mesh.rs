//! Numeric 3D graphics described by HP User Guide pp.596/600–601.
//!
//! Native point caches are versioned, portable JSON strings. They are not HP
//! firmware heap-bearing cache strings; unrecognized cache strings are rejected.
use super::*;
use serde_json::{Value as Json, json};
const CACHE: &str = "\u{e000}HP-PRIME-NATIVE-POINTS:1:";
const DEPTH: &str = "\u{e000}HP-PRIME-NATIVE-DEPTH:1:";
#[derive(Clone, Copy, Debug)]
struct Point {
    xyz: [f64; 3],
    original: [f64; 3],
    depth: f64,
    color: Color,
}
impl Point {
    fn between(a: Self, b: Self, t: f64) -> Self {
        let mix = |x: f64, y: f64| x + (y - x) * t;
        Self {
            xyz: std::array::from_fn(|i| mix(a.xyz[i], b.xyz[i])),
            original: std::array::from_fn(|i| mix(a.original[i], b.original[i])),
            depth: mix(a.depth, b.depth),
            color: blend_color(a.color, b.color, t),
        }
    }
}
#[derive(Default)]
struct Transform {
    matrix: Option<Vec<Vec<f64>>>,
    eye: Option<[f64; 3]>,
    clip: Option<[f64; 6]>,
    normalize: bool,
}
struct Depth {
    w: usize,
    h: usize,
    values: Vec<u8>,
}
impl Depth {
    fn read(value: &Value, w: usize, h: usize) -> RtResult<Self> {
        let source = value.expect_str("z-string")?;
        let count = w * h;
        let values = if let Some(encoded) = source.strip_prefix(DEPTH) {
            let mut parts = encoded.splitn(3, ':');
            let ew = parts.next().and_then(|v| v.parse::<usize>().ok());
            let eh = parts.next().and_then(|v| v.parse::<usize>().ok());
            let data = parts.next().unwrap_or("");
            if ew != Some(w) || eh != Some(h) || data.len() != count * 2 || !data.is_ascii() {
                return Err(invalid(
                    "Native z-string dimensions do not match the graphic",
                ));
            }
            (0..count)
                .map(|i| {
                    u8::from_str_radix(&data[i * 2..i * 2 + 2], 16)
                        .map_err(|_| invalid("Invalid native z-string"))
                })
                .collect::<RtResult<Vec<_>>>()?
        } else {
            let units = source
                .chars()
                .map(|c| u16::try_from(c as u32).map_err(|_| invalid("Invalid packed z-string")))
                .collect::<RtResult<Vec<_>>>()?;
            if units.len() != count.div_ceil(2) {
                return Err(invalid("Z-string dimensions do not match the graphic"));
            }
            units
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .take(count)
                .collect()
        };
        Ok(Self { w, h, values })
    }
    fn write(&self) -> Value {
        let mut text = String::new();
        for pair in self.values.chunks(2) {
            let unit = u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&255)]);
            if let Some(ch) = char::from_u32(unit as u32) {
                text.push(ch);
            } else {
                // Rust str cannot contain isolated UTF16 surrogates. Keep such
                // native buffers lossless using an explicitly tagged fallback.
                use std::fmt::Write;
                let mut encoded = format!("{DEPTH}{}:{}:", self.w, self.h);
                for value in &self.values {
                    write!(encoded, "{value:02x}").unwrap();
                }
                return Value::str(encoded);
            }
        }
        Value::str(text)
    }
    fn accepts(&mut self, index: usize, z: f64) -> bool {
        let z = z.round().clamp(0.0, 255.0) as u8;
        if z > self.values[index] {
            false
        } else {
            self.values[index] = z;
            true
        }
    }
}
fn invalid(message: &str) -> RuntimeError {
    RuntimeError::new(ErrorKind::BadArgValue, message)
}
fn row(value: &Value) -> RtResult<Vec<Value>> {
    match value {
        Value::List(v) => Ok(v.to_vec()),
        Value::Matrix(v) if v.vector => Ok(v.data.clone()),
        _ => Err(RuntimeError::kind(ErrorKind::BadArgType)),
    }
}
fn records(value: &Value) -> RtResult<Vec<Vec<Value>>> {
    match value {
        Value::List(v) => v.iter().map(row).collect(),
        Value::Matrix(v) if !v.vector => Ok((0..v.rows).map(|r| v.row(r).to_vec()).collect()),
        Value::Matrix(v) => Ok(vec![v.data.clone()]),
        _ => Err(RuntimeError::kind(ErrorKind::BadArgType)),
    }
}
fn point(value: &Value) -> RtResult<Point> {
    let (xyz, color) = if let Value::Complex(v) = value {
        ([v.re.to_f64(), v.im.to_f64(), 0.0], BLACK)
    } else if value.as_real().is_some() {
        // The VM normalizes (x,0) complex literals to real values.
        ([number(value)?, 0.0, 0.0], BLACK)
    } else {
        let values = row(value)?;
        if let [Value::Complex(v), color] = values.as_slice() {
            ([v.re.to_f64(), v.im.to_f64(), 0.0], color_of(color)?)
        } else if let [x, Value::Int(color)] = values.as_slice() {
            // A real-axis complex point may have normalized to a real value.
            ([number(x)?, 0.0, 0.0], color.unsigned() as u32)
        } else {
            if !(2..=4).contains(&values.len()) {
                return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
            }
            let x = number(&values[0])?;
            let y = number(&values[1])?;
            let (z, color) = match values.as_slice() {
                [_, _, Value::Int(c)] => (0.0, c.unsigned() as u32),
                [_, _, z] => (number(z)?, BLACK),
                [_, _, z, c] => (number(z)?, color_of(c)?),
                _ => (0.0, BLACK),
            };
            ([x, y, z], color)
        }
    };
    Ok(Point {
        xyz,
        original: xyz,
        depth: xyz[2],
        color,
    })
}
fn number(value: &Value) -> RtResult<f64> {
    let n = value.expect_real("3D coordinate")?.to_f64();
    if !n.is_finite() {
        Err(invalid("3D coordinates must be finite"))
    } else {
        Ok(n)
    }
}
fn numeric_row<const N: usize>(value: &Value) -> RtResult<[f64; N]> {
    let values = row(value)?;
    if values.len() != N {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    let mut result = [0.0; N];
    for (target, value) in result.iter_mut().zip(values) {
        *target = number(&value)?;
    }
    Ok(result)
}
fn omitted(value: &Value) -> bool {
    value
        .as_real()
        .is_some_and(|n| n == ppl_math::Real::from_i64(-1))
}
fn transformation(value: Option<&Value>) -> RtResult<Transform> {
    let mut transform = Transform::default();
    let Some(value) = value else {
        return Ok(transform);
    };
    let (matrix, options) = match value {
        Value::List(options) if !options.is_empty() => (&options[0], Some(options.as_slice())),
        value => (value, None),
    };
    if options.is_none() && !matches!(matrix, Value::Matrix(_)) {
        return Err(invalid("Rotation must be a matrix or an options list"));
    }
    if !omitted(matrix) {
        let Value::Matrix(matrix) = matrix else {
            return Err(invalid("Rotation requires a 2x2 through 3x4 matrix or -1"));
        };
        if !(2..=3).contains(&matrix.rows) || !(2..=4).contains(&matrix.cols) {
            return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
        }
        transform.matrix = Some(
            (0..matrix.rows)
                .map(|r| matrix.row(r).iter().map(number).collect())
                .collect::<RtResult<_>>()?,
        );
    }
    if let Some(options) = options {
        if options.len() > 4 {
            return Err(invalid("Too many transformation options"));
        }
        if let Some(normalize) = options.get(1) {
            let n = normalize.expect_str("normalization")?;
            if !n.is_empty() && &*n != "N" {
                return Err(invalid("Normalization must be N or empty"));
            }
            transform.normalize = &*n == "N";
        }
        if let Some(eye) = options.get(2).filter(|v| !omitted(v)) {
            transform.eye = Some(numeric_row(eye)?);
        }
        if let Some(clip) = options.get(3) {
            let bounds = numeric_row::<6>(clip)?;
            if (0..3).any(|i| bounds[i * 2] > bounds[i * 2 + 1]) {
                return Err(invalid("Clip bounds require min <= max"));
            }
            transform.clip = Some(bounds);
        }
    }
    Ok(transform)
}
fn transform(points: &mut [Point], options: &Transform) -> RtResult<()> {
    for point in points.iter_mut() {
        if let Some(matrix) = &options.matrix {
            // Rectangular matrices multiply the matching prefix of [x,y,z,1].
            // Local firmware AFFINE confirms 2x3 consumes z, not an implicit 1.
            let input = [point.xyz[0], point.xyz[1], point.xyz[2], 1.0];
            let mut out = point.xyz;
            for (axis, row) in matrix.iter().enumerate() {
                out[axis] = row.iter().zip(input).map(|(a, b)| a * b).sum();
            }
            point.xyz = out;
        }
        if !point.xyz.iter().all(|n| n.is_finite()) {
            return Err(invalid("Transformed coordinates exceed numeric range"));
        }
        point.depth = point.xyz[2];
    }
    if options.normalize && !points.is_empty() {
        let low = points.iter().map(|p| p.depth).fold(f64::INFINITY, f64::min);
        let high = points
            .iter()
            .map(|p| p.depth)
            .fold(f64::NEG_INFINITY, f64::max);
        for point in points {
            point.depth = if low == high {
                0.0
            } else {
                if (high - low).is_finite() {
                    (point.depth - low) / (high - low) * 255.0
                } else {
                    let scale = low.abs().max(high.abs());
                    (point.depth / scale - low / scale) / (high / scale - low / scale) * 255.0
                }
            };
        }
    }
    Ok(())
}
fn cache(points: &[Point], options: &Transform) -> Value {
    Value::str(format!(
        "{CACHE}{}",
        json!({"points":points.iter().map(|p|json!([p.xyz,p.original,p.depth,p.color])).collect::<Vec<_>>(),"eye":options.eye,"clip":options.clip})
    ))
}
fn json_numbers<const N: usize>(value: &Json) -> RtResult<[f64; N]> {
    let values = value
        .as_array()
        .filter(|v| v.len() == N)
        .ok_or_else(|| invalid("Invalid native point cache"))?;
    let mut out = [0.0; N];
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| invalid("Invalid native point cache coordinate"))?;
    }
    Ok(out)
}
fn uncache(source: &str) -> RtResult<(Vec<Point>, Transform)> {
    let encoded = source.strip_prefix(CACHE).ok_or_else(|| {
        RuntimeError::new(
            ErrorKind::Unsupported,
            "Opaque firmware point caches cannot be used in Native mode",
        )
    })?;
    let json: Json =
        serde_json::from_str(encoded).map_err(|_| invalid("Invalid native point cache"))?;
    let rows = json["points"]
        .as_array()
        .ok_or_else(|| invalid("Invalid native point cache"))?;
    let points = rows
        .iter()
        .map(|row| {
            Ok(Point {
                xyz: json_numbers(&row[0])?,
                original: json_numbers(&row[1])?,
                depth: row[2]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| invalid("Invalid cached depth"))?,
                color: row[3]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| invalid("Invalid cached color"))?,
            })
        })
        .collect::<RtResult<Vec<_>>>()?;
    let options = Transform {
        eye: if json["eye"].is_null() {
            None
        } else {
            Some(json_numbers(&json["eye"])?)
        },
        clip: if json["clip"].is_null() {
            None
        } else {
            Some(json_numbers(&json["clip"])?)
        },
        ..Default::default()
    };
    if options
        .clip
        .is_some_and(|c| (0..3).any(|i| c[i * 2] > c[i * 2 + 1]))
    {
        return Err(invalid("Invalid cached clip bounds"));
    }
    Ok((points, options))
}
fn load_points(value: &Value) -> RtResult<Vec<Point>> {
    match value {
        Value::List(v) => v.iter().map(point).collect(),
        Value::Matrix(v) if v.vector => v.data.iter().map(point).collect(),
        Value::Matrix(v) => (0..v.rows)
            .map(|r| point(&Value::list(v.row(r).to_vec())))
            .collect(),
        _ => Err(RuntimeError::kind(ErrorKind::BadArgType)),
    }
}
struct PrimitiveDefinitions {
    rows: Vec<Vec<Value>>,
    color: Option<Color>,
    alpha: Option<i64>,
}
fn primitives(value: &Value) -> RtResult<PrimitiveDefinitions> {
    if let Value::List(items) = value
        && items.first().is_some_and(|v| v.as_real().is_some())
    {
        let color = if omitted(&items[0]) {
            None
        } else {
            Some(color_of(&items[0])?)
        };
        let alpha = items
            .get(1)
            .filter(|v| v.as_real().is_some())
            .map(|v| v.expect_i64("alpha"))
            .transpose()?;
        let start = if alpha.is_some() { 2 } else { 1 };
        return Ok(PrimitiveDefinitions {
            rows: items[start..].iter().map(row).collect::<RtResult<_>>()?,
            color,
            alpha,
        });
    }
    Ok(PrimitiveDefinitions {
        rows: records(value)?,
        color: None,
        alpha: None,
    })
}
fn vertex_index(value: &Value, count: usize) -> RtResult<usize> {
    let index = value.expect_i64("vertex index")?;
    if index < 1 || index as usize > count {
        Err(invalid("Vertex index out of range"))
    } else {
        Ok(index as usize - 1)
    }
}
fn project(
    point: Point,
    eye: Option<[f64; 3]>,
    cart: bool,
    window: &PlotWindow,
) -> RtResult<(f64, f64)> {
    let [mut x, mut y, z] = point.xyz;
    if let Some([ex, ey, ez]) = eye {
        if z == 0.0 {
            return Err(invalid("Projection divides by zero depth"));
        }
        x = ez / z * x - ex;
        y = ez / z * y - ey;
    }
    if cart {
        x = (x - window.xmin) / (window.xmax - window.xmin) * (SCREEN_W - 1) as f64;
        y = (window.ymax - y) / (window.ymax - window.ymin) * (SCREEN_H - 1) as f64;
    }
    if !x.is_finite() || !y.is_finite() {
        return Err(invalid("Projected coordinate exceeds numeric range"));
    }
    Ok((x, y))
}
fn clip_line(a: Point, b: Point, bounds: Option<[f64; 6]>) -> Option<(Point, Point)> {
    let Some(bounds) = bounds else {
        return Some((a, b));
    };
    let (mut near, mut far) = (0.0f64, 1.0f64);
    for axis in 0..3 {
        let delta = b.original[axis] - a.original[axis];
        if delta == 0.0 {
            if a.original[axis] < bounds[axis * 2] || a.original[axis] > bounds[axis * 2 + 1] {
                return None;
            }
        } else {
            let p = (bounds[axis * 2] - a.original[axis]) / delta;
            let q = (bounds[axis * 2 + 1] - a.original[axis]) / delta;
            near = near.max(p.min(q));
            far = far.min(p.max(q));
            if near > far {
                return None;
            }
        }
    }
    Some((Point::between(a, b, near), Point::between(a, b, far)))
}
fn clip_polygon(mut points: Vec<Point>, bounds: Option<[f64; 6]>) -> Vec<Point> {
    if let Some(bounds) = bounds {
        for axis in 0..3 {
            for side in 0..2 {
                let value = bounds[axis * 2 + side];
                let inside = |p: Point| {
                    if side == 0 {
                        p.original[axis] >= value
                    } else {
                        p.original[axis] <= value
                    }
                };
                let mut output = Vec::new();
                if let Some(mut a) = points.last().copied() {
                    for b in &points {
                        if inside(a) != inside(*b) {
                            let t =
                                (value - a.original[axis]) / (b.original[axis] - a.original[axis]);
                            output.push(Point::between(a, *b, t));
                        }
                        if inside(*b) {
                            output.push(*b);
                        }
                        a = *b;
                    }
                }
                points = output;
            }
        }
    }
    points
}
fn blend_color(a: Color, b: Color, t: f64) -> Color {
    (0..4).fold(0, |c, i| {
        let s = i * 8;
        let a = ((a >> s) & 255) as f64;
        let b = ((b >> s) & 255) as f64;
        c | ((a + (b - a) * t).round().clamp(0.0, 255.0) as u32) << s
    })
}
fn opacity(color: Color, alpha: Option<i64>) -> Color {
    alpha.map_or(color, |a| (color & 0xffffff) | ((255 - a) as u32) << 24)
}
fn check_interrupt(flag: Option<&std::sync::atomic::AtomicBool>) -> RtResult<()> {
    if flag.is_some_and(|f| f.load(std::sync::atomic::Ordering::Acquire)) {
        Err(RuntimeError::new(
            ErrorKind::Interrupted,
            "Graphics interrupted",
        ))
    } else {
        Ok(())
    }
}
fn draw_line(
    grob: &mut Grob,
    a: Point,
    b: Point,
    positions: [(f64, f64); 2],
    alpha: Option<i64>,
    depth: &mut Option<Depth>,
    interrupt: Option<&std::sync::atomic::AtomicBool>,
) -> RtResult<()> {
    let [(x1, y1), (x2, y2)] = positions;
    if !(x2 - x1).is_finite() || !(y2 - y1).is_finite() {
        return Err(invalid("Line span exceeds numeric range"));
    }
    let (mut low, mut high) = (0.0f64, 1.0f64);
    for (start, delta, min, max) in [
        (x1, x2 - x1, 0.0, (grob.width() - 1) as f64),
        (y1, y2 - y1, 0.0, (grob.height() - 1) as f64),
    ] {
        if delta == 0.0 {
            if start < min || start > max {
                return Ok(());
            }
        } else {
            let p = (min - start) / delta;
            let q = (max - start) / delta;
            low = low.max(p.min(q));
            high = high.min(p.max(q));
            if low > high {
                return Ok(());
            }
        }
    }
    let a = Point::between(a, b, low);
    let b = Point::between(a, b, (high - low) / (1.0 - low).max(f64::EPSILON));
    let start = (x1 + (x2 - x1) * low, y1 + (y2 - y1) * low);
    let end = (x1 + (x2 - x1) * high, y1 + (y2 - y1) * high);
    let steps = ((end.0 - start.0).abs().max((end.1 - start.1).abs()).ceil() as usize).max(1);
    for step in 0..=steps {
        if step % 256 == 0 {
            check_interrupt(interrupt)?;
        }
        let t = step as f64 / steps as f64;
        let x = (start.0 + (end.0 - start.0) * t).round() as i64;
        let y = (start.1 + (end.1 - start.1) * t).round() as i64;
        let z = a.depth + (b.depth - a.depth) * t;
        if depth
            .as_mut()
            .is_none_or(|d| d.accepts(y as usize * grob.width() + x as usize, z))
        {
            grob.set(x, y, opacity(blend_color(a.color, b.color, t), alpha));
        }
    }
    Ok(())
}
fn draw_triangle(
    grob: &mut Grob,
    vertices: [Point; 3],
    positions: [(f64, f64); 3],
    alpha: Option<i64>,
    depth: &mut Option<Depth>,
    interrupt: Option<&std::sync::atomic::AtomicBool>,
) -> RtResult<()> {
    let cross = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };
    let area = cross(positions[0], positions[1], positions[2]);
    if area == 0.0 || !area.is_finite() {
        return Ok(());
    }
    let minx = positions
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as usize;
    let maxx = positions
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min((grob.width() - 1) as f64) as usize;
    let miny = positions
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as usize;
    let maxy = positions
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min((grob.height() - 1) as f64) as usize;
    if minx > maxx || miny > maxy {
        return Ok(());
    }
    for y in miny..=maxy {
        check_interrupt(interrupt)?;
        for x in minx..=maxx {
            let p = (x as f64 + 0.5, y as f64 + 0.5);
            let weights = [
                cross(positions[1], positions[2], p) / area,
                cross(positions[2], positions[0], p) / area,
                cross(positions[0], positions[1], p) / area,
            ];
            if weights.iter().all(|w| *w >= -1e-10) {
                let z = (0..3).map(|i| weights[i] * vertices[i].depth).sum();
                if depth
                    .as_mut()
                    .is_none_or(|d| d.accepts(y * grob.width() + x, z))
                {
                    let color = (0..4).fold(0, |color, channel| {
                        color
                            | ((0..3)
                                .map(|i| {
                                    weights[i] * ((vertices[i].color >> (channel * 8)) & 255) as f64
                                })
                                .sum::<f64>()
                                .round()
                                .clamp(0.0, 255.0) as u32)
                                << (channel * 8)
                    });
                    grob.set(x as i64, y as i64, opacity(color, alpha));
                }
            }
        }
    }
    Ok(())
}
/// Returns the relative z-string argument of an advanced or direct triangle call.
pub(super) fn z_index(args: &[Value], triangles: bool) -> Option<usize> {
    if matches!(args.first(), Some(Value::Str(_))) {
        return (args.len() == 3 && matches!(args[2], Value::Str(_))).then_some(2);
    }
    if triangles && args.len() == 4 && args.iter().take(3).all(is_vertex) {
        return matches!(args.last(), Some(Value::Str(_))).then_some(args.len() - 1);
    }
    if matches!(args.first(), Some(Value::List(_) | Value::Matrix(_))) {
        return (args.len() == 4 && matches!(args[3], Value::Str(_))).then_some(3);
    }
    if triangles && args.len() >= 4 && matches!(args.get(args.len() - 4), Some(Value::Str(_))) {
        return Some(args.len() - 4);
    }
    None
}
/// Basic triangles with a mutable depth buffer reuse this renderer too.
pub(super) fn render(
    machine: &mut Machine,
    g: usize,
    args: &[Value],
    cart: bool,
    window: &PlotWindow,
    triangles: bool,
    writable: bool,
) -> RtResult<(Value, Option<Value>)> {
    let zpos = z_index(args, triangles);
    if zpos.is_some() && !writable {
        return Err(RuntimeError::new(
            ErrorKind::Unsupported,
            "Z-string must be a writable variable",
        ));
    }
    let direct = triangles && (3..=4).contains(&args.len()) && args.iter().take(3).all(is_vertex);
    let scalar = triangles
        && !matches!(
            args.first(),
            Some(Value::Str(_) | Value::List(_) | Value::Matrix(_))
        );
    let (mut points, options, rows, default_color, default_alpha, is_cache) = if direct || scalar {
        let (points, alpha) = if direct {
            if args.len() != 4 {
                return Err(invalid(
                    "Depth triangle expects three vertices and a z-string",
                ));
            }
            let mut p = Vec::new();
            for v in &args[..3] {
                if matches!(v, Value::Complex(_) | Value::Real(_) | Value::Int(_)) {
                    p.push(point(v)?);
                    continue;
                }
                let data = row(v)?;
                let xyz = [
                    number(&data[0])?,
                    number(&data[1])?,
                    data.get(3).map(number).transpose()?.unwrap_or(0.0),
                ];
                p.push(Point {
                    xyz,
                    original: xyz,
                    depth: xyz[2],
                    color: opt_color(&data, 2, BLACK)?,
                });
            }
            (p, None)
        } else {
            let index = zpos.ok_or_else(|| invalid("Missing triangle z-string"))?;
            if args.len() != index + 4 || !(7..=10).contains(&index) {
                return Err(invalid("Invalid scalar triangle depth arguments"));
            }
            let first = opt_color(args, 6, BLACK)?;
            let colors = if index >= 9 {
                [first, color_of(&args[7])?, color_of(&args[8])?]
            } else {
                [first; 3]
            };
            let alpha = if index == 8 {
                Some(args[7].expect_i64("alpha")?)
            } else if index == 10 {
                Some(args[9].expect_i64("alpha")?)
            } else {
                None
            };
            let mut p = Vec::new();
            for i in 0..3 {
                let xyz = [
                    number(&args[i * 2])?,
                    number(&args[i * 2 + 1])?,
                    number(&args[index + 1 + i])?,
                ];
                p.push(Point {
                    xyz,
                    original: xyz,
                    depth: xyz[2],
                    color: colors[i],
                });
            }
            (p, alpha)
        };
        (
            points,
            Transform::default(),
            vec![vec![
                Value::from_i64(1),
                Value::from_i64(2),
                Value::from_i64(3),
            ]],
            None,
            alpha,
            false,
        )
    } else {
        let (points, options) = if let Some(Value::Str(source)) = args.first() {
            if args.len() < 2
                || args.len() > 3
                || (args.len() == 3 && !matches!(args[2], Value::Str(_)))
            {
                return Err(invalid(
                    "Cached graphics expect points, primitives and optional z-string",
                ));
            }
            uncache(source)?
        } else {
            if !(3..=4).contains(&args.len())
                || (args.len() == 4 && !matches!(args[3], Value::Str(_)))
            {
                return Err(invalid(
                    "3D graphics expect points, primitives, transform and optional z-string",
                ));
            }
            let mut points = load_points(arg(args, 0, "3D graphics")?)?;
            let options = transformation(args.get(2))?;
            transform(&mut points, &options)?;
            (points, options)
        };
        let PrimitiveDefinitions { rows, color, alpha } = primitives(arg(args, 1, "3D graphics")?)?;
        (points, options, rows, color, alpha, true)
    };
    machine.check_len(points.len())?;
    let grob = machine
        .grobs
        .get(g)
        .and_then(Option::as_ref)
        .ok_or_else(|| RuntimeError::kind(ErrorKind::InvalidDimension))?;
    let mut target = grob.clone();
    let mut depth = zpos
        .map(|i| Depth::read(&args[i], grob.width(), grob.height()))
        .transpose()?;
    if depth.is_some() && points.iter().any(|p| !(0.0..=255.0).contains(&p.depth)) {
        return Err(invalid(
            "Z clipping requires depth from 0 through 255 or N normalization",
        ));
    }
    let required = if triangles { 3 } else { 2 };
    let interrupt = machine.host.interrupt_flag();
    for primitive in rows {
        machine.check_limits()?;
        if primitive.len() < required || primitive.len() > required + 2 {
            return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
        }
        let indices = primitive[..required]
            .iter()
            .map(|v| vertex_index(v, points.len()))
            .collect::<RtResult<Vec<_>>>()?;
        let color = primitive
            .get(required)
            .filter(|v| !omitted(v))
            .map(color_of)
            .transpose()?
            .or(default_color);
        let alpha = primitive
            .get(required + 1)
            .map(|v| v.expect_i64("alpha"))
            .transpose()?
            .or(default_alpha);
        if alpha.is_some_and(|a| !(0..=255).contains(&a)) {
            return Err(invalid("Alpha must be 0 through 255"));
        }
        let selected = indices
            .iter()
            .map(|&i| {
                let mut p = points[i];
                if let Some(color) = color {
                    p.color = color;
                }
                p
            })
            .collect::<Vec<_>>();
        if triangles {
            let polygon = clip_polygon(selected, options.clip);
            for i in 1..polygon.len().saturating_sub(1) {
                let vertices = [polygon[0], polygon[i], polygon[i + 1]];
                let positions = [
                    project(vertices[0], options.eye, cart, window)?,
                    project(vertices[1], options.eye, cart, window)?,
                    project(vertices[2], options.eye, cart, window)?,
                ];
                draw_triangle(
                    &mut target,
                    vertices,
                    positions,
                    alpha,
                    &mut depth,
                    interrupt.as_deref(),
                )?;
            }
        } else if let Some((a, b)) = clip_line(selected[0], selected[1], options.clip) {
            let positions = [
                project(a, options.eye, cart, window)?,
                project(b, options.eye, cart, window)?,
            ];
            draw_line(
                &mut target,
                a,
                b,
                positions,
                alpha,
                &mut depth,
                interrupt.as_deref(),
            )?;
        }
    }
    let result = if is_cache {
        cache(&points, &options)
    } else {
        Value::zero()
    };
    let updated = depth.map(|d| d.write());
    if let Value::Str(text) = &result {
        machine.check_len(text.chars().count())?;
    }
    if let Some(Value::Str(text)) = &updated {
        machine.check_len(text.chars().count())?;
    }
    check_interrupt(interrupt.as_deref())?;
    machine.grobs[g] = Some(target);
    if g == 0 {
        machine.screen_dirty = true;
    }
    points.clear();
    Ok((result, updated))
}
