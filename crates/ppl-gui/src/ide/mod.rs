//! IDE desktop architecture, shared state, and panel orchestration.

pub mod automation;
pub mod calculator;
pub mod compare;
pub mod console;
pub mod debugger;
pub mod device;
pub mod diagnostics;
pub mod dialogs;
pub mod display;
pub mod editor;
pub mod execution;
pub mod explorer;
pub mod firmware;
pub mod icons;
pub mod keypad;
pub mod layout;
pub mod menu;
pub mod native;
pub mod problems;
pub mod programs;
pub mod send;
pub mod status;
pub mod syntax;
pub mod toolbar;

use std::collections::{HashMap, HashSet};
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
#[serde(default)]
pub struct PersistentPanelState {
    pub layout: layout::WorkspaceLayout,
    #[serde(default)]
    pub previous_layout: layout::WorkspaceLayout,
    pub calculator_mode: layout::CalculatorMode,
    pub smooth_display: bool,
    pub firmware_backend: bool,
    pub firmware: firmware::FirmwareSettings,
    pub show_explorer: bool,
    pub show_calculator: bool,
    pub show_bottom_dock: bool,
    pub show_status_bar: bool,
    /// Keep the screen comparison dock visible across restarts.
    pub show_compare: bool,
    pub active_dock_tab: DockTab,
    pub explorer_width: f32,
    pub calculator_width: f32,
    pub bottom_height: f32,
    /// Code editor font size (points). Absent in older layout files.
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: f32,
    /// Folder shown in the Explorer (absent: the launch directory).
    pub workspace_root: Option<PathBuf>,
    pub recent_files: Vec<PathBuf>,
    /// Calculator program name each file is sent as.
    pub send_names: HashMap<PathBuf, String>,
}

fn default_editor_font_size() -> f32 {
    syntax::CodeStyle::DEFAULT_FONT_SIZE
}

/// Smallest and largest editor font sizes offered by zoom and settings.
pub const EDITOR_FONT_RANGE: std::ops::RangeInclusive<f32> = 9.0..=28.0;

impl Default for PersistentPanelState {
    fn default() -> Self {
        Self {
            layout: layout::WorkspaceLayout::Split,
            previous_layout: layout::WorkspaceLayout::Split,
            calculator_mode: layout::CalculatorMode::Full,
            smooth_display: false,
            firmware_backend: false,
            firmware: firmware::FirmwareSettings::default(),
            show_explorer: true,
            show_calculator: true,
            show_bottom_dock: true,
            show_status_bar: true,
            show_compare: false,
            active_dock_tab: DockTab::Output,
            explorer_width: 220.0,
            calculator_width: 400.0,
            bottom_height: 180.0,
            editor_font_size: default_editor_font_size(),
            workspace_root: None,
            recent_files: Vec::new(),
            send_names: HashMap::new(),
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

/// What the compiler says about the current revision of a buffer.
///
/// Only `Valid`, `ValidWithWarnings`, `Invalid` and `AnalysisFailed` are
/// verdicts; the others mean the current text has not been analysed yet,
/// so "no problems" must not be claimed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AnalysisState {
    #[default]
    NotAnalyzed,
    /// Changed; waiting for the typing pause before analysis.
    Queued,
    /// Sent to the background compiler.
    Analyzing,
    Valid,
    /// Compiles; carries the number of warnings.
    ValidWithWarnings(usize),
    Invalid {
        errors: usize,
        warnings: usize,
    },
    /// The compiler failed internally: neither valid nor invalid source.
    AnalysisFailed(String),
}

impl AnalysisState {
    /// True once the current revision has a compiler verdict.
    pub fn is_settled(&self) -> bool {
        !matches!(
            self,
            AnalysisState::NotAnalyzed | AnalysisState::Queued | AnalysisState::Analyzing
        )
    }

    /// Short status-bar text.
    pub fn label(&self) -> String {
        match self {
            AnalysisState::NotAnalyzed => "PPL: not analysed".into(),
            AnalysisState::Queued | AnalysisState::Analyzing => "PPL: analysing…".into(),
            AnalysisState::Valid => "PPL: valid".into(),
            AnalysisState::ValidWithWarnings(n) => format!("PPL: valid, {n} warning(s)"),
            AnalysisState::Invalid { errors, .. } => format!("PPL: {errors} error(s)"),
            AnalysisState::AnalysisFailed(_) => "PPL: analysis failed".into(),
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
    /// Dismiss finished Native output without changing its retained outcome/history.
    DismissOutput,
    NativeHome,
    NativeCatalog,
    NativeSubmit,
    NativeRunSelected,
    RestartProgram,
    StartDebugging,
    ContinueExecution,
    StepInto,
    StepOver,
    StepOut,
    ToggleBreakpoint(usize),
    CheckSyntax,
    ManagePrograms,
    /// Save, check and send the active program to the calculator.
    SendToCalculator,
    /// Read the connected calculator's identity (read-only).
    ShowCalculatorStatus,
    /// Read the calculator's program list (read-only, may be incomplete).
    ListCalculatorPrograms,
    /// Read one program from the calculator into a new untitled tab.
    OpenFromCalculator(String),
    /// Capture the calculator screen and show it beside the emulator's.
    CompareScreens,
    /// Choose the folder shown in the Explorer.
    OpenFolder,
    FormatSource,
    ToggleExplorer,
    ToggleCalculator,
    ToggleBottomDock,
    ToggleStatusBar,
    ResetLayout,
    SetLayout(layout::WorkspaceLayout),
    SetCalculatorMode(layout::CalculatorMode),
    SelectBackend(bool),
    ShowShortcuts,
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
    pub native: native::NativeUiState,
    pub input_owner: native::InputOwner,
    pub layout: layout::WorkspaceLayout,
    pub previous_layout: layout::WorkspaceLayout,
    pub calculator_mode: layout::CalculatorMode,
    pub smooth_display: bool,
    pub firmware_backend: bool,
    pub firmware: firmware::FirmwareSettings,
    pub key_transitions: Vec<(i64, bool)>,
    pub firmware_input_enabled: bool,
    // Panel visibility toggles
    pub show_explorer: bool,
    pub show_calculator: bool,
    pub show_bottom_dock: bool,
    pub show_status_bar: bool,
    /// Keep the screen comparison dock visible across restarts.
    pub show_compare: bool,

    // Active bottom dock tab
    pub active_dock_tab: DockTab,

    // Panel dimensions
    pub explorer_width: f32,
    pub calculator_width: f32,
    pub bottom_height: f32,

    // Workspace & file management
    pub workspace_root: Option<PathBuf>,
    pub recent_files: Vec<PathBuf>,
    /// Calculator program name each file is sent as.
    pub send_names: HashMap<PathBuf, String>,

    // Physical calculator
    pub device_status: device::DeviceStatus,
    /// The device job in progress, if any.
    pub device_busy: Option<device::Purpose>,

    // Execution & metrics
    pub vm_state: VmExecutionState,
    /// Compiler verdict for the active tab's current text.
    pub analysis: AnalysisState,
    /// Encoding of the active tab's file, as saved back.
    pub encoding_label: String,
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
            native: native::NativeUiState::default(),
            input_owner: native::InputOwner::Editor,
            layout: layout::WorkspaceLayout::Split,
            previous_layout: layout::WorkspaceLayout::Split,
            calculator_mode: layout::CalculatorMode::Full,
            smooth_display: false,
            firmware_backend: false,
            firmware: firmware::FirmwareSettings::default(),
            show_explorer: true,
            show_calculator: true,
            show_bottom_dock: true,
            show_status_bar: true,
            show_compare: false,
            active_dock_tab: DockTab::Output,
            explorer_width: 220.0,
            calculator_width: 400.0,
            bottom_height: 180.0,
            workspace_root: std::env::current_dir().ok(),
            recent_files: Vec::new(),
            send_names: HashMap::new(),
            device_status: device::DeviceStatus::Unknown,
            device_busy: None,
            vm_state: VmExecutionState::Idle,
            analysis: AnalysisState::NotAnalyzed,
            encoding_label: "UTF-8".into(),
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
            key_transitions: Vec::new(),
            firmware_input_enabled: true,
            keys_down: HashSet::new(),
            pending_actions: Vec::new(),
        }
    }
}

impl IdeAppState {
    pub fn to_persistent(&self) -> PersistentPanelState {
        PersistentPanelState {
            layout: self.layout,
            previous_layout: self.previous_layout,
            calculator_mode: self.calculator_mode,
            smooth_display: self.smooth_display,
            firmware_backend: self.firmware_backend,
            firmware: self.firmware.clone(),
            show_explorer: self.show_explorer,
            show_calculator: self.show_calculator,
            show_bottom_dock: self.show_bottom_dock,
            show_status_bar: self.show_status_bar,
            show_compare: self.show_compare,
            active_dock_tab: self.active_dock_tab,
            explorer_width: self.explorer_width,
            calculator_width: self.calculator_width,
            bottom_height: self.bottom_height,
            editor_font_size: self.editor_font_size,
            workspace_root: self.workspace_root.clone(),
            recent_files: self.recent_files.clone(),
            send_names: self.send_names.clone(),
        }
    }

    pub fn apply_persistent(&mut self, p: &PersistentPanelState) {
        self.layout = p.layout;
        self.previous_layout = if p.previous_layout == layout::WorkspaceLayout::Calculator {
            layout::WorkspaceLayout::Split
        } else {
            p.previous_layout
        };
        self.calculator_mode = p.calculator_mode;
        self.smooth_display = p.smooth_display;
        self.firmware_backend = p.firmware_backend;
        self.firmware = p.firmware.clone();
        self.show_explorer = p.show_explorer;
        self.show_calculator = p.show_calculator;
        self.show_bottom_dock = p.show_bottom_dock;
        self.show_status_bar = p.show_status_bar;
        self.show_compare = p.show_compare;
        self.active_dock_tab = p.active_dock_tab;
        self.explorer_width = layout::valid_size(p.explorer_width, 220.0, 160.0, 600.0);
        self.calculator_width = layout::valid_size(p.calculator_width, 400.0, 280.0, 900.0);
        self.bottom_height = layout::valid_size(p.bottom_height, 180.0, 100.0, 900.0);
        self.editor_font_size = p
            .editor_font_size
            .clamp(*EDITOR_FONT_RANGE.start(), *EDITOR_FONT_RANGE.end());
        if let Some(root) = p.workspace_root.as_ref().filter(|r| r.is_dir()) {
            self.workspace_root = Some(root.clone());
        }
        self.recent_files = p
            .recent_files
            .iter()
            .filter(|f| f.is_file())
            .cloned()
            .collect();
        self.send_names = p.send_names.clone();
    }

    /// Resets all panel toggles and sizes to their default desktop layout.
    pub fn reset_layout(&mut self) {
        self.layout = layout::WorkspaceLayout::Split;
        self.previous_layout = layout::WorkspaceLayout::Split;
        self.calculator_mode = layout::CalculatorMode::Full;
        self.show_explorer = true;
        self.show_calculator = true;
        self.show_bottom_dock = true;
        self.show_status_bar = true;
        self.show_compare = false;
        self.explorer_width = 220.0;
        self.calculator_width = 400.0;
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

    pub fn can_dismiss_output(&self) -> bool {
        !self.firmware_backend
            && matches!(
                self.native.view,
                native::NativeView::ProgramOutput | native::NativeView::Error
            )
            && matches!(
                self.vm_state,
                VmExecutionState::Terminated | VmExecutionState::Error(_)
            )
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
