//! VM execution machinery for the IDE and the standalone window.
//!
//! * [`controller`] — the execution coordinator owned by the IDE
//!   (`ExecutionController`), the debug bridge (`VmBridge`) and the worker's
//!   debugger (`WorkerDebugger`).
//! * [`worker`] — the background VM worker thread (`WorkerHandle`,
//!   `WorkerConfig`, `WorkerOutcome`).

pub mod controller;
pub mod worker;

pub use controller::{
    ControllerCommand, DebugCommand, DebugPauseSnapshot, ExecutionController, RunStart,
    StackFrameSnapshot, VariableDetailSnapshot, VariableSnapshot, VmBridge, make_worker_debugger,
};
pub use worker::{FinishNotifier, WorkerConfig, WorkerHandle, WorkerOutcome};
