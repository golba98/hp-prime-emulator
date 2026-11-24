//! Portable managed-call state, including the interrupted UI's full context.
//! Caller must hold coordinated owner/memory exclusion; this is one component
//! of a live machine, not a standalone calculator snapshot.
use super::{Active, FRAME_BYTES, GuestCalls, Plan, RETURN_SVC, Returned, State};
use crate::{
    cpu_state::ArmCpuImage,
    direct::{
        Outcome,
        image::{PlanImage as CompilerImage, valid_time},
    },
    memory::GuestMemory,
    sha256,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};
use unicorn_engine::Unicorn;

// Keep the existing compiler JSON representation byte-for-byte compatible.
// A text plan has its own unambiguous tagged object instead of borrowing
// compiler phases or fabricating a Check outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum PlanImage {
    Compiler(CompilerImage),
    Text(TextImage),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextImage {
    desktop_text: super::text::Image,
}
impl PlanImage {
    fn validate(&self, mem: &GuestMemory) -> Result<(), String> {
        match self {
            Self::Compiler(p) => p.validate(mem),
            Self::Text(TextImage { desktop_text: p }) => p.validate(mem),
        }
    }
    fn started(&self) -> bool {
        match self {
            Self::Compiler(p) => p.started(),
            Self::Text(TextImage { desktop_text: p }) => p.started,
        }
    }
    fn frame_words(&self) -> usize {
        match self {
            Self::Compiler(p) => p.frame_words(),
            Self::Text(_) => 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveImage {
    saved: ArmCpuImage,
    frame_va: u32,
    frame_words: usize,
    elapsed_ms: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestCallsImage {
    version: u32,
    resource_identity: String,
    plan: Option<PlanImage>,
    active: Option<ActiveImage>,
    last: Option<Returned>,
    return_va: u32,
    sha256: String,
}
/// New transport binding, supplied from the restored owning job's exact source.
pub struct OutputBinding<'a> {
    pub request: u64,
    pub source: &'a str,
    pub output: Arc<Mutex<Option<Outcome>>>,
}
impl GuestCallsImage {
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Guest call image exceeds byte budget".into());
        }
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 65536 {
            return Err("Guest call image exceeds byte budget".into());
        }
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
    fn digest(&self) -> Result<String, String> {
        serde_json::to_vec(&(
            self.version,
            &self.resource_identity,
            &self.plan,
            &self.active,
            &self.last,
            self.return_va,
        ))
        .map(|bytes| sha256::hex(&bytes))
        .map_err(|e| e.to_string())
    }
    pub fn binding(&self) -> Option<&crate::direct::image::Binding> {
        match self.plan.as_ref() {
            Some(PlanImage::Compiler(plan)) => Some(plan.binding()),
            _ => None,
        }
    }
    pub(crate) fn text_binding(&self) -> Option<&super::text::Binding> {
        match self.plan.as_ref() {
            Some(PlanImage::Text(TextImage { desktop_text: plan })) => Some(&plan.binding),
            _ => None,
        }
    }
    fn validate(
        &self,
        mem: &GuestMemory,
        identity: &str,
        cpu: &Unicorn<'_, ()>,
    ) -> Result<(), String> {
        if self.version != 1
            || identity.is_empty()
            || identity.len() > 256
            || self.resource_identity != identity
            || self.sha256 != self.digest()?
        {
            return Err("Guest call image version/identity/checksum mismatch".into());
        }
        if self.return_va != 0 {
            if self.return_va % 4 != 0
                || mem.heap_size_of(self.return_va).is_none_or(|size| size < 8)
            {
                return Err("Guest return trap is misaligned or has no live allocation".into());
            }
            let bytes = mem
                .read_bytes_alloc(self.return_va, 8)
                .map_err(|e| e.to_string())?;
            let expected = (0xEF00_0000 | RETURN_SVC).to_le_bytes().repeat(2);
            if bytes != expected {
                return Err("Guest return trap code differs".into());
            }
        }
        let Some(plan) = &self.plan else {
            return if self.active.is_none() && self.last.is_none() {
                Ok(())
            } else {
                Err("Guest call execution has no owning plan".into())
            };
        };
        plan.validate(mem)?;
        if self.return_va == 0
            || (self.active.is_some() && self.last.is_some())
            || plan.started() != (self.active.is_some() || self.last.is_some())
        {
            return Err("Guest call phase disagrees with its active/returned call".into());
        }
        if let Some(active) = &self.active {
            active.saved.validate(cpu)?;
            if active.saved.cpsr & 0x20 != 0
                || active.saved.gprs[13]
                    .checked_sub(FRAME_BYTES)
                    .map(|sp| sp & !7)
                    != Some(active.frame_va)
                || active.frame_words != plan.frame_words()
                || !valid_time(active.elapsed_ms)
            {
                return Err("Guest call saved context/frame mismatch".into());
            }
            mem.validate_range(active.saved.gprs[15], 4)
                .map_err(|e| e.to_string())?;
            mem.validate_range(active.frame_va, FRAME_BYTES as usize)
                .map_err(|e| e.to_string())?;
        }
        if let Some(last) = &self.last {
            if last.frame.len() != plan.frame_words()
                || last.frame_va % 8 != 0
                || !valid_time(last.elapsed_ms)
            {
                return Err("Guest call returned frame shape/timing mismatch".into());
            }
            mem.validate_range(last.frame_va, FRAME_BYTES as usize)
                .map_err(|e| e.to_string())?;
            for (index, word) in last.frame.iter().enumerate() {
                if mem
                    .read_u32(last.frame_va + index as u32 * 4)
                    .map_err(|e| e.to_string())?
                    != *word
                {
                    return Err("Guest call returned frame differs from guest memory".into());
                }
            }
        }
        Ok(())
    }
    /// Pure admission never constructs a buffer-owning text plan whose Drop
    /// could free the stopped world's arguments before real installation.
    pub(crate) fn admit_in_world(
        &self,
        mem: &GuestMemory,
        identity: &str,
        binding: Option<OutputBinding<'_>>,
        text_calls: &super::text::Mailbox,
    ) -> Result<(), String> {
        let mut cpu = Unicorn::new(
            unicorn_engine::unicorn_const::Arch::ARM,
            unicorn_engine::unicorn_const::Mode::ARM,
        )
        .map_err(|e| e.to_string())?;
        cpu.ctl_set_cpu_model(unicorn_engine::ArmCpuModel::Model_926 as i32)
            .map_err(|e| e.to_string())?;
        self.validate(mem, identity, &cpu)?;
        match (&self.plan, binding) {
            (Some(PlanImage::Compiler(plan)), Some(binding)) => {
                plan.restore(mem, binding.request, binding.source, binding.output)?;
            }
            (Some(PlanImage::Text(TextImage { desktop_text: plan })), None) => {
                text_calls.validate_binding(&plan.binding, true)?
            }
            (None, None) => {}
            _ => return Err("Guest operation owner binding mismatch".into()),
        }
        Ok(())
    }
    /// Admit the complete component before constructing a fresh call queue.
    /// The supplied CPU is used for model/schema admission and remains unchanged.
    pub fn restore(
        &self,
        mem: &GuestMemory,
        identity: &str,
        cpu: &Unicorn<'_, ()>,
        binding: Option<OutputBinding<'_>>,
    ) -> Result<GuestCalls, String> {
        self.restore_inner(mem, identity, cpu, binding, None)
    }
    fn restore_inner(
        &self,
        mem: &GuestMemory,
        identity: &str,
        cpu: &Unicorn<'_, ()>,
        binding: Option<OutputBinding<'_>>,
        text_world: Option<(Arc<GuestMemory>, Arc<super::text::Mailbox>)>,
    ) -> Result<GuestCalls, String> {
        self.validate(mem, identity, cpu)?;
        let plan = match (&self.plan, binding, text_world) {
            (Some(PlanImage::Compiler(plan)), Some(binding), _) => Some(Plan::Compiler(
                plan.restore(mem, binding.request, binding.source, binding.output)?,
            )),
            (
                Some(PlanImage::Text(TextImage { desktop_text: plan })),
                None,
                Some((memory, mailbox)),
            ) => Some(Plan::Text(plan.restore(memory, mailbox)?)),
            (None, None, _) => None,
            _ => {
                return Err("Guest call output binding does not match the owning operation".into());
            }
        };
        Ok(GuestCalls {
            state: Mutex::new(State {
                plan,
                last: self.last.clone(),
                return_va: self.return_va,
                active: self.active.as_ref().map(|active| Active {
                    saved: active.saved.clone(),
                    frame_va: active.frame_va,
                    frame_words: active.frame_words,
                    started: Instant::now(),
                    elapsed_ms: active.elapsed_ms,
                }),
            }),
            idle: Condvar::new(),
        })
    }
}
impl GuestCalls {
    /// Install only into an unused stopped world's queue. All decoding and
    /// source/channel validation happens before replacing its native state.
    pub(crate) fn install_image(
        &self,
        image: &GuestCallsImage,
        mem: &Arc<GuestMemory>,
        identity: &str,
        binding: Option<OutputBinding<'_>>,
        text_calls: Arc<super::text::Mailbox>,
    ) -> Result<(), String> {
        let mut cpu = Unicorn::new(
            unicorn_engine::unicorn_const::Arch::ARM,
            unicorn_engine::unicorn_const::Mode::ARM,
        )
        .map_err(|e| e.to_string())?;
        cpu.ctl_set_cpu_model(unicorn_engine::ArmCpuModel::Model_926 as i32)
            .map_err(|e| e.to_string())?;
        let mut target = self
            .state
            .lock()
            .map_err(|_| "Guest call queue is poisoned")?;
        if target.plan.is_some()
            || target.active.is_some()
            || target.last.is_some()
            || target.return_va != 0
        {
            return Err("Guest call image installation requires an unused stopped queue".into());
        }
        let restored = image.restore_inner(
            mem,
            identity,
            &cpu,
            binding,
            Some((mem.clone(), text_calls)),
        )?;
        *target = restored
            .state
            .into_inner()
            .map_err(|_| "Restored call queue is poisoned")?;
        Ok(())
    }

    /// A fresh matching backend admits the saved CPU schema without executing
    /// code or registering an additional guest owner in the paused machine.
    pub fn image_while_guarded(
        &self,
        mem: &GuestMemory,
        identity: &str,
        gate: &crate::memory::snapshot::Snapshot<'_>,
    ) -> Result<GuestCallsImage, String> {
        if !gate.guards(&mem.snapshot_gate) || mem.snapshot_gate.counts() != (0, 0) {
            return Err("Guest call capture requires the matching quiescent memory guard".into());
        }
        let mut cpu = Unicorn::new(
            unicorn_engine::unicorn_const::Arch::ARM,
            unicorn_engine::unicorn_const::Mode::ARM,
        )
        .map_err(|e| e.to_string())?;
        cpu.ctl_set_cpu_model(unicorn_engine::ArmCpuModel::Model_926 as i32)
            .map_err(|e| e.to_string())?;
        self.image_under_gate(mem, identity, &cpu)
    }
    pub fn image(
        &self,
        mem: &GuestMemory,
        identity: &str,
        cpu: &Unicorn<'_, ()>,
    ) -> Result<GuestCallsImage, String> {
        let _gate = mem
            .snapshot_gate
            .try_snapshot()
            .ok_or("Guest call capture has active engines")?;
        if mem.snapshot_gate.counts() != (0, 0) {
            return Err("Guest call capture has native continuations".into());
        }
        self.image_under_gate(mem, identity, cpu)
    }
    pub(crate) fn image_under_gate(
        &self,
        mem: &GuestMemory,
        identity: &str,
        cpu: &Unicorn<'_, ()>,
    ) -> Result<GuestCallsImage, String> {
        let state = self
            .state
            .try_lock()
            .map_err(|_| "Guest call queue is busy or poisoned")?;
        let mut image = GuestCallsImage {
            version: 1,
            resource_identity: identity.into(),
            plan: state
                .plan
                .as_ref()
                .map(|plan| match plan {
                    Plan::Compiler(plan) => plan.image(mem).map(PlanImage::Compiler),
                    Plan::Text(plan) => plan
                        .image()
                        .map(|desktop_text| PlanImage::Text(TextImage { desktop_text })),
                })
                .transpose()?,
            active: state.active.as_ref().map(|active| ActiveImage {
                saved: active.saved.clone(),
                frame_va: active.frame_va,
                frame_words: active.frame_words,
                elapsed_ms: active.elapsed_ms + active.started.elapsed().as_secs_f64() * 1000.0,
            }),
            last: state.last.clone(),
            return_va: state.return_va,
            sha256: String::new(),
        };
        image.sha256 = image.digest()?;
        image.validate(mem, identity, cpu)?;
        Ok(image)
    }
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
