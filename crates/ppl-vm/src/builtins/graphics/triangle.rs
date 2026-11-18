//! Scan-convert two-dimensional TRIANGLE/TRIANGLE_P commands.
//! Meshes and triangles with depth buffers use the mesh renderer.
use std::sync::atomic::{AtomicBool, Ordering};

use ppl_graphics::{Color, Grob};

use crate::error::{ErrorKind, RtResult, RuntimeError};

#[derive(Clone, Copy)]
struct Vertex {
    x: i64,
    y: i64,
    color: Color,
}

const UNIT: i128 = 1 << 16;

/// The raster uses a truncated reciprocal for representable spans. Its
/// crossings can fall short even at exact integer coordinates.
fn interpolate(
    from: i64,
    to: i64,
    offset: i128,
    span: i128,
    reciprocal: Option<i128>,
    wrap_fixed_point: bool,
) -> i64 {
    if offset == span {
        return to;
    }
    if offset == 0 || from == to {
        return from;
    }
    let weighted = i128::from(from) * (span - offset) + i128::from(to) * offset;
    match reciprocal {
        // Coordinate crossings on the calculator use a signed 32-bit accumulator.
        Some(inverse) if wrap_fixed_point => i64::from((weighted * inverse) as i32) / UNIT as i64,
        // This product is bounded by the maximum endpoint magnitude times UNIT.
        Some(inverse) => (weighted * inverse / UNIT) as i64,
        None => (weighted / span) as i64,
    }
}

fn interpolate_color(
    from: Color,
    to: Color,
    offset: i128,
    span: i128,
    reciprocal: Option<i128>,
    wrap_fixed_point: bool,
) -> Color {
    if from == to || offset == span {
        return to;
    }
    let component = |shift: u32| {
        let a = i64::from((from >> shift) & 255);
        let b = i64::from((to >> shift) & 255);
        interpolate(a, b, offset, span, reciprocal, wrap_fixed_point).min(255) << shift
    };
    // Signed channels on a wrapped row can borrow across bytes when packed.
    (component(16) + component(8) + component(0)) as Color
}

fn crossing(from: Vertex, to: Vertex, y: i64, calculator_coordinates: bool) -> (i64, Color) {
    if y == from.y {
        return (from.x, from.color);
    }
    if y == to.y {
        return (to.x, to.color);
    }
    let span = i128::from(to.y) - i128::from(from.y);
    let offset = i128::from(y) - i128::from(from.y);
    // A tall calculator edge has a zero reciprocal. Outside the signed
    // coordinate range, retain the bounded exact-division fallback.
    let inverse = (span <= UNIT || calculator_coordinates).then(|| UNIT / span);
    (
        interpolate(from.x, to.x, offset, span, inverse, calculator_coordinates),
        interpolate_color(from.color, to.color, offset, span, inverse, false),
    )
}

fn row(
    grob: &mut Grob,
    y: i64,
    mut left: (i64, Color),
    mut right: (i64, Color),
    triangle_bounds: Option<(i64, i64)>,
    transparency: u8,
    opaque_weight: u32,
) {
    if left.0 > right.0 {
        std::mem::swap(&mut left, &mut right);
    }
    let width = grob.width();
    let (left_bound, right_bound) = triangle_bounds.unwrap_or((0, width as i64 - 1));
    let x_start = left.0.max(left_bound);
    let x_end = right.0.min(right_bound);
    if x_start > x_end {
        return;
    }
    let span = i128::from(right.0) - i128::from(left.0);
    let wrap_colors = triangle_bounds.is_some() && span > i128::from(i32::MAX);
    let span = if wrap_colors {
        i128::from(span as i32)
    } else {
        span
    };
    let inverse = (span != 0).then(|| UNIT / span);
    let opaque_weight = if wrap_colors && left.1 == right.1 {
        256
    } else {
        opaque_weight
    };
    // Clip writes only. The gradient starts at the original crossing.
    let start = y as usize * width + x_start as usize;
    let pixels = &mut grob.pixels_mut()[start..start + (x_end - x_start + 1) as usize];
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let offset = i128::from(x_start) + index as i128 - i128::from(left.0);
        let offset = if wrap_colors {
            i128::from(offset as i32)
        } else {
            offset
        };
        let color = interpolate_color(left.1, right.1, offset, span, inverse, wrap_colors);
        *pixel = if transparency == 0 {
            // GETPIX_P can observe the full packed word on a wrapped row.
            color
        } else {
            // The calculator adds the requested transparency to the packed word.
            let effective = u32::from(transparency.wrapping_add((color >> 24) as u8));
            if effective == 255 {
                continue;
            }
            if effective == 0 {
                color & 0xff_ff_ff
            } else {
                let color_weight = opaque_weight - effective;
                let blend = |shift: u32| {
                    (((color >> shift) & 255) * color_weight
                        + ((*pixel >> shift) & 255) * effective)
                        >> 8
                };
                blend(16) << 16 | blend(8) << 8 | blend(0)
            }
        };
    }
}

pub(super) fn draw(
    grob: &mut Grob,
    points: [(i64, i64); 3],
    colors: [Color; 3],
    transparency: u8,
    interrupt: Option<&AtomicBool>,
) -> RtResult<()> {
    if transparency == 255 || grob.width() == 0 || grob.height() == 0 {
        return Ok(());
    }
    let mut vertices = std::array::from_fn(|i| Vertex {
        x: points[i].0,
        y: points[i].1,
        color: colors[i] & 0xff_ff_ff,
    });
    vertices.sort_by_key(|v| (v.y, v.x));
    let [top, middle, bottom] = vertices;
    let height = i128::from(bottom.y) - i128::from(top.y);
    let xmin = top.x.min(middle.x).min(bottom.x);
    let xmax = top.x.max(middle.x).max(bottom.x);
    let calculator_coordinates = i32::try_from(height).is_ok()
        && vertices
            .iter()
            .all(|v| i32::try_from(v.x).is_ok() && i32::try_from(v.y).is_ok());
    // Clip collapsed or wrapped crossings to the triangle's bounds. The
    // gradient still uses the original span endpoints.
    let triangle_bounds =
        calculator_coordinates.then(|| (xmin.max(0), xmax.min(grob.width() as i64 - 1)));
    let delta = |v: Vertex| {
        (
            (i128::from(v.x) - i128::from(top.x)) as f64,
            (i128::from(v.y) - i128::from(top.y)) as f64,
        )
    };
    let (dx, dy) = delta(bottom);
    let (middle_dx, middle_dy) = delta(middle);
    // Resolve which edge is on the left before rounding either crossing.
    let short_on_left = middle_dx * dy < middle_dy * dx;
    let solid = top.color == middle.color && middle.color == bottom.color;
    for y in top.y.max(0)..=bottom.y.min(grob.height() as i64 - 1) {
        if interrupt.is_some_and(|cancel| cancel.load(Ordering::Relaxed)) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        let flat = top.y == bottom.y;
        let long = crossing(top, bottom, y, calculator_coordinates);
        let short = if flat {
            let last = if top.x == bottom.x { middle } else { bottom };
            (last.x, last.color)
        } else if y < middle.y {
            crossing(top, middle, y, calculator_coordinates)
        } else {
            crossing(middle, bottom, y, calculator_coordinates)
        };
        // A span collapsed to one pixel receives the active short-edge color.
        let (left, right) = if short_on_left && long.0 != short.0 {
            (short, long)
        } else {
            (long, short)
        };
        let isolated_vertex =
            (y == top.y && top.y < middle.y) || (y == bottom.y && middle.y < bottom.y);
        let weight = if solid || isolated_vertex { 256 } else { 255 };
        row(grob, y, left, right, triangle_bounds, transparency, weight);
    }
    Ok(())
}
