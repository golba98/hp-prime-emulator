//! Prime command rasterization, independently controlled against guest pixels.
use crate::{Color, Grob};

pub(crate) fn line(grob: &mut Grob, a: (i64, i64), b: (i64, i64), color: Color) {
    let (mut x1, mut y1) = (i128::from(a.0), i128::from(a.1));
    let (mut x2, mut y2) = (i128::from(b.0), i128::from(b.1));
    if x1 > x2 {
        std::mem::swap(&mut x1, &mut x2);
        std::mem::swap(&mut y1, &mut y2);
    }
    let dx = (x2 - x1) as u128;
    let dy = y2.abs_diff(y1);
    let sign = if y2 >= y1 { 1 } else { -1 };
    if dx == 0 && dy == 0 {
        line_point(grob, x1, y1, color);
        return;
    }
    if dy == 0 {
        span(grob, x1, x2, y1, color);
        return;
    }
    if dx == 0 {
        for y in y1.min(y2).max(0)..=y1.max(y2).min(grob.height() as i128 - 1) {
            line_point(grob, x1, y, color);
        }
        return;
    }
    if dx == 1 && dy == 1 {
        line_point(grob, x1, y1, color);
        line_point(grob, x2, y2, color);
        return;
    }
    // The guest orders endpoints by x, including steep lines, and clips x
    // before y. Reconstruct its accumulator at each boundary; rounding a
    // continuous line or skipping ordinary Bresenham pixels loses that phase.
    let (dx, dy) = (dx as i128, dy as i128);
    let (right, bottom) = (grob.width() as i128 - 1, grob.height() as i128 - 1);
    if x2 < 0 || x1 > right || y1.max(y2) < 0 || y1.min(y2) > bottom {
        return;
    }
    if dx > dy {
        let mut error = -(dx / 2);
        if x1 < 0 {
            let (q, r) = mul_div(-x1, dy, error, dx);
            // This branch changes y only when the clipped accumulator is
            // strictly positive. A zero accumulator advances after painting.
            if q > 0 || r > 0 {
                y1 += sign * (q + i128::from(r != 0));
                error = if r == 0 { 0 } else { r - dx };
            } else {
                error = r;
            }
            x1 = 0;
        }
        if (sign > 0 && y1 > bottom) || (sign < 0 && y1 < 0) {
            return;
        }
        let distance = if sign > 0 { y1 } else { bottom - y1 };
        if distance < 0 {
            let (q, mut r) = mul_div(distance, dx, error, dy);
            y1 = if sign > 0 { 0 } else { bottom };
            x1 -= q;
            // Collapse the original correction loop analytically so even a
            // line spanning the entire i64 range needs only visible work.
            let corrections = (r + dx - 1).max(0) / dy;
            r -= corrections * dy;
            x1 -= corrections;
            error = r;
        }
        for x in x1.max(0)..=x2.min(right) {
            if y1 < 0 || y1 > bottom {
                break;
            }
            line_point(grob, x, y1, color);
            error += dy;
            if error >= 0 {
                y1 += sign;
                error -= dx;
            }
        }
    } else {
        let mut error = -(dy / 2);
        if x1 < 0 {
            let (q, mut r) = mul_div(x1, dy, error, dx);
            y1 -= sign * q;
            let corrections = (r + dy - 1).max(0) / dx;
            r -= corrections * dx;
            y1 -= sign * corrections;
            error = r;
            x1 = 0;
        }
        let distance = if sign > 0 { -y1 } else { y1 - bottom };
        if distance > 0 {
            let (q, r) = mul_div(distance, dx, error, dy);
            y1 = if sign > 0 { 0 } else { bottom };
            x1 += q + i128::from(r != 0);
            error = if r == 0 { 0 } else { r - dy };
        }
        let last = if sign > 0 { y2.min(bottom) } else { y2.max(0) };
        let count = (last - y1) * sign;
        for offset in 0..=count.min(grob.height() as i128) {
            if x1 < 0 || x1 > x2.min(right) {
                break;
            }
            line_point(grob, x1, y1 + sign * offset, color);
            error += dx;
            if error >= 0 {
                x1 += 1;
                error -= dy;
            }
        }
    }
}

// Signed quotient/remainder for offset*span+phase without overflowing i128
// at extreme i64 coordinates. The accumulator supplied here is nonpositive.
fn mul_div(offset: i128, span: i128, phase: i128, divisor: i128) -> (i128, i128) {
    debug_assert!(phase <= 0 && span > 0 && divisor > 0);
    let product = offset.unsigned_abs() * span as u128;
    let bias = phase.unsigned_abs();
    let (negative, magnitude) = if offset < 0 {
        (true, product + bias)
    } else if product < bias {
        (true, bias - product)
    } else {
        (false, product - bias)
    };
    let sign = if negative { -1 } else { 1 };
    (
        sign * (magnitude / divisor as u128) as i128,
        sign * (magnitude % divisor as u128) as i128,
    )
}

pub(crate) fn point(grob: &mut Grob, x: i128, y: i128, color: Color) {
    blend(grob, x, y, color, 255);
}

fn line_point(grob: &mut Grob, x: i128, y: i128, color: Color) {
    blend(grob, x, y, color, 256);
}

fn blend(grob: &mut Grob, x: i128, y: i128, color: Color, scale: u32) {
    if x >= 0 && x < grob.width() as i128 && y >= 0 && y < grob.height() as i128 {
        let transparency = color >> 24;
        if transparency == 255 {
            return;
        }
        let color = if transparency == 0 {
            color
        } else {
            let background = grob.get(x as i64, y as i64).unwrap();
            let mix = |shift: u32| {
                (((color >> shift) & 255) * (scale - transparency)
                    + ((background >> shift) & 255) * transparency)
                    >> 8
            };
            mix(16) << 16 | mix(8) << 8 | mix(0)
        };
        grob.set(x as i64, y as i64, color);
    }
}

pub(crate) fn span(grob: &mut Grob, x1: i128, x2: i128, y: i128, color: Color) {
    if y < 0 || y >= grob.height() as i128 || x1 > x2 {
        return;
    }
    for x in x1.max(0)..=x2.min(grob.width() as i128 - 1) {
        line_point(grob, x, y, color);
    }
}

/// Rectangle borders and interior are disjoint: transparency is applied once
/// per pixel, including corners and degenerate rows/columns. Clip the work,
/// retaining original border coordinates so clipping never creates an edge.
pub(crate) fn rectangle(grob: &mut Grob, a: (i64, i64), b: (i64, i64), edge: Color, fill: Color) {
    let (left, right) = (i128::from(a.0.min(b.0)), i128::from(a.0.max(b.0)));
    let (top, bottom) = (i128::from(a.1.min(b.1)), i128::from(a.1.max(b.1)));
    for y in top.max(0)..=bottom.min(grob.height() as i128 - 1) {
        if y == top || y == bottom {
            span(grob, left, right, y, edge);
        } else {
            line_point(grob, left, y, edge);
            if left != right {
                span(grob, left + 1, right - 1, y, fill);
                line_point(grob, right, y, edge);
            }
        }
    }
}
