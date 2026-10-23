//! IDE desktop architecture, shared state, and panel orchestration.

pub mod automation;
pub mod console;
pub mod debugger;
pub mod diagnostics;
pub mod dialogs;
pub mod display;
pub mod editor;
pub mod execution;
pub mod explorer;
pub mod keypad;
pub mod menu;
pub mod problems;
pub mod status;
pub mod syntax;
pub mod toolbar;

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

/// Common interface for modular IDE panels.
pub trait IdePanel {
    /// Renders panel content into the given UI and updates application state.
    fn render(&mut self, ui: &mut egui::Ui, state: &mut IdeAppState);
}

/// Dock tabs available in the bottom dock panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum DockTab {
    #[default]
    Output,
    Problems,
    Variables,
    CallStack,
}

/// Serializable subset of IDE state persisted across application runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersistentPanelState {
    pub show_explorer: bool,
    pub show_calculator: bool,
    pub show_bottom_dock: bool,
    pub show_status_bar: bool,
    pub active_dock_tab: DockTab,
    pub explorer_width: f32,
    pub calculator_width: f32,
    pub bottom_height: f32,
    /// Code editor font size (points). Absent in older layout files.
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: f32,
}

fn default_editor_font_size() -> f32 {
    syntax::CodeStyle::DEFAULT_FONT_SIZE
}

/// Smallest and largest editor font sizes offered by zoom and settings.
pub const EDITOR_FONT_RANGE: std::ops::RangeInclusive<f32> = 9.0..=28.0;

impl Default for PersistentPanelState {
    fn default() -> Self {
        Self {
            show_explorer: true,
            show_calculator: true,
            show_bottom_dock: true,
            show_status_bar: true,
            active_dock_tab: DockTab::Output,
            explorer_width: 240.0,
            calculator_width: 360.0,
            bottom_height: 180.0,
            editor_font_size: default_editor_font_size(),
        }
    }
}

impl DockTab {
    pub fn title(&self) -> &'static str {
        match self {
            DockTab::Output => "Output",
            DockTab::Problems => "Problems",
            DockTab::Variables => "Variables",
            DockTab::CallStack => "Call Stack",
        }
    }
}

/// Current status of the PPL VM execution worker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VmExecutionState {
    #[default]
    Idle,
    Running,
    Paused,
    Terminated,
    Error(String),
}

impl VmExecutionState {
    pub fn label(&self) -> &'static str {
        match self {
            VmExecutionState::Idle => "Idle",
            VmExecutionState::Running => "Running",
            VmExecutionState::Paused => "Paused",
            VmExecutionState::Terminated => "Done",
            VmExecutionState::Error(_) => "Error",
        }
    }
}

/// Pending user actions queued by menus, toolbars, or keyboard shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdeAction {
    NewProgram,
    OpenFile(Option<PathBuf>),
    SaveActiveFile,
    SaveActiveFileAs,
    CloseActiveTab,
    RunProgram,
    StopProgram,
    RestartProgram,
    StartDebugging,
    ContinueExecution,
    StepInto,
    StepOver,
    StepOut,
    ToggleBreakpoint(usize),
    CheckSyntax,
    FormatSource,
    ToggleExplorer,
    ToggleCalculator,
    ToggleBottomDock,
    ToggleStatusBar,
    ResetLayout,
    ShowAboutDialog,
    ShowSettingsDialog,
    ShowNewProgramDialog,
    ShowGoToLineDialog,
    /// Open the editor's find bar (Ctrl+F).
    ShowFind,
    /// Open the editor's find bar with the replace row (Ctrl+H).
    ShowFindReplaceDialog,
    /// Run an editing command on the active editor (Edit menu).
    Edit(EditCommand),
    /// Open the run configuration dialog (entry point + arguments).
    ShowRunConfigDialog,
    /// Increase (1) / decrease (-1) / reset (0) the editor font size.
    ZoomEditorFont(i8),
    TakeScreenshot,
    /// Navigates to a line (1-based) and column (1-based) in the given tab
    /// (`None`: the active tab).
    NavigateToLocation {
        tab: Option<editor::TabId>,
        line: usize,
        col: usize,
    },
}

/// Editing commands offered by the Edit menu. They are replayed into the
/// editor as the corresponding keyboard input, so menu and keyboard share
/// one code path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Indent,
    Outdent,
    ToggleComment,
    TriggerCompletion,
}

/// Global state shared across all IDE panels and desktop components.
#[derive(Debug)]
pub struct IdeAppState {
    // Panel visibility toggles
    pub show_explorer: bool,
    pub show_calculator: bool,
    pub show_bottom_dock: bool,
    pub show_status_bar: bool,

    // Active bottom dock tab
    pub active_dock_tab: DockTab,

    // Panel dimensions
    pub explorer_width: f32,
    pub calculator_width: f32,
    pub bottom_height: f32,

    // Workspace & file management
    pub workspace_root: Option<PathBuf>,
    pub recent_files: Vec<PathBuf>,

    // Execution & metrics
    pub vm_state: VmExecutionState,
    pub step_count: u64,
    pub elapsed_time: Duration,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub display_scale: f32,
    pub status_message: String,
    /// Line the debugger is paused on (0 = none); highlights in the editor.
    pub paused_line: usize,
    /// When true, PC keyboard input is routed to the VM (calculator focus)
    /// instead of the code editor.
    pub calc_input_focus: bool,
    /// Monospaced editor font size.
    pub editor_font_size: f32,

    /// Queue of pending HP keycodes from virtual keypad clicks or PC keyboard.
    pub key_events: Vec<i64>,
    /// Set of HP keycodes currently held down (for ISKEYDOWN).
    pub keys_down: HashSet<i64>,

    // Queue of pending actions
    pub pending_actions: Vec<IdeAction>,
}

impl Default for IdeAppState {
    fn default() -> Self {
        Self {
            show_explorer: true,
            show_calculator: true,
            show_bottom_dock: true,
            show_status_bar: true,
            active_dock_tab: DockTab::Output,
            explorer_width: 240.0,
            calculator_width: 360.0,
            bottom_height: 180.0,
            workspace_root: std::env::current_dir().ok(),
            recent_files: Vec::new(),
            vm_state: VmExecutionState::Idle,
            step_count: 0,
            elapsed_time: Duration::ZERO,
            cursor_line: 1,
            cursor_col: 1,
            display_scale: 1.0,
            status_message: "Ready".to_string(),
            paused_line: 0,
            calc_input_focus: false,
            editor_font_size: default_editor_font_size(),
            key_events: Vec::new(),
            keys_down: HashSet::new(),
            pending_actions: Vec::new(),
        }
    }
}

impl IdeAppState {
    pub fn to_persistent(&self) -> PersistentPanelState {
        PersistentPanelState {
            show_explorer: self.show_explorer,
            show_calculator: self.show_calculator,
            show_bottom_dock: self.show_bottom_dock,
            show_status_bar: self.show_status_bar,
            active_dock_tab: self.active_dock_tab,
            explorer_width: self.explorer_width,
            calculator_width: self.calculator_width,
            bottom_height: self.bottom_height,
            editor_font_size: self.editor_font_size,
        }
    }

    pub fn apply_persistent(&mut self, p: &PersistentPanelState) {
        self.show_explorer = p.show_explorer;
        self.show_calculator = p.show_calculator;
        self.show_bottom_dock = p.show_bottom_dock;
        self.show_status_bar = p.show_status_bar;
        self.active_dock_tab = p.active_dock_tab;
        self.explorer_width = p.explorer_width;
        self.calculator_width = p.calculator_width;
        self.bottom_height = p.bottom_height;
        self.editor_font_size = p
            .editor_font_size
            .clamp(*EDITOR_FONT_RANGE.start(), *EDITOR_FONT_RANGE.end());
    }

    /// Resets all panel toggles and sizes to their default desktop layout.
    pub fn reset_layout(&mut self) {
        self.show_explorer = true;
        self.show_calculator = true;
        self.show_bottom_dock = true;
        self.show_status_bar = true;
        self.explorer_width = 240.0;
        self.calculator_width = 360.0;
        self.bottom_height = 180.0;
    }

    /// Queues an action for handling in the next frame cycle.
    pub fn queue_action(&mut self, action: IdeAction) {
        self.pending_actions.push(action);
    }

    /// Takes all pending actions from the queue.
    pub fn take_actions(&mut self) -> Vec<IdeAction> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Pushes an HP keycode into the application key events queue.
    pub fn push_key_event(&mut self, code: i64) {
        self.key_events.push(code);
    }

    /// Sets or clears the held state of a key in the application state.
    pub fn set_key_down(&mut self, code: i64, down: bool) {
        if down {
            self.keys_down.insert(code);
        } else {
            self.keys_down.remove(&code);
        }
    }

    /// Clears all held keys in the application state.
    pub fn clear_held_keys(&mut self) {
        self.keys_down.clear();
    }
}
