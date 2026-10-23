//! Runtime errors.

use std::fmt;

use ppl_math::MathError;
use ppl_syntax::Span;

/// Error classes. The `message()` texts follow the HP Prime User Guide's
/// error list; whether the G1 shows exactly these strings is unverified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    BadArgType,
    BadArgValue,
    InvalidDimension,
    DivisionByZero,
    Overflow,
    NonReal,
    Undefined,
    InvalidInput,
    InsufficientMemory,
    /// A known Prime command this emulator does not implement.
    Unsupported,
    /// Host protection limits (not calculator behaviour).
    ResourceLimit(ResourceKind),
    /// Stopped by the user (On key / Ctrl-C / window closed).
    Interrupted,
    /// The headless input script ran out.
    ScriptExhausted,
    /// `KILL` statement.
    Killed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Steps,
    Time,
    Depth,
    Memory,
    Output,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Steps => "STEPS",
            ResourceKind::Time => "TIME",
            ResourceKind::Depth => "DEPTH",
            ResourceKind::Memory => "MEMORY",
            ResourceKind::Output => "OUTPUT",
        }
    }
}

impl ErrorKind {
    /// Text as shown by the calculator ("Error: …").
    pub fn message(&self) -> &'static str {
        match self {
            ErrorKind::BadArgType => "Bad argument type",
            ErrorKind::BadArgValue => "Bad argument value",
            ErrorKind::InvalidDimension => "Invalid dimension",
            ErrorKind::DivisionByZero => "Division by 0",
            ErrorKind::Overflow => "Overflow",
            ErrorKind::NonReal => "Non-real result",
            ErrorKind::Undefined => "Undefined result",
            ErrorKind::InvalidInput => "Invalid input",
            ErrorKind::InsufficientMemory => "Insufficient memory",
            ErrorKind::Unsupported => "Unsupported by emulator",
            ErrorKind::ResourceLimit(_) => "Resource limit exceeded",
            ErrorKind::Interrupted => "Program interrupted",
            ErrorKind::ScriptExhausted => "Input script exhausted",
            ErrorKind::Killed => "Program killed",
        }
    }

    /// Whether an `IFERR` handler may catch it. Host-level stops are not
    /// program errors.
    pub fn catchable(&self) -> bool {
        !matches!(
            self,
            ErrorKind::ResourceLimit(_)
                | ErrorKind::Interrupted
                | ErrorKind::ScriptExhausted
                | ErrorKind::Killed
        )
    }
}

/// One entry of a runtime stack trace.
#[derive(Clone, Debug)]
pub struct TraceFrame {
    pub function: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RuntimeError {
    pub kind: ErrorKind,
    pub detail: String,
    /// Innermost frame first.
    pub trace: Vec<TraceFrame>,
}

impl RuntimeError {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> RuntimeError {
        RuntimeError {
            kind,
            detail: detail.into(),
            trace: Vec::new(),
        }
    }

    pub fn kind(kind: ErrorKind) -> RuntimeError {
        RuntimeError::new(kind, String::new())
    }

    pub fn span(&self) -> Option<Span> {
        self.trace.first().map(|f| f.span)
    }
}

impl From<MathError> for RuntimeError {
    fn from(e: MathError) -> Self {
        let kind = match e {
            MathError::Overflow => ErrorKind::Overflow,
            MathError::DivisionByZero => ErrorKind::DivisionByZero,
            MathError::NonReal => ErrorKind::NonReal,
            MathError::Undefined => ErrorKind::Undefined,
        };
        RuntimeError::kind(kind)
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error: {}", self.kind.message())?;
        if let ErrorKind::ResourceLimit(k) = &self.kind {
            write!(f, " ({})", k.as_str())?;
        }
        if !self.detail.is_empty() {
            write!(f, " ({})", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for RuntimeError {}

pub type RtResult<T> = Result<T, RuntimeError>;
