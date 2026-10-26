//! Desktop live screen mirror viewer for a physical HP Prime G1.
//!
//! Continuously fetches 320×240 screenshots from the physical calculator over
//! USB HID (`FC 08`), decodes the little-endian RGB555 PNG bytes, and presents them in a
//! responsive desktop window via `winit` and `softbuffer`.
//!
//! The capture loop runs asynchronously on a dedicated worker thread so that
//! USB latency never blocks the UI event loop. Outdated unrendered frames are
//! dropped in favor of the latest frame to prevent latency buildup.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ppl_device::screenshot::ScreenshotTimings;
use ppl_device::{DeviceError, Prime};
use ppl_graphics::ScreenshotDecodeTimings;

mod capture;
mod diagnostics;
pub use capture::UsbMirrorSource;
pub use diagnostics::capture_diagnostics;
use diagnostics::{CaptureDiagnostics, DisplayDiagnostics};
use ppl_graphics::Grob;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::scaler::{LOGICAL_HEIGHT, LOGICAL_WIDTH, ScaleFilter, Viewport, blit_filtered};

/// Default background color for window letterbox borders.
pub const DEFAULT_BG_COLOR: u32 = 0x0010_1014;
/// Border color when connection is lost / stale.
pub const STALE_BG_COLOR: u32 = 0x003A_1A1A;
/// Default read timeout for screenshot requests during mirroring.
pub const SCREENSHOT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Status of the live mirror connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorStatus {
    Connecting,
    Connected,
    Capturing,
    Paused,
    WaitingForCalculator,
    Disconnected(String),
    Reconnecting,
    Stopped,
}

impl MirrorStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            MirrorStatus::Connected | MirrorStatus::Capturing | MirrorStatus::Paused
        )
    }

    pub fn tag(&self) -> &'static str {
        match self {
            MirrorStatus::Connecting => "Connecting",
            MirrorStatus::Connected => "Connected",
            MirrorStatus::Capturing => "Capturing",
            MirrorStatus::Paused => "PAUSED",
            MirrorStatus::WaitingForCalculator => "Waiting for calculator",
            MirrorStatus::Disconnected(_) => "DISCONNECTED",
            MirrorStatus::Reconnecting => "Reconnecting...",
            MirrorStatus::Stopped => "Stopped",
        }
    }
}

/// Full retains all 320×240 pixels. Fast retains RGB555 color at 160×120.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MirrorQuality {
    #[default]
    Full,
    Fast,
}
impl MirrorQuality {
    pub fn format(self) -> u8 {
        match self {
            Self::Full => 8,
            Self::Fast => 10,
        }
    }
    pub fn dimensions(self) -> (usize, usize) {
        match self {
            Self::Full => (320, 240),
            Self::Fast => (160, 120),
        }
    }
}
impl std::str::FromStr for MirrorQuality {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "full" => Ok(Self::Full),
            "fast" => Ok(Self::Fast),
            _ => Err("quality must be full or fast".into()),
        }
    }
}

/// Physical incremental mode stays gated until reviewed hardware acceptance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MirrorCapture {
    #[default]
    Full,
    Incremental,
}
impl std::str::FromStr for MirrorCapture {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "full" => Ok(Self::Full),
            "incremental" => Ok(Self::Incremental),
            _ => Err("capture must be full or incremental".into()),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct UpdateDiagnostics {
    pub format: u8,
    pub origin: Option<(u16, u16)>,
    pub dimensions: (usize, usize),
    pub baseline: bool,
    pub changed: bool,
    pub composition: Duration,
}

pub struct SourceFrame {
    pub grob: Arc<Grob>,
    pub requested: Instant,
    pub completed: Instant,
    pub usb: ScreenshotTimings,
    pub decode: ScreenshotDecodeTimings,
    pub raw: Option<Vec<u8>>,
    pub update: Option<UpdateDiagnostics>,
}

/// A single captured screen frame.
#[derive(Clone, Debug)]
pub struct CapturedFrame {
    pub grob: Arc<Grob>,
    pub seq: u64,
    pub timestamp: Instant,
    pub published: Instant,
    pub capture_duration: Duration,
    pub decode_duration: Duration,
}

/// Metrics measuring actual mirror performance.
#[derive(Clone, Debug, Default)]
pub struct MirrorMetrics {
    pub rolling_fps: f64,
    pub total_captured: u64,
    pub total_requests: u64,
    pub resynchronizations: u64,
    pub total_displayed: u64,
    pub total_dropped: u64,
    pub last_capture_duration: Duration,
    pub last_decode_duration: Duration,
    pub total_published: u64,
    pub total_unchanged: u64,
    pub errors: u64,
    pub published_fps: f64,
    pub displayed_fps: f64,
    pub last_success: Option<Instant>,
    pub frame_age: Duration,
    pub last_render_duration: Duration,
}

/// Trait abstracting the hardware screenshot capture source.
pub trait MirrorSource: Send + 'static {
    /// Captures one raw screenshot and returns the decoded Grob.
    fn capture_frame(&mut self) -> Result<Grob, DeviceError>;
    fn capture(&mut self, cancel: &AtomicBool) -> Result<SourceFrame, DeviceError> {
        let requested = Instant::now();
        if cancel.load(Ordering::Acquire) {
            return Err(DeviceError::Refused("capture cancelled".into()));
        }
        let grob = self.capture_frame()?;
        Ok(SourceFrame {
            grob: Arc::new(grob),
            requested,
            completed: Instant::now(),
            usb: ScreenshotTimings {
                total: requested.elapsed(),
                ..Default::default()
            },
            decode: Default::default(),
            raw: None,
            update: None,
        })
    }
    fn screenshot_requests(&self) -> Option<u64> {
        None
    }

    fn recovery_attempt_limit(&self) -> Option<usize> {
        None
    }

    fn failure_stage(&self) -> &'static str {
        "capture"
    }

    fn reconnect(&mut self, _cancel: &AtomicBool) -> Result<bool, DeviceError> {
        self.try_reconnect()
    }

    /// Identifying model or name of the device.
    fn device_name(&self) -> &str;
    /// Optional serial number/fingerprint of the device.
    fn device_serial(&self) -> Option<&str>;
    /// Attempts to re-establish connection after an error or disconnect.
    fn try_reconnect(&mut self) -> Result<bool, DeviceError>;
}

/// Mock screenshot source for deterministic testing without USB hardware.
pub struct MockMirrorSource {
    pub frames: VecDeque<Result<Grob, DeviceError>>,
    pub name: String,
    pub serial: Option<String>,
    pub capture_delay: Duration,
    pub reconnect_succeeds: bool,
    pub captures_count: usize,
}

impl MockMirrorSource {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            frames: VecDeque::new(),
            name: name.into(),
            serial: Some("mock-serial-12345".into()),
            capture_delay: Duration::ZERO,
            reconnect_succeeds: true,
            captures_count: 0,
        }
    }

    pub fn with_solid_frames(mut self, colors: &[u32]) -> Self {
        for &c in colors {
            let grob = Grob::new(LOGICAL_WIDTH as usize, LOGICAL_HEIGHT as usize, c);
            self.frames.push_back(Ok(grob));
        }
        self
    }
}

impl MirrorSource for MockMirrorSource {
    fn capture_frame(&mut self) -> Result<Grob, DeviceError> {
        self.captures_count += 1;
        if !self.capture_delay.is_zero() {
            std::thread::sleep(self.capture_delay);
        }
        if let Some(res) = self.frames.pop_front() {
            res
        } else {
            Ok(Grob::screen())
        }
    }

    fn device_name(&self) -> &str {
        &self.name
    }

    fn device_serial(&self) -> Option<&str> {
        self.serial.as_deref()
    }

    fn try_reconnect(&mut self) -> Result<bool, DeviceError> {
        Ok(self.reconnect_succeeds)
    }
}

/// Custom events sent to the winit event loop across threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorCustomEvent {
    /// A newer screenshot frame is available.
    NewFrame,
    /// Connection or mirror status has changed.
    StatusChanged(MirrorStatus),
    /// Background worker has shut down.
    WorkerStopped,
}

/// Shared state between the capture worker and the UI window.
pub struct MirrorSharedState {
    pub stop_requested: AtomicBool,
    pub paused: AtomicBool,
    pub status: Mutex<MirrorStatus>,
    pub latest_frame: Mutex<Option<CapturedFrame>>,
    pub metrics: Mutex<MirrorMetrics>,
    notification_pending: AtomicBool,
}

impl MirrorSharedState {
    pub fn new() -> Self {
        Self {
            stop_requested: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            status: Mutex::new(MirrorStatus::Connecting),
            latest_frame: Mutex::new(None),
            metrics: Mutex::new(MirrorMetrics::default()),
            notification_pending: AtomicBool::new(false),
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_requested.load(Ordering::Acquire)
    }

    pub fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn toggle_pause(&self) -> bool {
        let was_paused = self.paused.fetch_xor(true, Ordering::AcqRel);
        !was_paused
    }

    pub fn set_status(&self, status: MirrorStatus) {
        let mut s = self.status.lock().unwrap();
        *s = status;
    }

    pub fn get_status(&self) -> MirrorStatus {
        self.status.lock().unwrap().clone()
    }
}

impl Default for MirrorSharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle to the background capture worker.
pub struct MirrorWorkerHandle {
    pub state: Arc<MirrorSharedState>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MirrorWorkerHandle {
    pub fn spawn<S: MirrorSource>(
        source: S,
        target_fps: f64,
        proxy: Option<EventLoopProxy<MirrorCustomEvent>>,
    ) -> Self {
        Self::spawn_configured(source, target_fps, proxy, None, None)
            .expect("failed to spawn mirror worker")
    }

    pub(crate) fn spawn_configured<S: MirrorSource>(
        mut source: S,
        target_fps: f64,
        proxy: Option<EventLoopProxy<MirrorCustomEvent>>,
        diagnostics: Option<&Path>,
        limit: Option<u64>,
    ) -> std::io::Result<Self> {
        let diagnostic = diagnostics.map(CaptureDiagnostics::open).transpose()?;
        let state = Arc::new(MirrorSharedState::new());
        let worker_state = state.clone();
        let stopped_proxy = proxy.clone();
        let thread = std::thread::Builder::new()
            .name("ppl-mirror-worker".into())
            .spawn(move || {
                run_capture_worker_loop(
                    &mut source,
                    target_fps,
                    &worker_state,
                    proxy,
                    diagnostic,
                    limit,
                );
                // Handle and advisory lock are gone before notifying the GUI.
                drop(source);
                worker_state.set_status(MirrorStatus::Stopped);
                if let Some(proxy) = stopped_proxy {
                    let _ = proxy.send_event(MirrorCustomEvent::WorkerStopped);
                }
            })?;
        Ok(Self {
            state,
            thread: Some(thread),
        })
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    pub fn stop(&mut self) {
        self.state.request_stop();
    }

    pub fn join_timeout(&mut self, timeout: Duration) {
        self.stop();
        let start = Instant::now();
        while !self.is_finished() && start.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(5));
        }
        // A timeout retains ownership. Never detach a thread holding the device.
        if self.is_finished() {
            self.join();
        }
    }

    pub fn join(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for MirrorWorkerHandle {
    fn drop(&mut self) {
        self.join();
    }
}

fn frame_interval(fps: f64) -> Option<Duration> {
    if !fps.is_finite() || fps <= 0.0 {
        return None;
    }
    Duration::try_from_secs_f64(1.0 / fps)
        .ok()
        .filter(|d| !d.is_zero() && Instant::now().checked_add(*d).is_some())
}

fn rolling_rate(times: &VecDeque<Instant>, now: Instant) -> f64 {
    match (times.front(), times.back()) {
        (Some(first), Some(last))
            if times.len() > 1
                && now.duration_since(*last) < Duration::from_secs(2)
                && last > first =>
        {
            (times.len() - 1) as f64 / now.duration_since(*first).as_secs_f64()
        }
        _ => 0.0,
    }
}

fn push_time(times: &mut VecDeque<Instant>, now: Instant) {
    times.push_back(now);
    if times.len() > 20 {
        times.pop_front();
    }
}

fn wait_until(deadline: Instant, state: &MirrorSharedState, interrupt_on_pause: bool) {
    while !(state.is_stopped() || interrupt_on_pause && state.is_paused()) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

fn notify_status(
    state: &MirrorSharedState,
    proxy: &Option<EventLoopProxy<MirrorCustomEvent>>,
    status: MirrorStatus,
) {
    state.set_status(status.clone());
    if let Some(p) = proxy {
        let _ = p.send_event(MirrorCustomEvent::StatusChanged(status));
    }
}

fn run_capture_worker_loop<S: MirrorSource>(
    source: &mut S,
    target_fps: f64,
    state: &Arc<MirrorSharedState>,
    proxy: Option<EventLoopProxy<MirrorCustomEvent>>,
    mut diagnostics: Option<CaptureDiagnostics>,
    limit: Option<u64>,
) {
    let Some(interval) = frame_interval(target_fps) else {
        return;
    };
    let mut seq = 0u64;
    let mut captures = VecDeque::with_capacity(20);
    let mut publications = VecDeque::with_capacity(20);
    let mut previous: Option<Arc<Grob>> = None;
    let mut republish = false;
    let mut previous_seq = 0;
    notify_status(state, &proxy, MirrorStatus::Connected);
    let mut attempts = 0;
    let mut consecutive_unchanged: u32 = 0;
    'capture: while !state.is_stopped() {
        if state.is_paused() {
            notify_status(state, &proxy, MirrorStatus::Paused);
            {
                let mut metrics = state.metrics.lock().unwrap();
                metrics.rolling_fps = 0.0;
                metrics.published_fps = 0.0;
                metrics.displayed_fps = 0.0;
            }
            while state.is_paused() && !state.is_stopped() {
                std::thread::sleep(Duration::from_millis(10));
            }
            captures.clear();
            publications.clear();
            republish = true;
            consecutive_unchanged = 0;
            if state.is_stopped() {
                break;
            }
        }
        let cycle = Instant::now();
        match source.capture(&state.stop_requested) {
            Ok(source_frame) => {
                if source_frame.update.as_ref().is_none_or(|u| !u.baseline) {
                    attempts = 0;
                }
                if source_frame.update.as_ref().is_some_and(|u| u.baseline) && seq > 0 {
                    state.metrics.lock().unwrap().resynchronizations += 1;
                }
                if state.is_stopped() {
                    break;
                }
                seq += 1;
                let now = Instant::now();
                push_time(&mut captures, now);
                let paused = state.is_paused();
                let changed = !paused
                    && source_frame.update.as_ref().map_or_else(
                        || previous.as_deref() != Some(source_frame.grob.as_ref()),
                        |u| u.changed || previous.is_none(),
                    );
                if changed {
                    consecutive_unchanged = 0;
                } else if !paused {
                    consecutive_unchanged = consecutive_unchanged.saturating_add(1);
                }
                let decode = source_frame.decode.decompression + source_frame.decode.conversion;
                let publication = Instant::now();
                {
                    let mut latest = state.latest_frame.lock().unwrap();
                    let mut m = state.metrics.lock().unwrap();
                    m.total_captured += 1;
                    m.total_requests = source.screenshot_requests().unwrap_or(m.total_captured);
                    m.last_success = Some(now);
                    m.last_capture_duration = source_frame.usb.total;
                    m.last_decode_duration = decode;
                    m.rolling_fps = rolling_rate(&captures, now);
                    if changed || (!paused && republish) {
                        republish = false;
                        if changed {
                            previous_seq = seq;
                        }
                        let grob = source_frame.grob.clone();
                        if latest.is_some() {
                            m.total_dropped += 1;
                        }
                        *latest = Some(CapturedFrame {
                            grob: grob.clone(),
                            seq: previous_seq,
                            timestamp: source_frame.requested,
                            published: Instant::now(),
                            capture_duration: source_frame.usb.total,
                            decode_duration: decode,
                        });
                        previous = Some(grob);
                        if changed {
                            m.total_published += 1;
                            push_time(&mut publications, now);
                        } else {
                            m.total_unchanged += 1;
                        }
                    } else if !paused {
                        m.total_unchanged += 1;
                    }
                    m.published_fps = rolling_rate(&publications, now);
                }
                if !paused {
                    state.set_status(MirrorStatus::Capturing);
                }
                let publication_duration = publication.elapsed();
                if let Some(d) = &mut diagnostics {
                    if let Err(e) = d.record(
                        seq,
                        &source_frame,
                        publication_duration,
                        changed,
                        source.screenshot_requests(),
                    ) {
                        eprintln!("warning: mirror diagnostics failed: {e}");
                        diagnostics = None;
                    }
                }
                // A completion updates title/health, but only changed pixels redraw.
                if let Some(p) = &proxy {
                    if !state.notification_pending.swap(true, Ordering::AcqRel) {
                        let _ = p.send_event(MirrorCustomEvent::NewFrame);
                    }
                }
                if limit.is_some_and(|n| seq >= n) {
                    break;
                }
                // Bounded adaptive capture pacing: if the display has been static on
                // incremental capture, gradually relax polling up to 200ms (5 FPS)
                // to avoid spinning on repeated 1x1 unchanged responses. As soon as
                // any pixel changes, immediately return to full rate.
                let pacing_interval = if source_frame.update.is_some() {
                    if consecutive_unchanged >= 8 {
                        interval.max(Duration::from_millis(200))
                    } else if consecutive_unchanged >= 4 {
                        interval.max(Duration::from_millis(100))
                    } else if consecutive_unchanged >= 2 {
                        interval.max(Duration::from_millis(50))
                    } else {
                        interval
                    }
                } else {
                    interval
                };
                wait_until(cycle + pacing_interval, state, true);
            }
            Err(e) => {
                if state.is_stopped() {
                    break;
                }
                {
                    let mut m = state.metrics.lock().unwrap();
                    m.errors += 1;
                    m.total_requests = source
                        .screenshot_requests()
                        .unwrap_or(m.total_captured + m.errors);
                    m.rolling_fps = 0.0;
                    m.published_fps = 0.0;
                    m.displayed_fps = 0.0;
                }
                if let Some(d) = &mut diagnostics {
                    let _ = d.error(
                        source.failure_stage(),
                        &e.to_string(),
                        source.screenshot_requests(),
                    );
                }
                notify_status(state, &proxy, MirrorStatus::Disconnected(e.to_string()));
                captures.clear();
                publications.clear();
                loop {
                    if source
                        .recovery_attempt_limit()
                        .is_some_and(|limit| attempts >= limit)
                    {
                        notify_status(
                            state,
                            &proxy,
                            MirrorStatus::Disconnected(
                                "Recovery exhausted; last frame is stale".into(),
                            ),
                        );
                        // Preserve the last verified image and stale status until
                        // the user closes; no further reconnect or USB requests.
                        while !state.is_stopped() {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        break 'capture;
                    }
                    attempts += 1;
                    wait_until(Instant::now() + Duration::from_secs(1), state, false);
                    if state.is_stopped() {
                        break;
                    }
                    notify_status(state, &proxy, MirrorStatus::Reconnecting);
                    if matches!(source.reconnect(&state.stop_requested), Ok(true)) {
                        notify_status(state, &proxy, MirrorStatus::Connected);
                        break;
                    }
                    notify_status(
                        state,
                        &proxy,
                        MirrorStatus::Disconnected("Waiting for original calculator".into()),
                    );
                }
            }
        }
    }
}

/// Configuration options for the mirror window.
#[derive(Clone, Debug)]
pub struct MirrorConfig {
    pub title: String,
    pub scale: u32,
    pub fps: f64,
    pub fullscreen: bool,
    pub snapshot: Option<PathBuf>,
    pub integer_scaling: bool,
    pub filter: ScaleFilter,
    pub quality: MirrorQuality,
    pub capture: MirrorCapture,
    pub diagnostics: Option<PathBuf>,
    pub bg_color: u32,
    pub exit_after: Option<Duration>,
    /// Legacy CLI compatibility only; never grants a streaming permit.
    pub acknowledge_experimental_fc04: bool,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            title: "HP Prime G1 — Live Screen".into(),
            scale: 2,
            fps: 10.0,
            fullscreen: false,
            snapshot: None,
            integer_scaling: true,
            filter: ScaleFilter::Nearest,
            quality: MirrorQuality::Full,
            capture: MirrorCapture::Full,
            diagnostics: None,
            bg_color: DEFAULT_BG_COLOR,
            exit_after: None,
            acknowledge_experimental_fc04: false,
        }
    }
}

/// Native desktop mirror viewer application.
pub struct MirrorApp {
    pub config: MirrorConfig,
    pub worker: Option<MirrorWorkerHandle>,
    pub window: Option<Arc<Window>>,
    pub surface_context: Option<softbuffer::Context<Arc<Window>>>,
    pub surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    pub viewport: Viewport,
    pub current_scale: u32,
    pub last_frame: Option<CapturedFrame>,
    pub status: MirrorStatus,
    pub notice: Option<(String, Instant)>,
    pub initial_snapshot_saved: bool,
    pub displayed_count: u64,
    pub started: Instant,
    pub closed: bool,
    surface_size: Option<(u32, u32)>,
    presented_seq: Option<u64>,
    display_times: VecDeque<Instant>,
    diagnostics: Option<DisplayDiagnostics>,
}

impl MirrorApp {
    pub fn new(config: MirrorConfig, worker: Option<MirrorWorkerHandle>) -> Self {
        let scale = config.scale.clamp(1, 4);
        let win_w = LOGICAL_WIDTH * scale;
        let win_h = LOGICAL_HEIGHT * scale;
        let viewport = Viewport::calculate(
            win_w,
            win_h,
            config.integer_scaling && config.filter == ScaleFilter::Nearest,
        );

        Self {
            config,
            worker,
            window: None,
            surface_context: None,
            surface: None,
            viewport,
            current_scale: scale,
            last_frame: None,
            status: MirrorStatus::Connecting,
            notice: None,
            initial_snapshot_saved: false,
            displayed_count: 0,
            started: Instant::now(),
            closed: false,
            surface_size: None,
            presented_seq: None,
            display_times: VecDeque::with_capacity(20),
            diagnostics: None,
        }
    }

    /// Sets a temporary user notice displayed in the title bar for a few seconds.
    pub fn set_notice(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), Instant::now()));
        self.update_title();
    }

    /// Formats the current dynamic window title based on state, FPS, and status.
    pub fn dynamic_title(&self) -> String {
        if let Some((notice, at)) = &self.notice {
            if at.elapsed() < Duration::from_secs(3) {
                return format!("{} [{notice}]", self.config.title);
            }
        }

        match &self.status {
            MirrorStatus::Paused => format!("{} — Paused", self.config.title),
            MirrorStatus::Disconnected(_) => {
                format!("{} — Disconnected (last image stale)", self.config.title)
            }
            MirrorStatus::Reconnecting => {
                format!("{} [Reconnecting; last image stale]", self.config.title)
            }
            MirrorStatus::WaitingForCalculator => {
                format!("{} [Waiting for calculator...]", self.config.title)
            }
            MirrorStatus::Connecting => format!("{} [Connecting...]", self.config.title),
            MirrorStatus::Connected | MirrorStatus::Capturing => {
                if let Some(w) = &self.worker {
                    let m = w.state.metrics.lock().unwrap();
                    if m.last_success
                        .is_some_and(|at| at.elapsed() > Duration::from_secs(3))
                    {
                        return format!(
                            "{} [Stale: no capture for {:.1}s]",
                            self.config.title,
                            m.last_success.unwrap().elapsed().as_secs_f64()
                        );
                    }
                    let quality = if self.config.quality == MirrorQuality::Fast {
                        " | 160×120 low detail"
                    } else {
                        ""
                    };
                    format!(
                        "{} | {:.1} capture/s | {:.1} changed/s | {:.1} presented/s{}",
                        self.config.title, m.rolling_fps, m.published_fps, m.displayed_fps, quality
                    )
                } else {
                    format!("{} [Live]", self.config.title)
                }
            }
            MirrorStatus::Stopped => format!("{} [Stopped]", self.config.title),
        }
    }

    /// Updates window title bar if window is created.
    pub fn update_title(&self) {
        if let Some(w) = &self.window {
            w.set_title(&self.dynamic_title());
        }
    }

    /// Saves the latest displayed frame as PNG.
    pub fn save_snapshot(&mut self, path: Option<&Path>) -> Result<PathBuf, String> {
        let frame = self
            .last_frame
            .as_ref()
            .ok_or_else(|| "no valid frame captured yet".to_string())?;

        let save_path = if let Some(p) = path {
            p.to_path_buf()
        } else if let Some(cfg_p) = &self.config.snapshot {
            cfg_p.clone()
        } else {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            PathBuf::from(format!("prime-screen-{timestamp}.png"))
        };

        if let Some(parent) = save_path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        frame
            .grob
            .save_png(&save_path)
            .map_err(|e| format!("failed to save PNG {}: {e}", save_path.display()))?;

        Ok(save_path)
    }

    /// Changes integer display scale (resizes window).
    pub fn set_integer_scale(&mut self, scale: u32) {
        let scale = scale.clamp(1, 4);
        self.current_scale = scale;
        if let Some(w) = &self.window {
            w.set_fullscreen(None);
            let size = LogicalSize::new(LOGICAL_WIDTH * scale, LOGICAL_HEIGHT * scale);
            let _ = w.request_inner_size(size);
            w.request_redraw();
        }
    }

    /// Closes the application cleanly.
    pub fn close(&mut self, event_loop: &ActiveEventLoop) {
        self.closed = true;
        if let Some(worker) = &mut self.worker {
            worker.stop();
            if !worker.is_finished() {
                if let Some(window) = &self.window {
                    window.set_visible(false);
                }
                event_loop.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_millis(10),
                ));
                return;
            }
            worker.join();
        }
        self.worker = None;
        event_loop.exit();
    }

    /// Handles keyboard events logically (unit-testable headlessly).
    pub fn handle_key(&mut self, logical: &Key, physical: PhysicalKey, state: ElementState) {
        if state != ElementState::Pressed {
            return;
        }

        match (logical, physical) {
            (Key::Named(NamedKey::Escape), _) | (_, PhysicalKey::Code(KeyCode::Escape)) => {
                self.closed = true;
            }
            (Key::Named(NamedKey::Space), _) | (_, PhysicalKey::Code(KeyCode::Space)) => {
                if let Some(w) = &self.worker {
                    let paused = w.state.toggle_pause();
                    self.display_times.clear();
                    self.status = if paused {
                        MirrorStatus::Paused
                    } else {
                        MirrorStatus::Capturing
                    };
                    self.update_title();
                }
            }
            (Key::Character(s), _) if s.eq_ignore_ascii_case("s") => {
                match self.save_snapshot(None) {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("screenshot.png");
                        self.set_notice(format!("Saved {name}"));
                    }
                    Err(e) => {
                        self.set_notice(format!("Error: {e}"));
                    }
                }
            }
            (Key::Named(NamedKey::F12), _)
            | (_, PhysicalKey::Code(KeyCode::F12 | KeyCode::KeyS)) => {
                match self.save_snapshot(None) {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("screenshot.png");
                        self.set_notice(format!("Saved {name}"));
                    }
                    Err(e) => {
                        self.set_notice(format!("Error: {e}"));
                    }
                }
            }
            (Key::Character(s), _) if s == "1" => self.set_integer_scale(1),
            (Key::Character(s), _) if s == "2" => self.set_integer_scale(2),
            (Key::Character(s), _) if s == "3" => self.set_integer_scale(3),
            (Key::Character(s), _) if s == "4" => self.set_integer_scale(4),
            (_, PhysicalKey::Code(KeyCode::Digit1)) => self.set_integer_scale(1),
            (_, PhysicalKey::Code(KeyCode::Digit2)) => self.set_integer_scale(2),
            (_, PhysicalKey::Code(KeyCode::Digit3)) => self.set_integer_scale(3),
            (_, PhysicalKey::Code(KeyCode::Digit4)) => self.set_integer_scale(4),
            _ => {}
        }
    }
}

impl ApplicationHandler<MirrorCustomEvent> for MirrorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let scale = self.current_scale;
        let size = LogicalSize::new(LOGICAL_WIDTH * scale, LOGICAL_HEIGHT * scale);
        let min_size = LogicalSize::new(LOGICAL_WIDTH, LOGICAL_HEIGHT);

        let mut attrs = Window::default_attributes()
            .with_title(self.dynamic_title())
            .with_inner_size(size)
            .with_min_inner_size(min_size);

        if self.config.fullscreen {
            attrs = attrs.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("error: failed to create mirror window: {e}");
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

        window.request_redraw();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: MirrorCustomEvent) {
        match event {
            MirrorCustomEvent::NewFrame => {
                let frame_opt = if let Some(w) = &self.worker {
                    w.state.notification_pending.store(false, Ordering::Release);
                    let mut latest = w.state.latest_frame.lock().unwrap();
                    if w.state.is_paused() {
                        latest.take();
                        None
                    } else {
                        latest.take()
                    }
                } else {
                    None
                };
                let changed = frame_opt.is_some();
                if let Some(frame) = frame_opt {
                    if let Some(d) = &mut self.diagnostics {
                        if let Err(e) = d.notified(&frame) {
                            eprintln!("warning: notification diagnostics failed: {e}");
                        }
                    }
                    self.last_frame = Some(frame);
                    if !self.initial_snapshot_saved && self.config.snapshot.is_some() {
                        match self.save_snapshot(None) {
                            Ok(path) => {
                                println!("Saved initial screen snapshot: {}", path.display());
                                self.initial_snapshot_saved = true;
                            }
                            Err(e) => self.set_notice(e),
                        }
                    }
                }
                if let Some(w) = &self.worker {
                    self.status = if w.state.is_paused() {
                        MirrorStatus::Paused
                    } else {
                        w.state.get_status()
                    };
                    w.state.metrics.lock().unwrap().displayed_fps =
                        rolling_rate(&self.display_times, Instant::now());
                }
                self.update_title();
                if changed {
                    if let Some(win) = &self.window {
                        win.request_redraw();
                    }
                }
            }
            MirrorCustomEvent::StatusChanged(new_status) => {
                if new_status == MirrorStatus::Paused
                    && self.worker.as_ref().is_some_and(|w| !w.state.is_paused())
                {
                    return;
                }
                if matches!(
                    new_status,
                    MirrorStatus::Paused
                        | MirrorStatus::Disconnected(_)
                        | MirrorStatus::Reconnecting
                ) {
                    self.display_times.clear();
                }
                self.status = new_status;
                self.update_title();
                if let Some(win) = &self.window {
                    win.request_redraw();
                }
            }
            MirrorCustomEvent::WorkerStopped => {
                self.status = MirrorStatus::Stopped;
                self.close(event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.closed {
            self.close(event_loop);
            return;
        }

        let now = Instant::now();
        let mut deadline = self.config.exit_after.map(|limit| self.started + limit);
        if matches!(
            self.status,
            MirrorStatus::Connected | MirrorStatus::Capturing
        ) {
            // Health refresh only: static images never require a redraw.
            self.update_title();
            let health = now + Duration::from_secs(1);
            deadline = Some(deadline.map_or(health, |d| d.min(health)));
        }
        if deadline.is_some_and(|d| now >= d) {
            self.close(event_loop);
            return;
        }
        if let Some((_, at)) = &self.notice {
            let expiry = *at + Duration::from_secs(3);
            if now >= expiry {
                self.notice = None;
                self.update_title();
            } else {
                deadline = Some(deadline.map_or(expiry, |d| d.min(expiry)));
            }
        }
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
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

                if self.surface_size != Some((width.get(), height.get())) {
                    if let Err(e) = surface.resize(width, height) {
                        eprintln!("warning: surface resize failed: {e}");
                        return;
                    }
                    self.surface_size = Some((width.get(), height.get()));
                }

                self.viewport = Viewport::calculate(
                    win_size.width,
                    win_size.height,
                    self.config.integer_scaling && self.config.filter == ScaleFilter::Nearest,
                );

                let mut buffer = match surface.buffer_mut() {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("warning: surface buffer_mut failed: {e}");
                        return;
                    }
                };

                let border_color = if matches!(
                    self.status,
                    MirrorStatus::Disconnected(_) | MirrorStatus::Reconnecting
                ) {
                    STALE_BG_COLOR
                } else {
                    self.config.bg_color
                };

                let scaling_start = Instant::now();
                if let Some(frame) = &self.last_frame {
                    blit_filtered(
                        frame.grob.pixels(),
                        frame.grob.width(),
                        frame.grob.height(),
                        &mut buffer,
                        win_size.width as usize,
                        win_size.height as usize,
                        &self.viewport,
                        border_color,
                        self.config.filter,
                    );
                } else {
                    // Blank border before first frame
                    buffer.fill(border_color);
                }

                let scaling_duration = scaling_start.elapsed();
                if let (Some(d), Some(frame)) = (&mut self.diagnostics, &self.last_frame) {
                    if let Err(e) = d.surface(frame, &buffer, win_size.width, win_size.height) {
                        eprintln!("warning: surface evidence failed: {e}");
                    }
                }
                let presenting = Instant::now();
                if let Err(e) = buffer.present() {
                    eprintln!("warning: surface present failed: {e}");
                    return;
                }
                let present_duration = presenting.elapsed();
                let now = Instant::now();
                if let Some(frame) = &self.last_frame {
                    let unique = self.presented_seq != Some(frame.seq);
                    let age = now.duration_since(frame.timestamp);
                    if unique {
                        self.presented_seq = Some(frame.seq);
                        self.displayed_count += 1;
                        push_time(&mut self.display_times, now);
                    }
                    if let Some(w) = &self.worker {
                        let mut m = w.state.metrics.lock().unwrap();
                        if unique {
                            m.total_displayed += 1;
                        }
                        m.displayed_fps = rolling_rate(&self.display_times, now);
                        m.frame_age = age;
                        m.last_render_duration = scaling_duration + present_duration;
                    }
                    if let Some(d) = &mut self.diagnostics {
                        if let Err(e) = d.presented(
                            frame,
                            scaling_duration,
                            present_duration,
                            age,
                            win_size.width,
                            win_size.height,
                            window.scale_factor(),
                            unique,
                        ) {
                            eprintln!("warning: presentation diagnostics failed: {e}");
                            self.diagnostics = None;
                        }
                    }
                }
                self.update_title();
            }

            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }

            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.close(event_loop);
            }

            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_key(&event.logical_key, event.physical_key, event.state);
                if self.closed {
                    self.close(event_loop);
                }
            }

            _ => {}
        }
    }
}

/// Runs the mirror desktop window with a generic `MirrorSource`.
pub fn run_mirror<S: MirrorSource>(
    config: MirrorConfig,
    source: S,
) -> Result<(), Box<dyn std::error::Error>> {
    if !(1..=4).contains(&config.scale) || frame_interval(config.fps).is_none() {
        return Err("scale must be 1–4 and fps must be positive, finite, and representable".into());
    }
    let diagnostics = config
        .diagnostics
        .as_deref()
        .map(DisplayDiagnostics::open)
        .transpose()?;
    let event_loop = EventLoop::<MirrorCustomEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    let worker = MirrorWorkerHandle::spawn_configured(
        source,
        config.fps,
        Some(proxy),
        config.diagnostics.as_deref(),
        None,
    )?;
    let mut app = MirrorApp::new(config, Some(worker));
    app.diagnostics = diagnostics;

    event_loop.run_app(&mut app)?;
    Ok(())
}

/// Runs the mirror desktop window connected to a physical HP Prime.
pub fn run_mirror_usb(
    config: MirrorConfig,
    prime: Prime,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = UsbMirrorSource::new(prime)
        .with_quality(config.quality)
        .with_capture(config.capture)?;
    run_mirror(config, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_config_defaults() {
        let cfg = MirrorConfig::default();
        assert_eq!(cfg.scale, 2);
        assert_eq!(cfg.fps, 10.0);
        assert!(!cfg.fullscreen);
        assert!(cfg.snapshot.is_none());
        assert_eq!(cfg.title, "HP Prime G1 — Live Screen");
    }

    #[test]
    fn test_mirror_worker_delivers_frames_and_drops_outdated() {
        let colors = [0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00];
        let mock = MockMirrorSource::new("Mock G1").with_solid_frames(&colors);
        let mut worker = MirrorWorkerHandle::spawn(mock, 100.0, None);

        // Allow worker to run several iterations
        std::thread::sleep(Duration::from_millis(50));

        let latest = worker.state.latest_frame.lock().unwrap().take();
        assert!(latest.is_some(), "worker should have produced a frame");

        let metrics = worker.state.metrics.lock().unwrap().clone();
        assert!(metrics.total_captured > 0);

        worker.stop();
        worker.join_timeout(Duration::from_millis(500));
        assert!(worker.state.is_stopped());
    }

    #[test]
    fn test_mirror_worker_pause_resume() {
        let mock = MockMirrorSource::new("Mock G1").with_solid_frames(&[0x112233]);
        let mut worker = MirrorWorkerHandle::spawn(mock, 20.0, None);

        std::thread::sleep(Duration::from_millis(30));
        assert!(!worker.state.is_paused());

        // Pause
        assert!(worker.state.toggle_pause());
        assert!(worker.state.is_paused());
        std::thread::sleep(Duration::from_millis(30));

        // Resume
        assert!(!worker.state.toggle_pause());
        assert!(!worker.state.is_paused());

        worker.stop();
        worker.join_timeout(Duration::from_millis(500));
    }

    #[test]
    fn test_mirror_app_headless_shortcuts() {
        let cfg = MirrorConfig::default();
        let mut app = MirrorApp::new(cfg, None);

        // Escape closes
        assert!(!app.closed);
        app.handle_key(
            &Key::Named(NamedKey::Escape),
            PhysicalKey::Code(KeyCode::Escape),
            ElementState::Pressed,
        );
        assert!(app.closed);

        // Scale changes
        app.set_integer_scale(1);
        assert_eq!(app.current_scale, 1);
        app.set_integer_scale(3);
        assert_eq!(app.current_scale, 3);
        app.set_integer_scale(4);
        assert_eq!(app.current_scale, 4);
    }

    #[test]
    fn test_mirror_app_save_snapshot() {
        let snap_path =
            std::env::temp_dir().join(format!("test-prime-snap-{}.png", std::process::id()));
        if snap_path.exists() {
            let _ = std::fs::remove_file(&snap_path);
        }

        let cfg = MirrorConfig {
            snapshot: Some(snap_path.clone()),
            ..MirrorConfig::default()
        };
        let mut app = MirrorApp::new(cfg, None);

        // Before any frame, saving returns an error
        assert!(app.save_snapshot(None).is_err());

        // Supply a mock frame
        let grob = Arc::new(Grob::new(320, 240, 0x00FF_0000)); // Solid Red
        app.last_frame = Some(CapturedFrame {
            grob: grob.clone(),
            seq: 1,
            timestamp: Instant::now(),
            published: Instant::now(),
            capture_duration: Duration::from_millis(50),
            decode_duration: Duration::from_millis(5),
        });

        // Save snapshot
        let res = app.save_snapshot(None);
        assert!(res.is_ok());
        assert!(snap_path.exists());

        // Verify saved PNG can be read back and matches
        let bytes = std::fs::read(&snap_path).unwrap();
        let decoded = Grob::read_png(&bytes).unwrap();
        assert_eq!(decoded.width(), 320);
        assert_eq!(decoded.height(), 240);
        assert_eq!(decoded.get(10, 10), Some(0x00FF_0000));
        let _ = std::fs::remove_file(&snap_path);
    }

    #[test]
    fn test_mirror_status_titles() {
        let cfg = MirrorConfig::default();
        let mut app = MirrorApp::new(cfg, None);

        app.status = MirrorStatus::Paused;
        assert!(app.dynamic_title().contains("Paused"));

        app.status = MirrorStatus::Disconnected("Device I/O error".into());
        assert!(app.dynamic_title().contains("Disconnected"));

        app.status = MirrorStatus::Reconnecting;
        assert!(app.dynamic_title().contains("Reconnecting"));

        app.set_notice("Saved screenshot.png");
        assert!(app.dynamic_title().contains("Saved screenshot.png"));

        // Capture status with worker metrics
        let mock = MockMirrorSource::new("Mock G1").with_solid_frames(&[0x111111]);
        let worker = MirrorWorkerHandle::spawn(mock, 10.0, None);
        {
            let mut m = worker.state.metrics.lock().unwrap();
            m.rolling_fps = 5.0;
            m.published_fps = 0.0;
            m.total_unchanged = 10;
        }
        app.worker = Some(worker);
        app.status = MirrorStatus::Capturing;
        app.notice = None;
        assert_eq!(
            app.dynamic_title(),
            "HP Prime G1 — Live Screen | 5.0 capture/s | 0.0 changed/s | 0.0 presented/s"
        );
    }

    #[test]
    fn test_frame_order_and_monotonic_seq() {
        let colors = [0x111111, 0x222222, 0x333333, 0x444444];
        let mock = MockMirrorSource::new("Mock G1").with_solid_frames(&colors);
        let mut worker = MirrorWorkerHandle::spawn(mock, 100.0, None);

        let mut seen_seqs = Vec::new();
        let start = Instant::now();
        while seen_seqs.len() < 3 && start.elapsed() < Duration::from_millis(500) {
            if let Some(frame) = worker.state.latest_frame.lock().unwrap().take() {
                if seen_seqs.last() != Some(&frame.seq) {
                    seen_seqs.push(frame.seq);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        worker.stop();
        worker.join_timeout(Duration::from_millis(500));

        assert!(seen_seqs.len() >= 2);
        for w in seen_seqs.windows(2) {
            assert!(w[0] < w[1], "frame sequences must be strictly monotonic");
        }
    }

    #[test]
    fn test_worker_handles_error_and_reconnects() {
        let mut mock = MockMirrorSource::new("Mock G1");
        // Frame 1 ok, Frame 2 error, Frame 3 ok
        mock.frames.push_back(Ok(Grob::screen()));
        mock.frames
            .push_back(Err(DeviceError::Hid("cable unplugged".into())));
        mock.frames.push_back(Ok(Grob::screen()));
        mock.reconnect_succeeds = true;

        let mut worker = MirrorWorkerHandle::spawn(mock, 50.0, None);

        // Wait for worker to encounter error and attempt reconnect
        let start = Instant::now();
        let mut saw_error_or_reconnect = false;
        while start.elapsed() < Duration::from_secs(2) {
            let s = worker.state.get_status();
            if matches!(
                s,
                MirrorStatus::Disconnected(_) | MirrorStatus::Reconnecting
            ) {
                saw_error_or_reconnect = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        worker.stop();
        worker.join_timeout(Duration::from_millis(500));

        assert!(
            saw_error_or_reconnect,
            "worker must enter disconnected or reconnecting state"
        );
    }

    #[test]
    fn test_viewport_aspect_ratio_and_letterboxing() {
        // Standard 4:3 window: 640x480 -> integer scale 2, perfect fit
        let vp = Viewport::calculate(640, 480, true);
        assert_eq!(vp.width, 640);
        assert_eq!(vp.height, 480);
        assert_eq!(vp.x, 0);
        assert_eq!(vp.y, 0);
        assert_eq!(vp.scale, 2);

        // Wide window: 800x480 -> pillarboxing (bars left and right)
        let vp_wide = Viewport::calculate(800, 480, true);
        assert_eq!(vp_wide.width, 640);
        assert_eq!(vp_wide.height, 480);
        assert_eq!(vp_wide.x, (800 - 640) / 2);
        assert_eq!(vp_wide.y, 0);

        // Tall window: 640x600 -> letterboxing (bars top and bottom)
        let vp_tall = Viewport::calculate(640, 600, true);
        assert_eq!(vp_tall.width, 640);
        assert_eq!(vp_tall.height, 480);
        assert_eq!(vp_tall.x, 0);
        assert_eq!(vp_tall.y, (600 - 480) / 2);
    }

    #[test]
    fn test_last_frame_preserved_on_disconnect() {
        let cfg = MirrorConfig::default();
        let mut app = MirrorApp::new(cfg, None);

        let red_grob = Arc::new(Grob::new(320, 240, 0x00FF_0000));
        app.last_frame = Some(CapturedFrame {
            grob: red_grob.clone(),
            seq: 42,
            timestamp: Instant::now(),
            published: Instant::now(),
            capture_duration: Duration::from_millis(40),
            decode_duration: Duration::from_millis(3),
        });

        // Simulating disconnect event
        app.status = MirrorStatus::Disconnected("Device lost".into());

        // Frame must still be preserved
        assert!(app.last_frame.is_some());
        assert_eq!(app.last_frame.as_ref().unwrap().seq, 42);
        assert_eq!(
            app.last_frame.as_ref().unwrap().grob.get(0, 0),
            Some(0x00FF_0000)
        );
    }
    fn assert_screenshot_pipeline(raw: &[u8]) {
        let standalone = Grob::from_prime_screenshot(raw).unwrap();
        let mirrored = ppl_graphics::PrimeScreenshotDecoder::default()
            .decode(raw, Some((320, 240)))
            .unwrap()
            .0;
        assert_eq!(standalone.pixels(), mirrored.pixels());
        let mut surface = vec![0; 320 * 240];
        blit_filtered(
            mirrored.pixels(),
            320,
            240,
            &mut surface,
            320,
            240,
            &Viewport::calculate_aspect_fit(320, 240),
            0,
            ScaleFilter::Nearest,
        );
        assert_eq!(surface, standalone.pixels());
    }

    #[test]
    fn standalone_mirror_and_unscaled_surface_match_the_same_png() {
        assert_screenshot_pipeline(include_bytes!(
            "../../../tests/fixtures/prime-rgb555-le.png"
        ));
    }

    #[test]
    #[ignore = "set PPL_MIRROR_FIXTURE to a preserved private raw hardware screenshot"]
    fn private_hardware_fixture_matches_the_standalone_decoder() {
        let path = std::env::var_os("PPL_MIRROR_FIXTURE").expect("PPL_MIRROR_FIXTURE is required");
        assert_screenshot_pipeline(&std::fs::read(path).unwrap());
    }

    #[test]
    fn unchanged_captures_are_counted_without_republication_or_unbounded_memory() {
        let source = MockMirrorSource::new("static");
        let mut worker =
            MirrorWorkerHandle::spawn_configured(source, 1000.0, None, None, Some(20)).unwrap();
        while !worker.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        worker.join();
        let m = worker.state.metrics.lock().unwrap();
        assert_eq!(m.total_captured, 20);
        assert_eq!(m.total_published, 1);
        assert_eq!(m.total_unchanged, 19);
        assert_eq!(m.total_dropped, 0);
        assert_eq!(m.total_displayed, 0);
        assert!(worker.state.latest_frame.lock().unwrap().is_some());
    }

    #[test]
    fn slow_captures_do_not_receive_an_additional_frame_interval() {
        let mut source = MockMirrorSource::new("slow");
        source.capture_delay = Duration::from_millis(40);
        let start = Instant::now();
        let mut worker =
            MirrorWorkerHandle::spawn_configured(source, 100.0, None, None, Some(3)).unwrap();
        while !worker.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        worker.join();
        assert!(start.elapsed() < Duration::from_millis(200));
        assert_eq!(worker.state.metrics.lock().unwrap().total_captured, 3);
        assert!(frame_interval(f64::NAN).is_none());
        assert!(frame_interval(0.0).is_none());
    }
    #[test]
    fn worker_reconnects_after_error_and_releases_owned_source_on_join() {
        struct Observed {
            inner: MockMirrorSource,
            dropped: Arc<AtomicBool>,
        }
        impl Drop for Observed {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::Release);
            }
        }
        impl MirrorSource for Observed {
            fn capture_frame(&mut self) -> Result<Grob, DeviceError> {
                self.inner.capture_frame()
            }
            fn device_name(&self) -> &str {
                self.inner.device_name()
            }
            fn device_serial(&self) -> Option<&str> {
                self.inner.device_serial()
            }
            fn try_reconnect(&mut self) -> Result<bool, DeviceError> {
                self.inner.try_reconnect()
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let mut inner = MockMirrorSource::new("recover");
        inner
            .frames
            .push_back(Err(DeviceError::Hid("unplugged".into())));
        inner.frames.push_back(Ok(Grob::screen()));
        let source = Observed {
            inner,
            dropped: dropped.clone(),
        };
        let mut worker =
            MirrorWorkerHandle::spawn_configured(source, 100.0, None, None, Some(2)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.is_finished());
        worker.join();
        assert!(dropped.load(Ordering::Acquire));
        let m = worker.state.metrics.lock().unwrap();
        assert_eq!(m.errors, 1);
        assert_eq!(m.total_captured, 2);
    }
}

#[cfg(test)]
mod incremental_recovery_tests {
    use super::*;
    struct FailedSource(Arc<std::sync::atomic::AtomicUsize>);
    impl MirrorSource for FailedSource {
        fn capture_frame(&mut self) -> Result<Grob, DeviceError> {
            Err(DeviceError::Protocol("synthetic lost sync".into()))
        }
        fn device_name(&self) -> &str {
            "synthetic"
        }
        fn device_serial(&self) -> Option<&str> {
            None
        }
        fn try_reconnect(&mut self) -> Result<bool, DeviceError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        }
        fn recovery_attempt_limit(&self) -> Option<usize> {
            Some(3)
        }
    }
    #[test]
    fn consecutive_recovery_failures_stop_usb_attempts_but_keep_stale_view_open() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut worker = MirrorWorkerHandle::spawn(FailedSource(attempts.clone()), 10.0, None);
        let deadline = Instant::now() + Duration::from_secs(6);
        while worker.state.metrics.lock().unwrap().errors < 4 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        assert!(!worker.is_finished()); // stale window remains available until user closes
        worker.stop();
        worker.join();
    }
}

#[cfg(test)]
mod incremental_baseline_recovery_tests {
    use super::*;
    struct BaselineOnly {
        baseline: bool,
        attempts: Arc<std::sync::atomic::AtomicUsize>,
    }
    impl MirrorSource for BaselineOnly {
        fn capture_frame(&mut self) -> Result<Grob, DeviceError> {
            unreachable!()
        }
        fn capture(&mut self, _: &AtomicBool) -> Result<SourceFrame, DeviceError> {
            if !self.baseline {
                return Err(DeviceError::Protocol(
                    "synthetic bad incremental update".into(),
                ));
            }
            self.baseline = false;
            Ok(SourceFrame {
                grob: Arc::new(Grob::screen()),
                requested: Instant::now(),
                completed: Instant::now(),
                usb: Default::default(),
                decode: Default::default(),
                raw: None,
                update: Some(UpdateDiagnostics {
                    baseline: true,
                    ..Default::default()
                }),
            })
        }
        fn device_name(&self) -> &str {
            "synthetic"
        }
        fn device_serial(&self) -> Option<&str> {
            None
        }
        fn try_reconnect(&mut self) -> Result<bool, DeviceError> {
            self.attempts.fetch_add(1, Ordering::Relaxed);
            self.baseline = true;
            Ok(true)
        }
        fn recovery_attempt_limit(&self) -> Option<usize> {
            Some(3)
        }
    }
    #[test]
    fn a_successful_baseline_does_not_reset_a_repeatedly_failing_incremental_budget() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut worker = MirrorWorkerHandle::spawn(
            BaselineOnly {
                baseline: true,
                attempts: attempts.clone(),
            },
            10.0,
            None,
        );
        let deadline = Instant::now() + Duration::from_secs(6);
        while worker.state.metrics.lock().unwrap().errors < 4 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(worker.state.metrics.lock().unwrap().errors, 4);
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
        worker.stop();
        worker.join();
    }
}

#[cfg(test)]
mod rate_acceptance {
    use super::*;
    #[test]
    fn duplicate_gaps_decay_rates_and_static_rates_expire() {
        let start = Instant::now();
        let times = VecDeque::from([start, start + Duration::from_millis(100)]);
        assert_eq!(
            rolling_rate(&times, start + Duration::from_millis(100)),
            10.0
        );
        assert_eq!(
            rolling_rate(&times, start + Duration::from_millis(500)),
            2.0
        );
        assert_eq!(rolling_rate(&times, start + Duration::from_secs(3)), 0.0);
    }
    #[test]
    fn controlled_duplicates_publish_only_actual_changes() {
        let mut source = MockMirrorSource::new("controlled")
            .with_solid_frames(&[0x112233, 0x112233, 0x445566, 0x445566]);
        source.capture_delay = Duration::from_millis(1);
        let mut worker =
            MirrorWorkerHandle::spawn_configured(source, 1000.0, None, None, Some(4)).unwrap();
        while !worker.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        worker.join();
        let metrics = worker.state.metrics.lock().unwrap();
        assert_eq!(metrics.total_captured, 4);
        assert_eq!(metrics.total_published, 2);
        assert_eq!(metrics.total_unchanged, 2);
        assert_eq!(
            worker
                .state
                .latest_frame
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .grob
                .get(0, 0),
            Some(0x445566)
        );
    }
}
