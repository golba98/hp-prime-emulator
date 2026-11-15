//! Host-requested guest function calls, made on the UI thread (guest
//! thread 0) where its main loop waits for the next event, in
//! `OSWaitForEvent` called from the `wait_event` shim.
//!
//! The UI thread is then quiescent: it is between two events, which is where
//! the firmware's own event handlers (such as the Program Editor's Check, on
//! this same thread) run too. A call saves every register, enters the target
//! with a return address that traps back to the host, and restores the saved
//! context with the PC on the wait's SVC, so the guest then simply waits
//! again. Guest code does all the work; the host only supplies arguments and
//! reads results.
//!
//! The typed [`crate::direct::Plan`] chains calls: each call's arguments can depend on earlier
//! results, and the next call starts as soon as the previous one returns.
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use unicorn_engine::{RegisterARM, Unicorn};

use crate::{cpu_state::ArmCpuImage, direct::Plan as CompilerPlan, memory::GuestMemory};

pub mod image;
pub(crate) mod text;

/// The guest thread that runs the firmware UI (observed for every Check).
pub const UI_THREAD: u32 = 0;
/// Return address inside the `wait_event` shim (`0x30693404`) after its
/// `OSWaitForEvent` call: the main loop's wait for the next event.
pub const MAIN_WAIT_CALLER: u32 = 0x3069_341C;
/// Private SVC number of the return trap, outside the firmware's service
/// range (`0x10000` + export ordinal).
pub const RETURN_SVC: u32 = 0xF_FFF0;
/// Stack bytes reserved below the waiting frame for call data.
const FRAME_BYTES: u32 = 256;

/// One argument register.
#[derive(Clone, Copy, Debug)]
pub enum Arg {
    Value(u32),
    /// The guest address of this byte offset in the call's stack frame.
    Frame(u32),
}

/// One ARM-state guest function call with up to four register arguments.
#[derive(Clone, Debug)]
pub struct Call {
    pub target: u32,
    pub args: [Arg; 4],
    /// Words placed in the call's stack frame (result records, out slots).
    pub frame: Vec<u32>,
}

impl Call {
    pub fn new(target: u32, args: [Arg; 4]) -> Call {
        Call {
            target,
            args,
            frame: Vec::new(),
        }
    }
    pub fn with_frame(mut self, frame: Vec<u32>) -> Call {
        self.frame = frame;
        self
    }
}

/// What a call returned.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Returned {
    pub r0: u32,
    /// The frame words as the callee left them.
    pub frame: Vec<u32>,
    pub frame_va: u32,
    /// Host time from entering the target to the return trap.
    pub elapsed_ms: f64,
}

pub enum Step {
    Call(Call),
    Done,
}

struct Active {
    saved: ArmCpuImage,
    frame_va: u32,
    frame_words: usize,
    started: Instant,
    elapsed_ms: f64,
}

enum Plan {
    Compiler(CompilerPlan),
    Text(text::Plan),
}
impl Plan {
    fn next(&mut self, mem: &GuestMemory, last: Option<Returned>) -> Step {
        match self {
            Self::Compiler(plan) => plan.next(mem, last),
            Self::Text(plan) => plan.next(last),
        }
    }
}
#[derive(Default)]
struct State {
    plan: Option<Plan>,
    active: Option<Active>,
    last: Option<Returned>,
    /// Address of the return trap (`svc #RETURN_SVC`) in guest heap memory.
    return_va: u32,
}

#[derive(Default)]
pub struct GuestCalls {
    state: Mutex<State>,
    idle: Condvar,
}

impl GuestCalls {
    /// Queues a plan; refused while another plan is pending or running.
    pub fn submit(&self, plan: CompilerPlan) -> Result<(), String> {
        let mut s = self.state.lock().unwrap();
        if s.plan.is_some() || s.active.is_some() {
            return Err("a guest call plan is already running".into());
        }
        s.plan = Some(Plan::Compiler(plan));
        s.last = None;
        Ok(())
    }

    pub(crate) fn submit_text(&self, plan: text::Plan) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.plan.is_some() || state.active.is_some() {
            return Err("A guest call is already active".into());
        }
        state.plan = Some(Plan::Text(plan));
        state.last = None;
        Ok(())
    }

    /// A plan is queued or a call is in progress.
    pub fn inspected_busy(&self) -> Option<bool> {
        self.state
            .try_lock()
            .ok()
            .map(|s| s.plan.is_some() || s.active.is_some())
    }
    pub fn busy(&self) -> bool {
        let s = self.state.lock().unwrap();
        s.plan.is_some() || s.active.is_some()
    }

    /// The return address calls use, allocating the trap on first use.
    pub fn prepare(&self, mem: &GuestMemory) -> Result<u32, String> {
        let mut s = self.state.lock().unwrap();
        if s.return_va == 0 {
            let va = mem.heap_alloc(8).map_err(|e| e.to_string())?;
            // ARM `svc #RETURN_SVC` (always), then a guard copy.
            for at in [va, va + 4] {
                mem.write_u32(at, 0xEF00_0000 | RETURN_SVC)
                    .map_err(|e| e.to_string())?;
            }
            s.return_va = va;
        }
        Ok(s.return_va)
    }

    /// A plan is waiting to start (or continue) on the UI thread.
    pub fn pending(&self) -> bool {
        let s = self.state.lock().unwrap();
        s.plan.is_some() && s.active.is_none()
    }

    /// Called from the UI thread's main-loop wait. Starts the plan's next call
    /// and returns true when the registers were redirected into it; the
    /// SVC must then not apply its normal return.
    pub fn inject(&self, uc: &mut Unicorn<'_, ()>, mem: &GuestMemory) -> Result<bool, String> {
        let mut s = self.state.lock().unwrap();
        if s.active.is_some() {
            return Ok(false);
        }
        let last = s.last.take();
        let Some(plan) = s.plan.as_mut() else {
            return Ok(false);
        };
        let call = match plan.next(mem, last) {
            Step::Call(c) => c,
            Step::Done => {
                s.plan = None;
                self.idle.notify_all();
                return Ok(false);
            }
        };
        if call.frame.len() * 4 + 16 > FRAME_BYTES as usize
            || call.target & 3 != 0
            || call.args.iter().any(|arg| {
                matches!(arg, Arg::Frame(offset)
                if *offset % 4 != 0 || (*offset as usize) >= call.frame.len() * 4)
            })
        {
            s.plan = None;
            return Err("unsupported guest call shape".into());
        }
        if s.return_va == 0 {
            s.plan = None;
            return Err("guest calls were not prepared".into());
        }
        mem.validate_range(call.target, 4)
            .map_err(|e| e.to_string())?;
        let mut saved = ArmCpuImage::capture(uc)?;
        // Resume on the SVC itself, so the wait is entered again unchanged.
        saved.gprs[15] = saved.gprs[15]
            .checked_sub(4)
            .ok_or("Guest wait PC underflow")?;
        let cpsr = saved.cpsr;
        if cpsr & 0x20 != 0 {
            s.plan = None;
            return Err("the wait trapped in Thumb state".into());
        }
        let sp = saved.gprs[13];
        let frame_va = sp
            .checked_sub(FRAME_BYTES)
            .ok_or("Guest call stack underflow")?
            & !7;
        mem.validate_range(frame_va, FRAME_BYTES as usize)
            .map_err(|e| e.to_string())?;
        for (i, w) in call.frame.iter().enumerate() {
            mem.write_u32(frame_va + 4 * i as u32, *w)
                .map_err(|e| e.to_string())?;
        }
        let write = |uc: &mut Unicorn<'_, ()>, r, v: u32| {
            uc.reg_write(r, v as u64).map_err(|e| e.to_string())
        };
        for (r, a) in [
            RegisterARM::R0,
            RegisterARM::R1,
            RegisterARM::R2,
            RegisterARM::R3,
        ]
        .into_iter()
        .zip(call.args)
        {
            let v = match a {
                Arg::Value(v) => v,
                Arg::Frame(off) => frame_va + off,
            };
            write(uc, r, v)?;
        }
        write(uc, RegisterARM::SP, frame_va)?;
        write(uc, RegisterARM::LR, s.return_va)?;
        write(uc, RegisterARM::PC, call.target)?;
        s.active = Some(Active {
            saved,
            frame_va,
            frame_words: call.frame.len(),
            started: Instant::now(),
            elapsed_ms: 0.0,
        });
        Ok(true)
    }

    /// The return trap: records the result and restores the UI thread's
    /// saved context. Returns false when no call is active (a stray trap).
    pub fn complete(&self, uc: &mut Unicorn<'_, ()>, mem: &GuestMemory) -> Result<bool, String> {
        let mut s = self.state.lock().unwrap();
        let Some(active) = &s.active else {
            return Ok(false);
        };
        let r0 = uc.reg_read(RegisterARM::R0).map_err(|e| e.to_string())? as u32;
        let mut frame = Vec::with_capacity(active.frame_words);
        for i in 0..active.frame_words {
            frame.push(
                mem.read_u32(active.frame_va + 4 * i as u32)
                    .map_err(|e| e.to_string())?,
            );
        }
        active.saved.restore(uc)?;
        let returned = Returned {
            r0,
            frame,
            frame_va: active.frame_va,
            elapsed_ms: active.elapsed_ms + active.started.elapsed().as_secs_f64() * 1000.0,
        };
        s.active = None;
        s.last = Some(returned);
        Ok(true)
    }

    /// Drops a plan that has not started a call (used on cancellation).
    pub fn cancel_pending(&self) {
        let mut s = self.state.lock().unwrap();
        if s.active.is_none() {
            s.plan = None;
            s.last = None;
            self.idle.notify_all();
        }
    }
}
