//! Standalone winit calculator window: the plain emulator frontend.
//!
//! Presents the scaled 320x240 framebuffer in a native window and routes
//! keyboard/mouse events to the VM. This is the `ppl gui` display, kept
//! separate from the eframe-based IDE (`crate::app::IdeDesktopApp`).

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::dialog::KeyPress;
use crate::host::GuiSharedState;
use crate::ide::execution::{WorkerConfig, WorkerHandle, WorkerOutcome};
use crate::keymap::{is_break_key, map_winit_key, typed_char};
use crate::scaler::{LOGICAL_HEIGHT, LOGICAL_WIDTH, Viewport, blit_scaled, window_to_framebuffer};
use crate::script::{KeyFeeder, KeyScript};

/// How long closing the window waits for the worker to stop before giving
/// up on it (the thread is then detached so the window still closes).
pub const CLOSE_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Custom events sent to the winit event loop across threads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuiCustomEvent {
    /// Screen dirty; window redraw requested.
    Redraw,
    /// Background worker thread finished execution.
    WorkerFinished,
}

/// Configuration options for the GUI window.
#[derive(Clone, Debug)]
pub struct GuiAppConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub integer_scaling: bool,
    pub bg_color: u32,
    /// Close the window as soon as the program finishes.
    pub exit_on_finish: bool,
    /// Close the window (cancelling a still-running program) after this long.
    pub exit_after: Option<Duration>,
    /// Scripted input fed into the key/touch queues once the window exists.
    pub key_script: Option<KeyScript>,
}

impl Default for GuiAppConfig {
    fn default() -> Self {
        Self {
            title: "HP Prime Emulator".into(),
            width: 640,
            height: 480,
            integer_scaling: true,
            bg_color: 0x0010_1014, // Dark theme border
            exit_on_finish: false,
            exit_after: None,
            key_script: None,
        }
    }
}

/// The main application state running on the window event loop.
pub struct GuiApp {
    pub config: GuiAppConfig,
    pub state: Arc<GuiSharedState>,
    pub worker: Option<WorkerHandle>,
    pub window: Option<Arc<Window>>,
    pub surface_context: Option<softbuffer::Context<Arc<Window>>>,
    pub surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pub viewport: Viewport,
    pub event_loop_proxy: Option<EventLoopProxy<GuiCustomEvent>>,
    pub is_mouse_down: bool,
    /// Last cursor position in window pixels (from `CursorMoved`).
    pub last_cursor: Option<(f64, f64)>,
    pub modifiers: ModifiersState,
    /// The worker has finished (the title shows it).
    pub finished: bool,
    /// The window is closing (user close, `exit_after` or `exit_on_finish`).
    pub closing: bool,
    pub outcome: Option<WorkerOutcome>,
    pub started: Instant,
    /// Feeder thread for `config.key_script`, started with the window.
    pub key_feeder: Option<KeyFeeder>,
}

impl GuiApp {
    pub fn new(
        config: GuiAppConfig,
        state: Arc<GuiSharedState>,
        worker: Option<WorkerHandle>,
        proxy: Option<EventLoopProxy<GuiCustomEvent>>,
    ) -> Self {
        let viewport = Viewport::calculate(config.width, config.height, config.integer_scaling);
        Self {
            config,
            state,
            worker,
            window: None,
            surface_context: None,
            surface: None,
            viewport,
            event_loop_proxy: proxy,
            is_mouse_down: false,
            last_cursor: None,
            modifiers: ModifiersState::empty(),
            finished: false,
            closing: false,
            outcome: None,
            started: Instant::now(),
            key_feeder: None,
        }
    }

    /// Takes the final worker execution outcome if finished.
    pub fn take_outcome(&mut self) -> Option<WorkerOutcome> {
        if self.outcome.is_none() {
            if let Some(w) = &self.worker {
                self.outcome = w.outcome();
            }
        }
        self.outcome.take()
    }

    /// Window title for the current run state.
    pub fn status_title(&self) -> String {
        if !self.finished {
            return self.config.title.clone();
        }
        let outcome = self
            .outcome
            .clone()
            .or_else(|| self.worker.as_ref().and_then(|w| w.outcome()));
        let tag = match outcome {
            Some(WorkerOutcome::Error(e)) if e.kind == ppl_vm::ErrorKind::Interrupted => {
                "(Interrupted)".to_string()
            }
            Some(WorkerOutcome::Error(e)) => format!("({e})"),
            _ => "(Done)".to_string(),
        };
        format!("{} {tag}", self.config.title)
    }

    /// Records that the worker finished; returns `true` if the window should
    /// close now (`exit_on_finish`).
    pub fn on_worker_finished(&mut self) -> bool {
        self.finished = true;
        if let Some(w) = &self.window {
            w.set_title(&self.status_title());
            w.request_redraw();
        }
        self.config.exit_on_finish
    }

    /// Cancels the program and waits (bounded) for the worker to stop.
    pub fn shutdown_worker(&mut self) {
        self.closing = true;
        self.state.interrupt();
        if let Some(mut feeder) = self.key_feeder.take() {
            feeder.stop();
        }
        if let Some(mut worker) = self.worker.take() {
            self.outcome = worker.join_timeout(CLOSE_JOIN_TIMEOUT);
            if self.outcome.is_none() {
                // `join_timeout` detached the thread, so dropping the handle
                // cannot block.
                eprintln!(
                    "warning: the program did not stop within {:.0} s; abandoning it",
                    CLOSE_JOIN_TIMEOUT.as_secs_f64()
                );
            }
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        self.shutdown_worker();
        event_loop.exit();
    }

    /// Updates the keyboard modifier state.
    pub fn handle_modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }

    /// Handle keyboard input logically (unit testable without window).
    pub fn handle_key_input(
        &mut self,
        logical: &winit::keyboard::Key,
        physical: winit::keyboard::PhysicalKey,
        text: Option<&str>,
        state: ElementState,
        repeat: bool,
    ) {
        if is_break_key(logical, physical, self.modifiers.control_key()) {
            if state == ElementState::Pressed {
                // The calculator's On key: stop the running program.
                self.state.interrupt();
            }
            return;
        }
        let code = map_winit_key(logical, physical, text);
        match state {
            ElementState::Pressed => {
                if let Some(code) = code {
                    if !repeat {
                        self.state.set_key_down(code, true);
                    }
                }
                let ch = if self.modifiers.control_key() || self.modifiers.alt_key() {
                    None
                } else {
                    typed_char(logical, text)
                };
                if code.is_some() || ch.is_some() {
                    self.state.push_key_press(KeyPress { code, ch });
                }
            }
            ElementState::Released => {
                if let Some(code) = code {
                    self.state.set_key_down(code, false);
                }
            }
        }
    }

    /// Handle cursor move logically (unit testable without window).
    pub fn handle_cursor_move(&mut self, win_x: f64, win_y: f64) {
        self.last_cursor = Some((win_x, win_y));
        if self.is_mouse_down {
            let touch = window_to_framebuffer(win_x, win_y, &self.viewport)
                .map(|(x, y)| ppl_vm::host::Touch { x, y });
            self.state.set_touch(touch);
        }
    }

    /// Handle mouse button logically (unit testable without window).
    /// `win_pos` overrides the last known cursor position.
    pub fn handle_mouse_button(
        &mut self,
        state: ElementState,
        button: MouseButton,
        win_pos: Option<(f64, f64)>,
    ) {
        if button == MouseButton::Left {
            match state {
                ElementState::Pressed => {
                    self.is_mouse_down = true;
                    if let Some((win_x, win_y)) = win_pos.or(self.last_cursor) {
                        let touch = window_to_framebuffer(win_x, win_y, &self.viewport)
                            .map(|(x, y)| ppl_vm::host::Touch { x, y });
                        self.state.set_touch(touch);
                    }
                }
                ElementState::Released => {
                    self.is_mouse_down = false;
                    self.state.set_touch(None);
                }
            }
        }
    }

    /// Starts feeding the configured key script (once).
    pub fn start_key_script(&mut self) {
        if self.key_feeder.is_none() {
            if let Some(script) = self.config.key_script.take() {
                self.key_feeder = Some(KeyFeeder::spawn(script, self.state.clone()));
            }
        }
    }

    /// The `exit_after` deadline, if configured.
    pub fn exit_deadline(&self) -> Option<Instant> {
        self.config.exit_after.map(|d| self.started + d)
    }
}

impl ApplicationHandler<GuiCustomEvent> for GuiApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let size = LogicalSize::new(self.config.width, self.config.height);
        let min_size = LogicalSize::new(LOGICAL_WIDTH, LOGICAL_HEIGHT);
        let attrs = Window::default_attributes()
            .with_title(self.status_title())
            .with_inner_size(size)
            .with_min_inner_size(min_size);

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("error: failed to create window: {e}");
                self.close(event_loop);
                return;
            }
        };

        let context = match softbuffer::Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: failed to create softbuffer context: {e}");
                self.close(event_loop);
                return;
            }
        };

        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to create softbuffer surface: {e}");
                self.close(event_loop);
                return;
            }
        };

        self.window = Some(window.clone());
        self.surface_context = Some(context);
        self.surface = Some(surface);

        // Setup redraw notifier
        if let Some(proxy) = &self.event_loop_proxy {
            let p = proxy.clone();
            self.state.set_redraw_callback(move || {
                let _ = p.send_event(GuiCustomEvent::Redraw);
            });
        }

        window.request_redraw();
        self.start_key_script();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: GuiCustomEvent) {
        match event {
            GuiCustomEvent::Redraw => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            GuiCustomEvent::WorkerFinished => {
                if self.on_worker_finished() {
                    self.close(event_loop);
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(deadline) = self.exit_deadline() {
            if Instant::now() >= deadline {
                self.close(event_loop);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::RedrawRequested => {
                let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
                    return;
                };

                let win_size = window.inner_size();
                let Some(width) = NonZeroU32::new(win_size.width) else {
                    return;
                };
                let Some(height) = NonZeroU32::new(win_size.height) else {
                    return;
                };

                if let Err(e) = surface.resize(width, height) {
                    eprintln!("warning: surface resize failed: {e}");
                    return;
                }

                self.viewport = Viewport::calculate(
                    win_size.width,
                    win_size.height,
                    self.config.integer_scaling,
                );

                let mut buffer = match surface.buffer_mut() {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("warning: surface buffer_mut failed: {e}");
                        return;
                    }
                };

                let grob = self.state.get_frame();
                blit_scaled(
                    grob.pixels(),
                    LOGICAL_WIDTH as usize,
                    LOGICAL_HEIGHT as usize,
                    &mut buffer,
                    win_size.width as usize,
                    win_size.height as usize,
                    &self.viewport,
                    self.config.bg_color,
                );

                if let Err(e) = buffer.present() {
                    eprintln!("warning: surface present failed: {e}");
                }
            }

            WindowEvent::Resized(_) => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.close(event_loop);
            }

            WindowEvent::ModifiersChanged(m) => {
                self.handle_modifiers(m.state());
            }

            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_key_input(
                    &event.logical_key,
                    event.physical_key,
                    event.text.as_deref(),
                    event.state,
                    event.repeat,
                );
            }

            WindowEvent::CursorMoved { position, .. } => {
                self.handle_cursor_move(position.x, position.y);
            }

            WindowEvent::MouseInput { state, button, .. } => {
                self.handle_mouse_button(state, button, None);
            }

            _ => {}
        }
    }
}

/// Runs the GUI application with an optional worker. Blocks until window is closed.
///
/// Reports the worker's outcome (`None` when there was no worker, or it did
/// not stop in time after the window closed) and whether the window was
/// closed by the user/`exit_after` rather than after the program finished.
pub fn run_gui_session(
    config: GuiAppConfig,
    worker_config: Option<WorkerConfig>,
) -> Result<GuiSession, Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<GuiCustomEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let state = Arc::new(GuiSharedState::new());

    let worker = worker_config.map(|wc| {
        let p = proxy.clone();
        state.set_redraw_callback(move || {
            let _ = p.send_event(GuiCustomEvent::Redraw);
        });
        let finished = proxy.clone();
        WorkerHandle::spawn_with_notify(
            wc,
            state.clone(),
            Some(Box::new(move || {
                let _ = finished.send_event(GuiCustomEvent::WorkerFinished);
            })),
        )
    });

    let mut app = GuiApp::new(config, state, worker, Some(proxy));
    let run = event_loop.run_app(&mut app);
    // Whatever ended the loop, make sure the worker is stopped.
    if app.worker.is_some() {
        app.shutdown_worker();
    }
    run?;
    let outcome = app.take_outcome();
    let cancelled = app.closing
        && match &outcome {
            None => true,
            Some(WorkerOutcome::Error(e)) => e.kind == ppl_vm::ErrorKind::Interrupted,
            Some(WorkerOutcome::Success { .. }) => false,
        };
    Ok(GuiSession { outcome, cancelled })
}

/// Result of [`run_gui_session`].
#[derive(Debug)]
pub struct GuiSession {
    pub outcome: Option<WorkerOutcome>,
    /// The window was closed while the program was still running, so the
    /// program was interrupted (or abandoned).
    pub cancelled: bool,
}

/// Runs the GUI application with an optional worker. Blocks until window is closed.
pub fn run_gui(
    config: GuiAppConfig,
    worker_config: Option<WorkerConfig>,
) -> Result<Option<WorkerOutcome>, Box<dyn std::error::Error>> {
    Ok(run_gui_session(config, worker_config)?.outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{KEY_8, KEY_ENTER, KEY_TIMES};
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

    fn headless_app() -> (GuiApp, Arc<GuiSharedState>) {
        let state = Arc::new(GuiSharedState::new());
        let app = GuiApp::new(GuiAppConfig::default(), state.clone(), None, None);
        (app, state)
    }

    fn press(app: &mut GuiApp, logical: Key, physical: KeyCode, text: Option<&str>) {
        app.handle_key_input(
            &logical,
            PhysicalKey::Code(physical),
            text,
            ElementState::Pressed,
            false,
        );
    }

    #[test]
    fn test_click_without_prior_motion_uses_last_cursor() {
        let (mut app, state) = headless_app();
        // Cursor moved while the button was up: no touch yet.
        app.handle_cursor_move(320.0, 240.0);
        assert_eq!(*state.touch_pos.lock().unwrap(), None);
        // winit's MouseInput carries no position: the last cursor is used.
        app.handle_mouse_button(ElementState::Pressed, MouseButton::Left, None);
        assert_eq!(
            *state.touch_pos.lock().unwrap(),
            Some(ppl_vm::host::Touch { x: 160, y: 120 })
        );
        app.handle_mouse_button(ElementState::Released, MouseButton::Left, None);
        assert_eq!(*state.touch_pos.lock().unwrap(), None);
        // Other buttons do not touch.
        app.handle_mouse_button(ElementState::Pressed, MouseButton::Right, None);
        assert_eq!(*state.touch_pos.lock().unwrap(), None);
    }

    #[test]
    fn test_shifted_symbol_and_typed_char_are_queued() {
        let (mut app, state) = headless_app();
        press(
            &mut app,
            Key::Character("*".into()),
            KeyCode::Digit8,
            Some("*"),
        );
        assert_eq!(
            state.key_queue.lock().unwrap().pop_front(),
            Some(KeyPress {
                code: Some(KEY_TIMES),
                ch: Some('*')
            })
        );
        assert!(!state.held_keys.lock().unwrap().contains(&KEY_8));
        // A letter with no HP key still reaches INPUT dialogs as a character.
        press(
            &mut app,
            Key::Character("q".into()),
            KeyCode::KeyQ,
            Some("q"),
        );
        assert_eq!(
            state.key_queue.lock().unwrap().pop_front(),
            Some(KeyPress {
                code: None,
                ch: Some('q')
            })
        );
    }

    #[test]
    fn test_break_key_interrupts_instead_of_queueing() {
        let (mut app, state) = headless_app();
        press(
            &mut app,
            Key::Named(NamedKey::Escape),
            KeyCode::Escape,
            None,
        );
        assert!(!state.is_interrupted(), "Esc is HP key 4, not break");
        state.key_queue.lock().unwrap().clear();

        app.handle_modifiers(ModifiersState::CONTROL);
        press(&mut app, Key::Character("c".into()), KeyCode::KeyC, None);
        assert!(state.is_interrupted());
        assert!(state.key_queue.lock().unwrap().is_empty());

        let (mut app, state) = headless_app();
        press(&mut app, Key::Named(NamedKey::F12), KeyCode::F12, None);
        assert!(state.is_interrupted());
    }

    #[test]
    fn test_status_title_and_exit_on_finish() {
        let (mut app, _state) = headless_app();
        assert_eq!(app.status_title(), "HP Prime Emulator");
        assert!(!app.on_worker_finished());
        assert_eq!(app.status_title(), "HP Prime Emulator (Done)");
        app.outcome = Some(WorkerOutcome::Error(ppl_vm::RuntimeError::kind(
            ppl_vm::ErrorKind::Interrupted,
        )));
        assert_eq!(app.status_title(), "HP Prime Emulator (Interrupted)");

        let state = Arc::new(GuiSharedState::new());
        let cfg = GuiAppConfig {
            exit_on_finish: true,
            exit_after: Some(Duration::from_secs(5)),
            ..GuiAppConfig::default()
        };
        let mut app = GuiApp::new(cfg, state, None, None);
        assert!(app.on_worker_finished());
        assert_eq!(
            app.exit_deadline(),
            Some(app.started + Duration::from_secs(5))
        );
    }

    #[test]
    fn test_shutdown_worker_cancels_blocked_program_promptly() {
        use ppl_compiler::{CompileOptions, compile_source};
        let file = ppl_syntax::SourceFile::new("t.ppl", "EXPORT TST() BEGIN WAIT(0); END;");
        let prog = compile_source(&file, &CompileOptions::default()).0.unwrap();
        let state = Arc::new(GuiSharedState::new());
        let worker = WorkerHandle::spawn(WorkerConfig::new(prog), state.clone());
        let mut app = GuiApp::new(GuiAppConfig::default(), state, Some(worker), None);
        std::thread::sleep(Duration::from_millis(20));
        let t0 = Instant::now();
        app.shutdown_worker();
        assert!(t0.elapsed() < Duration::from_millis(500));
        assert!(app.closing);
        match app.take_outcome() {
            Some(WorkerOutcome::Error(e)) => assert_eq!(e.kind, ppl_vm::ErrorKind::Interrupted),
            other => panic!("expected interrupted outcome, got {other:?}"),
        }
    }

    #[test]
    fn test_gui_app_config_defaults() {
        let cfg = GuiAppConfig::default();
        assert_eq!(cfg.width, 640);
        assert_eq!(cfg.height, 480);
        assert!(cfg.integer_scaling);
        assert_eq!(cfg.bg_color, 0x0010_1014);
    }

    #[test]
    fn test_gui_app_event_handling_headless() {
        let state = Arc::new(GuiSharedState::new());
        let mut app = GuiApp::new(GuiAppConfig::default(), state.clone(), None, None);

        // Simulate KeyPress
        app.handle_key_input(
            &Key::Named(NamedKey::Enter),
            PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
            None,
            ElementState::Pressed,
            false,
        );

        assert!(state.held_keys.lock().unwrap().contains(&KEY_ENTER));
        assert_eq!(
            state.key_queue.lock().unwrap().pop_front(),
            Some(KeyPress::code(KEY_ENTER))
        );

        // Simulate KeyRelease
        app.handle_key_input(
            &Key::Named(NamedKey::Enter),
            PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Unidentified),
            None,
            ElementState::Released,
            false,
        );
        assert!(!state.held_keys.lock().unwrap().contains(&KEY_ENTER));
    }

    #[test]
    fn test_gui_app_mouse_handling_headless() {
        let state = Arc::new(GuiSharedState::new());
        let mut app = GuiApp::new(GuiAppConfig::default(), state.clone(), None, None);

        // Click at center (320, 240) in a 640x480 window -> logical (160, 120)
        app.handle_mouse_button(
            ElementState::Pressed,
            MouseButton::Left,
            Some((320.0, 240.0)),
        );
        assert!(app.is_mouse_down);
        assert_eq!(
            *state.touch_pos.lock().unwrap(),
            Some(ppl_vm::host::Touch { x: 160, y: 120 })
        );

        // Drag to (400, 300) -> logical (200, 150)
        app.handle_cursor_move(400.0, 300.0);
        assert_eq!(
            *state.touch_pos.lock().unwrap(),
            Some(ppl_vm::host::Touch { x: 200, y: 150 })
        );

        // Release
        app.handle_mouse_button(ElementState::Released, MouseButton::Left, None);
        assert!(!app.is_mouse_down);
        assert_eq!(*state.touch_pos.lock().unwrap(), None);
    }
}
