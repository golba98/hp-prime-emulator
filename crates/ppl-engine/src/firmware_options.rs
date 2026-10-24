//! Platform-independent configuration for the optional firmware helper.
use std::{path::PathBuf, time::Duration};
#[derive(Debug, Clone)]
pub struct Options {
    pub runner: Option<PathBuf>,
    pub firmware: Option<PathBuf>,
    pub os_rom: Option<PathBuf>,
    pub factory_a: Option<PathBuf>,
    pub init_data: Option<PathBuf>,
    pub artifacts_dir: Option<PathBuf>,
    pub program_name: Option<String>,
    pub entry: Option<String>,
    pub args: String,
    pub runtime_keys: Option<String>,
    pub screenshot: Option<PathBuf>,
    pub timeout: Duration,
    pub boot_timeout: Duration,
    pub compile_timeout: Duration,
    pub execution_timeout: Duration,
    pub max_instructions: u64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            runner: None,
            firmware: None,
            os_rom: None,
            factory_a: None,
            init_data: None,
            artifacts_dir: None,
            program_name: None,
            entry: None,
            args: "[]".into(),
            runtime_keys: None,
            screenshot: None,
            timeout: Duration::from_secs(360),
            boot_timeout: Duration::from_secs(60),
            compile_timeout: Duration::from_secs(30),
            execution_timeout: Duration::from_secs(30),
            max_instructions: 8_000_000_000,
        }
    }
}
