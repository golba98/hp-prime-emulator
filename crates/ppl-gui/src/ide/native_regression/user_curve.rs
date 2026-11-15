//! User Defined is a supplied curve, not a request to invent unknown parameters.
use ppl_syntax::ast::{Expr, ExprKind, Item, StmtKind};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
thread_local! {
    static CACHE: RefCell<VecDeque<(String, Rc<ppl_compiler::CompiledProgram>)>> = const { RefCell::new(VecDeque::new()) };
}
fn pure(expression: &Expr) -> bool {
    use ExprKind::*;
    match &expression.kind {
        Number(_) | HashInt(_) => true,
        Ident(name) => matches!(name.as_str(), "X" | "π" | "e" | "i"),
        Unary(_, inner) | Postfix(_, inner) => pure(inner),
        Binary(_, left, right) => pure(left) && pure(right),
        Call(callee, args) => {
            matches!(&callee.kind, Ident(name)
            if ppl_compiler::builtins::lookup(name).is_some_and(|spec|
                matches!(spec.category, ppl_compiler::builtins::Category::Math | ppl_compiler::builtins::Category::Bits)
                && !matches!(spec.builtin, ppl_compiler::builtins::Builtin::Random | ppl_compiler::builtins::Builtin::Randint | ppl_compiler::builtins::Builtin::Randnorm | ppl_compiler::builtins::Builtin::Randseed)))
                && args.iter().all(pure)
        }
        _ => false,
    }
}
fn compiled(expression: &str) -> Result<Rc<ppl_compiler::CompiledProgram>, String> {
    if expression.len() > 16_384 {
        return Err("User fit expression exceeds16KiB".into());
    }
    if let Some(program) = CACHE.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(s, _)| s == expression)
            .map(|(_, p)| p.clone())
    }) {
        return Ok(program);
    }
    let source = format!("EXPORT __USERFIT(X) BEGIN RETURN {expression}; END;");
    let (ast, diagnostics) = ppl_syntax::parse(&source);
    let valid = !diagnostics.has_errors()
        && matches!(ast.items.as_slice(), [Item::Function(function)]
        if matches!(function.body.as_slice(), [statement] if matches!(&statement.kind, StmtKind::Return(Some(expression)) if pure(expression))));
    if !valid {
        return Err(
            "User fit must be a numeric expression in X without assignments or unknown variables"
                .into(),
        );
    }
    let (program, diagnostics) = ppl_compiler::compile(&ast, &Default::default());
    let program = Rc::new(program.ok_or_else(|| {
        diagnostics
            .errors()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ")
    })?);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() == 8 {
            cache.pop_front();
        }
        cache.push_back((expression.into(), program.clone()));
    });
    Ok(program)
}
pub(super) fn validate(expression: &str) -> Result<(), String> {
    compiled(expression).map(|_| ())
}
pub(super) fn evaluate(
    expression: &str,
    x: f64,
    angle: ppl_math::AngleMode,
) -> Result<f64, String> {
    let mut machine = ppl_vm::Machine::new(
        compiled(expression)?,
        Box::new(ppl_vm::HeadlessHost::new()),
        ppl_vm::Settings {
            angle,
            ..Default::default()
        },
        ppl_vm::Limits {
            max_steps: Some(100_000),
            ..Default::default()
        },
    );
    machine.run_init().map_err(|e| e.to_string())?;
    let value = machine
        .call_function(
            "__USERFIT",
            vec![ppl_vm::Value::real(
                ppl_math::Real::from_f64(x).map_err(|e| e.to_string())?,
            )],
        )
        .map_err(|e| e.to_string())?
        .expect_real("User fit result")
        .map_err(|e| e.to_string())?
        .to_f64();
    if value.is_finite() {
        Ok(value)
    } else {
        Err("User fit result exceeds numerical range".into())
    }
}
