//! Source-fenced app access on a worker. Nested calls use local cloned models,
//! retaining the live caller namespace without reacquiring the GUI execution lease.
mod definition;
use super::{native_app_models::spreadsheet::Spreadsheet, native_apps::GraphApp};
use ppl_vm::{
    ErrorKind, HeadlessHost, Host, PortableValue, RuntimeError, SessionContext, SessionSnapshot,
    Value,
    host::{AppVariableRequest, Dialog, DialogResult, Touch},
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::InvalidInput, message.into())
}
fn unsupported(message: &str) -> RuntimeError {
    RuntimeError::new(ErrorKind::Unsupported, message)
}
#[derive(Clone)]
struct Resolver {
    sheet: Rc<Spreadsheet>,
    function: Rc<GraphApp>,
    cancel: Arc<AtomicBool>,
    deadline: Instant,
    calls: Rc<Cell<usize>>,
    stack: Vec<String>,
    home: ppl_vm::Settings,
}
/// Local model changes are committed by the UI only after its source/epoch fence.
pub struct AccessResult {
    pub value: PortableValue,
    pub sheet: Option<Spreadsheet>,
    pub function: Option<Vec<super::native_apps::Curve>>,
}
pub fn access(
    sheet: Spreadsheet,
    function: GraphApp,
    request: &AppVariableRequest,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<AccessResult, RuntimeError> {
    if request.app == "Spreadsheet" {
        return sheet
            .access(request, snapshot, cancel)
            .map(|result| AccessResult {
                value: result.value,
                sheet: result.sheet,
                function: None,
            });
    }
    if cancel.load(Ordering::Acquire) {
        return Err(RuntimeError::kind(ErrorKind::Interrupted));
    }
    if request.app == "Function"
        && let Some(value) = &request.value
    {
        if !request.indices.is_empty() {
            return Err(invalid(
                "Assign the Function definition without an argument",
            ));
        }
        let index = function_index(&request.name).ok_or_else(|| invalid("Unknown Function row"))?;
        let (source, result) = match value {
            PortableValue::String(text) if text.is_empty() => {
                (String::new(), PortableValue::String(String::new()))
            }
            PortableValue::Real(_) | PortableValue::Integer(_) => (
                value
                    .to_value()
                    .string_form(ppl_vm::value::default_format()),
                value.clone(),
            ),
            PortableValue::String(text) => {
                let definition = definition::Definition::parse(text, &snapshot.settings)?;
                (definition.source, definition.value)
            }
            PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(text)) => {
                let definition = definition::Definition::parse(text, &snapshot.settings)?;
                (definition.source, definition.value)
            }
            _ => {
                return Err(RuntimeError::new(
                    ErrorKind::BadArgType,
                    "A Function definition must be a real, word or expression",
                ));
            }
        };
        let mut function = function;
        while function.curves.len() <= index {
            function.curves.push(super::native_apps::Curve {
                expression: String::new(),
                second: String::new(),
                enabled: false,
                sequence: None,
                literal: None,
            });
        }
        let curve = &mut function.curves[index];
        if curve.expression.trim().is_empty() {
            curve.enabled = false;
        }
        curve.literal = matches!(result, PortableValue::Real(_) | PortableValue::Integer(_))
            .then(|| (source.clone(), ppl_vm::session::encode_value(&result)));
        curve.expression = source;
        if curve.expression.is_empty() {
            curve.enabled = false;
        }
        if cancel.load(Ordering::Acquire) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        return Ok(AccessResult {
            value: result,
            sheet: None,
            function: Some(function.curves),
        });
    }
    let resolver = Resolver {
        sheet: Rc::new(sheet),
        function: Rc::new(function),
        cancel,
        deadline: Instant::now() + Duration::from_secs(4),
        calls: Rc::new(Cell::new(0)),
        stack: vec![],
        home: snapshot.settings.clone(),
    };
    Ok(AccessResult {
        value: resolver.read(request.clone(), snapshot.clone())?,
        sheet: None,
        function: None,
    })
}
pub fn evaluate_expression(
    source: &str,
    sheet: Spreadsheet,
    function: GraphApp,
    snapshot: &SessionSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<Value, RuntimeError> {
    Resolver {
        sheet: Rc::new(sheet),
        function: Rc::new(function),
        cancel,
        deadline: Instant::now() + Duration::from_secs(4),
        calls: Rc::new(Cell::new(0)),
        stack: vec![],
        home: snapshot.settings.clone(),
    }
    .expression(source, None, snapshot)
}
/// Evaluate the Function grid with calculator BCD arguments and typed results.
/// The caller owns the worker and source/generation fence.
pub type NumericRows = Vec<Vec<Option<PortableValue>>>;
pub fn evaluate_numeric_rows(
    sheet: Spreadsheet,
    function: GraphApp,
    snapshot: &SessionSnapshot,
    values: Vec<ppl_math::Real>,
    cancel: Arc<AtomicBool>,
) -> Result<(NumericRows, Option<String>), RuntimeError> {
    if values.len() > 1000 {
        return Err(invalid("Numeric table row limit exceeded"));
    }
    let resolver = Resolver {
        sheet: Rc::new(sheet),
        function: Rc::new(function),
        cancel,
        deadline: Instant::now() + Duration::from_secs(4),
        calls: Rc::new(Cell::new(0)),
        stack: vec![],
        home: snapshot.settings.clone(),
    };
    let enabled = resolver
        .function
        .curves
        .iter()
        .enumerate()
        .filter(|(_, c)| c.enabled && !c.expression.trim().is_empty())
        .map(|(i, _)| format!("F{}", if i == 9 { 0 } else { i + 1 }))
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(values.len());
    let mut first_error = None;
    for x in values {
        resolver.check()?;
        let mut row = vec![Some(PortableValue::Real(x))];
        for name in &enabled {
            resolver.calls.set(0);
            let request = AppVariableRequest {
                app: "Function".into(),
                name: name.clone(),
                indices: vec![PortableValue::Real(x)],
                value: None,
            };
            match resolver.read(request, snapshot.clone()) {
                Ok(
                    value @ (PortableValue::Real(_)
                    | PortableValue::Integer(_)
                    | PortableValue::Complex(_)),
                ) => row.push(Some(value)),
                Ok(_) => row.push(None),
                Err(error) if error.kind == ErrorKind::Interrupted => return Err(error),
                Err(error)
                    if matches!(
                        error.kind,
                        ErrorKind::NonReal | ErrorKind::DivisionByZero | ErrorKind::Undefined
                    ) =>
                {
                    row.push(None)
                }
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error.to_string());
                    }
                    row.push(None);
                }
            }
        }
        rows.push(row);
    }
    Ok((rows, first_error))
}
impl Resolver {
    fn check(&self) -> Result<(), RuntimeError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(RuntimeError::kind(ErrorKind::Interrupted));
        }
        if Instant::now() >= self.deadline {
            return Err(invalid("Application calculation timed out"));
        }
        Ok(())
    }
    fn read(
        &self,
        request: AppVariableRequest,
        mut snapshot: SessionSnapshot,
    ) -> Result<PortableValue, RuntimeError> {
        self.check()?;
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() > 1024 || self.stack.len() >= 32 {
            return Err(invalid("Application function dependency limit exceeded"));
        }
        // Every nested VM retains actual Home settings; arithmetic overrides
        // are selected by the VM from the active app.
        snapshot.settings = self.home.clone();
        if request.app == "Spreadsheet" {
            if request.value.is_some() {
                return Err(unsupported(
                    "Function expressions cannot change worksheet data",
                ));
            }
            return Ok(self
                .sheet
                .access(&request, &snapshot, self.cancel.clone())?
                .value);
        }
        if request.app != "Function" {
            return Err(unsupported(
                "This Native application does not provide these variables",
            ));
        }
        if request.value.is_some() {
            return Err(unsupported("Assign Function definitions in Symbolic view"));
        }
        let index = function_index(&request.name).ok_or_else(|| invalid("Unknown Function row"))?;
        let curve = self
            .function
            .curves
            .get(index)
            .filter(|c| !c.expression.trim().is_empty())
            .ok_or_else(|| invalid("No definition in Symbolic view"))?;
        let source = curve.expression.as_str();
        if source.len() > 65_536 {
            return Err(invalid("Function definition exceeds64KiB"));
        }
        if request.indices.is_empty() {
            return curve.literal_value().map_err(invalid)?.map_or_else(
                || definition::Definition::parse(source, &snapshot.settings).map(|d| d.value),
                Ok,
            );
        }
        let [argument] = request.indices.as_slice() else {
            return Err(invalid("A Function call accepts one argument"));
        };
        if self.stack.contains(&request.name) {
            return Err(invalid("Circular Function definition"));
        }
        let mut nested = self.clone();
        nested.stack.push(request.name);
        let literal = curve.literal_value().map_err(invalid)?;
        nested
            .apply(source, argument.to_value(), literal.as_ref(), &snapshot, 0)
            .map(|v| PortableValue::from(&v))
    }
    fn apply(
        &self,
        source: &str,
        argument: Value,
        literal: Option<&PortableValue>,
        snapshot: &SessionSnapshot,
        list_depth: usize,
    ) -> Result<Value, RuntimeError> {
        self.check()?;
        if list_depth >= 32 {
            return Err(invalid("Function list nesting limit exceeded"));
        }
        if let Value::List(items) = argument {
            if items.len() > 16_384 {
                return Err(invalid("Function argument list is too large"));
            }
            let values = items
                .iter()
                .map(|v| {
                    self.calls.set(self.calls.get() + 1);
                    if self.calls.get() > 16_384 {
                        return Err(invalid("Function argument limit exceeded"));
                    }
                    self.apply(source, v.clone(), literal, snapshot, list_depth + 1)
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::list(values));
        }
        // Calculator words enter app functions as numeric values rather than
        // wrapping each intermediate operation at the word's bit width.
        let argument = match argument {
            Value::Int(word) => Value::Real(word.to_real()),
            value @ (Value::Real(_) | Value::Complex(_)) => value,
            _ => return Err(invalid("A Function argument must be numeric")),
        };
        if let Some(value) = literal {
            return Ok(value.to_value());
        }
        self.expression(source, Some(argument), snapshot)
    }
    fn expression(
        &self,
        source: &str,
        argument: Option<Value>,
        snapshot: &SessionSnapshot,
    ) -> Result<Value, RuntimeError> {
        self.check()?;
        if source.len() > 65_536 {
            return Err(invalid("Expression exceeds64KiB"));
        }
        let source = home_setting_literals(source, &self.home)?;
        let settings = self.home.clone();
        let mut context = SessionContext::new(settings.clone(), 0x46554e4354);
        context.active_app = snapshot.active_app.clone();
        context.app_settings = snapshot.app_settings.clone();
        context.app_numeric_settings = snapshot.app_numeric_settings.clone();
        context.app_files = snapshot.app_files.clone();
        context.cas = snapshot.cas.clone();
        for (name, value) in &snapshot.globals {
            if name.len() == 2 && name.starts_with('G') && name.as_bytes()[1].is_ascii_digit() {
                continue;
            }
            context.set_variable(name, value.clone())?;
        }
        if let Some(argument) = argument {
            context.set_variable("X", PortableValue::from(&argument))?;
        }
        // Imported live system aliases must not change caller app arithmetic.
        context.settings = settings;
        context.app_settings = snapshot.app_settings.clone();
        context.app_numeric_settings = snapshot.app_numeric_settings.clone();
        for (name, source) in &snapshot.programs {
            context.store_program(name, source.clone())?;
        }
        let before = context.snapshot();
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        let result = context
            .evaluate_home(
                &source,
                Box::new(LocalHost {
                    inner: HeadlessHost {
                        interrupt: Some(self.cancel.clone()),
                        ..Default::default()
                    },
                    resolver: self.clone(),
                }),
                ppl_vm::Limits {
                    max_steps: Some(1_000_000),
                    max_time: Some(remaining.min(Duration::from_secs(2))),
                    ..Default::default()
                },
            )
            .result;
        self.check()?;
        let after = context.snapshot();
        if super::native_readonly::changed(&before, &after) {
            return Err(unsupported(
                "Function expressions cannot change calculator memory or settings",
            ));
        }
        result
    }
}
pub fn function_index(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    if bytes.len() != 2 || bytes[0] != b'F' || !bytes[1].is_ascii_digit() {
        return None;
    }
    Some(if bytes[1] == b'0' {
        9
    } else {
        (bytes[1] - b'1') as usize
    })
}
fn home_setting_literals(
    source: &str,
    settings: &ppl_vm::Settings,
) -> Result<String, RuntimeError> {
    use ppl_syntax::ast::{Expr, ExprKind as E};
    let (expression, diagnostics) = ppl_syntax::parse_expression(source);
    if diagnostics.has_errors() {
        return Err(invalid(
            diagnostics.errors().next().unwrap().message.clone(),
        ));
    }
    fn visit(
        expr: &Expr,
        settings: &ppl_vm::Settings,
        out: &mut Vec<(usize, usize, i64)>,
    ) -> Result<(), RuntimeError> {
        match &expr.kind {
            E::Assign(..) | E::Store(..) => {
                return Err(unsupported(
                    "Assignments inside Function definitions are unavailable",
                ));
            }
            E::Ident(name) => {
                let value = match name.to_ascii_lowercase().as_str() {
                    "hangle" => Some(settings.angle.code()),
                    "hformat" => Some(settings.hformat),
                    "hdigits" => Some(settings.hdigits),
                    "hcomplex" => Some(settings.complex_code()),
                    _ => None,
                };
                if let Some(value) = value {
                    out.push((expr.span.start as usize, expr.span.end as usize, value));
                }
            }
            E::Unary(_, e) | E::Postfix(_, e) => visit(e, settings, out)?,
            E::Binary(_, a, b) | E::ComplexPair(a, b) => {
                visit(a, settings, out)?;
                visit(b, settings, out)?;
            }
            E::Call(_, args) => {
                for arg in args {
                    visit(arg, settings, out)?;
                }
            }
            E::Index(f, args) => {
                visit(f, settings, out)?;
                for arg in args {
                    visit(arg, settings, out)?;
                }
            }
            E::List(items) | E::Bracket(items) => {
                for item in items {
                    visit(item, settings, out)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut replacements = vec![];
    visit(&expression, settings, &mut replacements)?;
    let mut result = source.to_owned();
    for (start, end, value) in replacements.into_iter().rev() {
        result.replace_range(start..end, &value.to_string());
    }
    Ok(result)
}
struct LocalHost {
    inner: HeadlessHost,
    resolver: Resolver,
}
impl Host for LocalHost {
    fn app_variable_with_context(
        &mut self,
        request: AppVariableRequest,
        context: SessionSnapshot,
    ) -> Result<PortableValue, RuntimeError> {
        self.resolver.read(request, context)
    }
    fn print(&mut self, text: &str) {
        self.inner.print(text);
    }
    fn poll_key(&mut self) -> Result<Option<i64>, RuntimeError> {
        self.inner.poll_key()
    }
    fn is_key_down(&mut self, key: i64) -> Result<bool, RuntimeError> {
        self.inner.is_key_down(key)
    }
    fn wait_key(&mut self, timeout: Option<f64>) -> Result<Option<i64>, RuntimeError> {
        self.inner.wait_key(timeout)
    }
    fn sleep(&mut self, seconds: f64) -> Result<(), RuntimeError> {
        self.inner.sleep(seconds)
    }
    fn touch(&mut self) -> Result<Option<Touch>, RuntimeError> {
        self.inner.touch()
    }
    fn dialog(
        &mut self,
        dialog: &Dialog,
        screen: &ppl_graphics::Grob,
    ) -> Result<DialogResult, RuntimeError> {
        self.inner.dialog(dialog, screen)
    }
    fn now_ms(&mut self) -> f64 {
        self.inner.now_ms()
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        self.inner.local_time()
    }
    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        Some(self.resolver.cancel.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn production_function_body_cannot_change_numeric_settings_through_a_program() {
        for assignment in [
            "NumStart:=7",
            "NumStep:=2",
            "NumZoom:=3",
            "NumType:=1",
            "NumIndep:={2,4,6}",
        ] {
            let mut context = SessionContext::default();
            context
                .store_program(
                    "MUTATE",
                    format!("EXPORT MUTATE() BEGIN {assignment}; RETURN 7; END;"),
                )
                .unwrap();
            let before = context.memory_json();
            let mut function = GraphApp::new(super::super::native_apps::GraphKind::Function);
            function.curves[0].expression = "MUTATE()+0*X".into();
            let request = AppVariableRequest {
                app: "Function".into(),
                name: "F1".into(),
                indices: vec![PortableValue::Real(ppl_math::Real::ZERO)],
                value: None,
            };
            let result = access(
                Spreadsheet::default(),
                function,
                &request,
                &context.snapshot(),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(
                result
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("cannot change calculator memory"),
                "{assignment}"
            );
            assert_eq!(context.memory_json(), before);
        }
    }
    #[test]
    fn production_function_body_retains_all_four_original_home_complex_codes() {
        for code in [0, 1, 10, 11] {
            let mut context = SessionContext::default();
            context.settings.set_complex_code(code).unwrap();
            let snapshot = context.snapshot();
            let mut function = GraphApp::new(super::super::native_apps::GraphKind::Function);
            function.curves[0].expression = "HComplex+0*X".into();
            let request = AppVariableRequest {
                app: "Function".into(),
                name: "F1".into(),
                indices: vec![PortableValue::Real(ppl_math::Real::ZERO)],
                value: None,
            };
            let result = access(
                Spreadsheet::default(),
                function,
                &request,
                &snapshot,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
            assert_eq!(
                result.value,
                PortableValue::Real(ppl_math::Real::from_i64(code))
            );
            assert!(result.sheet.is_none());
            assert_eq!(snapshot.settings.complex_code(), code);
        }
    }
}
