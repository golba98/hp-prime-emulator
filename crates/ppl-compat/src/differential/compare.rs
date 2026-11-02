use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mismatch {
    pub backend: BackendId,
    pub reference: Option<BackendId>,
    pub step: String,
    pub path: String,
    pub expected: Option<Value>,
    pub actual: Option<Value>,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub schema_version: u32,
    pub fixture: String,
    pub fixture_sha256: String,
    pub source_sha256: String,
    pub passed: bool,
    pub assertions: usize,
    pub mismatches: Vec<Mismatch>,
    pub runs: Vec<BackendRun>,
}
#[allow(clippy::too_many_arguments)]
fn mismatch(
    out: &mut Vec<Mismatch>,
    backend: BackendId,
    reference: Option<BackendId>,
    step: &str,
    path: &str,
    expected: Option<&Value>,
    actual: Option<&Value>,
    reason: &str,
) {
    out.push(Mismatch {
        backend,
        reference,
        step: step.into(),
        path: path.into(),
        expected: expected.cloned(),
        actual: actual.cloned(),
        reason: reason.into(),
    });
}
/// Compare declared expectations and semantic observations, never metadata/timing.
/// Error messages and types remain exact; no generic error==error shortcut.
pub fn compare(s: &LoadedScenario, runs: Vec<BackendRun>) -> Comparison {
    let mut mismatches = vec![];
    let mut assertions = 0;
    let mut identities = std::collections::BTreeSet::new();
    for run in &runs {
        if !identities.insert(run.backend) {
            mismatch(
                &mut mismatches,
                run.backend,
                None,
                "",
                "",
                None,
                None,
                "duplicate backend result",
            );
        }
        if run.provenance.source_sha256 != s.source_sha256
            || run.provenance.fixture_sha256 != s.fixture_sha256
        {
            mismatch(
                &mut mismatches,
                run.backend,
                None,
                "",
                "/provenance",
                None,
                None,
                "source/fixture identity differs",
            );
            continue;
        }
        if run.status != RunStatus::Complete {
            if !s.optional(run.backend) {
                mismatch(
                    &mut mismatches,
                    run.backend,
                    None,
                    "",
                    "/status",
                    Some(&json!("complete")),
                    Some(&json!(run.status)),
                    run.reason.as_deref().unwrap_or("backend did not complete"),
                );
            }
            continue;
        }
        for step in &s.scenario.steps {
            let found = run
                .observations
                .iter()
                .filter(|o| o.step == step.id())
                .collect::<Vec<_>>();
            if found.len() != 1 {
                mismatch(
                    &mut mismatches,
                    run.backend,
                    None,
                    step.id(),
                    "",
                    None,
                    None,
                    "step has missing or duplicate observations",
                );
                continue;
            }
            let observation = found[0];
            let status = observation.data["status"].as_str().unwrap_or("unobserved");
            if status == "unobserved"
                || (status != "success"
                    && !s
                        .scenario
                        .expect
                        .iter()
                        .any(|e| e.step == step.id() && e.path == "/status" && e.value == status))
            {
                mismatch(
                    &mut mismatches,
                    run.backend,
                    None,
                    step.id(),
                    "/status",
                    Some(&json!("success or explicitly expected error")),
                    Some(&json!(status)),
                    "unexpected outcome or observation gap",
                );
            }
        }
        for e in &s.scenario.expect {
            assertions += 1;
            let actual = run
                .observations
                .iter()
                .find(|o| o.step == e.step)
                .and_then(|o| o.data.pointer(&e.path));
            if actual != Some(&e.value) {
                mismatch(
                    &mut mismatches,
                    run.backend,
                    None,
                    &e.step,
                    &e.path,
                    Some(&e.value),
                    actual,
                    "declared expectation differs or is absent",
                );
            }
        }
    }
    if let Some(reference) = runs.iter().find(|r| r.status == RunStatus::Complete) {
        for run in runs
            .iter()
            .filter(|r| r.backend != reference.backend && r.status == RunStatus::Complete)
        {
            for expected in &reference.observations {
                let actual = run.observations.iter().find(|o| o.step == expected.step);
                assertions += 1;
                match actual {
                    None => mismatch(
                        &mut mismatches,
                        run.backend,
                        Some(reference.backend),
                        &expected.step,
                        "",
                        Some(&expected.data),
                        None,
                        "backend omitted step",
                    ),
                    Some(actual) => diff(
                        &expected.data,
                        &actual.data,
                        "",
                        run.backend,
                        reference.backend,
                        &expected.step,
                        &mut mismatches,
                    ),
                }
            }
        }
    }
    if assertions == 0 {
        mismatch(
            &mut mismatches,
            BackendId::Native,
            None,
            "",
            "",
            None,
            None,
            "no comparisons performed; provide expectations or at least two available backends",
        );
    }
    let passed = mismatches.is_empty();
    Comparison {
        schema_version: 1,
        fixture: s.scenario.id.clone(),
        fixture_sha256: s.fixture_sha256.clone(),
        source_sha256: s.source_sha256.clone(),
        passed,
        assertions,
        mismatches,
        runs,
    }
}
fn diff(
    expected: &Value,
    actual: &Value,
    path: &str,
    backend: BackendId,
    reference: BackendId,
    step: &str,
    out: &mut Vec<Mismatch>,
) {
    if expected == actual {
        return;
    }
    if path.ends_with("/image") {
        if let (Ok(a), Ok(b)) = (
            serde_json::from_value::<Image>(expected.clone()),
            serde_json::from_value::<Image>(actual.clone()),
        ) {
            match compare_images(&a, &b, false) {
                Ok(result) if result.equal => {}
                Ok(result) => mismatch(
                    out,
                    backend,
                    Some(reference),
                    step,
                    path,
                    Some(&json!({"changed_pixels":0})),
                    Some(&json!(result)),
                    "exact image pixels or dimensions differ",
                ),
                Err(reason) => mismatch(
                    out,
                    backend,
                    Some(reference),
                    step,
                    path,
                    None,
                    None,
                    &reason,
                ),
            }
            return;
        }
    }
    match (expected, actual) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let p = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => diff(x, y, &p, backend, reference, step, out),
                    (x, y) => mismatch(
                        out,
                        backend,
                        Some(reference),
                        step,
                        &p,
                        x,
                        y,
                        "field absent from one backend",
                    ),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                let p = format!("{path}/{i}");
                match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => diff(x, y, &p, backend, reference, step, out),
                    (x, y) => mismatch(
                        out,
                        backend,
                        Some(reference),
                        step,
                        &p,
                        x,
                        y,
                        "array length or element differs",
                    ),
                }
            }
        }
        _ => mismatch(
            out,
            backend,
            Some(reference),
            step,
            path,
            Some(expected),
            Some(actual),
            "exact observable value differs",
        ),
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageDifference {
    pub equal: bool,
    pub changed_pixels: usize,
    pub first: Option<[usize; 2]>,
    pub dimensions_equal: bool,
}
/// Exact by default. RGB565 is an explicitly requested display encoding,
/// never a broad visual similarity tolerance. No implicit clock/status mask.
pub fn compare_images(a: &Image, b: &Image, rgb565: bool) -> Result<ImageDifference, String> {
    let valid = |i: &Image| i.width.checked_mul(i.height) == Some(i.pixels.len());
    if !valid(a) || !valid(b) {
        return Err("invalid image dimensions/pixel count".into());
    }
    if a.width != b.width || a.height != b.height {
        return Ok(ImageDifference {
            equal: false,
            changed_pixels: a.pixels.len().max(b.pixels.len()),
            first: None,
            dimensions_equal: false,
        });
    }
    let mut changed = 0;
    let mut first = None;
    for (i, (x, y)) in a.pixels.iter().zip(&b.pixels).enumerate() {
        let same = if rgb565 {
            ppl_graphics::to_rgb565(*x) == ppl_graphics::to_rgb565(*y)
        } else {
            (x & 0xffffff) == (y & 0xffffff)
        };
        if !same {
            changed += 1;
            if first.is_none() {
                first = Some([i % a.width, i / a.width]);
            }
        }
    }
    Ok(ImageDifference {
        equal: changed == 0,
        changed_pixels: changed,
        first,
        dimensions_equal: true,
    })
}
