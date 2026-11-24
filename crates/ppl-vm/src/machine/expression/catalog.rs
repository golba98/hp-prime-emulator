//! Runtime catalog extension preserves live frame indices, the host, debugger,
//! calculator state and execution limits. No second interpreter is involved.
use super::*;
use ppl_compiler::{CompileOptions, CompiledProgram};
use std::collections::BTreeSet;

impl Machine {
    /// Preserve the IDE's immutable source snapshot on Home's fast compiled
    /// path as well. Registering a catalog does not load or initialize it.
    pub(crate) fn seed_expression_catalog(
        &mut self,
        catalog: &BTreeMap<String, crate::ModuleSource>,
    ) -> RtResult<()> {
        if !self.frames.is_empty() {
            return Err(RuntimeError::new(
                ErrorKind::InvalidInput,
                "Expression catalogs must be attached before execution",
            ));
        }
        if !catalog.is_empty() {
            self.ensure_expression_image();
            self.linked
                .as_mut()
                .unwrap()
                .0
                .catalog
                .extend(catalog.clone());
        }
        Ok(())
    }
    fn expression_catalog(&self) -> BTreeMap<String, crate::ModuleSource> {
        let mut sources = self
            .linked
            .as_ref()
            .map(|(image, _)| image.catalog.clone())
            .unwrap_or_default();
        for (name, source) in &self.programs {
            if !sources.values().any(|module| module.source == *source) {
                sources.insert(
                    name.clone(),
                    crate::ModuleSource {
                        identity: crate::SourceIdentity::new(name, source),
                        source: source.clone(),
                    },
                );
            }
        }
        sources
    }

    pub(super) fn expression_catalog_options(&self, options: &mut CompileOptions) {
        let local_functions: BTreeSet<_> = options.external_functions.keys().cloned().collect();
        let mut owners = BTreeMap::<String, usize>::new();
        for module in self.expression_catalog().values() {
            let (ast, diagnostics) = ppl_syntax::parse(&module.source);
            if diagnostics.has_errors() {
                continue;
            }
            for item in ast.items {
                match item {
                    ppl_syntax::ast::Item::Function(function) if function.exported => {
                        let name = function.name.name;
                        if local_functions.contains(&name)
                            || ppl_compiler::builtins::lookup(&name).is_some()
                            || ppl_compiler::is_system_variable(&name)
                        {
                            continue;
                        }
                        *owners.entry(name.clone()).or_default() += 1;
                        options
                            .external_functions
                            .entry(name)
                            .or_insert(function.params.len());
                    }
                    ppl_syntax::ast::Item::Globals {
                        exported: true,
                        vars,
                        ..
                    } => {
                        for variable in vars {
                            if !ppl_compiler::is_system_variable(&variable.name.name) {
                                options.external_variables.insert(variable.name.name);
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
        options.ambiguous_externals.extend(
            owners
                .into_iter()
                .filter(|(_, count)| *count > 1)
                .map(|(name, _)| name),
        );
    }

    /// Link only the exports actually referenced by this successfully compiled
    /// expression. A broken unrelated catalog entry is never executed or linked.
    pub(super) fn load_expression_catalog(
        &mut self,
        compiled: &CompiledProgram,
        functions: &BTreeMap<String, (u32, bool)>,
        bindings: &BTreeMap<String, Binding>,
        options: &CompileOptions,
    ) -> RtResult<()> {
        let mut wanted = BTreeSet::new();
        for chunk in compiled
            .functions
            .iter()
            .chain(std::iter::once(&compiled.init))
        {
            for op in &chunk.code {
                if let Op::CallExternal { name, .. } = op {
                    let name = &compiled.names[*name as usize];
                    if !functions.contains_key(name) {
                        wanted.insert(format!(
                            "{name}({})",
                            vec!["0"; options.external_functions[name]].join(",")
                        ));
                    }
                }
            }
        }
        for slot in &compiled.globals {
            if slot.external && !bindings.contains_key(&slot.name) {
                wanted.insert(slot.name.clone());
            }
        }
        if wanted.is_empty() {
            return Ok(());
        }
        self.check_limits()?;
        let mut sources = self.expression_catalog();
        let root = "<dynamic expression dependency>";
        let source = format!(
            "__PPL_EXPR_IMPORT__() BEGIN RETURN {{{}}}; END;",
            wanted.into_iter().collect::<Vec<_>>().join(",")
        );
        sources.insert(
            root.into(),
            crate::ModuleSource {
                identity: crate::SourceIdentity::new(root, &source),
                source,
            },
        );
        let home = self
            .home_variables
            .keys()
            .cloned()
            .chain(
                self.prog
                    .globals
                    .iter()
                    .enumerate()
                    .filter_map(|(index, slot)| {
                        let shared = self
                            .linked
                            .as_ref()
                            .map_or(!slot.program_var, |(image, _)| {
                                image.global_owners[index].is_none()
                            });
                        (shared && !slot.name.starts_with('<')).then(|| slot.name.clone())
                    }),
            )
            .collect();
        let imported =
            crate::linker::link_home_sources(&sources, root, self.settings.int_base, &home)?;
        self.check_limits()?;
        self.ensure_expression_image();
        self.append_expression_modules(imported)
    }

    fn ensure_expression_image(&mut self) {
        if self.linked.is_some() {
            return;
        }
        let mut program = (*self.prog).clone();
        let initializer = program.functions.len();
        let mut init = program.init.clone();
        init.name = "<dynamic root file variables>".into();
        program.functions.push(init);
        let program = Rc::new(program);
        let exports = program
            .functions
            .iter()
            .enumerate()
            .filter(|(index, chunk)| chunk.exported && !self.expression_scopes.contains_key(index))
            .map(|(index, chunk)| (chunk.name.clone(), (0, index as u32)))
            .collect();
        let image = crate::LinkedProgram {
            program: program.clone(),
            modules: vec![
                self.root_identity
                    .clone()
                    .unwrap_or_else(|| crate::SourceIdentity::new("<running program>", "")),
            ],
            global_owners: program
                .globals
                .iter()
                .map(|slot| slot.program_var.then_some(0))
                .collect(),
            function_owners: vec![0; program.functions.len()],
            initializers: vec![initializer],
            root: 0,
            exports,
            diagnostics: vec![],
            catalog: Default::default(),
        };
        self.prog = program;
        self.linked = Some((image, vec![self.init_done]));
    }

    fn append_expression_modules(&mut self, imported: crate::LinkedProgram) -> RtResult<()> {
        let (mut image, mut states) = self.linked.as_ref().unwrap().clone();
        let mut program = (*self.prog).clone();
        let mut module_map = vec![None; imported.modules.len()];
        let mut new_modules = Vec::new();
        for (index, id) in imported.modules.iter().enumerate() {
            if index == imported.root {
                continue;
            }
            if let Some(existing) = image
                .modules
                .iter()
                .position(|module| module.name == id.name)
            {
                if image.modules[existing].revision != id.revision {
                    return Err(RuntimeError::new(
                        ErrorKind::Unsupported,
                        "An executing catalog revision cannot be replaced by EXPR; edit between invocations",
                    ));
                }
                module_map[index] = Some(existing);
            } else {
                let mapped = image.modules.len();
                module_map[index] = Some(mapped);
                image.modules.push(id.clone());
                image.initializers.push(usize::MAX);
                states.push(
                    self.catalog_states
                        .get(&id.name)
                        .is_some_and(|state| state.revision == id.revision && state.initialized),
                );
                new_modules.push(mapped);
            }
        }
        let code_elements = |program: &CompiledProgram| {
            program
                .functions
                .iter()
                .chain(std::iter::once(&program.init))
                .fold(
                    program
                        .globals
                        .len()
                        .saturating_add(program.consts.len())
                        .saturating_add(program.names.len()),
                    |count, chunk| {
                        count
                            .saturating_add(chunk.code.len())
                            .saturating_add(chunk.locals as usize)
                            .saturating_add(1)
                    },
                )
        };
        let count = code_elements(&program).saturating_add(code_elements(&imported.program));
        if image.modules.len() > 1024 || count > self.limits.max_collection_len {
            return Err(RuntimeError::new(
                ErrorKind::ResourceLimit(ResourceKind::Memory),
                "EXPR linked code exceeds the module/code element limit",
            ));
        }
        let mut function_map = vec![usize::MAX; imported.program.functions.len()];
        let mut pending_functions = Vec::new();
        for (index, chunk) in imported.program.functions.iter().enumerate() {
            let owner = imported.function_owners[index];
            let Some(mapped) = module_map[owner] else {
                continue;
            };
            if new_modules.contains(&mapped) {
                let target = program.functions.len() + pending_functions.len();
                function_map[index] = target;
                if index == imported.initializers[owner] {
                    image.initializers[mapped] = target;
                }
                pending_functions.push((index, mapped, chunk.clone()));
            } else {
                function_map[index] = if index == imported.initializers[owner] {
                    image.initializers[mapped]
                } else {
                    program.functions.iter().enumerate().find(|(candidate,existing)|image.function_owners[*candidate]==mapped&&existing.name.rsplit("::").next()==chunk.name.rsplit("::").next()).map(|(index,_)|index).ok_or_else(||RuntimeError::new(ErrorKind::InvalidInput,"Existing catalog function revision does not match its linked symbols"))?
                };
            }
        }
        let mut global_map = Vec::new();
        let mut pending_values = Vec::new();
        let mut new_home = Vec::new();
        for (index, slot) in imported.program.globals.iter().enumerate() {
            let owner = imported.global_owners[index]
                .map(|owner| module_map[owner].expect("imported module owns its slots"));
            if let Some(existing) = program
                .globals
                .iter()
                .enumerate()
                .find(|(candidate, existing)| {
                    image.global_owners[*candidate] == owner && existing.name == slot.name
                })
                .map(|(index, _)| index)
            {
                global_map.push(existing);
                continue;
            }
            let value = if let Some(owner) = owner {
                let id = &image.modules[owner];
                self.catalog_states
                    .get(&id.name)
                    .filter(|state| state.revision == id.revision)
                    .and_then(|state| state.values.get(&slot.name))
                    .map(crate::PortableValue::to_value)
            } else {
                self.home_variables.get(&slot.name).cloned()
            }
            .unwrap_or_else(|| initial_value(&slot.init));
            self.check_value(&value)?;
            let target = program.globals.len();
            global_map.push(target);
            program.globals.push(slot.clone());
            image.global_owners.push(owner);
            if owner.is_none() && !matches!(slot.init, GlobalInit::Setting(_)) {
                new_home.push((slot.name.clone(), value.clone()));
            }
            pending_values.push((
                value,
                match slot.init {
                    GlobalInit::Setting(setting) => Some(setting),
                    _ => None,
                },
            ));
        }
        let constant_base = program.consts.len() as u32;
        let name_base = program.names.len() as u32;
        program
            .consts
            .extend(imported.program.consts.iter().cloned());
        program.names.extend(imported.program.names.iter().cloned());
        for (_, owner, mut chunk) in pending_functions {
            for op in &mut chunk.code {
                *op = match *op {
                    Op::Const(index) => Op::Const(constant_base + index),
                    Op::LoadGlobal(index) => Op::LoadGlobal(global_map[index as usize] as u32),
                    Op::StoreGlobal(index) => Op::StoreGlobal(global_map[index as usize] as u32),
                    Op::GeometryAssignmentResult(index) => {
                        Op::GeometryAssignmentResult(global_map[index as usize] as u32)
                    }
                    Op::StoreIndexGlobal { slot, n } => Op::StoreIndexGlobal {
                        slot: global_map[slot as usize] as u32,
                        n,
                    },
                    Op::Call { func, argc } => Op::Call {
                        func: function_map[func as usize] as u32,
                        argc,
                    },
                    Op::CallLinked { func, argc } => Op::CallLinked {
                        func: function_map[func as usize] as u32,
                        argc,
                    },
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
                };
            }
            // Linker's dependency roots are namespaced, preserving the
            // original root entry names used by the running caller.
            program.functions.push(chunk);
            image.function_owners.push(owner);
        }
        for (name, (owner, function)) in imported.exports {
            if let Some(owner) = module_map[owner] {
                image
                    .exports
                    .insert(name, (owner, function_map[function as usize] as u32));
            }
        }
        for (identity, diagnostics) in imported.diagnostics {
            if image
                .modules
                .iter()
                .enumerate()
                .any(|(owner, id)| new_modules.contains(&owner) && *id == identity)
            {
                image.diagnostics.push((identity, diagnostics));
            }
        }
        for (name, module) in imported.catalog {
            if name != "<dynamic expression dependency>" {
                image.catalog.insert(name, module);
            }
        }
        let dependencies: Vec<_> = module_map.into_iter().flatten().collect();
        for owner in new_modules {
            self.dynamic_dependencies
                .insert(owner, dependencies.clone());
        }
        self.home_variables.extend(new_home);
        for (value, setting) in pending_values {
            self.globals.push(value);
            self.setting_slots.push(setting);
        }
        self.consts.extend(
            imported
                .program
                .consts
                .iter()
                .map(|value| constant_value(value, self.settings.numeric_version)),
        );
        self.prog = Rc::new(program);
        image.program = self.prog.clone();
        self.linked = Some((image, states));
        Ok(())
    }
}
