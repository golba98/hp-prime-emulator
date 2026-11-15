//! RGB indicator-pin controller and RTC-driven patterns, OS 0x3001A438.
//! Pin order is retained as the OS's low three bits; no physical LED wiring
//! or host device is guessed. Pattern waits retain data outside native hooks.
use super::{HandlerFault, SvcCtx};
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Indicator {
    pub raw: u32,
    pub active: Option<Pattern>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    pub owner: u32,
    pub record: u32,
    pub colors: Vec<u8>,
    pub period: u32,
    pub limit: u32,
    pub start: u32,
    pub saved: u32,
}
impl Pattern {
    pub fn sample(&self, now: u32) -> Option<(u32, u32, bool)> {
        if now < self.start {
            return None;
        }
        // Original unsigned division by zero returns quotient zero.
        let step = now
            .wrapping_sub(self.start)
            .checked_div(self.period)
            .unwrap_or(0);
        let color = u32::from(self.colors[(step % self.colors.len() as u32) as usize] & 7);
        Some((step, color, self.limit != u32::MAX && step >= self.limit))
    }
}
pub fn ioctl(
    ctx: &mut SvcCtx,
    request: u32,
    input: u32,
    size: usize,
    output: u32,
    output_size: usize,
) -> Result<u32, HandlerFault> {
    match request {
        256 if output_size == 4 => {
            ctx.mem
                .write_u32(output, ctx.shared.indicator.lock().unwrap().raw)
                .map_err(HandlerFault)?;
            Ok(0)
        }
        257 if size == 4 => {
            let raw = ctx.mem.read_u32(input).map_err(HandlerFault)?;
            ctx.shared.indicator.lock().unwrap().raw = raw;
            Ok(0)
        }
        258 => Ok(if ctx.shared.indicator.lock().unwrap().active.is_some() {
            0
        } else {
            u32::MAX
        }),
        259 => {
            let owner = ctx
                .shared
                .indicator
                .lock()
                .unwrap()
                .active
                .as_ref()
                .map(|pattern| pattern.owner);
            let Some(owner) = owner else {
                return Ok(u32::MAX);
            };
            poll(ctx.mem, ctx.shared, owner).map_err(HandlerFault)?;
            Ok(0)
        }
        260 if input != 0 && size == 40 => {
            ctx.mem.validate_range(input, 40).map_err(HandlerFault)?;
            let pointer = ctx.mem.read_u32(input).map_err(HandlerFault)?;
            let count = ctx.mem.read_u32(input + 4).map_err(HandlerFault)?;
            let period = ctx.mem.read_u32(input + 8).map_err(HandlerFault)?;
            let limit = ctx.mem.read_u32(input + 12).map_err(HandlerFault)?;
            let start = ctx.mem.read_u32(input + 16).map_err(HandlerFault)?;
            if pointer == 0 || count == 0 || period >= 60 {
                return Ok(u32::MAX);
            }
            if limit == 0 || start < ctx.shared.rtc.seconds().wrapping_add(2) {
                // The original function never dereferences the color array in
                // these inactive cases, even if its nonzero pointer is invalid.
                ctx.shared.indicator.lock().unwrap().active = None;
                return Ok(0);
            }
            let colors = ctx
                .mem
                .read_bytes_alloc(pointer, count as usize)
                .map_err(HandlerFault)?;
            let mut indicator = ctx.shared.indicator.lock().unwrap();
            if indicator.active.is_some() {
                return Ok(u32::MAX);
            }
            let saved = indicator.raw;
            indicator.active = Some(Pattern {
                owner: ctx.thread_id,
                record: input,
                colors,
                period,
                limit,
                start,
                saved,
            });
            drop(indicator);
            ctx.shared.enter_indicator_sleep();
            ctx.deferred = Some(super::pending::PendingService::new(
                ctx.args,
                super::pending::Wait::Indicator { record: input },
                ctx.thread_id,
            ));
            Ok(0)
        }
        _ => Ok(u32::MAX),
    }
}
/// CPU-owner polling is admitted during an indicator sleep. The RTC keeps
/// ticking; normal CPU deadlines remain frozen until completion or the ON key.
pub fn poll(
    mem: &crate::memory::GuestMemory,
    shared: &super::SharedState,
    owner: u32,
) -> crate::memory::MemResult<Option<u32>> {
    let asleep = shared.sleeping();
    let stopping = shared.stopping() || shared.guest_thread_terminated(owner);
    let mut indicator = shared.indicator.lock().unwrap();
    let Some(pattern) = indicator.active.as_ref() else {
        return Ok(Some(0));
    };
    if pattern.owner != owner {
        return Err(crate::memory::MemoryError {
            va: pattern.record,
            msg: "indicator wait belongs to a different CPU owner".into(),
        });
    }
    if pattern.colors.is_empty() || pattern.period >= 60 {
        return Err(crate::memory::MemoryError {
            va: pattern.record,
            msg: "invalid reconstructed indicator pattern".into(),
        });
    }
    let mut done = stopping || !asleep;
    if !done {
        if let Some((step, color, finished)) = pattern.sample(shared.rtc.seconds()) {
            mem.write_u32(pattern.record + 20, step)?;
            done = finished;
            indicator.raw = color;
        }
    }
    if done {
        let pattern = indicator.active.take().unwrap();
        indicator.raw = pattern.saved;
        drop(indicator);
        shared.finish_indicator_sleep();
        Ok(Some(0))
    } else {
        Ok(None)
    }
}
