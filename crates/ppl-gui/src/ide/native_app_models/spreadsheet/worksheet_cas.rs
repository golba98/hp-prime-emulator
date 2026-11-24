use super::{Address, Evaluator, has_assignment};
use ppl_cas::{AngleMode, CasSession, CasValue, Options};
use ppl_syntax::ast::ExprKind;
use ppl_vm::{Value, cas};

pub(crate) fn body(source: &str) -> Option<String> {
    let source = source.trim();
    if !source.get(..3)?.eq_ignore_ascii_case("CAS") || !source[3..].trim_start().starts_with('(') {
        return None;
    }
    let (expression, diagnostics) = ppl_syntax::parse_expression(source);
    if diagnostics.has_errors() {
        return None;
    }
    let ExprKind::Call(name, args) = expression.kind else {
        return None;
    };
    if let ExprKind::Ident(name) = name.kind
        && name.eq_ignore_ascii_case("CAS")
        && let [arg] = args.as_slice()
        && let ExprKind::Str(source) = &arg.kind
    {
        return Some(source.clone());
    }
    None
}

pub(crate) fn wrap(source: &str) -> String {
    format!("CAS({})", serde_json::Value::String(source.into()))
}

impl Evaluator<'_> {
    pub(super) fn cas_expression(
        &mut self,
        source: &str,
        address: Address,
        depth: usize,
    ) -> Result<Value, String> {
        let source = self.expand(source, address, depth)?;
        let (expression, diagnostics) = ppl_syntax::parse_expression(&source);
        if diagnostics.has_errors() {
            return Err(diagnostics.errors().next().unwrap().message.clone());
        }
        if has_assignment(&expression) {
            return Err("Assignments inside spreadsheet formulas are not supported".into());
        }
        let source = ppl_compiler::cas_entry_source(&expression);
        let snapshot = self.context.snapshot();
        let settings = self.context.effective_app_settings();
        let home = snapshot
            .globals
            .iter()
            .filter_map(|(name, value)| cas::to_cas(&value.to_value()).map(|v| (name.clone(), v)))
            .collect();
        let mut session = snapshot.cas;
        let bindings = session.bindings.clone();
        session.settings.approximate = false;
        session.settings.angle = match settings.angle {
            ppl_vm::AngleMode::Radians => AngleMode::Radians,
            ppl_vm::AngleMode::Degrees => AngleMode::Degrees,
            ppl_vm::AngleMode::Gradians => AngleMode::Gradians,
        };
        let result = session
            .evaluate_with_home(&source, &home, &self.cas_options(), &self.cancel)
            .map_err(|error| error.to_string())?;
        if !result.home_updates.is_empty() || session.bindings != bindings {
            return Err("Spreadsheet function attempted to change calculator memory".into());
        }
        Ok(cas::from_cas_in_model(
            result.value,
            settings.numeric_version,
        ))
    }

    pub(super) fn cas_frozen_formula(
        &mut self,
        value: &Value,
        address: Address,
    ) -> Result<Option<String>, String> {
        let Value::Symbolic(exact) = value else {
            return Ok(None);
        };
        let source = match exact.as_ref() {
            CasValue::Rational { .. } => exact.to_string(),
            CasValue::Symbolic(source) => {
                self.check()?;
                // Detect numeric expressions without applying live CAS bindings
                // to symbols in the frozen result.
                let projected = CasSession::default()
                    .evaluate(
                        &format!("evalf(({source}))"),
                        &self.cas_options(),
                        &self.cancel,
                    )
                    .map_err(|error| error.to_string())?;
                if !matches!(
                    projected,
                    CasValue::Integer(_) | CasValue::Decimal(_) | CasValue::Complex { .. }
                ) {
                    return Ok(None);
                }
                source.clone()
            }
            _ => return Ok(None),
        };
        // The numeric worksheet formula contains only the evaluated CAS result.
        // Keep the typed object when Home has no corresponding numeric form.
        match self.expression(&source, address, 0) {
            Ok(Value::Real(_) | Value::Complex(_)) => Ok(Some(source)),
            _ => {
                self.check()?;
                Ok(None)
            }
        }
    }

    fn cas_options(&self) -> Options {
        let mut options = Options::default();
        options.timeout = options.timeout.min(
            self.deadline
                .saturating_duration_since(std::time::Instant::now()),
        );
        options
    }
}
