//! The passive compiler observer is part of an in-flight managed request.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    version: u32,
    trace: CompilerTrace,
    direct_return: u32,
}
impl Observer {
    pub fn image(&self) -> Image {
        Image {
            version: 1,
            trace: self.snapshot(),
            direct_return: self.1.load(std::sync::atomic::Ordering::Acquire),
        }
    }
}
impl Image {
    pub fn restore(&self, mem: &GuestMemory) -> Result<Observer, String> {
        if self.version != 1 || self.trace.checks.len() > MAX_CHECKS {
            return Err("Compiler observer image version/count mismatch".into());
        }
        for (index, check) in self.trace.checks.iter().enumerate() {
            if check.sequence != index
                || !valid_time(check.started_ms)
                || check.returned_ms.is_some_and(|v| !valid_time(v))
                || check.message_ms.is_some_and(|v| !valid_time(v))
                || check.sources.len() > MAX_SOURCE_UNITS
                || check.errors.len() > 65536
                || check.parser_returns.len() > 65536
            {
                return Err("Invalid compiler observation sequence/timing/bounds".into());
            }
        }
        if self.direct_return != 0 {
            let actual = mem
                .read_bytes_alloc(self.direct_return, 8)
                .map_err(|e| e.to_string())?;
            let expected = (0xef000000 | crate::guest_call::RETURN_SVC)
                .to_le_bytes()
                .repeat(2);
            if actual != expected {
                return Err("Compiler observer return trap differs from restored memory".into());
            }
        }
        Ok(Observer(
            Mutex::new(self.trace.clone()),
            std::sync::atomic::AtomicU32::new(self.direct_return),
        ))
    }
}
fn valid_time(value: f64) -> bool {
    value.is_finite() && (0.0..=365.0 * 86400000.0).contains(&value)
}
