//! Geometry curves expressed by a complex function of a real parameter.
use super::{Extent, Point2, Shape, canonical, number, point, primitives as g};
use crate::ide::native_apps::{Evaluator, numeric};
use ppl_vm::{SessionSnapshot, Value};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parametric {
    pub(super) expression: String,
    pub(super) variable: String,
    bounds: [f64; 2],
    step: Option<f64>,
    pub(crate) paths: Vec<Vec<Point2>>,
    parameters: Vec<Vec<f64>>,
}
impl Parametric {
    pub(super) fn new(
        expression: String,
        variable: String,
        bounds: [f64; 2],
        step: Option<f64>,
    ) -> Result<Self, String> {
        let [lo, hi] = bounds;
        if !lo.is_finite() || !hi.is_finite() || hi <= lo {
            return Err("The curve needs an increasing, finite parameter interval".into());
        }
        if step.is_some_and(|s| !s.is_finite() || s <= 0. || (hi - lo) / s > 8192.) {
            return Err("Invalid parameter step or too many plot samples".into());
        }
        Ok(Self {
            expression,
            variable,
            bounds,
            step,
            paths: Vec::new(),
            parameters: Vec::new(),
        })
    }
    fn evaluator(
        &self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Evaluator, String> {
        Evaluator::geometry_curve(&self.expression, &self.variable, snapshot, cancel)
    }
    fn sample(e: &mut Evaluator, t: f64) -> Result<Option<Point2>, String> {
        let value = e.at_xy_value(t, 0.);
        let p = match value {
            Ok(Value::Complex(z)) => Point2 {
                x: z.re.to_f64(),
                y: z.im.to_f64(),
            },
            other => match numeric(other)? {
                Some(x) => Point2 { x, y: 0. },
                None => return Ok(None),
            },
        };
        Ok(Some(point(p)?))
    }
    pub(super) fn trace(
        &mut self,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let mut e = self.evaluator(snapshot, cancel)?;
        let [lo, hi] = self.bounds;
        let step = self.step.unwrap_or((hi - lo) / 256.);
        if step == 0. {
            return Err("Parameter interval is too small to sample".into());
        }
        let n = ((hi - lo) / step).ceil() as usize;
        let mut path = Vec::new();
        let mut ts = Vec::new();
        for i in 0..=n {
            let t = if i == n { hi } else { lo + step * i as f64 };
            if let Some(p) = Self::sample(&mut e, t)? {
                path.push(p);
                ts.push(t);
            } else if !path.is_empty() {
                self.paths.push(std::mem::take(&mut path));
                self.parameters.push(std::mem::take(&mut ts));
            }
        }
        if !path.is_empty() {
            self.paths.push(path);
            self.parameters.push(ts);
        }
        Ok(())
    }
    pub(super) fn source(&self) -> String {
        let [lo, hi] = self.bounds.map(canonical);
        format!("plotparam({},{},{lo},{hi})", self.expression, self.variable)
    }
    pub(super) fn bounds(&self) -> [f64; 2] {
        self.bounds
    }
    pub(super) fn at(
        &self,
        t: f64,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Point2, String> {
        Self::sample(&mut self.evaluator(snapshot, cancel)?, t)?
            .ok_or_else(|| "Point undefined at this parameter".into())
    }
    pub(super) fn nearest(&self, cursor: Point2) -> Result<(f64, Point2), String> {
        self.paths
            .iter()
            .zip(&self.parameters)
            .flat_map(|(path, ts)| path.windows(2).zip(ts.windows(2)))
            .filter_map(|(points, t)| {
                let p = g::closest(cursor, points[0], points[1], Extent::Segment).ok()?;
                let length = points[0].distance(points[1]).ok()?;
                let ratio = if length == 0. {
                    0.
                } else {
                    points[0].distance(p).ok()? / length
                };
                Some((cursor.distance(p).ok()?, t[0] + ratio * (t[1] - t[0]), p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t, p)| (t, p))
            .ok_or_else(|| "No defined curve segment".into())
    }
    pub(super) fn tangent(
        &self,
        t: f64,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Shape, String> {
        let mut e = self.evaluator(snapshot, cancel)?;
        let mut at =
            |t| Self::sample(&mut e, t)?.ok_or_else(|| String::from("Undefined tangent point"));
        let h = (t.abs().max(1.)) * 1e-5;
        let p = at(t)?;
        let d = g::delta(at(t - h)?, at(t + h)?);
        g::line(
            p,
            point(Point2 {
                x: p.x + d.x / (2. * h),
                y: p.y + d.y / (2. * h),
            })?,
            Extent::Line,
        )
    }
    pub(super) fn integral(
        &self,
        area: bool,
        snapshot: &SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<f64, String> {
        let mut e = self.evaluator(snapshot, cancel)?;
        let [lo, hi] = self.bounds;
        let mut previous = None;
        let mut extrapolated = None;
        for power in 4..=11 {
            let n = 1 << power;
            let mut last = None;
            let mut integral = 0.;
            for i in 0..=n {
                let t = lo + (hi - lo) * f64::from(i) / f64::from(n);
                let q = Self::sample(&mut e, t)?.ok_or("The curve has an undefined measurement")?;
                if let Some(p) = last {
                    let p: Point2 = p;
                    integral += if area {
                        -(q.x - p.x) * (q.y + p.y) * 0.5
                    } else {
                        p.distance(q)?
                    };
                }
                last = Some(q);
            }
            if let Some(coarse) = previous {
                let refined: f64 = integral + (integral - coarse) / 3.;
                if extrapolated
                    .is_some_and(|v: f64| (refined - v).abs() <= 1e-10 * (1. + refined.abs()))
                {
                    return number(refined);
                }
                extrapolated = Some(refined);
            }
            previous = Some(integral);
        }
        number(extrapolated.ok_or("Curve integral did not converge")?)
    }
}

/// Geometry's interval notation is local to plot commands. Keep byte spans
/// intact so edits, stored instructions and substitutions refer to the user's text.
pub(crate) fn plot_syntax(source: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if !source.contains('=') {
        return Cow::Borrowed(source);
    }
    let mut output = source.as_bytes().to_vec();
    let mut stack = Vec::new();
    let mut quoted = false;
    for (i, ch) in source.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            _ if quoted => {}
            '(' | '[' | '{' => {
                let name = source[..i]
                    .trim_end()
                    .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    .unwrap_or("");
                stack.push((
                    ch == '('
                        && ["plotparam", "plotpolar", "plotseq"]
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(name)),
                    0usize,
                    i + 1,
                ));
            }
            ',' | ')' | ']' | '}' => {
                if let Some((active, n, start)) = stack.last_mut() {
                    if *active && let Some((key, value)) = source[*start..i].split_once('=') {
                        let name = key.trim();
                        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                        {
                            let eq = *start + key.len();
                            if *n == 1 {
                                if let Some(pos) = value.find("..") {
                                    output[eq] = b',';
                                    output[eq + 1 + pos] = b',';
                                    output[eq + 2 + pos] = b' ';
                                } else if value.trim_start().starts_with('{') {
                                    output[eq] = b',';
                                }
                            } else if *n >= 2 && name.eq_ignore_ascii_case("tstep") {
                                output[*start..=eq].fill(b' ');
                            }
                        }
                    }
                    *n += 1;
                    *start = i + 1;
                }
                if ch != ',' {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    Cow::Owned(String::from_utf8(output).expect("Only ASCII delimiters were replaced"))
}
