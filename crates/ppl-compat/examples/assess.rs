//! Offline assessment driver. Never opens USB. Arguments: SUITE OUTPUT_DIR.
use std::path::Path;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 && args.len() != 5 {
        return Err("usage: assess SUITE OUTPUT_DIR [RUNS_DIR EVIDENCE_DIR]".into());
    }
    let runs = args
        .get(3)
        .map(String::as_str)
        .unwrap_or("tests/compat/runs");
    let evidence = args
        .get(4)
        .map(String::as_str)
        .unwrap_or("tests/compat/evidence");
    let suite = ppl_compat::load_suite(Path::new(&args[1]))?;
    let ev = ppl_compat::load_suite_evidence(Path::new(evidence), &suite.suite)?;
    let assessment =
        ppl_compat::assessment::assess(&suite, &ev, Path::new(runs), Path::new(evidence));
    let out = Path::new(&args[2]);
    std::fs::create_dir_all(out)?;
    std::fs::write(
        out.join(format!("assessment-{}.json", suite.suite)),
        serde_json::to_vec_pretty(&assessment)?,
    )?;
    println!("{}", assessment.report.markdown());
    for audit in assessment.audits {
        println!("{}: {} ({})", audit.program, audit.accepted, audit.reason);
    }
    Ok(())
}
