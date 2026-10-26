//! Passive observations of the G1 2.4.2 Program Editor's Check command.
//!
//! These addresses and conventions were traced through the genuine UI;
//! none is an entry point that the host calls. See the M6 report.
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use unicorn_engine::{RegisterARM, Unicorn, uc_error};

use crate::{memory::GuestMemory, sha256, trace::TraceHub};

pub const FIRMWARE_SHA256: &str =
    "ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39";
pub const CHECK_ENTRY: u32 = 0x308f_2c60;
pub const COMPILE_ENTRY: u32 = 0x306b_b740;
pub const SOURCE_CALL: u32 = 0x306b_b8b8;
pub const PARSE_RETURN: u32 = 0x306b_b8e0;
pub const CHECK_RETURN: u32 = 0x308f_2c94;
pub const DIALOG_ENTRY: u32 = 0x3068_5b28;
pub const DIALOG_TEXT: u32 = 0x3068_5b54;
pub const MAX_SOURCE_UNITS: usize = 1_048_576;
pub const MAX_CHECKS: usize = 64;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompilerTrace {
    pub checks: Vec<CheckObservation>,
    pub dropped: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckObservation {
    pub sequence: usize,
    pub step: Option<usize>,
    pub thread: u32,
    pub started_ms: f64,
    pub returned_ms: Option<f64>,
    pub message_ms: Option<f64>,
    pub cpsr: u32,
    pub compiler_entered: bool,
    pub program_va: Option<u32>,
    pub result_va: Option<u32>,
    pub return_value: Option<u32>,
    /// Only read on failure. On success the last two stack words are stale.
    pub raw_position: Option<RawPosition>,
    pub sources: Vec<ObservedSource>,
    /// Raw parser statuses, not a documented diagnostic-code namespace.
    pub parser_returns: Vec<u32>,
    pub dialog_resource: Option<u32>,
    pub message_va: Option<u32>,
    pub message: Option<String>,
    pub heap_before: Option<(usize, usize, usize)>,
    pub heap_after: Option<(usize, usize, usize)>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawPosition {
    pub source_index: i32,
    pub line: i32,
    pub offset_utf16: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservedSource {
    pub source_index: u32,
    pub address: u32,
    pub sha256: String,
    pub utf16_units: usize,
}

#[derive(Default)]
pub struct Observer(Mutex<CompilerTrace>);

impl Observer {
    pub fn reset(&self) {
        *self.0.lock().unwrap() = CompilerTrace::default();
    }
    pub fn snapshot(&self) -> CompilerTrace {
        self.0.lock().unwrap().clone()
    }

    fn observe(
        &self,
        uc: &Unicorn<'_, ()>,
        mem: &GuestMemory,
        trace: &TraceHub,
        thread: u32,
        pc: u32,
    ) -> Result<(), String> {
        let reg = |r| uc.reg_read(r).map(|v| v as u32).map_err(|e| e.to_string());
        let mut log = self.0.lock().unwrap();
        if pc == CHECK_ENTRY {
            if log.checks.len() == MAX_CHECKS {
                log.dropped += 1;
                return Ok(());
            }
            let sequence = log.checks.len();
            log.checks.push(CheckObservation {
                sequence,
                thread,
                step: trace.script_step(),
                started_ms: trace.elapsed_secs() * 1000.0,
                cpsr: reg(RegisterARM::CPSR)?,
                ..Default::default()
            });
            return Ok(());
        }
        // Only the current UI Check, on its own guest thread, may own data.
        let Some(c) = log.checks.last_mut().filter(|c| c.thread == thread) else {
            return Ok(());
        };
        if log_dropped(c, pc) {
            return Ok(());
        }
        match pc {
            COMPILE_ENTRY if reg(RegisterARM::LR)? == CHECK_RETURN => {
                if c.compiler_entered {
                    c.errors
                        .push("repeated compiler entry for one Check".into());
                }
                c.compiler_entered = true;
                c.program_va = Some(reg(RegisterARM::R0)?);
                c.result_va = Some(reg(RegisterARM::R3)?);
                c.heap_before = Some(mem.heap_stats());
                if reg(RegisterARM::R1)? != 1 || reg(RegisterARM::R2)? != 0 {
                    c.errors
                        .push("unrecognized Check compiler arguments".into());
                }
            }
            SOURCE_CALL if c.compiler_entered && c.return_value.is_none() => {
                if c.sources.len() >= 16 {
                    return Err("source observation limit exceeded".into());
                }
                let address = reg(RegisterARM::R0)?;
                let text = read_utf16(mem, address, MAX_SOURCE_UNITS)?;
                c.sources.push(ObservedSource {
                    source_index: reg(RegisterARM::R3)?,
                    address,
                    sha256: sha256::hex(text.as_bytes()),
                    utf16_units: text.encode_utf16().count(),
                });
            }
            PARSE_RETURN if c.compiler_entered && c.return_value.is_none() => {
                if c.parser_returns.len() >= 16 {
                    return Err("parser observation limit exceeded".into());
                }
                c.parser_returns.push(reg(RegisterARM::R0)?);
            }
            CHECK_RETURN if c.compiler_entered && c.return_value.is_none() => {
                let result = reg(RegisterARM::R0)?;
                c.return_value = Some(result);
                c.returned_ms = Some(trace.elapsed_secs() * 1000.0);
                c.heap_after = Some(mem.heap_stats());
                if c.result_va != Some(reg(RegisterARM::SP)?) {
                    return Err("compiler result pointer differs from the Check stack".into());
                }
                if result == 0 {
                    let p = reg(RegisterARM::SP)?;
                    c.raw_position = Some(RawPosition {
                        source_index: mem.read_u32(p).map_err(|e| e.to_string())? as i32,
                        line: mem.read_u32(p + 4).map_err(|e| e.to_string())? as i32,
                        offset_utf16: mem.read_u32(p + 8).map_err(|e| e.to_string())? as i32,
                    });
                }
            }
            DIALOG_ENTRY if c.return_value.is_some() && reg(RegisterARM::LR)? == 0x308f_2cf4 => {
                c.dialog_resource = Some(reg(RegisterARM::R0)?);
            }
            DIALOG_TEXT
                if c.dialog_resource == Some(reg(RegisterARM::R4)?) && c.message.is_none() =>
            {
                let address = reg(RegisterARM::R0)?;
                c.message_va = Some(address);
                c.message = Some(read_utf16(mem, address, 4096)?);
                c.message_ms = Some(trace.elapsed_secs() * 1000.0);
            }
            _ => {}
        }
        Ok(())
    }

    fn error(&self, thread: u32, e: String) {
        if let Some(c) = self
            .0
            .lock()
            .unwrap()
            .checks
            .last_mut()
            .filter(|c| c.thread == thread)
            && c.errors.len() < 16
        {
            c.errors.push(e);
        }
    }
}

fn log_dropped(c: &CheckObservation, pc: u32) -> bool {
    // Once the Check dialog has been captured, later editor saves/compiles
    // must not append observations to this invocation.
    c.message.is_some() && pc != CHECK_ENTRY
}

/// Strict UTF-16 decoding, including surrogate pairs. Never read past a
/// mapped block or accept a missing terminator/truncated diagnostic.
pub fn read_utf16(mem: &GuestMemory, address: u32, limit: usize) -> Result<String, String> {
    if address == 0 || address & 1 != 0 {
        return Err("null or unaligned UTF-16 pointer".into());
    }
    let mut units = Vec::new();
    for i in 0..limit {
        let at = address
            .checked_add((i as u32).checked_mul(2).ok_or("UTF-16 overflow")?)
            .ok_or("UTF-16 overflow")?;
        let u = mem.read_u16(at).map_err(|e| e.to_string())?;
        if u == 0 {
            return String::from_utf16(&units).map_err(|e| e.to_string());
        }
        units.push(u);
    }
    Err("unterminated UTF-16 buffer exceeds observation limit".into())
}

pub fn install(
    engine: &mut Unicorn<'static, ()>,
    mem: &Arc<GuestMemory>,
    trace: &Arc<TraceHub>,
    thread: u32,
) -> Result<(), uc_error> {
    if trace.compiler.is_none() {
        return Ok(());
    }
    for pc in [
        CHECK_ENTRY,
        COMPILE_ENTRY,
        SOURCE_CALL,
        PARSE_RETURN,
        CHECK_RETURN,
        DIALOG_ENTRY,
        DIALOG_TEXT,
    ] {
        let mem = mem.clone();
        let trace = trace.clone();
        engine.add_code_hook(
            pc as u64,
            pc as u64,
            Box::new(move |uc: &mut Unicorn<'_, ()>, _, _| {
                let observer = trace.compiler.as_ref().unwrap();
                if let Err(e) = observer.observe(uc, &mem, &trace, thread, pc) {
                    observer.error(thread, e);
                }
            }),
        )?;
    }
    Ok(())
}

/// A one-based Unicode-scalar column mapped from the guest's zero-based
/// UTF-16 offset. Keep the raw offset too; surrogate-interior offsets have
/// no scalar-column mapping and return None.
pub fn position(source: &str, raw: &RawPosition) -> Option<(usize, usize)> {
    if raw.source_index != 0 || raw.line < 1 || raw.offset_utf16 < 0 {
        return None;
    }
    let target = raw.offset_utf16 as usize;
    let (mut units, mut line, mut col) = (0, 1, 1);
    for ch in source.chars() {
        if units == target {
            return (line == raw.line as usize).then_some((line, col));
        }
        units += ch.len_utf16();
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (units == target && line == raw.line as usize).then_some((line, col))
}
