//! Headless HP Prime G1 firmware emulator (Phase 2, milestone M2).
//!
//! Executes the original `armfir.elf` ARM firmware under the Unicorn engine
//! and services the firmware's OS calls (SVC-ABI of the Windows CE-derived
//! platform, `SDKLIB.DLL` export numbering) from the host.
//!
//! Architecture map:
//! - [`memory`]: host-backed guest memory blocks + dynamic heap
//! - [`firmware`]: ELF / OS-ROM loading, verified static memory map
//! - [`services`]: 742-entry service registry, verified SVC dispatch ABI
//! - [`engine`]: per-thread Unicorn engine, hooks, slice run loop
//! - [`trace`]: bounded structured diagnostics (human + JSONL)
//! - [`runner`]: boot orchestration, limits, heartbeat detection, report

pub mod calcverify;
pub mod checkprog;
pub mod compiler;
pub mod direct;
pub mod display;
pub mod engine;
pub mod firmware;
pub mod guest_call;
pub mod hpprgm;
pub mod import;
pub mod keys;
pub mod memory;
pub mod progverify;
pub mod runner;
pub mod runprog;
pub mod runtime;
pub mod runtime_value;
pub mod script;
pub mod services;
pub mod session;
pub mod sha256;
pub mod storage;
pub mod trace;
pub mod verify;

pub use runner::{Outcome, RunConfig, RunReport, RunSummary};
