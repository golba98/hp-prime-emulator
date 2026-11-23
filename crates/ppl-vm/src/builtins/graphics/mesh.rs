//! Numeric 3D graphics described by HP User Guide pp.596/600–601.
//!
//! Native point caches are versioned, portable JSON strings. They are not HP
//! firmware heap-bearing cache strings; unrecognized cache strings are rejected.
use super::*;
use ppl_math::Real;
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
    source_z_transform: Option<[f64; 4]>,
}
struct Depth {
    values: Vec<u8>,
}
impl Depth {
    fn read(value: &Value, w: usize, h: usize) -> RtResult<Self> {
        let count = w * h;
        // Keep old explicitly tagged Native images readable. New z-strings
        // always use the calculator's exact packed 16-bit representation.
        let legacy = match value {
            Value::Str(text) => text.strip_prefix(DEPTH),
            _ => None,
        };
        let values = if let Some(encoded) = legacy {
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
            let units = value.expect_string_units("z-string")?;
            if units.len() != count.div_ceil(2) {
                return Err(invalid("Z-string dimensions do not match the graphic"));
            }
            units
                .iter()
                .copied()
                .flat_map(u16::to_le_bytes)
                .take(count)
                .collect()
        };
        Ok(Self { values })
    }
    fn write(&self) -> Value {
        Value::from_utf16(
            self.values
                .chunks(2)
                .map(|pair| u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&255)]))
                .collect::<Vec<_>>(),
        )
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
    fn accepts_line(&mut self, index: usize, z: i64) -> bool {
        let held = &mut self.values[index];
        if z >= i64::from(*held) {
            return false;
        }
        // A steep descending line can extrapolate a negative signed depth.
        // Compare it before writing the low byte back into the buffer.
        *held = z as u8;
        true
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
        if !(2..=4).contains(&values.len()) {
            return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
        }
        let x = number(&values[0])?;
        let y = number(&values[1])?;
        let (z, color) = match values.as_slice() {
            [_, _, Value::Int(c)] => (0.0, color_of(&Value::Int(*c))?),
            [_, _, z] => (number(z)?, BLACK),
            [_, _, z, c] => (number(z)?, color_of(c)?),
            _ => (0.0, BLACK),
        };
        ([x, y, z], color)
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
    transform.source_z_transform = match &transform.matrix {
        None => Some([0.0, 0.0, 1.0, 0.0]),
        Some(matrix) => match matrix.get(2).map(Vec::as_slice) {
            Some([x, y, scale, rest @ ..]) if *scale != 0.0 => {
                Some([*x, *y, *scale, rest.first().copied().unwrap_or(0.0)])
            }
            _ => None,
        },
    };
    Ok(transform)
}
fn transform(points: &mut [Point], options: &Transform, flat_pixel_line: bool) -> RtResult<()> {
    for point in points.iter_mut() {
        if let Some(matrix) = &options.matrix {
            // Rectangular matrices multiply the matching prefix of [x,y,z,1].
            // Local firmware AFFINE confirms 2x3 consumes z, not an implicit 1.
            let input = [point.xyz[0], point.xyz[1], point.xyz[2], 1.0];
            let mut out = point.xyz;
            for (axis, row) in matrix.iter().enumerate() {
                out[axis] = row.iter().zip(input).map(|(a, b)| a * b).sum();
            }
            // A two-row pixel line transform keeps source-space clipping
            // but projects onto depth zero when there is no eye projection.
            if flat_pixel_line && matrix.len() == 2 {
                out[2] = 0.0;
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
    let mut payload = json!({"points":points.iter().map(|p|json!([p.xyz,p.original,p.depth,p.color])).collect::<Vec<_>>(),"eye":options.eye,"clip":options.clip});
    if options.normalize {
        payload["normalized"] = Json::Bool(true);
        if clipped_source_z(points, options.clip).is_some()
            && let Some(transform) = options.source_z_transform
        {
            match transform {
                [0.0, 0.0, 1.0, 0.0] => payload["source_z_preserved"] = Json::Bool(true),
                [0.0, 0.0, scale, offset] => payload["source_z_affine"] = json!([scale, offset]),
                _ => payload["source_z_projection"] = json!(transform),
            }
        }
    }
    Value::str(format!("{CACHE}{payload}"))
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
        normalize: match json.get("normalized") {
            Some(value) => value
                .as_bool()
                .ok_or_else(|| invalid("Invalid cached normalization flag"))?,
            // Older caches retained normalized depth separately from z.
            None => points.iter().any(|p| p.depth != p.xyz[2]),
        },
        source_z_transform: match (
            json.get("source_z_projection"),
            json.get("source_z_affine"),
            json.get("source_z_preserved"),
        ) {
            (Some(value), None, None) => {
                let transform = json_numbers::<4>(value)?;
                if transform[2] == 0.0 {
                    return Err(invalid("Invalid cached source z scale"));
                }
                Some(transform)
            }
            (None, Some(value), None) => {
                let [scale, offset] = json_numbers::<2>(value)?;
                if scale == 0.0 {
                    return Err(invalid("Invalid cached source z scale"));
                }
                Some([0.0, 0.0, scale, offset])
            }
            (None, None, Some(value)) => value
                .as_bool()
                .ok_or_else(|| invalid("Invalid cached source z flag"))?
                .then_some([0.0, 0.0, 1.0, 0.0]),
            (None, None, None) => None,
            _ => return Err(invalid("Conflicting cached source z transforms")),
        },
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
fn has_complex_cell(values: &[Value]) -> bool {
    values
        .iter()
        .any(|value| matches!(value, Value::Complex(_)))
}

fn packed_point(values: &[Value], max_columns: usize) -> RtResult<Point> {
    if !(1..=max_columns).contains(&values.len()) {
        return Err(RuntimeError::kind(ErrorKind::InvalidDimension));
    }
    // Complex rows encode (x, y) in their first cell and (z, color)
    // in the optional second cell. A real cell has imaginary part zero.
    let components = |value: &Value| -> RtResult<_> {
        match value {
            Value::Complex(value) => Ok((value.re, value.im)),
            value => Ok((value.expect_real("mesh point")?, ppl_math::Real::ZERO)),
        }
    };
    let (x, y) = components(&values[0])?;
    let (z, color) = if let Some(value) = values.get(1) {
        let (z, color) = components(value)?;
        // The color is truncated and clamped to the nonnegative signed
        // 32-bit range before using its low three RGB bytes.
        let color = (number(&Value::Real(color))? as i32).max(0) as Color & 0xff_ff_ff;
        (z, color)
    } else {
        (ppl_math::Real::ZERO, BLACK)
    };
    let xyz = [
        number(&Value::Real(x))?,
        number(&Value::Real(y))?,
        number(&Value::Real(z))?,
    ];
    Ok(Point {
        xyz,
        original: xyz,
        depth: xyz[2],
        color,
    })
}

fn load_points(value: &Value, packed_vectors: bool) -> RtResult<Vec<Point>> {
    match value {
        Value::List(v) => v
            .iter()
            .map(|value| match value {
                Value::Matrix(row)
                    if packed_vectors && row.vector && has_complex_cell(&row.data) =>
                {
                    packed_point(&row.data, 2)
                }
                value => point(value),
            })
            .collect(),
        Value::Matrix(v) if v.vector => v.data.iter().map(point).collect(),
        Value::Matrix(v) if has_complex_cell(&v.data) => {
            (0..v.rows).map(|r| packed_point(v.row(r), 3)).collect()
        }
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
/// Original line controls establish an inclusive pixel count and strictly
/// lower interior channel boundary. Constant components retain their value.
fn line_color(a: Color, b: Color, t: f64) -> Color {
    if t == 0. {
        return a;
    }
    (0..4).fold(0, |color, index| {
        let shift = index * 8;
        let a = (a >> shift) & 255;
        let b = (b >> shift) & 255;
        let channel = if a == b {
            a
        } else {
            ((a as f64 + (b as f64 - a as f64) * t).ceil() - 1.).clamp(0., 255.) as u32
        };
        color | channel << shift
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
const LINE_NORMALIZED_MAX_DEPTH: i64 = 253;

fn line_depth_number(value: f64) -> Option<Real> {
    if value.abs() < 1e12 && value.fract() == 0.0 {
        // A small integral depth already fits the calculator's decimal
        // precision. Avoid formatting and reparsing it on cached redraw.
        Some(Real::from_i64(value as i64).decimal())
    } else {
        Real::from_f64(value).ok()
    }
}

fn clipped_source_z(points: &[Point], clip: Option<[f64; 6]>) -> Option<[f64; 6]> {
    let bounds = clip?;
    points
        .iter()
        .any(|point| !(bounds[4]..=bounds[5]).contains(&point.original[2]))
        .then_some(bounds)
}

struct LineNormalization {
    low: Real,
    scale: Real,
    clipped_depth_maximum: Option<ClippedDepthMaximum>,
}

struct ClippedDepthMaximum {
    source: Option<f64>,
    transformed: f64,
    reversed: bool,
}

impl LineNormalization {
    fn new(points: &[Point], options: &Transform) -> Option<Self> {
        let first = points.first()?.xyz[2];
        let (mut low, mut high) = points.iter().fold((first, first), |(low, high), point| {
            (low.min(point.xyz[2]), high.max(point.xyz[2]))
        });
        let mut clipped_depth_maximum = None;
        if let Some(bounds) = clipped_source_z(points, options.clip) {
            let transform = options.source_z_transform?;
            let transform_z = |[x, y, z]: [f64; 3]| {
                transform
                    .into_iter()
                    .zip([x, y, z, 1.0])
                    .map(|(scale, coordinate)| scale * coordinate)
                    .sum::<f64>()
            };
            let corner = |high: bool| {
                std::array::from_fn(|axis| {
                    let upper = (transform[axis] >= 0.0) == high;
                    bounds[2 * axis + usize::from(upper)]
                })
            };
            let lower_depth = transform_z(corner(false));
            let upper_depth = transform_z(corner(true));
            if bounds[4].fract() != 0.0
                || bounds[5].fract() != 0.0
                || [lower_depth, upper_depth]
                    .into_iter()
                    .any(|z| z.fract() != 0.0)
                || points.iter().any(|point| {
                    point.original[2].fract() != 0.0
                        || point.xyz[2].fract() != 0.0
                        || point.xyz[2] != transform_z(point.original)
                })
            {
                return None;
            }
            // Transform the source clipping box to a depth interval. Clamp
            // the original depth extrema, including unused vertices.
            low = low.max(lower_depth);
            high = high.min(upper_depth);
            if low > high {
                return None;
            }
            let reversed = transform[2] < 0.0;
            let source =
                (transform[..2] == [0.0, 0.0]).then_some(bounds[if reversed { 4 } else { 5 }]);
            clipped_depth_maximum = Some(ClippedDepthMaximum {
                source,
                transformed: upper_depth,
                reversed,
            });
        }
        let low = line_depth_number(low)?;
        let range = line_depth_number(high)?.sub(low).ok()?;
        let scale = if range.is_zero() {
            Real::ZERO
        } else {
            Real::from_i64(LINE_NORMALIZED_MAX_DEPTH).div(range).ok()?
        };
        Some(Self {
            low,
            scale,
            clipped_depth_maximum,
        })
    }

    fn depth(&self, point: Point, original_z: f64) -> Option<i64> {
        let z = line_depth_number(point.xyz[2])?;
        if let Some(plane) = &self.clipped_depth_maximum {
            if !z.is_integer() {
                return None;
            }
            // A newly clipped endpoint on the maximum-depth plane receives
            // the maximum word. An existing endpoint uses the decimal scale.
            let outside = plane.source.is_some_and(|boundary| {
                if plane.reversed {
                    original_z < boundary
                } else {
                    original_z > boundary
                }
            });
            if outside && z == line_depth_number(plane.transformed)? && !self.scale.is_zero() {
                return Some(LINE_NORMALIZED_MAX_DEPTH);
            }
        }
        let z = z.sub(self.low).ok()?;
        let depth = z.mul_trunc_to_i64(self.scale)?;
        (0..=255).contains(&depth).then_some(depth)
    }
}

const LINE_FIXED_UNIT: i64 = 1 << 16;

fn fixed_line_color(a: Color, b: Color, fraction: i64) -> Color {
    (0..4).fold(0, |color, index| {
        let shift = index * 8;
        let a = i64::from((a >> shift) & 255);
        let b = i64::from((b >> shift) & 255);
        let channel = (a * LINE_FIXED_UNIT + (b - a) * fraction) / LINE_FIXED_UNIT;
        color | (channel as u32) << shift
    })
}

fn clipped_line_colors(
    mut points: [Point; 2],
    bounds: Option<[f64; 6]>,
    reverse: bool,
) -> [Color; 2] {
    if let Some(bounds) = bounds {
        // The color accumulator quantizes each clipped endpoint before
        // clipping the next source plane.
        for axis in 0..3 {
            for side in 0..2 {
                let [a, b] = points;
                let bound = bounds[2 * axis + side];
                let outside = |p: Point| {
                    let coordinate = p.original[axis];
                    if side == 0 {
                        coordinate < bound
                    } else {
                        coordinate > bound
                    }
                };
                let index = match (outside(a), outside(b)) {
                    (true, false) => 0,
                    (false, true) => 1,
                    _ => continue,
                };
                let fraction = (bound - a.original[axis]) / (b.original[axis] - a.original[axis]);
                let mut point = Point::between(a, b, fraction);
                point.original[axis] = bound;
                point.color =
                    fixed_line_color(a.color, b.color, (fraction * LINE_FIXED_UNIT as f64) as i64);
                points[index] = point;
            }
        }
    }
    if reverse {
        points.reverse();
    }
    points.map(|p| p.color)
}

struct LineDepth {
    first: i64,
    second: i64,
    origin: i64,
    on_x: bool,
    count: i64,
    reciprocal: i64,
}

impl LineDepth {
    fn new(depths: [i64; 2], positions: [(f64, f64); 2]) -> Option<Self> {
        if positions.iter().any(|&(x, y)| {
            [x, y]
                .into_iter()
                .any(|v| v.fract() != 0.0 || v < f64::from(i32::MIN) || v > f64::from(i32::MAX))
        }) {
            return None;
        }
        let [mut first, mut second] = depths;
        let [(x1, y1), (x2, y2)] = positions.map(|(x, y)| (x as i64, y as i64));
        // Depth endpoints are ordered by x. A steep line advances by y with
        // an inclusive signed span, while a diagonal uses the x span length.
        let (dx, dy) = if x1 > x2 {
            std::mem::swap(&mut first, &mut second);
            (x1 - x2, y1 - y2)
        } else {
            (x2 - x1, y2 - y1)
        };
        let on_x = dx > dy.abs();
        let count = if dy.abs() > dx {
            (dy + 1).abs().max(2)
        } else {
            dx + 1
        };
        if i32::try_from(count).is_err() {
            return None;
        }
        Some(Self {
            first: if dx == 0 && dy == 0 {
                (first + second) / 2
            } else {
                first
            },
            second,
            origin: if on_x { x1.min(x2) } else { y1.min(y2) },
            on_x,
            count,
            reciprocal: LINE_FIXED_UNIT / count,
        })
    }

    fn offset_at(&self, x: i64, y: i64) -> i64 {
        (if self.on_x { x } else { y }) - self.origin
    }

    fn at(&self, x: i64, y: i64) -> i64 {
        self.sample(self.first, self.second, self.offset_at(x, y))
    }

    fn sample(&self, first: i64, last: i64, offset: i64) -> i64 {
        let weighted = first * (self.count - offset) + last * offset;
        weighted * self.reciprocal / LINE_FIXED_UNIT
    }

    fn perspective_color(&self, first: Color, last: Color, offset: i64) -> Color {
        if offset == 0 {
            return first;
        }
        // Constant channels retain their value. A negative extrapolated
        // channel borrows across bytes when the signed components are packed.
        (0..4)
            .map(|index| {
                let shift = index * 8;
                let a = i64::from((first >> shift) & 255);
                let b = i64::from((last >> shift) & 255);
                let channel = if a == b {
                    a
                } else {
                    self.sample(a, b, offset).min(255)
                };
                channel << shift
            })
            .sum::<i64>() as Color
    }
}

fn draw_line(
    grob: &mut Grob,
    points: [Point; 2],
    positions: [(f64, f64); 2],
    alpha: Option<i64>,
    depth: &mut Option<Depth>,
    phase_depths: Option<[i64; 2]>,
    interrupt: Option<&std::sync::atomic::AtomicBool>,
) -> RtResult<()> {
    // Depth advances on the entire projected segment. Clip the writes only.
    let phase = phase_depths.and_then(|depths| LineDepth::new(depths, positions));
    let [mut a, mut b] = points;
    let [(mut x1, mut y1), (mut x2, mut y2)] = positions;
    // Firmware walks along the increasing major axis in either endpoint order.
    if if (x2 - x1).abs() >= (y2 - y1).abs() {
        x2 < x1
    } else {
        y2 < y1
    } {
        std::mem::swap(&mut a, &mut b);
        std::mem::swap(&mut x1, &mut x2);
        std::mem::swap(&mut y1, &mut y2);
    }
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
    let original_span = (x2 - x1).abs().max((y2 - y1).abs()).ceil();
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
        let position = low + (high - low) * t;
        let color_position = if original_span == 0. {
            0.5
        } else {
            position * original_span / (original_span + 1.)
        };
        if depth.as_mut().is_none_or(|d| {
            let index = y as usize * grob.width() + x as usize;
            if let Some(phase) = &phase {
                d.accepts_line(index, phase.at(x, y))
            } else {
                d.accepts(index, a.depth + (b.depth - a.depth) * position)
            }
        }) {
            let color = if original_span == 0. {
                (0..4).fold(0, |color, index| {
                    let shift = index * 8;
                    color | ((((a.color >> shift) & 255) + ((b.color >> shift) & 255)) / 2) << shift
                })
            } else {
                line_color(a.color, b.color, color_position)
            };
            grob.set(x, y, opacity(color, alpha));
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
    if matches!(args.first(), Some(Value::Str(_) | Value::RawString(_))) {
        return (args.len() == 3 && args[2].is_string()).then_some(2);
    }
    if triangles && args.len() == 4 && args.iter().take(3).all(is_vertex) {
        return matches!(args.last(), Some(Value::Str(_) | Value::RawString(_)))
            .then_some(args.len() - 1);
    }
    if matches!(args.first(), Some(Value::List(_) | Value::Matrix(_))) {
        return (args.len() == 4 && args[3].is_string()).then_some(3);
    }
    if triangles
        && args.len() >= 4
        && matches!(
            args.get(args.len() - 4),
            Some(Value::Str(_) | Value::RawString(_))
        )
    {
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
            Some(Value::Str(_) | Value::RawString(_) | Value::List(_) | Value::Matrix(_))
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
            if args.len() < 2 || args.len() > 3 || (args.len() == 3 && !args[2].is_string()) {
                return Err(invalid(
                    "Cached graphics expect points, primitives and optional z-string",
                ));
            }
            uncache(source)?
        } else {
            if !(3..=4).contains(&args.len()) || (args.len() == 4 && !args[3].is_string()) {
                return Err(invalid(
                    "3D graphics expect points, primitives, transform and optional z-string",
                ));
            }
            let options = transformation(args.get(2))?;
            // Triangle point-vector rows require a transform with a z row.
            // Actual complex matrices can use either supported row count.
            let packed_vectors = !triangles
                || options
                    .matrix
                    .as_ref()
                    .is_some_and(|matrix| matrix.len() == 3);
            let mut points = load_points(arg(args, 0, "3D graphics")?, packed_vectors)?;
            let flat_pixel_line = !triangles && !cart && options.eye.is_none();
            transform(&mut points, &options, flat_pixel_line)?;
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
    let line_normalization = if !triangles && !cart && depth.is_some() && options.normalize {
        LineNormalization::new(&points, &options)
    } else {
        None
    };
    let required = if triangles { 3 } else { 2 };
    let clipped_pixel_line = !triangles && !cart && depth.is_some() && options.clip.is_some();
    let clipped_pixel_depth = clipped_pixel_line && options.eye.is_none();
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
        let primitive_color = primitive
            .get(required)
            .filter(|v| !omitted(v))
            .map(color_of)
            .transpose()?;
        let color = primitive_color.or(default_color);
        let primitive_alpha = primitive
            .get(required + 1)
            .map(|v| v.expect_i64("alpha"))
            .transpose()?;
        let requested_alpha = primitive_alpha.or(default_alpha);
        if requested_alpha.is_some_and(|a| !(0..=255).contains(&a)) {
            return Err(invalid("Alpha must be 0 through 255"));
        }
        let alpha = if clipped_pixel_depth {
            // Clipped buffered LINE_P uses a nonzero alpha on the row's
            // explicit color. A header alpha does not set row opacity.
            if (primitive_alpha.is_some() && primitive_color.is_none())
                || (default_alpha.is_some() && color.is_none())
            {
                return Err(invalid("LINE_P alpha requires a color"));
            }
            primitive_alpha.filter(|value| *value != 0)
        } else {
            requested_alpha
        };
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
            let phase_depths = if cart || depth.is_none() {
                None
            } else if options.normalize {
                line_normalization.as_ref().and_then(|normalization| {
                    Some([
                        normalization.depth(a, selected[0].original[2])?,
                        normalization.depth(b, selected[1].original[2])?,
                    ])
                })
            } else {
                Some([a.depth as i64, b.depth as i64])
            };
            let integral = positions.map(|(x, y)| (x.trunc(), y.trunc()));
            // In the clipped pixel overload, perspective changes the
            // endpoints but uses the same integer walk and depth phase.
            let integer_raster = !cart
                && options
                    .eye
                    .map_or(integral != positions, |_| options.clip.is_some());
            let phase = if integer_raster {
                let depths = if depth.is_some() {
                    phase_depths
                } else {
                    Some([0, 0])
                };
                depths.and_then(|depths| LineDepth::new(depths, integral))
            } else {
                None
            };
            let gradient = selected[0].color != selected[1].color;
            let perspective_transparency = if clipped_pixel_line && options.eye.is_some() {
                // A row alpha with an explicit RGB color is transparency.
                // A header alpha keeps the line opaque.
                match (primitive_alpha, primitive_color, default_alpha) {
                    (Some(a), Some(color), _) if color >> 24 == 0 => Some(a as u32),
                    (None, _, Some(_)) if color.is_some_and(|color| color >> 24 == 0) => Some(0),
                    _ => None,
                }
            } else {
                None
            };
            let buffered_alpha =
                (clipped_pixel_depth && alpha.is_some()) || perspective_transparency.is_some();
            let [(x1, y1), (x2, y2)] = integral;
            let width = (x2 - x1).abs();
            let height = (y2 - y1).abs();
            let y_major_eye_gradient = gradient
                && height > 0.0
                && height >= width
                && clipped_pixel_line
                && options.eye.is_some()
                && options.normalize
                && alpha.is_none()
                && (selected[0].color | selected[1].color) >> 24 == 0;
            if let Some(phase) = phase
                && (!gradient || phase.on_x || y_major_eye_gradient)
                && (buffered_alpha
                    || (opacity(a.color, alpha) | opacity(b.color, alpha)) >> 24 == 0)
            {
                if perspective_transparency == Some(255) {
                    continue;
                }
                // Fractional projected endpoints are truncated before the
                // integer walk. Depth and color advance on the same span.
                let size = (target.width(), target.height());
                let [a, b] = integral.map(|(x, y)| (x as i64, y as i64));
                let [first, last] = if gradient {
                    clipped_line_colors([selected[0], selected[1]], options.clip, a.0 > b.0)
                } else {
                    [selected[0].color, selected[0].color]
                };
                Grob::try_calculator_line_pixels(size, a, b, |x, y| -> RtResult<()> {
                    check_interrupt(interrupt.as_deref())?;
                    let index = y as usize * size.0 + x as usize;
                    if depth
                        .as_mut()
                        .is_none_or(|z| z.accepts_line(index, phase.at(x, y)))
                    {
                        let color = if gradient {
                            let offset = phase.offset_at(x, y);
                            if options.eye.is_some() {
                                phase.perspective_color(first, last, offset)
                            } else {
                                fixed_line_color(first, last, offset * phase.reciprocal)
                            }
                        } else {
                            first
                        };
                        if y_major_eye_gradient {
                            // The high byte of an opaque gradient sample is
                            // observable through GETPIX_P even when it is 255.
                            target.pixels_mut()[index] = color;
                        } else if let Some(transparency) = perspective_transparency {
                            target.set_calculator_line_pixel(x, y, color | (transparency << 24));
                        } else {
                            target.set(x, y, opacity(color, alpha));
                        }
                    }
                    Ok(())
                })?;
            } else {
                draw_line(
                    &mut target,
                    [a, b],
                    positions,
                    alpha,
                    &mut depth,
                    phase_depths,
                    interrupt.as_deref(),
                )?;
            }
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
    if let Some(value) = &updated {
        machine.check_len(value.expect_string_units("z-string")?.len())?;
    }
    check_interrupt(interrupt.as_deref())?;
    machine.grobs[g] = Some(target);
    if g == 0 {
        machine.screen_dirty = true;
    }
    points.clear();
    Ok((result, updated))
}
