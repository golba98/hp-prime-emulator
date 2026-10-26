//! Scripted key input with guest-observed synchronization and framebuffer
//! evidence (M4).
//!
//! The run loop calls [`ScriptDriver::tick`] between execution slices. The
//! driver never writes guest memory: it queues key presses on the same
//! host input queue `--inject-key` uses (GetEvent delivers them to the
//! firmware's own pump) and reads the framebuffer to observe the result.
//!
//! A key is "settled" when the guest has taken it (GetEvent delivered the
//! batch), the UI pump is parked again in GetEvent with nothing queued,
//! and the screen has not changed for `settle_ms`. Changes seen while no
//! input is pending (a blinking cursor, a clock) form an idle mask that is
//! excluded when attributing screen changes to a key.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::display::{Frame, FrameDiff, LcdView};
use crate::keys::Key;
use crate::memory::GuestMemory;
use crate::services::SharedState;

#[derive(Debug, Clone)]
pub struct ScriptConfig {
    pub keys: Vec<Key>,
    /// Screen-unchanged period that counts as settled.
    pub settle_ms: u64,
    /// Idle observation after boot (builds the idle-change mask).
    pub idle_observe_ms: u64,
    pub boot_timeout: Duration,
    pub key_timeout: Duration,
    /// `initial.png` and `NN_<key>.png` are written here when set.
    pub screenshot_dir: Option<PathBuf>,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        ScriptConfig {
            keys: Vec::new(),
            settle_ms: 300,
            idle_observe_ms: 1500,
            boot_timeout: Duration::from_secs(60),
            key_timeout: Duration::from_secs(20),
            screenshot_dir: None,
        }
    }
}

/// One framebuffer capture.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureRecord {
    pub checkpoint: String,
    pub elapsed_ms: u64,
    pub sha256: String,
    pub unique_colors: usize,
    /// Versus the previous capture (none for the first).
    pub changed_vs_previous: Option<FrameDiff>,
    pub png: Option<String>,
}

/// Evidence for one scripted key.
#[derive(Debug, Clone, Serialize)]
pub struct KeyRecord {
    pub index: usize,
    pub name: String,
    pub code: u16,
    pub queued_ms: u64,
    /// When GetEvent handed the batch to the guest.
    pub delivered_ms: Option<u64>,
    /// GetEvent delivery count after this key (batch sequence number).
    pub delivery_seq: u32,
    /// Thread that received it (the UI pump).
    pub delivered_to_thread: u32,
    /// Queue → first screen change outside the idle mask.
    pub first_change_latency_ms: Option<u64>,
    pub settled_ms: Option<u64>,
    /// Screen change (outside the idle mask) between before and after.
    pub change: FrameDiff,
    /// The same, also ignoring pixels that alternate between recurring
    /// screen states after the key (blinking cursor): the key's lasting
    /// effect.
    pub stable_change: FrameDiff,
    pub flicker_pixels: usize,
    /// Distinct screen states seen while the key was processed.
    pub states_seen: usize,
    /// Framebuffer stores while this key was processed (`--lcd-watch`).
    pub fb_writes: Option<u64>,
    /// Guest PCs that stored to the framebuffer, by count (`--lcd-watch`).
    pub fb_writer_pcs: Vec<(u32, u64)>,
    pub after: CaptureRecord,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScriptReport {
    pub status: String,
    pub display: Option<LcdView>,
    /// Pixels that changed with no input pending (excluded from key
    /// attribution).
    pub idle_mask_pixels: usize,
    pub boot_settled_ms: Option<u64>,
    pub captures: Vec<CaptureRecord>,
    pub keys: Vec<KeyRecord>,
}

enum Phase {
    Boot,
    IdleObserve { until: Instant },
    KeyStart { index: usize },
    KeyWait(Box<Pending>),
    Done,
    Failed(String),
}

struct Pending {
    index: usize,
    before: Frame,
    delivered_before: u32,
    /// Touch samples still to send, and touch deliveries to wait for.
    touch_left: std::collections::VecDeque<crate::services::TouchSample>,
    touch_target: u64,
    last_touch_push: Option<Instant>,
    writes_before: u64,
    queued_at: Instant,
    delivered_at: Option<Instant>,
    first_change: Option<Instant>,
}

/// Result of one tick.
#[derive(Debug, PartialEq, Eq)]
pub enum Tick {
    Continue,
    Done,
    Failed(String),
}

pub struct ScriptDriver {
    cfg: ScriptConfig,
    phase: Phase,
    started: Instant,
    view: Option<LcdView>,
    last: Option<Frame>,
    last_sample_at: Option<Instant>,
    last_change: Instant,
    idle_mask: Vec<bool>,
    /// Distinct screen states seen since the last reset (boot, or the
    /// current key): fingerprint, frame, times seen.
    states: Vec<(u64, Frame, u32)>,
    /// When a never-seen state last appeared. A screen whose only changes
    /// cycle through known states (a blinking cursor) has settled.
    last_new_state: Instant,
    /// Since when the UI pump has been continuously parked with empty
    /// input queues.
    idle_since: Option<Instant>,
    /// Recent `(time, main thread waiting)` observations: the main thread
    /// runs a timed tick loop, so its rest is measured as a duty cycle.
    main_samples: std::collections::VecDeque<(Instant, bool)>,
    /// Frame of the previous recorded capture.
    prev_capture: Option<Frame>,
    report: ScriptReport,
}

const SAMPLE_INTERVAL: Duration = Duration::from_millis(20);
const TOUCH_INTERVAL: Duration = Duration::from_millis(16);
/// Share of the settle window the main thread must spend in its event
/// wait for the UI to count as at rest.
const MAIN_WAIT_SHARE: f64 = 0.8;
/// Distinct states remembered per step (bounded memory).
const MAX_STATES: usize = 32;
/// Minimum time a touch step holds its input before the next step. See
/// [`ScriptDriver::tick`] — the firmware's tap-commit debounce.
const TOUCH_COMMIT_HOLD: Duration = Duration::from_millis(600);

impl ScriptDriver {
    pub fn new(cfg: ScriptConfig) -> ScriptDriver {
        let now = Instant::now();
        ScriptDriver {
            cfg,
            phase: Phase::Boot,
            started: now,
            view: None,
            last: None,
            last_sample_at: None,
            last_change: now,
            idle_mask: Vec::new(),
            states: Vec::new(),
            last_new_state: now,
            idle_since: None,
            main_samples: std::collections::VecDeque::new(),
            prev_capture: None,
            report: ScriptReport {
                status: "running".into(),
                display: None,
                idle_mask_pixels: 0,
                boot_settled_ms: None,
                captures: Vec::new(),
                keys: Vec::new(),
            },
        }
    }

    /// Index of the script item being processed (None during boot and
    /// after the script ends).
    pub fn current_step(&self) -> Option<usize> {
        match &self.phase {
            Phase::KeyStart { index } => Some(*index),
            Phase::KeyWait(p) => Some(p.index),
            _ => None,
        }
    }

    pub fn report(&self) -> &ScriptReport {
        &self.report
    }

    /// The most recent captured frame.
    pub fn last_frame(&self) -> Option<&Frame> {
        self.last.as_ref()
    }

    fn ms(&self, t: Instant) -> u64 {
        t.duration_since(self.started).as_millis() as u64
    }

    /// Samples the screen (rate-limited); tracks the last change outside
    /// the idle mask. During idle observation every change feeds the mask.
    fn sample(&mut self, mem: &GuestMemory, now: Instant, build_mask: bool) {
        if self
            .last_sample_at
            .is_some_and(|t| now.duration_since(t) < SAMPLE_INTERVAL)
        {
            return;
        }
        let Some(view) = self.view else { return };
        let Ok(frame) = view.capture(mem) else { return };
        self.last_sample_at = Some(now);
        if let Some(prev) = &self.last {
            if build_mask {
                frame.accumulate_changes(prev, &mut self.idle_mask);
            }
            let mask = (!build_mask && !self.idle_mask.is_empty()).then_some(&self.idle_mask[..]);
            if frame.diff(prev, mask).changed > 0 {
                self.last_change = now;
            }
        }
        let fp = frame.fingerprint();
        let same_as_last = self.last.as_ref().is_some_and(|l| l.fingerprint() == fp);
        match self.states.iter_mut().find(|(f, _, _)| *f == fp) {
            // A return to a known state after a different one (A→B→A) is a
            // recurrence; the same frame sampled twice in a row is not.
            Some(st) => {
                if !same_as_last {
                    st.2 += 1;
                }
            }
            None => {
                self.last_new_state = now;
                if self.states.len() < MAX_STATES {
                    self.states.push((fp, frame.clone(), 1));
                }
            }
        }
        self.last = Some(frame);
    }

    fn reset_states(&mut self, now: Instant) {
        self.states.clear();
        self.last_new_state = now;
    }

    /// Pixels that alternate between recurring states (returned to after a
    /// different state: blinking cursors, not one-off drawing or animation
    /// frames) relative to `current`.
    fn flicker_mask(&self, current: &Frame) -> Vec<bool> {
        let mut mask = vec![false; current.pixels.len()];
        for (_, f, seen) in &self.states {
            if *seen >= 2 {
                f.accumulate_changes(current, &mut mask);
            }
        }
        mask
    }

    /// The UI pump is parked in GetEvent with no input queued. (The main
    /// thread may still be handling what the pump forwarded to it; see
    /// [`Self::observe_idle`].)
    fn pump_idle(shared: &SharedState) -> bool {
        shared.getevent_parked()
            && shared.input_events_pending() == 0
            && shared.touch_pending() == 0
    }

    /// Records a capture of the current frame (and writes its PNG).
    fn capture(
        &mut self,
        mem: &GuestMemory,
        checkpoint: String,
        file: Option<String>,
    ) -> CaptureRecord {
        let now = Instant::now();
        if let Some(view) = self.view {
            if let Ok(f) = view.capture(mem) {
                self.last = Some(f);
                self.last_sample_at = Some(now);
            }
        }
        let frame = self.last.clone().expect("capture after the display exists");
        let changed_vs_previous = self.prev_capture.as_ref().map(|p| frame.diff(p, None));
        let png = match (&self.cfg.screenshot_dir, file) {
            (Some(dir), Some(name)) => {
                let path = dir.join(name);
                frame
                    .save_png(&path)
                    .ok()
                    .map(|_| path.display().to_string())
            }
            _ => None,
        };
        let rec = CaptureRecord {
            checkpoint,
            elapsed_ms: self.ms(now),
            sha256: frame.sha256(),
            unique_colors: frame.unique_colors(),
            changed_vs_previous,
            png,
        };
        self.prev_capture = Some(frame);
        self.report.captures.push(rec.clone());
        rec
    }

    /// How long the guest UI has been at rest: the pump parked with nothing
    /// queued throughout, and the main thread waiting at least
    /// [`MAIN_WAIT_SHARE`] of the time (and right now). Zero otherwise.
    fn observe_idle(&mut self, shared: &SharedState, now: Instant) -> Duration {
        let window = Duration::from_millis(self.cfg.settle_ms);
        let main = shared.main_waiting();
        self.main_samples.push_back((now, main));
        while self
            .main_samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > window)
        {
            self.main_samples.pop_front();
        }
        if !Self::pump_idle(shared) {
            self.idle_since = None;
            return Duration::ZERO;
        }
        let parked_for = now.duration_since(*self.idle_since.get_or_insert(now));
        let waiting = self.main_samples.iter().filter(|(_, w)| *w).count();
        let share = waiting as f64 / self.main_samples.len().max(1) as f64;
        if main && share >= MAIN_WAIT_SHARE {
            parked_for
        } else {
            Duration::ZERO
        }
    }

    pub fn tick(&mut self, mem: &GuestMemory, shared: &SharedState) -> Tick {
        let now = Instant::now();
        let idle_for = self.observe_idle(shared, now);
        if self.view.is_none() {
            let lcd = *shared.lcd_object.lock().unwrap();
            if let Ok(v) = LcdView::locate(mem, lcd) {
                self.view = Some(v);
                self.report.display = Some(v);
                self.idle_mask = vec![false; (v.width * v.height) as usize];
            }
        }
        let phase = std::mem::replace(&mut self.phase, Phase::Done);
        self.phase = match phase {
            Phase::Boot => {
                self.sample(mem, now, false);
                if now.duration_since(self.started) > self.cfg.boot_timeout {
                    Phase::Failed(format!(
                        "boot did not reach a stable screen within {:?} (display {}, pump idle {})",
                        self.cfg.boot_timeout,
                        if self.view.is_some() {
                            "located"
                        } else {
                            "not created"
                        },
                        Self::pump_idle(shared)
                    ))
                } else if self.view.is_some()
                    && idle_for >= Duration::from_millis(self.cfg.settle_ms)
                    && now.duration_since(self.last_new_state)
                        >= Duration::from_millis(self.cfg.settle_ms)
                {
                    self.report.boot_settled_ms = Some(self.ms(now));
                    Phase::IdleObserve {
                        until: now + Duration::from_millis(self.cfg.idle_observe_ms),
                    }
                } else {
                    Phase::Boot
                }
            }
            Phase::IdleObserve { until } => {
                self.sample(mem, now, true);
                if now < until {
                    Phase::IdleObserve { until }
                } else {
                    self.report.idle_mask_pixels = self.idle_mask.iter().filter(|&&m| m).count();
                    self.capture(mem, "initial".into(), Some("initial.png".into()));
                    self.last_change = now;
                    Phase::KeyStart { index: 0 }
                }
            }
            Phase::KeyStart { index } => {
                if index >= self.cfg.keys.len() {
                    self.report.status = "complete".into();
                    Phase::Done
                } else {
                    let before = match self.view.map(|v| v.capture(mem)) {
                        Some(Ok(f)) => f,
                        Some(Err(crate::display::DisplayError::Busy)) => {
                            self.phase = Phase::KeyStart { index };
                            return Tick::Continue;
                        }
                        _ => return self.fail("framebuffer unreadable before key".into()),
                    };
                    shared.lcd_watch.reset_pcs();
                    self.reset_states(now);
                    let key = &self.cfg.keys[index];
                    let touch_left: std::collections::VecDeque<_> = key
                        .touch
                        .iter()
                        .map(|&(status, x, y)| crate::services::TouchSample { status, x, y })
                        .collect();
                    let pending = Pending {
                        index,
                        before,
                        delivered_before: shared.input_delivered(),
                        touch_target: shared.touch_delivered() + touch_left.len() as u64,
                        touch_left,
                        last_touch_push: None,
                        writes_before: shared.lcd_watch.writes(),
                        queued_at: now,
                        delivered_at: None,
                        first_change: None,
                    };
                    let is_input = key.code != 0 || !key.touch.is_empty();
                    if key.touch.is_empty() && is_input {
                        shared.push_key_press(key.code);
                    }
                    Phase::KeyWait(Box::new(pending))
                }
            }
            Phase::KeyWait(mut p) => {
                self.sample(mem, now, false);
                // Touch: one sample per pump round, ~60 Hz like a panel.
                if !p.touch_left.is_empty()
                    && shared.touch_pending() == 0
                    && p.last_touch_push
                        .is_none_or(|t| now.duration_since(t) >= TOUCH_INTERVAL)
                {
                    shared.push_touch(p.touch_left.pop_front().unwrap());
                    p.last_touch_push = Some(now);
                }
                let item = &self.cfg.keys[p.index];
                let delivered = if item.code == 0 && item.touch.is_empty() {
                    true // IDLE: nothing to deliver
                } else if item.touch.is_empty() {
                    shared.input_delivered() > p.delivered_before
                } else {
                    p.touch_left.is_empty() && shared.touch_delivered() >= p.touch_target
                };
                if p.delivered_at.is_none() && delivered {
                    p.delivered_at = Some(now);
                }
                if p.first_change.is_none() {
                    if let Some(f) = &self.last {
                        let mask = (!self.idle_mask.is_empty()).then_some(&self.idle_mask[..]);
                        if f.diff(&p.before, mask).changed > 0 {
                            p.first_change = Some(now);
                        }
                    }
                }
                // Touch floor: the firmware commits a tap's UI effect
                // (M4: first-run wizard page transitions) on an internal
                // ~500 ms debounce that touches neither the event pump nor
                // the framebuffer — screen stability alone can settle a
                // step while the commit is still pending, and the NEXT tap
                // is then swallowed (observed: taps ~450 ms apart fail,
                // >= 550 ms pass; the previous agent's 6/6 baseline sat
                // ~50 ms above the threshold). Touch steps therefore hold
                // their input for at least TOUCH_COMMIT_HOLD in addition
                // to the guest-observed settlement below.
                let settle = if self.cfg.keys[p.index].touch.is_empty() {
                    Duration::from_millis(self.cfg.settle_ms)
                } else {
                    Duration::from_millis(self.cfg.settle_ms).max(TOUCH_COMMIT_HOLD)
                };
                let settled = p.delivered_at.is_some_and(|d| {
                    idle_for >= settle
                        && now.duration_since(d) >= settle
                        && now.duration_since(self.last_new_state) >= settle
                });
                if settled {
                    self.finish_key(mem, shared, *p, now)
                } else if now.duration_since(p.queued_at) > self.cfg.key_timeout {
                    let key = &self.cfg.keys[p.index];
                    Phase::Failed(format!(
                        "key {} ({}, {:#06X}) not settled within {:?}: delivered {}, pump idle {}",
                        p.index,
                        key.name,
                        key.code,
                        self.cfg.key_timeout,
                        p.delivered_at.is_some(),
                        Self::pump_idle(shared)
                    ))
                } else {
                    Phase::KeyWait(p)
                }
            }
            Phase::Done => Phase::Done,
            Phase::Failed(m) => Phase::Failed(m),
        };
        match &self.phase {
            Phase::Done => Tick::Done,
            Phase::Failed(m) => {
                self.report.status = format!("failed: {m}");
                Tick::Failed(m.clone())
            }
            _ => Tick::Continue,
        }
    }

    fn fail(&mut self, m: String) -> Tick {
        self.report.status = format!("failed: {m}");
        self.phase = Phase::Failed(m.clone());
        Tick::Failed(m)
    }

    fn finish_key(
        &mut self,
        mem: &GuestMemory,
        shared: &SharedState,
        p: Pending,
        now: Instant,
    ) -> Phase {
        let key = self.cfg.keys[p.index].clone();
        let file = format!(
            "{:02}_{}.png",
            p.index + 1,
            key.name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        );
        let after = self.capture(
            mem,
            format!("after key {} ({})", p.index, key.name),
            Some(file),
        );
        let frame = self.last.clone().expect("captured");
        let mask = (!self.idle_mask.is_empty()).then_some(&self.idle_mask[..]);
        let change = frame.diff(&p.before, mask);
        let mut flicker = self.flicker_mask(&frame);
        let flicker_pixels = flicker.iter().filter(|&&m| m).count();
        for (f, &i) in flicker.iter_mut().zip(&self.idle_mask) {
            *f |= i;
        }
        let stable_change = frame.diff(&p.before, Some(&flicker));
        let watching = shared
            .lcd_watch
            .enabled
            .load(std::sync::atomic::Ordering::Relaxed);
        self.report.keys.push(KeyRecord {
            index: p.index,
            name: key.name.clone(),
            code: key.code,
            queued_ms: self.ms(p.queued_at),
            delivered_ms: p.delivered_at.map(|t| self.ms(t)),
            delivery_seq: shared.input_delivered(),
            delivered_to_thread: shared.input_thread(),
            first_change_latency_ms: p
                .first_change
                .map(|t| t.duration_since(p.queued_at).as_millis() as u64),
            settled_ms: Some(self.ms(now)),
            change,
            stable_change,
            flicker_pixels,
            states_seen: self.states.len(),
            fb_writes: watching.then(|| shared.lcd_watch.writes() - p.writes_before),
            fb_writer_pcs: if watching {
                shared.lcd_watch.top_pcs(8)
            } else {
                Vec::new()
            },
            after,
        });
        Phase::KeyStart { index: p.index + 1 }
    }
}
