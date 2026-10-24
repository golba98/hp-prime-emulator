//! Structured, bounded diagnostics: human-readable lines on stderr plus an
//! optional machine-readable JSONL stream, with bounded ring buffers and
//! per-service call counters. Hot-path cost is one counter bump per SVC
//! when full SVC tracing is off.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde_json::json;

/// Categories every diagnostic event falls into. Kept closed so the report
/// stays mechanically checkable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Stage,
    Map,
    SvcCall,
    SvcUnsupported,
    Fault,
    Thread,
    Heartbeat,
    Limit,
    Info,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Stage => "stage",
            EventKind::Map => "map",
            EventKind::SvcCall => "svc_call",
            EventKind::SvcUnsupported => "svc_unsupported",
            EventKind::Fault => "fault",
            EventKind::Thread => "thread",
            EventKind::Heartbeat => "heartbeat",
            EventKind::Limit => "limit",
            EventKind::Info => "info",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TraceConfig {
    /// Write every SVC invocation as JSON (default off: counters only).
    pub svc_trace: bool,
    /// Per-instruction code-hook tracing (default off; heavy).
    pub code_trace: bool,
    /// JSONL output path; `None` disables the file stream.
    pub jsonl_path: Option<PathBuf>,
    /// 0 = quiet, 1 = stages/faults/heartbeat, 2 = + SVC summaries, 3 = all.
    pub verbosity: u8,
    /// Cap on retained human-readable ring entries.
    pub ring_capacity: usize,
    /// Record every guest file-service call (paths, modes, byte counts,
    /// directory searches) for the report (`--trace-fs`).
    pub fs_trace: bool,
    /// Passive, version-pinned M6 Check observations.
    pub compiler: bool,
    /// M7 runtime observation, enabled only for the pinned firmware.
    pub runtime: bool,
}

impl Default for TraceConfig {
    fn default() -> Self {
        TraceConfig {
            svc_trace: false,
            code_trace: false,
            jsonl_path: None,
            verbosity: 1,
            ring_capacity: 4096,
            fs_trace: false,
            compiler: false,
            runtime: false,
        }
    }
}

/// One human-readable log line retained for the final report.
#[derive(Clone, Debug)]
pub struct RingEntry {
    pub seq: u64,
    pub kind: EventKind,
    pub message: String,
}

pub struct TraceHub {
    cfg: TraceConfig,
    start: Instant,
    jsonl: Mutex<Option<BufWriter<File>>>,
    ring: Mutex<Vec<RingEntry>>,
    ring_dropped: AtomicU64,
    seq: AtomicU64,
    /// Ring of recently executed guest PCs (instruction tracing).
    code_ring: Mutex<std::collections::VecDeque<u32>>,
    /// Total watchpoint hits (the log itself is capped by `watch_hit`).
    watch_count: Mutex<u64>,
    /// (svc_id, caller_pc) -> invocation count; the raw data behind both the
    /// "unsupported services" report and heartbeat detection.
    call_sites: Mutex<HashMap<(u32, u32), u64>>,
    svc_totals: Mutex<HashMap<u32, u64>>,
    /// `--trace-fs` events (bounded at [`FS_LOG_CAP`]).
    fs_log: Mutex<Vec<serde_json::Value>>,
    fs_dropped: AtomicU64,
    /// Index of the key-script step being processed (`u64::MAX`: none),
    /// stamped on file-service events.
    script_step: AtomicU64,
    pub compiler: Option<crate::compiler::Observer>,
    pub runtime: Option<crate::runtime::Observer>,
}

/// Retained file-service events; later ones are counted, not kept.
pub const FS_LOG_CAP: usize = 50_000;

impl TraceHub {
    pub fn new(cfg: TraceConfig) -> std::io::Result<TraceHub> {
        let file = match &cfg.jsonl_path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Some(BufWriter::new(File::create(p)?))
            }
            None => None,
        };
        Ok(TraceHub {
            compiler: cfg.compiler.then(crate::compiler::Observer::default),
            runtime: cfg.runtime.then(crate::runtime::Observer::default),
            cfg,
            start: Instant::now(),
            jsonl: Mutex::new(file),
            ring: Mutex::new(Vec::new()),
            ring_dropped: AtomicU64::new(0),
            seq: AtomicU64::new(0),
            code_ring: Mutex::new(std::collections::VecDeque::new()),
            watch_count: Mutex::new(0),
            call_sites: Mutex::new(HashMap::new()),
            svc_totals: Mutex::new(HashMap::new()),
            fs_log: Mutex::new(Vec::new()),
            fs_dropped: AtomicU64::new(0),
            script_step: AtomicU64::new(u64::MAX),
        })
    }

    pub fn config(&self) -> &TraceConfig {
        &self.cfg
    }

    /// The key-script step now being processed (stamped on fs events).
    pub fn set_script_step(&self, step: Option<usize>) {
        self.script_step
            .store(step.map_or(u64::MAX, |s| s as u64), Ordering::Relaxed);
    }

    pub fn script_step(&self) -> Option<usize> {
        let s = self.script_step.load(Ordering::Relaxed);
        (s != u64::MAX).then_some(s as usize)
    }

    /// True when guest file services should be recorded.
    pub fn fs_enabled(&self) -> bool {
        self.cfg.fs_trace
    }

    /// Records one guest file-service call (`--trace-fs`): `op` plus
    /// operation details, the calling thread and guest PC, and the time.
    pub fn fs(&self, op: &str, tid: u32, caller_pc: u32, mut detail: serde_json::Value) {
        if !self.cfg.fs_trace {
            return;
        }
        detail["op"] = json!(op);
        detail["tid"] = json!(tid);
        detail["pc"] = json!(caller_pc);
        detail["t"] = json!(self.elapsed_secs());
        let step = self.script_step.load(Ordering::Relaxed);
        detail["step"] = if step == u64::MAX {
            json!(null)
        } else {
            json!(step)
        };
        {
            let mut log = self.fs_log.lock().unwrap();
            if log.len() < FS_LOG_CAP {
                log.push(detail.clone());
            } else {
                self.fs_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        let message = format!("fs {op} {detail}");
        self.record(EventKind::Info, message, json!({ "fs": detail }));
    }

    /// The recorded file-service events and how many were dropped.
    pub fn fs_log(&self) -> (Vec<serde_json::Value>, u64) {
        (
            self.fs_log.lock().unwrap().clone(),
            self.fs_dropped.load(Ordering::Relaxed),
        )
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn record(&self, kind: EventKind, message: String, json: serde_json::Value) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let at = self.start.elapsed().as_secs_f64();
        if let Some(w) = self.jsonl.lock().unwrap().as_mut() {
            let mut obj = json;
            obj["seq"] = json!(seq);
            obj["time"] = json!(at);
            obj["kind"] = json!(kind.as_str());
            let _ = writeln!(w, "{obj}");
            // flush lazily; bounded volume because per-SVC events are gated.
            if matches!(kind, EventKind::Fault | EventKind::Heartbeat) {
                let _ = w.flush();
            }
        }
        let verbose_enough = match self.cfg.verbosity {
            0 => false,
            1 => matches!(
                kind,
                EventKind::Stage | EventKind::Fault | EventKind::Heartbeat | EventKind::Limit
            ),
            2 => !matches!(kind, EventKind::SvcCall),
            _ => true,
        };
        if verbose_enough {
            eprintln!("[{at:8.3}] {} {}", kind.as_str(), message);
        }
        let mut ring = self.ring.lock().unwrap();
        if ring.len() >= self.cfg.ring_capacity {
            ring.remove(0);
            self.ring_dropped.fetch_add(1, Ordering::Relaxed);
        }
        ring.push(RingEntry { seq, kind, message });
    }

    pub fn info(&self, msg: impl Into<String>) {
        self.record(EventKind::Info, msg.into(), json!({}));
    }

    pub fn stage(&self, stage: &str, detail: impl Into<String>) {
        self.record(
            EventKind::Stage,
            format!("{stage}: {}", detail.into()),
            json!({ "stage": stage }),
        );
    }

    pub fn map(&self, name: &str, vaddr: u32, size: usize) {
        self.record(
            EventKind::Map,
            format!("mapped '{name}' at {vaddr:#010X} ({size} bytes)"),
            json!({ "name": name, "vaddr": vaddr, "size": size }),
        );
    }

    pub fn fault(&self, detail: impl Into<String>, pc: u32) {
        let detail = detail.into();
        self.record(
            EventKind::Fault,
            format!("{detail} at pc={pc:#010X}"),
            json!({ "detail": detail_into(detail), "pc": pc }),
        );
    }

    pub fn thread_event(&self, detail: impl Into<String>) {
        self.record(EventKind::Thread, detail.into(), json!({}));
    }

    pub fn limit(&self, detail: impl Into<String>) {
        self.record(EventKind::Limit, detail.into(), json!({}));
    }

    pub fn heartbeat(&self, detail: impl Into<String>) {
        self.record(EventKind::Heartbeat, detail.into(), json!({}));
    }

    /// Count a service invocation. Cheap: two hash bumps. Detailed JSON per
    /// call is only emitted when `svc_trace` is enabled.
    pub fn svc(
        &self,
        id: u32,
        name: Option<&str>,
        caller_pc: u32,
        r0: u32,
        supported: bool,
        thread_id: u32,
    ) {
        *self.svc_totals.lock().unwrap().entry(id).or_insert(0) += 1;
        *self
            .call_sites
            .lock()
            .unwrap()
            .entry((id, caller_pc))
            .or_insert(0) += 1;
        if self.cfg.svc_trace {
            self.record(
                if supported {
                    EventKind::SvcCall
                } else {
                    EventKind::SvcUnsupported
                },
                format!(
                    "svc {id:#07X} ({}) tid={thread_id} caller={caller_pc:#010X} r0={r0:#010X}",
                    name.unwrap_or("unknown"),
                ),
                json!({
                    "id": id,
                    "name": name,
                    "caller_pc": caller_pc,
                    "r0": r0,
                    "supported": supported,
                }),
            );
        } else if !supported {
            // Unsupported hits are always individually visible, even with
            // per-call tracing off: they are the worklist.
            self.record(
                EventKind::SvcUnsupported,
                format!(
                    "unsupported svc {id:#07X} ({}) caller={caller_pc:#010X} r0={r0:#010X}",
                    name.unwrap_or("unknown"),
                ),
                json!({ "id": id, "name": name, "caller_pc": caller_pc, "r0": r0 }),
            );
        }
    }

    /// Bounded log for synchronization/event lifecycle events (v2).
    pub fn sync_event(&self, what: &str, desc: u32, caller: u32, thread_id: u32) {
        if self.cfg.verbosity >= 2 {
            self.record(
                EventKind::Thread,
                format!("{what} desc={desc:#010X} tid={thread_id} caller={caller:#010X}"),
                json!({ "what": what, "desc": desc, "caller_pc": caller, "thread_id": thread_id }),
            );
        }
    }

    /// Bounded watchpoint hit log (first occurrences only).
    pub fn watch_hit(&self, addr: u32, size: usize, value: u32, pc: u32) {
        let mut hits = self.watch_count.lock().unwrap();
        *hits += 1;
        if *hits <= 64 {
            self.record(
                EventKind::Info,
                format!(
                    "watch: write {addr:#010X} ({size} bytes) = {value:#010X} at pc={pc:#010X}"
                ),
                json!({ "addr": addr, "size": size, "value": value, "pc": pc }),
            );
        }
    }

    /// Total watchpoint hits (the individual log is capped by `watch_hit`).
    pub fn watch_hits(&self) -> u64 {
        *self.watch_count.lock().unwrap()
    }

    pub fn svc_totals(&self) -> Vec<(u32, u64)> {
        let mut v: Vec<(u32, u64)> = self
            .svc_totals
            .lock()
            .unwrap()
            .iter()
            .map(|(&k, &c)| (k, c))
            .collect();
        v.sort_by_key(|&(id, _)| id);
        v
    }

    /// Call-site counters: ((svc_id, caller_pc), count), descending by count.
    pub fn call_sites(&self) -> Vec<((u32, u32), u64)> {
        let mut v: Vec<_> = self
            .call_sites
            .lock()
            .unwrap()
            .iter()
            .map(|(&k, &c)| (k, c))
            .collect();
        v.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
        v
    }

    pub fn call_site_count(&self, svc_id: u32, caller_pc: u32) -> u64 {
        *self
            .call_sites
            .lock()
            .unwrap()
            .get(&(svc_id, caller_pc))
            .unwrap_or(&0)
    }

    pub fn ring(&self) -> Vec<RingEntry> {
        self.ring.lock().unwrap().clone()
    }

    /// Record one executed guest PC (bounded ring, no output).
    pub fn note_code(&self, pc: u32) {
        let mut r = self.code_ring.lock().unwrap();
        if r.len() >= 8192 {
            r.pop_front();
        }
        r.push_back(pc);
    }

    /// The last `n` executed PCs, oldest first.
    pub fn last_code(&self, n: usize) -> Vec<u32> {
        let r = self.code_ring.lock().unwrap();
        r.iter().rev().take(n).rev().copied().collect()
    }

    pub fn ring_dropped(&self) -> u64 {
        self.ring_dropped.load(Ordering::Relaxed)
    }
}

fn detail_into(d: impl Into<String>) -> String {
    d.into()
}
