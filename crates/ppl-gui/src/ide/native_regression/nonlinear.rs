//! Bounded weighted numerical fits. All returned curves minimize observed
//! residuals; singular or nonfinite data produce errors, never canned curves.
type Point = (f64, f64, f64);
pub(super) fn logistic_value(limit: f64, z: f64) -> f64 {
    if z > 0. {
        let v = (-z).exp();
        limit * v / (1. + v)
    } else {
        limit / (1. + z.exp())
    }
}
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for i in 0..n {
        let pivot = (i..n).max_by(|&j, &k| a[j][i].abs().total_cmp(&a[k][i].abs()))?;
        if !a[pivot][i].is_finite() || a[pivot][i].abs() < 1e-18 {
            return None;
        }
        a.swap(i, pivot);
        b.swap(i, pivot);
        for j in i + 1..n {
            let ratio = a[j][i] / a[i][i];
            let (before, after) = a.split_at_mut(j);
            for (target, pivot) in after[0][i..].iter_mut().zip(&before[i][i..]) {
                *target -= ratio * pivot;
            }
            b[j] -= ratio * b[i];
        }
    }
    let mut out = vec![0.; n];
    for i in (0..n).rev() {
        out[i] = (b[i] - (i + 1..n).map(|j| a[i][j] * out[j]).sum::<f64>()) / a[i][i];
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}
fn loss(points: &[Point], p: &[f64], model: &impl Fn(f64, &[f64]) -> f64) -> f64 {
    let total = points.iter().map(|v| v.2).sum::<f64>();
    points
        .iter()
        .map(|&(x, y, w)| w / total * (y - model(x, p)).powi(2))
        .sum()
}
// Finite differences are central and scaled per parameter. Levenberg damping
// and accepted-step descent bound both evaluation count and numerical growth.
fn minimize(
    points: &[Point],
    mut p: Vec<f64>,
    model: &impl Fn(f64, &[f64]) -> f64,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Option<(f64, Vec<f64>)> {
    let n = p.len();
    let mut lambda = 1e-3;
    let mut error = loss(points, &p, model);
    if !error.is_finite() {
        return None;
    }
    let total = points.iter().map(|v| v.2).sum::<f64>();
    for _ in 0..160 {
        super::check_cancel(cancel).ok()?;
        let mut a = vec![vec![0.; n]; n];
        let mut b = vec![0.; n];
        for &(x, y, w) in points {
            let estimate = model(x, &p);
            let mut gradient = vec![0.; n];
            for j in 0..n {
                let h = 1e-5 * (1. + p[j].abs());
                let mut trial = p.clone();
                trial[j] += h;
                let hi = model(x, &trial);
                trial[j] -= 2. * h;
                gradient[j] = (hi - model(x, &trial)) / (2. * h);
            }
            for j in 0..n {
                b[j] += w / total * gradient[j] * (y - estimate);
                for k in 0..n {
                    a[j][k] += w / total * gradient[j] * gradient[k];
                }
            }
        }
        if error < 1e-26
            || b.iter().map(|v| v * v).sum::<f64>().sqrt() <= 1e-12 * (1. + error.sqrt())
        {
            return Some((error, p));
        }
        for (i, row) in a.iter_mut().enumerate() {
            row[i] += lambda * row[i].max(1e-12);
        }
        let delta = solve(a, b)?;
        let trial: Vec<_> = p.iter().zip(&delta).map(|(a, b)| a + b).collect();
        let next = loss(points, &trial, model);
        if next.is_finite() && next < error {
            let improvement = error - next;
            p = trial;
            error = next;
            lambda = (lambda / 3.).max(1e-12);
            if error < 1e-26 || improvement <= 1e-14 * error.max(1e-24) {
                return Some((error, p));
            }
        } else {
            lambda *= 10.;
            if lambda > 1e16 {
                return None;
            }
        }
    }
    None
}
fn normalized(points: &[Point]) -> Result<(Vec<Point>, f64, f64, f64), String> {
    let lo = points.iter().map(|v| v.0).fold(f64::INFINITY, f64::min);
    let hi = points.iter().map(|v| v.0).fold(f64::NEG_INFINITY, f64::max);
    let center = lo / 2. + hi / 2.;
    let scale = hi / 2. - lo / 2.;
    let ys = points.iter().map(|v| v.1.abs()).fold(0., f64::max);
    if !scale.is_finite() || scale <= 0. || ys == 0. {
        return Err("Fit requires distinct x coordinates and nonzero observations".into());
    }
    Ok((
        points
            .iter()
            .map(|&(x, y, w)| ((x - center) / scale, y / ys, w))
            .collect(),
        center,
        scale,
        ys,
    ))
}
pub(super) fn logistic(
    points: &[Point],
    fixed: Option<f64>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<f64>, String> {
    if points.len() < 3 {
        return Err("Logistic fit needs at least three distinct observations".into());
    }
    if fixed.is_some_and(|v| !v.is_finite() || v < 0.) {
        return Err("Logistic saturation L must be zero or positive".into());
    }
    if points.iter().any(|p| p.1 <= 0.) {
        return Err("Logistic fit requires positive y values".into());
    }
    let (p, center, scale, ys) = normalized(points)?;
    let fixed = fixed.filter(|v| *v > 0.).map(|v| v / ys);
    if fixed.is_none() && p.iter().all(|v| (v.1 - p[0].1).abs() < 1e-12) {
        return Err("Constant observations do not determine a logistic saturation".into());
    }
    if fixed.is_some_and(|limit| p.iter().any(|v| v.1 >= limit)) {
        return Err("Observations must lie below the fixed logistic saturation".into());
    }
    let model = |x: f64, a: &[f64]| match fixed {
        Some(limit) => logistic_value(limit, a[0] - a[1] * x),
        None => logistic_value(a[0].exp(), a[1] - a[2] * x),
    };
    let mut best: Option<(f64, Vec<f64>)> = None;
    for limit in [1.001f64, 1.05, 1.25, 2., 4., 10.] {
        super::check_cancel(cancel)?;
        let limit = fixed.unwrap_or(limit);
        let transformed: Vec<_> = p
            .iter()
            .map(|&(x, y, w)| (x, (limit / y - 1.).ln(), w))
            .collect();
        let sw = transformed.iter().map(|p| p.2).sum::<f64>();
        let mx = transformed.iter().map(|p| p.0 * p.2 / sw).sum::<f64>();
        let my = transformed.iter().map(|p| p.1 * p.2 / sw).sum::<f64>();
        let xx = transformed
            .iter()
            .map(|p| (p.0 - mx).powi(2) * p.2 / sw)
            .sum::<f64>();
        if xx == 0. {
            continue;
        }
        let slope = transformed
            .iter()
            .map(|p| (p.0 - mx) * (p.1 - my) * p.2 / sw)
            .sum::<f64>()
            / xx;
        let mut initial = vec![my - slope * mx, -slope];
        if fixed.is_none() {
            initial.insert(0, limit.ln());
        }
        if let Some(candidate) = minimize(&p, initial, &model, cancel) {
            if best.as_ref().is_none_or(|b| candidate.0 < b.0) {
                best = Some(candidate);
            }
        }
        if fixed.is_some() {
            break;
        }
    }
    super::check_cancel(cancel)?;
    let (_, a) = best.ok_or("Logistic fit did not converge")?;
    let (limit, log_a, b) = match fixed {
        Some(l) => (l, a[0], a[1]),
        None => (a[0].exp(), a[1], a[2]),
    };
    let b = b / scale;
    let result = vec![limit * ys, (log_a + b * center).exp(), b];
    if result.iter().all(|v| v.is_finite()) && result[0] > 0. && result[1] > 0. {
        Ok(result)
    } else {
        Err("Logistic coefficients exceed numerical range".into())
    }
}
pub(super) fn trigonometric(
    points: &[Point],
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Vec<f64>, String> {
    if points.len() < 3 {
        return Err("Trigonometric fit needs at least three observations".into());
    }
    let (p, center, scale, ys) = normalized(points)?;
    if p.iter().all(|v| (v.1 - p[0].1).abs() < 1e-12) {
        return Err("Constant observations do not determine a trigonometric frequency".into());
    }
    let mut coordinates: Vec<_> = p.iter().map(|v| v.0).collect();
    coordinates.sort_by(f64::total_cmp);
    coordinates.dedup();
    if coordinates.len() < 3 {
        return Err("Trigonometric fit needs three distinct x coordinates".into());
    }
    let mut steps: Vec<_> = coordinates.windows(2).map(|v| v[1] - v[0]).collect();
    steps.sort_by(f64::total_cmp);
    let max_frequency =
        (std::f64::consts::PI / steps[steps.len() / 2]).min(256. * std::f64::consts::PI);
    let samples = ((max_frequency * 16.).ceil() as usize).clamp(128, 4096);
    let mut candidates = Vec::new();
    for i in 1..=samples {
        super::check_cancel(cancel)?;
        let frequency = max_frequency * i as f64 / samples as f64;
        let mut a = vec![vec![0.; 3]; 3];
        let mut b = vec![0.; 3];
        for &(x, y, w) in &p {
            let row = [(frequency * x).sin(), (frequency * x).cos(), 1.];
            for j in 0..3 {
                b[j] += w * y * row[j];
                for k in 0..3 {
                    a[j][k] += w * row[j] * row[k];
                }
            }
        }
        if let Some(c) = solve(a, b) {
            let initial = vec![c[0].hypot(c[1]), frequency, c[1].atan2(c[0]), c[2]];
            let error = loss(&p, &initial, &|x, c| c[0] * (c[1] * x + c[2]).sin() + c[3]);
            if error.is_finite() {
                candidates.push((error, initial));
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let model = |x: f64, c: &[f64]| c[0] * (c[1] * x + c[2]).sin() + c[3];
    let mut best = None;
    for (_, initial) in candidates.into_iter().take(12) {
        super::check_cancel(cancel)?;
        if let Some(candidate) = minimize(&p, initial, &model, cancel) {
            if best
                .as_ref()
                .is_none_or(|b: &(f64, Vec<f64>)| candidate.0 < b.0)
            {
                best = Some(candidate);
            }
        }
    }
    super::check_cancel(cancel)?;
    let (_, c) = best.ok_or("Trigonometric fit is singular or did not converge")?;
    let b = c[1] / scale;
    let result = vec![
        c[0] * ys,
        b,
        (c[2] - b * center).rem_euclid(std::f64::consts::TAU),
        c[3] * ys,
    ];
    if result.iter().all(|v| v.is_finite()) {
        Ok(result)
    } else {
        Err("Trigonometric coefficients exceed numerical range".into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cancellation_during_real_model_evaluation_retires_the_solver() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let cancel = AtomicBool::new(false);
        let calls = std::cell::Cell::new(0);
        let model = |x: f64, p: &[f64]| {
            calls.set(calls.get() + 1);
            if calls.get() == 5 {
                cancel.store(true, Ordering::Release);
            }
            p[0] * x
        };
        let points = [(1., 2., 1.), (2., 4., 1.), (3., 6., 1.)];
        assert!(super::minimize(&points, vec![1.], &model, Some(&cancel)).is_none());
        assert!(cancel.load(Ordering::Acquire));
    }
}
