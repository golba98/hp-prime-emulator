//! Nested text evaluation uses the running machine, including its host and
//! execution budget. Only the temporary compiled code and captured-local slots
//! are removed on return; normal PPL side effects remain normal PPL side effects.
use super::*;
use std::collections::BTreeMap;
mod catalog;
mod cleanup;
mod locale;

impl Machine {
    /// Visible globals in shadowing order, followed by the caller's locals.
    fn expression_bindings(&self) -> BTreeMap<String, Binding> {
        let mut bindings = self
            .home_variables
            .keys()
            .map(|name| (name.clone(), Binding::Home))
            .collect::<BTreeMap<_, _>>();
        let owner = self.expression_owner();
        for private in [false, true] {
            for (index, slot) in self.prog.globals.iter().enumerate() {
                if slot.name.starts_with('<') || self.captured_globals.contains_key(&index) {
                    continue;
                }
                let visible = match &self.linked {
                    Some((image, _)) if private => image.global_owners[index] == owner,
                    Some((image, _)) => image.global_owners[index].is_none() || slot.exported,
                    None => !private,
                };
                if visible {
                    bindings.insert(slot.name.clone(), Binding::Global(index));
                }
            }
        }
        if let Some(frame) = self.frames.last() {
            if let Some(scope) = self.expression_scopes.get(&frame.func) {
                bindings.extend(
                    scope
                        .bindings
                        .iter()
                        .map(|(name, binding)| (name.clone(), *binding)),
                );
            }
            for (index, name) in self.chunk(frame.func).local_names.iter().enumerate() {
                if !name.starts_with('<')
                    && self
                        .chunk(frame.func)
                        .local_visible(index, frame.ip.saturating_sub(1))
                {
                    bindings.insert(name.clone(), Binding::Local(frame.base + index));
                }
            }
        }
        bindings
    }

    fn expression_owner(&self) -> Option<usize> {
        self.linked.as_ref().map(|(image, _)| {
            self.frames
                .last()
                .and_then(|frame| image.function_owners.get(frame.func))
                .copied()
                .unwrap_or(image.root)
        })
    }

    pub(crate) fn expression_values(&self) -> BTreeMap<String, ppl_cas::CasValue> {
        self.expression_bindings()
            .into_iter()
            .filter_map(|(name, binding)| {
                let value = match binding {
                    Binding::Local(index) => self.stack[index].clone(),
                    Binding::Global(index) => self.load_global(index),
                    Binding::Home => self.home_variables[&name].clone(),
                };
                crate::cas::to_cas(&value).map(|value| (name, value))
            })
            .collect()
    }

    pub(crate) fn store_expression_value(&mut self, name: &str, value: Value) -> RtResult<()> {
        match self.expression_bindings().get(name) {
            Some(Binding::Local(index)) => self.stack[*index] = value,
            Some(Binding::Global(index)) => return self.store_global(*index, value),
            _ => return self.store_home_value(name, value),
        }
        Ok(())
    }

    fn expression_functions(&self) -> BTreeMap<String, (u32, bool)> {
        let owner = self.expression_owner();
        let mut functions = BTreeMap::new();
        if let Some((image, _)) = &self.linked {
            functions.extend(
                image
                    .exports
                    .iter()
                    .map(|(name, (_, index))| (name.clone(), (*index, true))),
            );
        }
        for (index, chunk) in self.prog.functions.iter().enumerate() {
            if self.expression_scopes.contains_key(&index) {
                continue;
            }
            if self
                .linked
                .as_ref()
                .is_none_or(|(image, _)| Some(image.function_owners[index]) == owner)
            {
                let name = chunk.name.rsplit("::").next().unwrap();
                if !name.starts_with('<') {
                    functions.insert(name.to_owned(), (index as u32, false));
                }
            }
        }
        functions
    }

    /// Evaluate INPUT/EXPR text against live calculator, file and frame values.
    /// Calls, assignments, graphics, randomness and errors use the same machine.
    pub fn eval_text(&mut self, text: &str) -> RtResult<Value> {
        self.check_limits()?;
        self.check_value(&Value::str(text))?;
        let parts = locale::expressions(text, self.settings.digit_grouping.decimal() == ',')?;
        let canonical = parts.join(";\n");
        match ppl_cas::units::translate_home(&canonical) {
            Ok(Some(source)) => return crate::cas::evaluate_expression(self, &source),
            Ok(None) => (),
            Err(error) => {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    error.to_string(),
                ));
            }
        }
        let bindings = self.expression_bindings();
        let functions = self.expression_functions();
        let mut options = ppl_compiler::CompileOptions {
            default_int_base: self.settings.int_base,
            default_int_width: self.settings.integer_width_code(),
            external_variables: bindings.keys().cloned().collect(),
            external_functions: functions
                .iter()
                .map(|(name, (index, _))| {
                    (
                        name.clone(),
                        self.prog.functions[*index as usize].params as usize,
                    )
                })
                .collect(),
            ..Default::default()
        };
        self.expression_catalog_options(&mut options);
        let file = ppl_syntax::SourceFile::new(
            "<expr>",
            format!(
                "EXPORT __EVAL() BEGIN {}\nRETURN {}\n; END;",
                parts[..parts.len() - 1]
                    .iter()
                    .map(|part| format!("{part}\n;"))
                    .collect::<String>(),
                parts.last().unwrap()
            ),
        );
        let (ast, parse_diagnostics) = ppl_syntax::parse(&file.text);
        let (compiled, diagnostics) = if parse_diagnostics.has_errors() {
            (None, parse_diagnostics)
        } else {
            // EXPR admits local statements and control flow, as the firmware
            // does, but cannot escape its evaluation frame to define another
            // calculator program or close the generated function early.
            if !matches!(ast.items.as_slice(), [ppl_syntax::ast::Item::Function(function)]
                if function.name.name == "__EVAL" && function.span.end as usize == file.text.len() - 1)
            {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "EXPR cannot define or close a calculator program",
                ));
            }
            ppl_compiler::compile(&ast, &options)
        };
        let compiled = compiled.ok_or_else(|| {
            RuntimeError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot evaluate `{text}`: {}",
                    diagnostics
                        .errors()
                        .map(|d| d.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            )
        })?;

        self.load_expression_catalog(&compiled, &functions, &bindings, &options)?;
        let bindings = self.expression_bindings();
        let owner = self.expression_owner();
        let functions = self.expression_functions();
        let original = self.prog.clone();
        let global_count = self.globals.len();
        let constant_count = self.consts.len();
        let mut extended = (*original).clone();
        let mut slots = Vec::new();
        for mut slot in compiled.globals.iter().cloned() {
            if let Some(Binding::Global(index)) = bindings.get(&slot.name) {
                slots.push(*index as u32);
                continue;
            }
            let index = self.globals.len();
            let local = match bindings.get(&slot.name) {
                Some(Binding::Local(local)) => Some(*local),
                _ => None,
            };
            slot.external = false;
            slot.program_var = local.is_some();
            let value = if let Some(local) = local {
                self.captured_globals.insert(index, local);
                self.stack[local].clone()
            } else if let Some(value) = self.home_variables.get(&slot.name) {
                value.clone()
            } else {
                initial_value(&slot.init)
            };
            self.setting_slots.push(match slot.init {
                GlobalInit::Setting(setting) => Some(setting),
                _ => None,
            });
            self.globals.push(value);
            extended.globals.push(slot);
            slots.push(index as u32);
        }
        extended.consts.extend(compiled.consts.iter().cloned());
        self.consts.extend(
            compiled
                .consts
                .iter()
                .map(|value| constant_value(value, self.settings.numeric_version)),
        );
        let name_base = extended.names.len() as u32;
        extended.names.extend(compiled.names.iter().cloned());
        let entry = extended.functions.len();
        for mut chunk in compiled.functions {
            for op in &mut chunk.code {
                *op = match *op {
                    Op::Const(index) => Op::Const(constant_count as u32 + index),
                    Op::LoadGlobal(index) => Op::LoadGlobal(slots[index as usize]),
                    Op::StoreGlobal(index) => Op::StoreGlobal(slots[index as usize]),
                    Op::StoreIndexGlobal { slot, n } => Op::StoreIndexGlobal {
                        slot: slots[slot as usize],
                        n,
                    },
                    Op::Call { func, argc } => Op::Call {
                        func: entry as u32 + func,
                        argc,
                    },
                    Op::CallExternal { name, argc } => {
                        let (func, linked) = functions[&compiled.names[name as usize]];
                        if linked {
                            Op::CallLinked { func, argc }
                        } else {
                            Op::Call { func, argc }
                        }
                    }
                    Op::Unsupported { name, argc } => Op::Unsupported {
                        name: name_base + name,
                        argc,
                    },
                    Op::AppVariable { name, argc, store } => Op::AppVariable {
                        name: name_base + name,
                        argc,
                        store,
                    },
                    Op::CasVariable { name, store } => Op::CasVariable {
                        name: name_base + name,
                        store,
                    },
                    other => other,
                }
            }
            extended.functions.push(chunk);
        }
        self.prog = Rc::new(extended);
        for function in entry..self.prog.functions.len() {
            self.expression_scopes.insert(
                function,
                Scope {
                    bindings: bindings.clone(),
                    source: crate::SourceIdentity::new("<expr>", text),
                },
            );
        }
        if let Some((image, _)) = &mut self.linked {
            image.program = self.prog.clone();
            for index in global_count..self.globals.len() {
                image
                    .global_owners
                    .push(if self.captured_globals.contains_key(&index) {
                        owner
                    } else {
                        None
                    });
            }
            image
                .function_owners
                .resize(self.prog.functions.len(), owner.unwrap());
        }
        let temporary = cleanup::Temporary {
            globals: global_count..self.globals.len(),
            constants: constant_count..self.consts.len(),
            functions: entry..self.prog.functions.len(),
            names: name_base as usize..self.prog.names.len(),
        };
        let result = self.execute(entry, vec![]);
        self.remove_expression_code(temporary);
        result
    }
}

#[derive(Clone, Copy)]
enum Binding {
    Home,
    Global(usize),
    Local(usize),
}

pub(super) struct Scope {
    bindings: BTreeMap<String, Binding>,
    pub(super) source: crate::SourceIdentity,
}

pub(super) fn initial_value(init: &GlobalInit) -> Value {
    match init {
        GlobalInit::Real(value) => Value::Real(*value),
        GlobalInit::Zero | GlobalInit::Setting(_) => Value::zero(),
        GlobalInit::EmptyList => Value::list(vec![]),
        GlobalInit::ZeroMatrix => Value::Matrix(Rc::new(Matrix::zeros(1, 1))),
        GlobalInit::ComplexZero => Value::Complex(Complex::new(Real::ZERO, Real::ZERO)),
        GlobalInit::Grob(index) => Value::Grob(*index),
    }
}
pub(super) fn constant_value(value: &Const, version: ppl_math::NumericVersion) -> Value {
    match value {
        Const::Real(value) => Value::Real(*value),
        Const::Number { source, legacy } => Value::Real(match version {
            ppl_math::NumericVersion::Legacy15157 => *legacy,
            ppl_math::NumericVersion::Current15751 => {
                Real::parse_versioned(source, version).expect("compiler-validated numeric literal")
            }
        }),
        Const::Pi => Value::Real(match version {
            ppl_math::NumericVersion::Legacy15157 => Real::parse("3.14159265359").unwrap(),
            ppl_math::NumericVersion::Current15751 => Real::exact_pi_multiple(false, 1, 1).unwrap(),
        }),
        Const::Int(value) => Value::Int(*value),
        Const::Complex(value) => Value::Complex(*value),
        Const::Str(value) => Value::str(value.as_str()),
        Const::Symbolic(value) => {
            Value::Symbolic(Rc::new(ppl_cas::CasValue::Symbolic(value.clone())))
        }
    }
}
