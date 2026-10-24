//! The PPL virtual machine: values, execution, built-in commands and the
//! host interface.

pub mod builtins;
pub mod collections;
pub mod error;
pub mod host;
pub mod machine;
pub mod ops;
pub mod rng;
pub mod settings;
pub mod value;

pub use error::{ErrorKind, ResourceKind, RtResult, RuntimeError};
pub use host::{
    Dialog, DialogResult, DisplayMode, HeadlessHost, Host, ScriptEvent, ScriptedInput,
    parse_key_script,
};
pub use machine::{DebugAction, Debugger, Limits, Machine, eval_text_with};
pub use settings::{AngleMode, Settings};
pub use value::Value;
