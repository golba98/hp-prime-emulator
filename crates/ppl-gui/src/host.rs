//! Thread-safe GUI Host implementing `ppl_vm::Host`.
//!
//! Bridges the background VM execution thread and the main window event loop.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ppl_graphics::Grob;
use ppl_graphics::text::render_terminal;
use ppl_vm::error::{ErrorKind, RuntimeError};
use ppl_vm::host::{Dialog, DialogResult, Host, Touch};

pub use crate::dialog::KeyPress;
use crate::dialog::make_dialog;

pub type RedrawCallback = Box<dyn Fn() + Send + Sync + 'static>;

/// Upper bound for a single condvar wait. Every blocking wait re-checks the
/// interrupt flag at least this often, so a missed notification can delay a
/// cancel by at most this long instead of hanging forever.
const WAIT_SLICE: Duration = Duration::from_millis(100);

/// Locks a mutex, recovering the data if another thread panicked with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shared state between the VM worker thread and the GUI window thread.
pub struct GuiSharedState {
    /// Currently presented framebuffer (G0 or terminal).
    pub screen: Mutex<Grob>,
    /// Monotonically increasing counter bumped on each present.
    pub screen_generation: AtomicU64,
    /// FIFO queue for `GETKEY`, `WAIT` and dialogs. Entries without an HP
    /// key code (a typed character only) are skipped by `GETKEY`/`WAIT`.
    pub key_queue: Mutex<VecDeque<KeyPress>>,
    /// Set of currently pressed keys for `ISKEYDOWN`.
    pub held_keys: Mutex<HashSet<i64>>,
    /// Condvar to wake the worker thread when keys or events arrive. It is
    /// only ever waited on together with `key_queue`.
    pub key_condvar: Condvar,
    /// Current mouse/touch state.
    pub touch_pos: Mutex<Option<Touch>>,
    /// Terminal output history.
    pub terminal_lines: Mutex<Vec<String>>,
    /// Interruption flag for clean worker cancellation.
    pub interrupt: Arc<AtomicBool>,
    /// Optional callback to notify event loop to redraw.
    pub redraw_callback: Mutex<Option<RedrawCallback>>,
    /// Start instant for virtual clock.
    pub start_time: Instant,
}

/// Type alias bridging IDE host state and GUI host state.
pub type IdeSharedState = GuiSharedState;

impl Default for GuiSharedState {
    fn default() -> Self {
        Self::new()
    }
}

impl GuiSharedState {
    pub fn new() -> Self {
        Self {
            screen: Mutex::new(Grob::screen()),
            screen_generation: AtomicU64::new(0),
            key_queue: Mutex::new(VecDeque::new()),
            held_keys: Mutex::new(HashSet::new()),
            key_condvar: Condvar::new(),
            touch_pos: Mutex::new(None),
            terminal_lines: Mutex::new(Vec::new()),
            interrupt: Arc::new(AtomicBool::new(false)),
            redraw_callback: Mutex::new(None),
            start_time: Instant::now(),
        }
    }

    /// Sets the redraw notification callback.
    pub fn set_redraw_callback<F>(&self, cb: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        *lock(&self.redraw_callback) = Some(Box::new(cb));
    }

    /// Calls the redraw callback, if any.
    pub fn notify_redraw(&self) {
        if let Some(cb) = lock(&self.redraw_callback).as_ref() {
            cb();
        }
    }

    /// Feeds an HP keypress from the GUI into the key queue.
    pub fn push_key(&self, key: i64) {
        self.push_key_press(KeyPress::code(key));
    }

    /// Feeds a key event (HP code and/or typed character) into the queue.
    pub fn push_key_press(&self, press: KeyPress) {
        lock(&self.key_queue).push_back(press);
        self.key_condvar.notify_all();
    }

    /// Updates the pressed/released state of a key.
    pub fn set_key_down(&self, key: i64, down: bool) {
        {
            let mut set = lock(&self.held_keys);
            if down {
                set.insert(key);
            } else {
                set.remove(&key);
            }
        }
        self.key_condvar.notify_all();
    }

    /// Checks if a key is currently held down.
    pub fn is_key_down(&self, key: i64) -> bool {
        lock(&self.held_keys).contains(&key)
    }

    /// Clears all currently held keys (called on focus loss).
    pub fn clear_held_keys(&self) {
        {
            let mut set = lock(&self.held_keys);
            set.clear();
        }
        self.key_condvar.notify_all();
    }

    /// Updates the touch/mouse position.
    pub fn set_touch(&self, touch: Option<Touch>) {
        *lock(&self.touch_pos) = touch;
    }

    /// Requests immediate interruption of the VM.
    ///
    /// The flag is set while holding `key_queue`, the mutex every waiter
    /// checks the flag under; a waiter is therefore either before its check
    /// (and sees the flag) or already parked in `wait` (and gets the
    /// notification). Without the lock the store and notify could both land
    /// between a waiter's check and its `wait`, losing the wakeup.
    pub fn interrupt(&self) {
        {
            let _guard = lock(&self.key_queue);
            self.interrupt.store(true, Ordering::SeqCst);
        }
        self.key_condvar.notify_all();
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupt.load(Ordering::SeqCst)
    }

    /// Resets the interruption flag to false before starting a new run or restarting.
    pub fn reset_interrupt(&self) {
        let _guard = lock(&self.key_queue);
        self.interrupt.store(false, Ordering::SeqCst);
    }

    /// Resets all ephemeral execution state (interrupt flag, terminal lines, key queue,
    /// held keys, touch state) before starting a new run or restarting.
    pub fn reset_for_run(&self) {
        self.reset_interrupt();
        lock(&self.terminal_lines).clear();
        lock(&self.key_queue).clear();
        self.clear_held_keys();
        self.set_touch(None);
    }

    /// Fetches a clone of the current screen.
    pub fn get_frame(&self) -> Grob {
        lock(&self.screen).clone()
    }

    /// Blocks until a key event accepted by `accept` arrives (events it
    /// rejects are dropped), the optional `deadline` passes (`Ok(None)`), or
    /// the worker is interrupted.
    pub fn wait_press(
        &self,
        deadline: Option<Instant>,
        accept: impl Fn(&KeyPress) -> bool,
    ) -> Result<Option<KeyPress>, RuntimeError> {
        let mut queue = lock(&self.key_queue);
        loop {
            if self.is_interrupted() {
                return Err(RuntimeError::kind(ErrorKind::Interrupted));
            }
            while let Some(press) = queue.pop_front() {
                if accept(&press) {
                    return Ok(Some(press));
                }
            }
            let slice = match deadline {
                Some(dl) => {
                    let now = Instant::now();
                    if now >= dl {
                        return Ok(None);
                    }
                    (dl - now).min(WAIT_SLICE)
                }
                None => WAIT_SLICE,
            };
            queue = self
                .key_condvar
                .wait_timeout(queue, slice)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// The GUI Host given to `ppl_vm::Machine`.
pub struct GuiHost {
    state: Arc<GuiSharedState>,
    /// Also write `PRINT` output to stdout (like `ppl run`).
    echo: bool,
}

impl GuiHost {
    pub fn new(state: Arc<GuiSharedState>) -> Self {
        Self { state, echo: false }
    }

    /// Echo `PRINT` output to stdout as well as the emulated terminal.
    pub fn with_echo(mut self, echo: bool) -> Self {
        self.echo = echo;
        self
    }

    pub fn state(&self) -> &Arc<GuiSharedState> {
        &self.state
    }

    fn check_interrupt(&self) -> Result<(), RuntimeError> {
        if self.state.is_interrupted() {
            Err(RuntimeError::kind(ErrorKind::Interrupted))
        } else {
            Ok(())
        }
    }
}

impl Host for GuiHost {
    fn print(&mut self, text: &str) {
        if self.echo {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{text}");
            let _ = out.flush();
        }
        lock(&self.state.terminal_lines).push(text.to_string());
    }

    fn clear_terminal(&mut self) {
        lock(&self.state.terminal_lines).clear();
    }

    fn present(&mut self, screen: &Grob, terminal: Option<&[String]>) {
        let frame = if let Some(lines) = terminal {
            render_terminal(lines)
        } else {
            screen.clone()
        };
        *lock(&self.state.screen) = frame;
        self.state.screen_generation.fetch_add(1, Ordering::Release);
        self.state.notify_redraw();
    }

    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.check_interrupt()?;
        let mut q = lock(&self.state.key_queue);
        while let Some(press) = q.pop_front() {
            if press.code.is_some() {
                return Ok(press.code);
            }
        }
        Ok(None)
    }

    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.check_interrupt()?;
        Ok(lock(&self.state.held_keys).contains(&key))
    }

    fn wait_key(&mut self, timeout_s: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        let deadline = timeout_s.map(|s| Instant::now() + Duration::from_secs_f64(s.max(0.0)));
        Ok(self
            .state
            .wait_press(deadline, |p| p.code.is_some())?
            .and_then(|p| p.code))
    }

    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.check_interrupt()?;
        if seconds <= 0.0 || !seconds.is_finite() {
            return Ok(());
        }
        let target = Instant::now() + Duration::from_secs_f64(seconds);
        // Wait on the condvar's own mutex (std requires one mutex per
        // condvar); key presses wake us early, so loop until the target.
        let mut guard = lock(&self.state.key_queue);
        loop {
            if self.state.is_interrupted() {
                return Err(RuntimeError::kind(ErrorKind::Interrupted));
            }
            let now = Instant::now();
            if now >= target {
                return Ok(());
            }
            let chunk = (target - now).min(WAIT_SLICE);
            guard = self
                .state
                .key_condvar
                .wait_timeout(guard, chunk)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.check_interrupt()?;
        Ok(*lock(&self.state.touch_pos))
    }

    fn dialog(&mut self, dialog: &Dialog, screen: &Grob) -> Result<DialogResult, RuntimeError> {
        self.check_interrupt()?;
        let mut modal = make_dialog(dialog);
        let result = loop {
            self.present(&modal.render(screen), None);
            let press = match self.state.wait_press(None, |_| true) {
                Ok(Some(p)) => p,
                Ok(None) => continue,
                Err(e) => {
                    self.present(screen, None);
                    return Err(e);
                }
            };
            if let Some(r) = modal.handle(press) {
                break r;
            }
        };
        // Restore the program's screen.
        self.present(screen, None);
        Ok(result)
    }

    fn now_ms(&mut self) -> f64 {
        self.state.start_time.elapsed().as_secs_f64() * 1000.0
    }

    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        breakdown_system_time(SystemTime::now())
    }

    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        Some(self.state.interrupt.clone())
    }
}

/// Exact breakdown of SystemTime into (year, month, day, hour, minute, second).
fn breakdown_system_time(st: SystemTime) -> (i64, i64, i64, i64, i64, f64) {
    let dur = st.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let subsec = dur.subsec_nanos() as f64 / 1_000_000_000.0;

    let sec_in_day = total_secs % 86400;
    let hour = (sec_in_day / 3600) as i64;
    let minute = ((sec_in_day % 3600) / 60) as i64;
    let second = (sec_in_day % 60) as f64 + subsec;

    // Days since Jan 1 1970
    let mut days = (total_secs / 86400) as i64;

    // Howard Hinnant's civil day algorithm
    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as i64;
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gui_shared_state_key_queue() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());

        assert_eq!(host.poll_key().unwrap(), None);

        state.push_key(30);
        state.push_key(4);

        assert_eq!(host.poll_key().unwrap(), Some(30));
        assert_eq!(host.poll_key().unwrap(), Some(4));
        assert_eq!(host.poll_key().unwrap(), None);
    }

    #[test]
    fn test_gui_shared_state_is_key_down() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());

        assert!(!host.is_key_down(4).unwrap());
        state.set_key_down(4, true);
        assert!(host.is_key_down(4).unwrap());
        state.set_key_down(4, false);
        assert!(!host.is_key_down(4).unwrap());
    }

    #[test]
    fn test_gui_shared_state_touch() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());

        assert_eq!(host.touch().unwrap(), None);
        state.set_touch(Some(Touch { x: 100, y: 150 }));
        assert_eq!(host.touch().unwrap(), Some(Touch { x: 100, y: 150 }));
    }

    #[test]
    fn test_gui_shared_state_interrupt() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());

        assert!(host.sleep(0.01).is_ok());

        state.interrupt();
        assert!(matches!(
            host.poll_key().unwrap_err().kind,
            ErrorKind::Interrupted
        ));
        assert!(matches!(
            host.wait_key(Some(1.0)).unwrap_err().kind,
            ErrorKind::Interrupted
        ));
        assert!(matches!(
            host.sleep(1.0).unwrap_err().kind,
            ErrorKind::Interrupted
        ));
    }

    #[test]
    fn test_wait_key_timeout() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state);

        let t0 = Instant::now();
        let res = host.wait_key(Some(0.02)).unwrap();
        assert_eq!(res, None);
        assert!(t0.elapsed() >= Duration::from_millis(15));
    }

    /// Regression test for the close-hang race: `interrupt()` must wake a
    /// worker blocked in `wait_key(None)` promptly, every time.
    #[test]
    fn test_interrupt_wakes_blocked_wait_key_repeatedly() {
        for i in 0..500 {
            let state = Arc::new(GuiSharedState::new());
            let worker_state = state.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let mut host = GuiHost::new(worker_state);
                let _ = tx.send(host.wait_key(None));
            });
            // Vary the timing so the interrupt sometimes lands before the
            // wait starts, sometimes right around the check, sometimes later.
            if i % 3 == 1 {
                std::thread::yield_now();
            } else if i % 3 == 2 {
                std::thread::sleep(Duration::from_micros(50));
            }
            let t0 = Instant::now();
            state.interrupt();
            let res = rx
                .recv_timeout(Duration::from_millis(200))
                .unwrap_or_else(|_| panic!("iteration {i}: wait_key did not wake within 200 ms"));
            assert!(t0.elapsed() < Duration::from_millis(200));
            assert!(matches!(res.unwrap_err().kind, ErrorKind::Interrupted));
            handle.join().unwrap();
        }
    }

    /// Deterministic check of the wakeup protocol: a waiter holds
    /// `key_queue` from its flag check until it parks in `wait`, so
    /// `interrupt()` must not be able to set the flag (and notify) inside
    /// that window. Simulate the window by holding the lock.
    #[test]
    fn test_interrupt_serialises_with_waiters_lock() {
        let state = Arc::new(GuiSharedState::new());
        let guard = state.key_queue.lock().unwrap();
        let s2 = state.clone();
        let t = std::thread::spawn(move || s2.interrupt());
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !state.is_interrupted(),
            "interrupt() published the flag without taking the waiter's lock (lost wakeup)"
        );
        drop(guard);
        t.join().unwrap();
        assert!(state.is_interrupted());
    }

    #[test]
    fn test_interrupt_wakes_sleep_and_dialog() {
        let state = Arc::new(GuiSharedState::new());
        let s2 = state.clone();
        let sleeper = std::thread::spawn(move || GuiHost::new(s2).sleep(30.0));
        let s3 = state.clone();
        let dialog = std::thread::spawn(move || {
            GuiHost::new(s3).dialog(&Dialog::Message { text: "x".into() }, &Grob::screen())
        });
        std::thread::sleep(Duration::from_millis(20));
        let t0 = Instant::now();
        state.interrupt();
        assert!(matches!(
            sleeper.join().unwrap().unwrap_err().kind,
            ErrorKind::Interrupted
        ));
        assert!(matches!(
            dialog.join().unwrap().unwrap_err().kind,
            ErrorKind::Interrupted
        ));
        assert!(t0.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn test_sleep_survives_key_presses() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.push_key(30);
        let t0 = Instant::now();
        host.sleep(0.05).unwrap();
        assert!(t0.elapsed() >= Duration::from_millis(45));
        // The key is still queued for the program.
        assert_eq!(host.poll_key().unwrap(), Some(30));
    }

    #[test]
    fn test_char_only_events_are_invisible_to_getkey() {
        let state = Arc::new(GuiSharedState::new());
        let mut host = GuiHost::new(state.clone());
        state.push_key_press(KeyPress::with_char(None, 'q'));
        assert_eq!(host.poll_key().unwrap(), None);
        state.push_key_press(KeyPress::with_char(None, 'q'));
        state.push_key_press(KeyPress::with_char(Some(38), '5'));
        assert_eq!(host.wait_key(Some(1.0)).unwrap(), Some(38));
    }

    #[test]
    fn test_input_dialog_through_host() {
        use ppl_vm::host::InputField;
        let state = Arc::new(GuiSharedState::new());
        let s2 = state.clone();
        let worker = std::thread::spawn(move || {
            GuiHost::new(s2).dialog(
                &Dialog::Input {
                    title: "T".into(),
                    fields: vec![InputField {
                        label: "A".into(),
                        help: String::new(),
                        current: "1".into(),
                    }],
                },
                &Grob::screen(),
            )
        });
        let gen0 = state.screen_generation.load(Ordering::Acquire);
        for c in ['4', '2'] {
            state.push_key_press(KeyPress::with_char(None, c));
        }
        state.push_key(crate::keymap::KEY_ENTER);
        let r = worker.join().unwrap().unwrap();
        assert_eq!(r, DialogResult::Entered(vec!["42".into()]));
        // The dialog was drawn and the screen restored afterwards.
        assert!(state.screen_generation.load(Ordering::Acquire) > gen0);
        assert_eq!(state.get_frame().pixels(), Grob::screen().pixels());
    }

    #[test]
    fn test_choose_dialog_through_host_ignores_unknown_keys() {
        let state = Arc::new(GuiSharedState::new());
        let s2 = state.clone();
        let worker = std::thread::spawn(move || {
            GuiHost::new(s2).dialog(
                &Dialog::Choose {
                    title: "T".into(),
                    items: vec!["a".into(), "b".into()],
                    current: 1,
                },
                &Grob::screen(),
            )
        });
        state.push_key(crate::keymap::KEY_PLUS); // ignored
        state.push_key(crate::keymap::KEY_9); // out of range: ignored
        state.push_key(crate::keymap::KEY_DOWN);
        state.push_key(crate::keymap::KEY_ENTER);
        assert_eq!(worker.join().unwrap().unwrap(), DialogResult::Chosen(2));
    }

    #[test]
    fn test_breakdown_system_time() {
        // Test unix epoch
        let (y, m, d, h, mi, s) = breakdown_system_time(UNIX_EPOCH);
        assert_eq!((y, m, d, h, mi), (1970, 1, 1, 0, 0));
        assert_eq!(s, 0.0);

        // Test current time sanity
        let (y, m, d, _, _, _) = breakdown_system_time(SystemTime::now());
        assert!(y >= 2026);
        assert!((1..=12).contains(&m));
        assert!((1..=31).contains(&d));
    }

    #[test]
    fn test_reset_interrupt_and_reset_for_run() {
        let state = Arc::new(GuiSharedState::new());
        state.interrupt();
        assert!(state.is_interrupted());

        state.reset_interrupt();
        assert!(!state.is_interrupted());

        state.interrupt();
        lock(&state.terminal_lines).push("out".into());
        state.push_key(42);
        state.set_key_down(42, true);
        state.set_touch(Some(Touch { x: 10, y: 20 }));
        assert!(state.is_interrupted());
        assert_eq!(lock(&state.terminal_lines).len(), 1);
        assert!(!lock(&state.key_queue).is_empty());
        assert!(state.is_key_down(42));
        assert!(lock(&state.touch_pos).is_some());

        state.reset_for_run();
        assert!(!state.is_interrupted());
        assert!(lock(&state.terminal_lines).is_empty());
        assert!(lock(&state.key_queue).is_empty());
        assert!(!state.is_key_down(42));
        assert!(lock(&state.touch_pos).is_none());
    }
}
