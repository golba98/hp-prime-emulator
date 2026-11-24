//! Data for the production G1 compiler call sequence. Output channels are
//! deliberately rebound by the owning request, never serialized host pointers.
use super::{Outcome, Plan, Stage, code_matches};
use crate::{memory::GuestMemory, sha256};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

const MAX_UNITS: usize = 1024 * 1024;
const MAX_MILLISECONDS: f64 = 365.0 * 24.0 * 60.0 * 60.0 * 1000.0;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub request: u64,
    pub source_sha256: String,
    pub utf16_units: usize,
}
impl Binding {
    pub fn new(request: u64, source: &str) -> Result<Self, String> {
        let utf16_units = source.encode_utf16().count();
        if utf16_units > MAX_UNITS || source.contains('\0') {
            return Err("Direct compiler source exceeds bounds or contains NUL".into());
        }
        Ok(Self {
            request,
            source_sha256: sha256::hex(source.as_bytes()),
            utf16_units,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanImage {
    version: u32,
    binding: Binding,
    ctx: u32,
    text_va: u32,
    stage: Stage,
    item: u32,
    copy: u32,
    elapsed_ms: Option<f64>,
    outcome: Outcome,
}

pub(crate) fn valid_time(ms: f64) -> bool {
    ms.is_finite() && (0.0..=MAX_MILLISECONDS).contains(&ms)
}
impl PlanImage {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    pub(crate) fn started(&self) -> bool {
        self.elapsed_ms.is_some()
    }
    pub(crate) fn frame_words(&self) -> usize {
        match self.stage {
            Stage::Push => 1,
            Stage::Compile => 3,
            _ => 0,
        }
    }

    fn source(&self, mem: &GuestMemory, address: u32) -> Result<String, String> {
        if address == 0 || address % 2 != 0 || self.binding.utf16_units > MAX_UNITS {
            return Err("Invalid direct compiler source address/length".into());
        }
        let bytes = mem
            .read_bytes_alloc(address, (self.binding.utf16_units + 1) * 2)
            .map_err(|e| e.to_string())?;
        let mut units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        if units.pop() != Some(0) || units.contains(&0) {
            return Err("Direct compiler source terminator mismatch".into());
        }
        String::from_utf16(&units).map_err(|e| e.to_string())
    }

    pub(crate) fn validate(&self, mem: &GuestMemory) -> Result<(), String> {
        const PHASES: [Stage; 8] = [
            Stage::ClearSources,
            Stage::ClearSecond,
            Stage::NewItem,
            Stage::CopyText,
            Stage::InitItem,
            Stage::Push,
            Stage::Compile,
            Stage::Message,
        ];
        let index = PHASES
            .iter()
            .position(|phase| *phase == self.stage)
            .ok_or("Completed direct plans belong to their request's result, not the call queue")?;
        if self.version != 1
            || self.binding.utf16_units > MAX_UNITS
            || self.elapsed_ms.is_some_and(|ms| !valid_time(ms))
            || (self.elapsed_ms.is_none() && (index != 0 || !self.outcome.calls.is_empty()))
            || self.outcome.text_va != Some(self.text_va)
            || self.outcome.error.is_some()
            || self.outcome.message.is_some()
            || self.outcome.calls.len() != index
            || self
                .outcome
                .calls
                .iter()
                .zip(PHASES)
                .any(|((name, ms), phase)| name != &format!("{phase:?}") || !valid_time(*ms))
            || self.ctx == 0
            || self.ctx % 4 != 0
        {
            return Err("Invalid direct compiler phase, timing or outcome".into());
        }
        mem.validate_range(self.ctx, 0x320)
            .map_err(|e| e.to_string())?;
        if mem
            .heap_size_of(self.text_va)
            .is_none_or(|size| size < (self.binding.utf16_units + 1) * 2)
        {
            return Err("Direct compiler source has no matching live heap allocation".into());
        }
        if Binding::new(self.binding.request, &self.source(mem, self.text_va)?)? != self.binding {
            return Err("Direct compiler source identity mismatch".into());
        }
        if index >= 3 {
            if self.item == 0 || self.item % 4 != 0 {
                return Err("Direct compiler item is missing or misaligned".into());
            }
            mem.validate_range(self.item, 0x48)
                .map_err(|e| e.to_string())?;
        } else if self.item != 0 {
            return Err("Direct compiler item precedes its allocation".into());
        }
        if index >= 4 {
            if Binding::new(self.binding.request, &self.source(mem, self.copy)?)? != self.binding {
                return Err("Direct compiler copied source identity mismatch".into());
            }
        } else if self.copy != 0 {
            return Err("Direct compiler copied source precedes its allocation".into());
        }
        if index == 7 {
            if !matches!(self.outcome.return_value, Some(0 | 1))
                || self.outcome.record.is_none()
                || self.outcome.record_va.is_none()
                || self.outcome.heap_after.is_none()
            {
                return Err("Direct compiler message phase lacks its compiler return".into());
            }
            mem.validate_range(self.outcome.record_va.unwrap(), 12)
                .map_err(|e| e.to_string())?;
        } else if self.outcome.return_value.is_some()
            || self.outcome.record.is_some()
            || self.outcome.record_va.is_some()
            || self.outcome.heap_after.is_some()
        {
            return Err("Direct compiler return precedes compilation".into());
        }
        code_matches(mem)
    }

    pub fn restore(
        &self,
        mem: &GuestMemory,
        request: u64,
        source: &str,
        output: Arc<Mutex<Option<Outcome>>>,
    ) -> Result<Plan, String> {
        self.validate(mem)?;
        if self.binding != Binding::new(request, source)? {
            return Err("Direct compiler request/source binding mismatch".into());
        }
        if output
            .lock()
            .map_err(|_| "Direct compiler output mutex poisoned")?
            .is_some()
        {
            return Err("Direct compiler restored output is already occupied".into());
        }
        Ok(Plan {
            binding: self.binding.clone(),
            ctx: self.ctx,
            text_va: self.text_va,
            stage: self.stage,
            item: self.item,
            copy: self.copy,
            started: self.elapsed_ms.map(|_| Instant::now()),
            elapsed_ms: self.elapsed_ms.unwrap_or(0.0),
            out: output,
            o: self.outcome.clone(),
        })
    }
}
impl Plan {
    pub fn image(&self, mem: &GuestMemory) -> Result<PlanImage, String> {
        if self
            .out
            .lock()
            .map_err(|_| "Direct compiler output mutex poisoned")?
            .is_some()
        {
            return Err("Active direct compiler output is already occupied".into());
        }
        let image = PlanImage {
            version: 1,
            binding: self.binding.clone(),
            ctx: self.ctx,
            text_va: self.text_va,
            stage: self.stage,
            item: self.item,
            copy: self.copy,
            elapsed_ms: self
                .started
                .map(|start| self.elapsed_ms + start.elapsed().as_secs_f64() * 1000.0),
            outcome: self.o.clone(),
        };
        image.validate(mem)?;
        Ok(image)
    }
}
