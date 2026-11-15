//! Function definition values preserve their syntax rather than evaluating names.
use super::{PortableValue, RuntimeError, invalid, unsupported};
use ppl_math::Real;
use ppl_syntax::ast::{Expr, ExprKind as E, UnOp};

pub(super) struct Definition {
    pub source: String,
    pub value: PortableValue,
}
impl Definition {
    pub fn parse(source: &str, settings: &ppl_vm::Settings) -> Result<Self, RuntimeError> {
        if source.len() > 65_536 {
            return Err(invalid("Function definition exceeds 64 KiB"));
        }
        let (expression, diagnostics) = ppl_syntax::parse_expression(source);
        if diagnostics.has_errors() {
            return Err(invalid(
                diagnostics.errors().next().unwrap().message.clone(),
            ));
        }
        if matches!(expression.kind, E::Str(_) | E::Bracket(_)) {
            return Err(RuntimeError::new(
                ppl_vm::ErrorKind::BadArgType,
                "A Function definition must be a real, word or expression",
            ));
        }
        let source = ppl_compiler::ppl_expression_source(&expression).map_err(|error| {
            use ppl_compiler::SourceError;
            match error {
                SourceError::NestingLimit => invalid("Function definition nesting limit exceeded"),
                SourceError::Sexagesimal => {
                    unsupported("Sexagesimal Function definitions are unavailable")
                }
                SourceError::Assignment => {
                    unsupported("Assignments inside Function definitions are unavailable")
                }
                SourceError::Invalid => invalid("Invalid Function definition"),
            }
        })?;
        let value = match literal(&expression, settings)? {
            Some(value) => value,
            None => PortableValue::Symbolic(ppl_cas::CasValue::Symbolic(source.clone())),
        };
        Ok(Self { source, value })
    }
}
fn literal(
    expression: &Expr,
    settings: &ppl_vm::Settings,
) -> Result<Option<PortableValue>, RuntimeError> {
    match &expression.kind {
        E::Number(number) => Real::parse_versioned(number, settings.numeric_version)
            .map(|r| Some(PortableValue::Real(r)))
            .ok_or_else(|| invalid("Invalid Function number")),
        E::HashInt(number) => {
            let format = settings.home_format();
            let width = i64::from(format.int_bits) * if format.int_signed { -1 } else { 1 };
            ppl_math::HpInt::parse_literal_with_defaults(number, format.int_base, width)
                .map(|v| Some(PortableValue::Integer(v)))
                .ok_or_else(|| invalid("Invalid Function word"))
        }
        E::Unary(UnOp::Neg, inner) => Ok(match literal(inner, settings)? {
            Some(PortableValue::Real(r)) => Some(PortableValue::Real(r.neg())),
            _ => None,
        }),
        E::Quoted(inner) => literal(inner, settings),
        _ => Ok(None),
    }
}
