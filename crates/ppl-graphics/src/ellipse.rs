//! Integer ellipse spans and quantized sector clipping for ARC's radius-pair
//! and edge/fill forms. Scalar unfilled circles have a separate raster path.
use crate::{Color, Grob, arc, raster};

const TRANSPARENT: Color = 0xff00_0000;
type Interval = (i64, i64);

struct Paint<'a> {
    grob: &'a mut Grob,
    center: (i64, i64),
    edge: Color,
    fill: Color,
}

impl Paint<'_> {
    fn span(&mut self, y: i64, interval: Interval, color: Color) {
        raster::span(
            self.grob,
            i128::from(self.center.0) + i128::from(interval.0),
            i128::from(self.center.0) + i128::from(interval.1),
            i128::from(self.center.1) + i128::from(y),
            color,
        );
    }

    fn point(&mut self, x: i64, y: i64, color: Color) {
        raster::point(
            self.grob,
            i128::from(self.center.0) + i128::from(x),
            i128::from(self.center.1) + i128::from(y),
            color,
        );
    }

    fn clipped(&mut self, y: i64, width: i64, previous: i64, clip: Interval) {
        let (left, right) = (clip.0.max(-width), clip.1.min(width));
        if self.edge == TRANSPARENT {
            self.span(y, (left, right), self.fill);
            return;
        }
        self.span(y, (left, right.min(-previous)), self.edge);
        self.span(y, (left.max(previous), right), self.edge);
        let inner = (left.max(1 - previous), right.min(previous - 1));
        // The original span path requires two interior pixels. Retaining this
        // boundary matters for narrow sectors and cannot be replaced by a
        // continuous angular mask.
        if inner.0 < inner.1 {
            self.span(y, inner, self.fill);
        }
    }

    fn radial(&mut self, endpoint: (i64, i64)) {
        raster::line(
            self.grob,
            self.center,
            (self.center.0 + endpoint.0, self.center.1 + endpoint.1),
            self.edge,
        );
    }
}

fn widths(rx: i64, ry: i64) -> Vec<i64> {
    let (a, b) = (2 * rx, 2 * ry);
    // The initial derivative is a signed 32-bit product before the wide
    // accumulator takes over. All subsequent arithmetic is exact and bounded.
    let initial = (4i32)
        .wrapping_mul(1 - a as i32)
        .wrapping_mul(b as i32)
        .wrapping_mul(b as i32);
    let mut dx = i64::from(initial);
    let mut dy = 4 * a * a;
    let (step_y, step_x) = (8 * a * a, 8 * b * b);
    let mut error = dx + dy;
    let (mut x, mut y) = (rx, 0usize);
    // Some original thin-ellipse paths leave end-cap slots uninitialized.
    // These slots are initialized here; heap-dependent stray pixels are not
    // imported into a deterministic Native framebuffer.
    let mut rows = vec![0; ry as usize + 4];
    rows[0] = rx;
    rows[1] = rx;
    while x > 0 {
        let twice = 2 * error;
        if twice <= dy {
            dy += step_y;
            error += dy;
            y += 1;
        }
        if twice >= dx || 2 * error > dy {
            dx += step_x;
            error += dx;
            x -= 1;
        }
        rows[y] = x;
    }
    rows
}

fn endpoint(radii: (i64, i64), angle: i64) -> (i64, i64) {
    (
        radii.0 * arc::sine(angle + 1024) / 65536,
        radii.1 * arc::sine(angle) / 65536,
    )
}

fn intersection(point: (i64, i64), row: i64, radius: i64) -> i64 {
    ((point.0 * row + point.0 / 2) / point.1).clamp(-radius - 1, radius + 1)
}

fn center_intervals(
    width: i64,
    angles: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
) -> (Vec<Interval>, Vec<i64>) {
    let (e, s) = (end.1.signum(), start.1.signum());
    let full = || (vec![(-width, width)], vec![-width, width]);
    let origin = || (vec![(0, 0)], vec![]);
    let left = || (vec![(-width, 0)], vec![-width]);
    let right = || (vec![(0, width)], vec![width]);
    match (e, s) {
        (0, 0) => (vec![(-width, width)], vec![]),
        (-1, -1) => {
            if end.0 < start.0 {
                full()
            } else {
                origin()
            }
        }
        (1, 1) => {
            if start.0 <= end.0 {
                full()
            } else {
                origin()
            }
        }
        (-1, 1) => left(),
        (1, -1) => right(),
        (0, _) => {
            if angles.1.rem_euclid(4096) == 0 {
                if s > 0 {
                    (vec![(-width, width)], vec![-width])
                } else {
                    (vec![(0, width)], vec![])
                }
            } else if s > 0 {
                (vec![(-width, 0)], vec![])
            } else {
                (vec![(-width, width)], vec![width])
            }
        }
        (_, 0) => {
            if angles.0.rem_euclid(4096) == 0 {
                if e < 0 {
                    (vec![(-width, width)], vec![-width])
                } else {
                    (vec![(0, width)], vec![])
                }
            } else if e < 0 {
                (vec![(-width, 0)], vec![])
            } else {
                (vec![(-width, width)], vec![width])
            }
        }
        _ => unreachable!("signum is -1,0,1"),
    }
}

fn sector_rows(
    width: i64,
    angles: (i64, i64),
    start: (i64, i64),
    end: (i64, i64),
    row: i64,
    radius: i64,
) -> (Vec<Interval>, Vec<Interval>) {
    let (ef, sf) = (end.1.signum(), start.1.signum());
    let e = if ef != 0 {
        intersection(end, row * ef, radius)
    } else {
        0
    };
    let s = if sf != 0 {
        intersection(start, row * sf, radius)
    } else {
        0
    };
    let whole = vec![(-width, width)];
    let wide = (angles.1 - angles.0).rem_euclid(4096) >= 2048;
    match (ef, sf) {
        (1, 1) if wide => (whole, vec![(-width, s), (e, width)]),
        (1, 1) => (vec![], vec![(e, s)]),
        (-1, -1) if wide => (vec![(-width, e), (s, width)], whole),
        (-1, -1) => (vec![(s, e)], vec![]),
        (1, -1) => (vec![(s, width)], vec![(e, width)]),
        (-1, 1) => (vec![(-width, e)], vec![(-width, s)]),
        (1, 0) => (
            if angles.0.rem_euclid(4096) == 2048 {
                whole
            } else {
                vec![]
            },
            vec![(e, width)],
        ),
        (-1, 0) => (
            vec![(-width, e)],
            if angles.0.rem_euclid(4096) == 0 {
                whole
            } else {
                vec![]
            },
        ),
        (0, 1) => (
            if angles.1.rem_euclid(4096) == 0 {
                whole
            } else {
                vec![]
            },
            vec![(-width, s)],
        ),
        (0, -1) => (
            vec![(s, width)],
            if angles.1.rem_euclid(4096) == 2048 {
                whole
            } else {
                vec![]
            },
        ),
        (0, 0) if angles.1.rem_euclid(4096) == 0 => (whole, vec![]),
        (0, 0) => (vec![], whole),
        _ => unreachable!("signum is -1,0,1"),
    }
}

pub(crate) fn draw(
    grob: &mut Grob,
    center: (i64, i64),
    radii: (i64, i64),
    angles: (f64, f64),
    edge: Color,
    fill: Option<Color>,
) -> Result<(), &'static str> {
    let (rx, ry) = radii;
    if rx <= 0 || ry <= 0 || rx > 0x2000_0000 || ry > 0x2000_0000 {
        return Ok(());
    }
    let (cx, cy) = (i128::from(center.0), i128::from(center.1));
    if cx - i128::from(rx) >= grob.width() as i128
        || cx + i128::from(rx) < 0
        || cy - i128::from(ry) >= grob.height() as i128
        || cy + i128::from(ry) < 0
    {
        return Ok(());
    }
    let angles = (arc::units(angles.0), arc::units(angles.1));
    if radii == (1, 1) {
        raster::point(grob, cx, cy, fill.unwrap_or(TRANSPARENT));
        return Ok(());
    }
    if rx >= 500 || ry >= 500 {
        return polygon(grob, center, radii, angles, edge, fill);
    }
    let mut paint = Paint {
        grob,
        center,
        edge,
        fill: fill.unwrap_or(TRANSPARENT),
    };
    let (start, end) = (endpoint(radii, angles.0), endpoint(radii, angles.1));
    if angles.0 == angles.1 {
        paint.radial(start);
        return Ok(());
    }
    let rows = widths(rx, ry);
    if (angles.1 - angles.0).abs() >= 4095 {
        for y in -ry..=ry {
            let row = y.unsigned_abs() as usize;
            let (c, previous) = (rows[row], rows[row + 1]);
            if edge == TRANSPARENT {
                paint.span(y, (-c, c), paint.fill);
                continue;
            }
            let inner = c.min(previous);
            paint.span(y, (1 - inner, inner - 1), paint.fill);
            // Symmetric edge calls coincide at the center row; the fill path
            // explicitly avoids its second call there. Transparency exposes
            // this difference even when the opaque masks are identical.
            for _ in 0..if y == 0 { 2 } else { 1 } {
                if c == previous {
                    paint.point(c, y, edge);
                    paint.point(-c, y, edge);
                } else {
                    paint.span(y, (previous, c - 1), edge);
                    paint.span(y, (1 - c, -previous), edge);
                }
            }
        }
    } else {
        for row in (1..=ry as usize).rev() {
            let (c, previous) = (rows[row], rows[row + 1]);
            let (top, bottom) = sector_rows(c, angles, start, end, row as i64, rx);
            for clip in top {
                paint.clipped(-(row as i64), c, previous, clip);
            }
            for clip in bottom {
                paint.clipped(row as i64, c, previous, clip);
            }
        }
        let (filled, caps) = center_intervals(rows[0], angles, start, end);
        for span in filled {
            paint.span(0, span, paint.fill);
        }
        for x in caps {
            paint.point(x, 0, edge);
        }
        paint.radial(start);
        paint.radial(end);
    }
    Ok(())
}

fn polygon(
    grob: &mut Grob,
    center: (i64, i64),
    radii: (i64, i64),
    angles: (i64, i64),
    edge: Color,
    fill: Option<Color>,
) -> Result<(), &'static str> {
    let steps = (2 * radii.0.max(radii.1)).min(256);
    let step = 4096 / steps;
    let count = (angles.1 - angles.0).max(0) / step;
    if count > 2048 {
        return Err("ARC polygon exceeds the bounded point budget");
    }
    let partial = (angles.1 - angles.0).abs() % 4096 != 0;
    let mut points = Vec::with_capacity(count as usize + 3);
    if partial {
        points.push(center);
    }
    // These drawing products are signed fixed-point32 values. Preserve their
    // wrap before conversion instead of widening them into a different path.
    let vertex = |angle| {
        let product = |r: i64, q| (r as i32).wrapping_mul(arc::sine(q) as i32) / 65536;
        (
            center.0 + i64::from(product(radii.0, angle + 1024)),
            center.1 + i64::from(product(radii.1, angle)),
        )
    };
    let mut angle = angles.0;
    while angle < angles.1 {
        points.push(vertex(angle));
        angle += step;
    }
    points.push(vertex(angles.1));
    if let Some(fill) = fill.filter(|c| c >> 24 != 255) {
        let vertices = points
            .iter()
            .map(|&(x, y)| (x as f64, y as f64))
            .collect::<Vec<_>>();
        grob.fill_polygon(&vertices, fill);
    }
    for pair in points.windows(2) {
        raster::line(grob, pair[0], pair[1], edge);
    }
    if partial && points.len() > 1 {
        raster::line(grob, *points.last().unwrap(), points[0], edge);
    }
    Ok(())
}
