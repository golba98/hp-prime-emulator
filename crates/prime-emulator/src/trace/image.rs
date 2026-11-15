//! Observer and file-log cursors shared with managed program requests.
//! Diagnostic sinks are supplied by the new process; no saved host file handle
//! or JSONL output path is reopened from an image.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    version: u32,
    elapsed: Duration,
    fs_enabled: bool,
    fs_log: Vec<serde_json::Value>,
    fs_dropped: u64,
    script_step: u64,
    compiler: Option<crate::compiler::image::Image>,
    runtime: Option<crate::runtime::image::Image>,
    unsupported: Vec<(u32, Option<String>, u32)>,
    call_sites: Vec<(u32, u32, u64)>,
    svc_totals: BTreeMap<u32, u64>,
    threads: BTreeMap<u32, ThreadObservation>,
    threads_dropped: u64,
    ring: Vec<RingEntry>,
    ring_dropped: u64,
    sequence: u64,
    code: VecDeque<u32>,
    watches: u64,
    probes: u64,
}
impl TraceHub {
    /// Caller holds the same owner/memory exclusion as the machine components.
    pub fn image(&self) -> Image {
        Image {
            version: 1,
            elapsed: self.elapsed_before + self.start.elapsed(),
            fs_enabled: self.cfg.fs_trace,
            fs_log: self.fs_log.lock().unwrap().clone(),
            fs_dropped: self.fs_dropped.load(Ordering::Acquire),
            script_step: self.script_step.load(Ordering::Acquire),
            compiler: self.compiler.as_ref().map(crate::compiler::Observer::image),
            runtime: self.runtime.as_ref().map(crate::runtime::Observer::image),
            unsupported: self.unsupported_sites.lock().unwrap().clone(),
            call_sites: self
                .call_sites
                .lock()
                .unwrap()
                .iter()
                .map(|(&(service, pc), &count)| (service, pc, count))
                .collect(),
            svc_totals: self
                .svc_totals
                .lock()
                .unwrap()
                .iter()
                .map(|(&k, &v)| (k, v))
                .collect(),
            threads: self.threads.lock().unwrap().clone(),
            threads_dropped: self.thread_observations_dropped.load(Ordering::Acquire),
            ring: self.ring.lock().unwrap().clone(),
            ring_dropped: self.ring_dropped.load(Ordering::Acquire),
            sequence: self.seq.load(Ordering::Acquire),
            code: self.code_ring.lock().unwrap().clone(),
            watches: *self.watch_count.lock().unwrap(),
            probes: *self.probe_count.lock().unwrap(),
        }
    }
}
impl Image {
    pub fn restore(
        &self,
        mem: &crate::memory::GuestMemory,
        cfg: TraceConfig,
    ) -> Result<TraceHub, String> {
        if self.version != 1
            || self.elapsed > Duration::from_secs(365 * 86400)
            || self.fs_log.len() > FS_LOG_CAP
            || self.threads.len() > 128
            || self.call_sites.len() > 65536
            || self.svc_totals.len() > 65536
            || self.unsupported.len() > 65536
            || self.ring.len() > 65536
            || self.code.len() > 65536
            || self.fs_enabled != cfg.fs_trace
            || self.compiler.is_some() != cfg.compiler
            || self.runtime.is_some() != cfg.runtime
            || self
                .call_sites
                .iter()
                .map(|(s, pc, _)| (*s, *pc))
                .collect::<BTreeSet<_>>()
                .len()
                != self.call_sites.len()
            || self.threads.iter().any(|(&id, t)| {
                t.thread_id != id || !t.observed_ms.is_finite() || t.observed_ms < 0.
            })
        {
            return Err(
                "Trace/observer image version, configuration, cursor or bounds mismatch".into(),
            );
        }
        // Admit passive observers before creating any new output sink.
        let compiler = self
            .compiler
            .as_ref()
            .map(|image| image.restore(mem))
            .transpose()?;
        let runtime = self
            .runtime
            .as_ref()
            .map(|image| image.restore(mem))
            .transpose()?;
        let mut trace = TraceHub::new(cfg).map_err(|e| e.to_string())?;
        trace.elapsed_before = self.elapsed;
        trace.start = Instant::now();
        trace.compiler = compiler;
        trace.runtime = runtime;
        trace.fs_log = Mutex::new(self.fs_log.clone());
        trace.fs_dropped.store(self.fs_dropped, Ordering::Release);
        trace.script_step.store(self.script_step, Ordering::Release);
        trace.unsupported_sites = Mutex::new(self.unsupported.clone());
        trace.call_sites = Mutex::new(
            self.call_sites
                .iter()
                .map(|&(s, pc, n)| ((s, pc), n))
                .collect(),
        );
        trace.svc_totals = Mutex::new(self.svc_totals.clone().into_iter().collect());
        trace.threads = Mutex::new(self.threads.clone());
        trace
            .thread_observations_dropped
            .store(self.threads_dropped, Ordering::Release);
        trace.ring = Mutex::new(self.ring.clone());
        trace
            .ring_dropped
            .store(self.ring_dropped, Ordering::Release);
        trace.seq.store(self.sequence, Ordering::Release);
        trace.code_ring = Mutex::new(self.code.clone());
        trace.watch_count = Mutex::new(self.watches);
        trace.probe_count = Mutex::new(self.probes);
        Ok(trace)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_cursors_observer_modes_and_timestamps_continue_from_the_saved_trace() {
        let mut mem = crate::memory::GuestMemory::new();
        mem.map("trace", 0x1000, 4096).unwrap();
        let cfg = TraceConfig {
            fs_trace: true,
            compiler: true,
            runtime: true,
            verbosity: 0,
            ..Default::default()
        };
        let trace = TraceHub::new(cfg.clone()).unwrap();
        trace.fs("fread", 0, 0x1000, serde_json::json!({"path":"C:/source"}));
        trace.set_script_step(Some(3));
        trace.note_code(0x1000);
        let image: Image =
            serde_json::from_slice(&serde_json::to_vec(&trace.image()).unwrap()).unwrap();
        let restored = image.restore(&mem, cfg.clone()).unwrap();
        assert_eq!(restored.fs_log_since(0), trace.fs_log_since(0));
        assert_eq!(restored.script_step(), Some(3));
        assert_eq!(restored.last_code(8), vec![0x1000]);
        restored.fs("fseek", 0, 0x1004, serde_json::json!({"offset":7}));
        let (events, cursor, dropped) = restored.fs_log_since(1);
        assert_eq!((events.len(), cursor, dropped), (1, 2, 0));
        assert_eq!(events[0]["op"], "fseek");
        assert!(restored.elapsed_secs() >= image.elapsed.as_secs_f64());
        let mut mismatched = cfg;
        mismatched.runtime = false;
        assert!(image.restore(&mem, mismatched).is_err());
    }
}
