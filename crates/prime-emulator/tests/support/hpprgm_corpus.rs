use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Case {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub source_file: Option<String>,
    #[serde(skip)]
    pub source: Option<String>,
    pub container: bool,
    pub encoding: String,
    pub name: Option<String>,
    pub as_source: bool,
    pub import_result: String,
    pub firmware: bool,
}
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/hpprgm")
}
pub fn cases() -> Vec<Case> {
    let mut cases: Vec<Case> =
        serde_json::from_slice(&std::fs::read(root().join("cases.json")).unwrap()).unwrap();
    for case in &mut cases {
        case.source = case
            .source_file
            .as_ref()
            .map(|p| std::fs::read_to_string(root().join(p)).unwrap());
    }
    cases
}
