//! A separate Giac process evaluates symbolic mathematics. No firmware or USB.
//!
//! The executable is built separately (see `tools/giac-helper`); this crate does
//! not link Giac into the Rust application. Every operation starts from an
//! explicit, serializable environment. Cancellation or errors never commit a
//! half-updated CAS context.
pub mod units;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

pub const PROTOCOL_VERSION: u32 = 1;
/// Home-variable exchange is an optional extension. Ordinary CAS requests
/// retain protocol 1 compatibility with existing separately installed helpers.
pub const HOME_PROTOCOL_VERSION: u32 = 2;
const MAX_REQUEST: usize = 4 * 1024 * 1024;
const MAX_VALUE_NODES: usize = 1_000_000;
const MAX_VALUE_DEPTH: usize = 64;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CasValue {
    Integer(String),
    Rational {
        numerator: String,
        denominator: String,
    },
    Decimal(String),
    Complex {
        real: Box<CasValue>,
        imaginary: Box<CasValue>,
    },
    Symbolic(String),
    String(String),
    List(Vec<CasValue>),
    Matrix {
        rows: usize,
        cols: usize,
        values: Vec<CasValue>,
    },
}
impl CasValue {
    /// Validate values loaded from profiles or constructed by an adapter, not
    /// only values produced by the helper. No evaluation or process is run.
    pub fn validate(&self) -> Result<(), CasError> {
        validate_value(self, 0, &mut Budget::default())
    }
    pub fn try_expression(&self) -> Result<String, CasError> {
        self.validate()?;
        Ok(self.validated_expression())
    }
    /// Exact Giac expression, suitable for storage or subsequent evaluation.
    /// Invalid external values render a conspicuous, non-expression marker;
    /// evaluation adapters should use `try_expression` to propagate the error.
    pub fn expression(&self) -> String {
        self.try_expression()
            .unwrap_or_else(|_| "<invalid CAS value>".into())
    }
    fn validated_expression(&self) -> String {
        match self {
            Self::Integer(s) | Self::Decimal(s) | Self::Symbolic(s) => s.clone(),
            Self::Rational {
                numerator,
                denominator,
            } => format!("({numerator})/({denominator})"),
            Self::Complex { real, imaginary } => {
                format!(
                    "({})+i*({})",
                    real.validated_expression(),
                    imaginary.validated_expression()
                )
            }
            Self::String(s) => serde_json::to_string(s).unwrap(),
            Self::List(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(Self::validated_expression)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Self::Matrix { rows, cols, values } => format!(
                "[{}]",
                (0..*rows)
                    .map(|row| format!(
                        "[{}]",
                        values[row * cols..(row + 1) * cols]
                            .iter()
                            .map(Self::validated_expression)
                            .collect::<Vec<_>>()
                            .join(",")
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
}
impl fmt::Display for CasValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.validate().is_err() {
            return f.write_str("<invalid CAS value>");
        }
        match self {
            Self::Rational {
                numerator,
                denominator,
            } => write!(f, "{numerator}/{denominator}"),
            _ => f.write_str(&self.validated_expression()),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AngleMode {
    #[default]
    Radians,
    Degrees,
    Gradians,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CasSettings {
    pub angle: AngleMode,
    pub approximate: bool,
    pub digits: u8,
}
impl Default for CasSettings {
    fn default() -> Self {
        Self {
            angle: AngleMode::Radians,
            approximate: false,
            digits: 12,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CasSession {
    pub bindings: BTreeMap<String, String>,
    pub settings: CasSettings,
    pub last_value: Option<CasValue>,
    pub helper_version: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CasEvaluation {
    pub value: CasValue,
    pub home_updates: BTreeMap<String, CasValue>,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub helper: PathBuf,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            helper: std::env::var_os("PPL_CAS_HELPER")
                .map(PathBuf::from)
                .or_else(|| {
                    let data = std::env::var_os("XDG_DATA_HOME")
                        .map(PathBuf::from)
                        .or_else(|| {
                            std::env::var_os("HOME")
                                .map(|home| PathBuf::from(home).join(".local/share"))
                        })?;
                    let helper = data.join("hp-prime-emulator/cas/ppl-giac-helper");
                    helper.is_file().then_some(helper)
                })
                .or_else(|| {
                    let helper = std::env::current_exe()
                        .ok()?
                        .parent()?
                        .join("ppl-giac-helper");
                    helper.is_file().then_some(helper)
                })
                .unwrap_or_else(|| "ppl-giac-helper".into()),
            timeout: Duration::from_secs(30),
            max_output_bytes: 4 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasError {
    Unavailable(String),
    InvalidRequest(String),
    Evaluation(String),
    Protocol(String),
    Cancelled,
    TimedOut,
    Io(String),
}
impl fmt::Display for CasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(s) => write!(
                f,
                "CAS helper unavailable: {s}; build tools/giac-helper and set PPL_CAS_HELPER"
            ),
            Self::InvalidRequest(s) => write!(f, "Invalid CAS request: {s}"),
            Self::Evaluation(s) => f.write_str(s),
            Self::Protocol(s) => write!(f, "CAS protocol: {s}"),
            Self::Cancelled => f.write_str("CAS evaluation interrupted"),
            Self::TimedOut => f.write_str("CAS evaluation timed out"),
            Self::Io(s) => write!(f, "CAS I/O: {s}"),
        }
    }
}
impl std::error::Error for CasError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u32,
    request: u64,
    engine: String,
    result: Option<CasValue>,
    error: Option<String>,
    bindings: BTreeMap<String, String>,
    #[serde(default)]
    home_updates: BTreeMap<String, CasValue>,
}

struct Helper(Child);
impl Helper {
    fn stop(&mut self) {
        #[cfg(unix)]
        {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(self.0.id() as i32),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
impl Drop for Helper {
    fn drop(&mut self) {
        self.stop();
    }
}

fn drain(mut pipe: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut overflow = false;
    let mut buf = [0; 8192];
    loop {
        let n = pipe.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(output.len()));
        output.extend_from_slice(&buf[..keep]);
        overflow |= keep < n;
    }
    if overflow {
        Err("helper output exceeded configured limit".into())
    } else {
        Ok(output)
    }
}
fn field(out: &mut Vec<u8>, value: &str) -> Result<(), CasError> {
    let n = u32::try_from(value.len())
        .map_err(|_| CasError::InvalidRequest("field too large".into()))?;
    out.extend(n.to_le_bytes());
    out.extend(value.as_bytes());
    Ok(())
}
impl CasSession {
    /// Profiles must call this after deserialization and before committing the
    /// restored state. Validation is also applied to every outbound request.
    pub fn validate(&self) -> Result<(), CasError> {
        if !(1..=100).contains(&self.settings.digits) {
            return Err(CasError::InvalidRequest("digits must be 1–100".into()));
        }
        let mut budget = Budget::default();
        for (name, value) in &self.bindings {
            budget.node()?;
            if !valid_name(name) {
                return Err(CasError::InvalidRequest(format!(
                    "invalid binding name {name:?}"
                )));
            }
            budget.text(name)?;
            budget.text(value)?;
            if value.trim().is_empty() || value.contains('\0') {
                return Err(CasError::InvalidRequest(
                    "empty/NUL CAS binding expression".into(),
                ));
            }
        }
        if let Some(value) = &self.last_value {
            validate_value(value, 0, &mut budget)?;
        }
        if let Some(version) = &self.helper_version {
            budget.text(version)?;
            if version.len() > 4096 || !version.starts_with("giac-") {
                return Err(CasError::Protocol("invalid saved helper identity".into()));
            }
        }
        Ok(())
    }
    pub fn evaluate(
        &mut self,
        expression: &str,
        options: &Options,
        cancelled: &AtomicBool,
    ) -> Result<CasValue, CasError> {
        self.evaluate_request(expression, None, options, cancelled)
            .map(|evaluation| evaluation.value)
    }
    /// Evaluate with an explicit Home view. Home names are scoped to this
    /// request and never inserted into persisted CAS bindings. Uppercase
    /// preset variables default to their Native initial values. `Ans` remains
    /// the CAS answer, even if the caller supplies a Home answer.
    ///
    /// The caller should evaluate a cloned session and commit it together with
    /// `home_updates` under its calculator-memory operation lease.
    pub fn evaluate_with_home(
        &mut self,
        expression: &str,
        home: &BTreeMap<String, CasValue>,
        options: &Options,
        cancelled: &AtomicBool,
    ) -> Result<CasEvaluation, CasError> {
        self.evaluate_request(expression, Some(home), options, cancelled)
    }
    fn evaluate_request(
        &mut self,
        expression: &str,
        home: Option<&BTreeMap<String, CasValue>>,
        options: &Options,
        cancelled: &AtomicBool,
    ) -> Result<CasEvaluation, CasError> {
        let start = Instant::now();
        if cancelled.load(Ordering::Acquire) {
            return Err(CasError::Cancelled);
        }
        self.validate()?;
        if expression.trim().is_empty() {
            return Err(CasError::InvalidRequest("empty expression".into()));
        }
        if !(1..=100).contains(&self.settings.digits) {
            return Err(CasError::InvalidRequest("digits must be 1–100".into()));
        }
        let answer = self
            .last_value
            .as_ref()
            .map(CasValue::try_expression)
            .transpose()?
            .unwrap_or_else(|| "0".into());
        let expression = replace_answer(expression, &answer);
        let bridge = home
            .map(|home| HomeBridge::new(self, home, &expression))
            .transpose()?;
        let expression = bridge
            .as_ref()
            .map_or_else(|| expression.clone(), |bridge| bridge.rewrite(&expression));
        let bindings = bridge.as_ref().map_or_else(
            || self.bindings.clone(),
            |bridge| bridge.cas_bindings(&self.bindings),
        );
        let version = if bridge.is_some() {
            HOME_PROTOCOL_VERSION
        } else {
            PROTOCOL_VERSION
        };
        let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let mut input = if bridge.is_some() {
            b"PPLCAS02".to_vec()
        } else {
            b"PPLCAS01".to_vec()
        };
        input.extend(request.to_le_bytes());
        input.push(match self.settings.angle {
            AngleMode::Radians => 0,
            AngleMode::Degrees => 1,
            AngleMode::Gradians => 2,
        });
        input.push(self.settings.approximate as u8);
        input.push(self.settings.digits);
        field(&mut input, &expression)?;
        input.extend(
            u32::try_from(bindings.len())
                .map_err(|_| CasError::InvalidRequest("too many bindings".into()))?
                .to_le_bytes(),
        );
        for (name, value) in &bindings {
            if !valid_name(name) {
                return Err(CasError::InvalidRequest(format!(
                    "invalid binding name {name:?}"
                )));
            }
            field(&mut input, name)?;
            field(&mut input, value)?;
        }
        if let Some(bridge) = &bridge {
            input.extend((bridge.home.len() as u32).to_le_bytes());
            for (name, value) in &bridge.home {
                field(&mut input, &bridge.aliases[name])?;
                field(&mut input, &bridge.rewrite(&value.try_expression()?))?;
            }
        }
        if input.len() > MAX_REQUEST {
            return Err(CasError::InvalidRequest("request exceeds 4 MiB".into()));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(CasError::Cancelled);
        }
        if start.elapsed() >= options.timeout {
            return Err(CasError::TimedOut);
        }
        let mut command = Command::new(&options.helper);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        // A freshly installed helper can briefly remain write-locked by the
        // filesystem. Retry only this pre-execution failure within the same
        // operation deadline; never retry a process that has already started.
        let spawn_start = Instant::now();
        let child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(error) => {
                    #[cfg(unix)]
                    let busy = error.raw_os_error() == Some(nix::errno::Errno::ETXTBSY as i32);
                    #[cfg(not(unix))]
                    let busy = false;
                    if !busy || spawn_start.elapsed() >= Duration::from_millis(100) {
                        return Err(CasError::Unavailable(error.to_string()));
                    }
                    if cancelled.load(Ordering::Acquire) {
                        return Err(CasError::Cancelled);
                    }
                    if start.elapsed() >= options.timeout {
                        return Err(CasError::TimedOut);
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            }
        };
        let mut helper = Helper(child);
        let stdin = helper.0.stdin.take().unwrap();
        let stdout = helper.0.stdout.take().unwrap();
        let stderr = helper.0.stderr.take().unwrap();
        let writer = thread::spawn(move || {
            let mut stdin = stdin;
            stdin.write_all(&input).map_err(|e| e.to_string())
        });
        let limit = options.max_output_bytes.min(16 * 1024 * 1024);
        let reader = thread::spawn(move || drain(stdout, limit));
        let errors = thread::spawn(move || drain(stderr, 64 * 1024));
        let stopped = loop {
            if cancelled.load(Ordering::Acquire) {
                break Err(CasError::Cancelled);
            }
            if start.elapsed() >= options.timeout {
                break Err(CasError::TimedOut);
            }
            match helper.0.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {}
                Err(e) => break Err(CasError::Io(e.to_string())),
            }
            thread::sleep(Duration::from_millis(5));
        };
        // Kill descendants as well: none may retain the pipes after the helper
        // exits or prevent the reader/writer threads from completing.
        helper.stop();
        let written = writer
            .join()
            .map_err(|_| CasError::Io("stdin worker panicked".into()))?;
        let output = reader
            .join()
            .map_err(|_| CasError::Io("stdout worker panicked".into()))?
            .map_err(CasError::Protocol)?;
        let diagnostics = errors
            .join()
            .map_err(|_| CasError::Io("stderr worker panicked".into()))?
            .unwrap_or_default();
        let status = stopped?;
        if !status.success() {
            return Err(CasError::Evaluation(
                String::from_utf8_lossy(&diagnostics).trim().to_string(),
            ));
        }
        written.map_err(CasError::Io)?;
        let reply: Reply =
            serde_json::from_slice(&output).map_err(|e| CasError::Protocol(e.to_string()))?;
        if reply.version != version
            || reply.request != request
            || !reply.engine.starts_with("giac-")
        {
            return Err(CasError::Protocol(
                "helper identity/version/request mismatch".into(),
            ));
        }
        if let Some(error) = reply.error {
            return Err(CasError::Evaluation(error));
        }
        let value = reply
            .result
            .ok_or_else(|| CasError::Protocol("no evaluation result".into()))?;
        let (value, bindings, home_updates) = if let Some(bridge) = bridge {
            bridge.finish(value, reply.bindings, reply.home_updates)?
        } else {
            if !reply.home_updates.is_empty() {
                return Err(CasError::Protocol("unsolicited Home changes".into()));
            }
            (value, reply.bindings, BTreeMap::new())
        };
        let next = Self {
            bindings,
            settings: self.settings.clone(),
            last_value: Some(value.clone()),
            helper_version: Some(reply.engine),
        };
        next.validate()
            .map_err(|e| CasError::Protocol(e.to_string()))?;
        if cancelled.load(Ordering::Acquire) {
            return Err(CasError::Cancelled);
        }
        if start.elapsed() >= options.timeout {
            return Err(CasError::TimedOut);
        }
        *self = next;
        Ok(CasEvaluation {
            value,
            home_updates,
        })
    }
}
pub fn valid_variable_name(name: &str) -> bool {
    valid_name(name)
}
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

type BridgedResult = (
    CasValue,
    BTreeMap<String, String>,
    BTreeMap<String, CasValue>,
);

struct HomeBridge {
    home: BTreeMap<String, CasValue>,
    aliases: BTreeMap<String, String>,
    originals: BTreeMap<String, String>,
}
impl HomeBridge {
    fn new(
        session: &CasSession,
        supplied: &BTreeMap<String, CasValue>,
        expression: &str,
    ) -> Result<Self, CasError> {
        let mut home = BTreeMap::new();
        for letter in b'A'..=b'Z' {
            home.insert(
                char::from(letter).to_string(),
                CasValue::Integer("0".into()),
            );
        }
        home.insert("θ".into(), CasValue::Integer("0".into()));
        for index in 0..10 {
            home.insert(format!("L{index}"), CasValue::List(Vec::new()));
            home.insert(
                format!("M{index}"),
                CasValue::Matrix {
                    rows: 1,
                    cols: 1,
                    values: vec![CasValue::Integer("0".into())],
                },
            );
            home.insert(
                format!("Z{index}"),
                CasValue::Complex {
                    real: Box::new(CasValue::Integer("0".into())),
                    imaginary: Box::new(CasValue::Integer("0".into())),
                },
            );
        }
        let mut budget = Budget::default();
        for (name, value) in supplied {
            if !valid_name(name) {
                return Err(CasError::InvalidRequest(format!(
                    "invalid Home variable name {name:?}"
                )));
            }
            budget.text(name)?;
            validate_value(value, 0, &mut budget)?;
            // CAS and Home answers are independent. Existing CAS custom names
            // take precedence over same-named custom Home objects; calculator
            // preset names always address Home.
            if name == "Ans" || (session.bindings.contains_key(name) && !home.contains_key(name)) {
                continue;
            }
            home.insert(name.clone(), value.clone());
        }
        let prefix = loop {
            let candidate = format!(
                "pplHomeBridge{}_",
                NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
            );
            if !expression.contains(&candidate)
                && session
                    .bindings
                    .iter()
                    .all(|(name, value)| !name.contains(&candidate) && !value.contains(&candidate))
                && supplied.keys().all(|name| !name.contains(&candidate))
            {
                break candidate;
            }
        };
        let aliases: BTreeMap<_, _> = home
            .keys()
            .enumerate()
            .map(|(index, name)| (name.clone(), format!("{prefix}{index}")))
            .collect();
        let originals = aliases
            .iter()
            .map(|(name, alias)| (alias.clone(), name.clone()))
            .collect();
        Ok(Self {
            home,
            aliases,
            originals,
        })
    }
    fn rewrite(&self, source: &str) -> String {
        rewrite_identifiers(source, &self.aliases)
    }
    fn cas_bindings(&self, bindings: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        bindings
            .iter()
            .filter(|(name, _)| !self.home.contains_key(*name))
            .map(|(name, value)| (name.clone(), self.rewrite(value)))
            .collect()
    }
    fn finish(
        self,
        value: CasValue,
        bindings: BTreeMap<String, String>,
        updates: BTreeMap<String, CasValue>,
    ) -> Result<BridgedResult, CasError> {
        let value = rewrite_value(value, &self.originals);
        let bindings = bindings
            .into_iter()
            .filter(|(name, _)| !self.originals.contains_key(name))
            .map(|(name, value)| (name, rewrite_identifiers(&value, &self.originals)))
            .collect();
        let mut home_updates = BTreeMap::new();
        let mut budget = Budget::default();
        for (alias, value) in updates {
            let name = self
                .originals
                .get(&alias)
                .ok_or_else(|| CasError::Protocol("helper changed an unknown Home alias".into()))?;
            let value = rewrite_value(value, &self.originals);
            validate_value(&value, 0, &mut budget)?;
            home_updates.insert(name.clone(), value);
        }
        Ok((value, bindings, home_updates))
    }
}

fn rewrite_value(value: CasValue, names: &BTreeMap<String, String>) -> CasValue {
    match value {
        CasValue::Symbolic(source) => CasValue::Symbolic(rewrite_identifiers(&source, names)),
        CasValue::Complex { real, imaginary } => CasValue::Complex {
            real: Box::new(rewrite_value(*real, names)),
            imaginary: Box::new(rewrite_value(*imaginary, names)),
        },
        CasValue::List(values) => CasValue::List(
            values
                .into_iter()
                .map(|value| rewrite_value(value, names))
                .collect(),
        ),
        CasValue::Matrix { rows, cols, values } => CasValue::Matrix {
            rows,
            cols,
            values: values
                .into_iter()
                .map(|value| rewrite_value(value, names))
                .collect(),
        },
        other => other,
    }
}

/// Identifier replacement for the Giac compatibility boundary. Double-quoted
/// strings and comments are data. Single quotes hold expressions, so their
/// identifiers retain their lookup identity through a renamed, held symbol.
fn rewrite_identifiers(source: &str, names: &BTreeMap<String, String>) -> String {
    let mut result = String::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            result.push(ch);
            let mut escaped = false;
            for c in chars.by_ref() {
                result.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'/') {
            result.push(ch);
            result.push(chars.next().unwrap());
            for c in chars.by_ref() {
                result.push(c);
                if c == '\n' {
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'*') {
            result.push(ch);
            result.push(chars.next().unwrap());
            let mut star = false;
            for c in chars.by_ref() {
                result.push(c);
                if star && c == '/' {
                    break;
                }
                star = c == '*';
            }
        } else if ch == '_' && chars.peek() == Some(&'(') {
            // Unit-group symbols are unit names, not Home/CAS variables.
            // Keep names such as A, K, m and s out of alias substitution.
            result.push(ch);
            let mut depth = 0;
            for c in chars.by_ref() {
                result.push(c);
                if c == '(' {
                    depth += 1;
                }
                if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
        } else if ch.is_ascii_digit()
            || (ch == '.' && chars.peek().is_some_and(char::is_ascii_digit))
        {
            // E/e in a scientific literal is an exponent marker, not the
            // calculator's Home E variable (including signed exponents).
            result.push(ch);
            while chars.peek().is_some_and(char::is_ascii_digit) {
                result.push(chars.next().unwrap());
            }
            if ch != '.' && chars.peek() == Some(&'.') {
                result.push(chars.next().unwrap());
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    result.push(chars.next().unwrap());
                }
            }
            if chars.peek().is_some_and(|c| matches!(c, 'e' | 'E')) {
                let mut exponent = chars.clone();
                exponent.next();
                if exponent.peek().is_some_and(|c| matches!(c, '-' | '+')) {
                    exponent.next();
                }
                if exponent.peek().is_some_and(char::is_ascii_digit) {
                    result.push(chars.next().unwrap());
                    if chars.peek().is_some_and(|c| matches!(c, '-' | '+')) {
                        result.push(chars.next().unwrap());
                    }
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        result.push(chars.next().unwrap());
                    }
                }
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let mut name = String::from(ch);
            while chars
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                name.push(chars.next().unwrap());
            }
            result.push_str(names.get(&name).unwrap_or(&name));
        } else {
            result.push(ch);
        }
    }
    result
}

struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            nodes: MAX_VALUE_NODES,
            bytes: MAX_REQUEST,
        }
    }
}
impl Budget {
    fn node(&mut self) -> Result<(), CasError> {
        self.nodes = self
            .nodes
            .checked_sub(1)
            .ok_or_else(|| CasError::Protocol("CAS value exceeds element limit".into()))?;
        Ok(())
    }
    fn text(&mut self, text: &str) -> Result<(), CasError> {
        self.bytes = self
            .bytes
            .checked_sub(text.len())
            .ok_or_else(|| CasError::Protocol("CAS value exceeds text limit".into()))?;
        Ok(())
    }
}
fn integer(text: &str) -> bool {
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit())
}
fn decimal(text: &str) -> bool {
    let text = text.strip_prefix(['-', '+']).unwrap_or(text);
    let mut parts = text.split(['e', 'E']);
    let mantissa = parts.next().unwrap_or("");
    let exponent = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let mut digits = 0;
    let mut dots = 0;
    for c in mantissa.bytes() {
        if c.is_ascii_digit() {
            digits += 1;
        } else if c == b'.' {
            dots += 1;
        } else {
            return false;
        }
    }
    digits > 0 && dots <= 1 && exponent.is_none_or(integer)
}
fn validate_value(value: &CasValue, depth: usize, budget: &mut Budget) -> Result<(), CasError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(CasError::Protocol("result nesting exceeds 64".into()));
    }
    budget.node()?;
    match value {
        CasValue::Integer(text) => {
            budget.text(text)?;
            if !integer(text) {
                return Err(CasError::Protocol("invalid exact integer".into()));
            }
        }
        CasValue::Decimal(text) => {
            budget.text(text)?;
            if !decimal(text) {
                return Err(CasError::Protocol("invalid decimal representation".into()));
            }
        }
        CasValue::Rational {
            numerator,
            denominator,
        } => {
            budget.text(numerator)?;
            budget.text(denominator)?;
            if !integer(numerator) || !integer(denominator) {
                return Err(CasError::Protocol("invalid exact rational".into()));
            }
            if denominator.bytes().all(|c| matches!(c, b'0' | b'-' | b'+')) {
                return Err(CasError::Protocol("zero rational denominator".into()));
            }
        }
        CasValue::Symbolic(text) => {
            budget.text(text)?;
            if text.trim().is_empty() || text.contains('\0') {
                return Err(CasError::Protocol("invalid symbolic expression".into()));
            }
        }
        CasValue::String(text) => budget.text(text)?,
        CasValue::Complex { real, imaginary } => {
            validate_value(real, depth + 1, budget)?;
            validate_value(imaginary, depth + 1, budget)?;
        }
        CasValue::List(values) => {
            for v in values {
                validate_value(v, depth + 1, budget)?;
            }
        }
        CasValue::Matrix { rows, cols, values } => {
            if *rows > MAX_VALUE_NODES
                || *cols > MAX_VALUE_NODES
                || rows.checked_mul(*cols) != Some(values.len())
            {
                return Err(CasError::Protocol("matrix shape mismatch".into()));
            }
            for v in values {
                validate_value(v, depth + 1, budget)?;
            }
        }
    }
    Ok(())
}

/// HP's Ans token is not assignable in desktop Giac. Substitute only complete
/// unquoted identifiers at the compatibility boundary; string literals stay text.
fn replace_answer(source: &str, answer: &str) -> String {
    let mut result = String::new();
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\"' || ch == '\'' {
            result.push(ch);
            let quote = ch;
            let mut escaped = false;
            for c in chars.by_ref() {
                result.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    break;
                }
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let mut name = String::from(ch);
            while chars
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                name.push(chars.next().unwrap());
            }
            if name == "Ans" {
                result.push('(');
                result.push_str(answer);
                result.push(')');
            } else {
                result.push_str(&name);
            }
        } else {
            result.push(ch);
        }
    }
    result
}
