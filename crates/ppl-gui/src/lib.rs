//! Native GUI frontend for the HP Prime PPL Emulator.
//!
//! Provides a 320×240 pixel-perfect emulator display scaled to modern windows,
//! PC keyboard to HP Prime keycode mapping (0–50 grid), mouse touch input,
//! and a non-blocking background worker thread executing PPL programs with
//! full window event loop responsiveness.

pub mod app;
pub mod dialog;
pub mod host;
pub mod ide;
pub mod keymap;
pub mod mirror;
pub mod scaler;
pub mod script;
pub mod theme;
pub mod window;

pub use app::{IdeDesktopApp, LaunchOptions, launch_ide, launch_ide_with};
pub use dialog::{KeyPress, ModalDialog, make_dialog};
pub use host::{GuiHost, GuiSharedState, IdeSharedState};
pub use ide::execution::worker::{WorkerConfig, WorkerHandle, WorkerOutcome};
pub use ide::{DockTab, IdeAction, IdeAppState, IdePanel, VmExecutionState};
pub use keymap::{
    KEY_MAPPINGS, KeyMapping, egui_key_to_hp, hp_key_name, lookup_by_device_code, lookup_by_pc_key,
    lookup_by_physical, lookup_by_ppl_code, map_char, map_winit_key,
};
pub use mirror::{
    CapturedFrame, MirrorApp, MirrorCapture, MirrorConfig, MirrorCustomEvent, MirrorMetrics,
    MirrorQuality, MirrorSource, MirrorStatus, MockMirrorSource, UsbMirrorSource, run_mirror,
    run_mirror_usb,
};
pub use scaler::{
    ASPECT_RATIO, LOGICAL_HEIGHT, LOGICAL_WIDTH, ScaleFilter, Viewport, blit_filtered, blit_scaled,
    window_to_framebuffer,
};
pub use script::{KeyFeeder, KeyScript, ScriptItem};
pub use window::{
    CLOSE_JOIN_TIMEOUT, GuiApp, GuiAppConfig, GuiCustomEvent, GuiSession, run_gui, run_gui_session,
};
