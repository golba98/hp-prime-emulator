//! Link independently compiled Native modules without concatenating source.
use crate::{ErrorKind, RtResult, RuntimeError, SourceIdentity};
use ppl_compiler::{Chunk, CompiledProgram, GlobalSlot, Op};
use std::{collections::BTreeMap, rc::Rc};

#[derive(Clone, Debug)]
pub struct ModuleSource {
    pub identity: SourceIdentity,
    pub source: String,
}
#[derive(Clone)]
pub struct LinkedProgram {
    pub(crate) program: Rc<CompiledProgram>,
    pub(crate) modules: Vec<SourceIdentity>,
    pub(crate) global_owners: Vec<Option<usize>>,
    pub(crate) function_owners: Vec<usize>,
    pub(crate) initializers: Vec<usize>,
    pub(crate) root: usize,
    diagnostics: Vec<(SourceIdentity, ppl_syntax::Diagnostics)>,
}

impl LinkedProgram {
    pub fn diagnostics(&self) -> &[(SourceIdentity, ppl_syntax::Diagnostics)] {
        &self.diagnostics
    }
    pub fn program(&self) -> &CompiledProgram {
        &self.program
    }
    pub fn module_identities(&self) -> &[SourceIdentity] {
        &self.modules
    }

    /// Read-only graph of calculator slots reachable from one function and the
    /// file initializers selected by the Native module lifecycle. Compiler scopes distinguish local
    /// parameters/file globals from shared calculator globals. No VM code runs.
    pub fn calculator_dependencies(&self, entry: &str) -> RtResult<CalculatorDependencies> {
        let entry = self.program.function_index(entry).ok_or_else(|| {
            RuntimeError::new(ErrorKind::Undefined, "Dependency entry function is missing")
        })?;
        let mut output = CalculatorDependencies::default();
        // Machine::initialize_dependencies runs every selected module's file
        // initializer before the graph root. Include those actual reads even
        // when a module was linked by another function's uncalled dependency.
        let mut pending = self.initializers.clone();
        pending.push(entry);
        let mut functions = std::collections::BTreeSet::new();
        while let Some(function) = pending.pop() {
            if !functions.insert(function) {
                continue;
            }
            let owner = self.function_owners[function];
            pending.push(self.initializers[owner]);
            let chunk = &self.program.functions[function];
            let mut positions = vec![0usize];
            let mut seen = std::collections::BTreeSet::new();
            while let Some(position) = positions.pop() {
                if position >= chunk.code.len() || !seen.insert(position) {
                    continue;
                }
                let op = chunk.code[position];
                match op {
                    Op::LoadGlobal(slot)
                    | Op::StoreGlobal(slot)
                    | Op::StoreIndexGlobal { slot, .. } => {
                        if let Some(owner) = self.global_owners[slot as usize] {
                            pending.push(self.initializers[owner]);
                        } else {
                            let name = self.program.globals[slot as usize].name.clone();
                            if !matches!(op, Op::StoreGlobal(_)) {
                                output.reads.insert(name.clone());
                            }
                            if !matches!(op, Op::LoadGlobal(_)) {
                                output.writes.insert(name);
                            }
                        }
                    }
                    Op::Call { func, .. } | Op::CallLinked { func, .. } => {
                        pending.push(func as usize)
                    }
                    _ => (),
                }
                match op {
                    Op::Jump(target) => positions.push(target as usize),
                    Op::JumpIfFalse(target) | Op::TryBegin(target) => {
                        positions.push(target as usize);
                        positions.push(position + 1);
                    }
                    Op::Return | Op::Kill => (),
                    _ => positions.push(position + 1),
                }
            }
        }
        Ok(output)
    }
}

/// Shared calculator names potentially accessed by a reachable call graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CalculatorDependencies {
    pub reads: std::collections::BTreeSet<String>,
    pub writes: std::collections::BTreeSet<String>,
}

/// Build one execution image, sharing calculator slots but isolating file locals.
/// Imports/edits initialize through SessionContext::run_linked; this only links.
pub fn link_sources(
    sources: &BTreeMap<String, ModuleSource>,
    root: &str,
) -> RtResult<LinkedProgram> {
    link_sources_with_base(sources, root, ppl_math::IntBase::Hex)
}
pub fn link_sources_with_base(
    sources: &BTreeMap<String, ModuleSource>,
    root: &str,
    default_int_base: ppl_math::IntBase,
) -> RtResult<LinkedProgram> {
    link_sources_internal(sources, root, default_int_base, None)
}
/// Home may call installed program exports inside an arbitrary expression.
/// Calculator-owned names remain shared slots rather than module imports.
pub fn link_home_sources(
    sources: &BTreeMap<String, ModuleSource>,
    root: &str,
    base: ppl_math::IntBase,
    variables: &std::collections::BTreeSet<String>,
) -> RtResult<LinkedProgram> {
    link_sources_internal(sources, root, base, Some(variables))
}
fn link_sources_internal(
    sources: &BTreeMap<String, ModuleSource>,
    root: &str,
    default_int_base: ppl_math::IntBase,
    home: Option<&std::collections::BTreeSet<String>>,
) -> RtResult<LinkedProgram> {
    let mut signatures = BTreeMap::new();
    let mut owners = BTreeMap::<String, Vec<String>>::new();
    let mut variable_owners = BTreeMap::<String, Vec<String>>::new();
    for (name, module) in sources {
        let (ast, diags) = ppl_syntax::parse(&module.source);
        // Broken unrelated editor tabs must not block a valid dependency closure.
        if diags.has_errors() {
            continue;
        }
        for item in ast.items {
            if let ppl_syntax::ast::Item::Globals {
                exported: true,
                vars,
                ..
            } = &item
            {
                for var in vars {
                    if ppl_compiler::is_system_variable(&var.name.name)
                        || ppl_compiler::builtins::lookup(&var.name.name).is_some()
                    {
                        continue;
                    }
                    variable_owners
                        .entry(var.name.name.clone())
                        .or_default()
                        .push(name.clone());
                }
            }
            if let ppl_syntax::ast::Item::Function(f) = item {
                if f.exported
                    && ppl_compiler::builtins::lookup(&f.name.name).is_none()
                    && !ppl_compiler::is_system_variable(&f.name.name)
                {
                    owners
                        .entry(f.name.name.clone())
                        .or_default()
                        .push(name.clone());
                    signatures.entry(f.name.name).or_insert(f.params.len());
                }
            }
        }
    }
    if !sources.contains_key(root) {
        return Err(RuntimeError::new(
            ErrorKind::Undefined,
            "root program is not installed",
        ));
    }
    let mut options = ppl_compiler::CompileOptions {
        default_int_base,
        external_functions: signatures,
        external_variables: variable_owners.keys().cloned().collect(),
        ambiguous_externals: owners
            .iter()
            .filter(|(_, names)| names.len() > 1)
            .map(|(name, _)| name.clone())
            .collect(),
    };
    if let Some(home) = home {
        options.external_variables.extend(home.iter().cloned());
    }
    let mut diagnostics = Vec::new();
    let mut selected = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(name) = pending.pop() {
        if selected.contains_key(&name) {
            continue;
        }
        let module = &sources[&name];
        let file = ppl_syntax::SourceFile::new(&name, &module.source);
        let (unit, diags) = if name == root && home.is_some() {
            let (mut ast, parse_diags) = ppl_syntax::parse(&module.source);
            if parse_diags.has_errors() {
                return Err(compile_error(&name, &parse_diags));
            }
            let [ppl_syntax::ast::Item::Function(function)] = ast.items.as_mut_slice() else {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "Home expects an expression or statement",
                ));
            };
            if let Some(last) = function.body.last_mut()
                && let ppl_syntax::ast::StmtKind::Expr(expression) = &last.kind
            {
                last.kind = ppl_syntax::ast::StmtKind::Return(Some(expression.clone()));
            }
            let (mut unit, diags) = ppl_compiler::compile(&ast, &options);
            if let Some(unit) = &mut unit {
                for slot in &mut unit.globals {
                    if slot.external && home.is_some_and(|names| names.contains(&slot.name)) {
                        slot.external = false;
                        slot.program_var = false;
                    }
                }
            }
            (unit, diags)
        } else {
            ppl_compiler::compile_source(&file, &options)
        };
        let mut unit = unit.ok_or_else(|| compile_error(&name, &diags))?;
        // A dependency may be the only reader of a calculator-owned name.
        // Such slots must exist in the shared image even when the Home root
        // expression never mentions them directly.
        for slot in &mut unit.globals {
            if slot.external
                && home.is_some_and(|names| names.contains(&slot.name))
                && !variable_owners.contains_key(&slot.name)
            {
                slot.external = false;
                slot.program_var = false;
            }
        }
        diagnostics.push((module.identity.clone(), diags));
        for slot in unit.globals.iter().filter(|slot| slot.external) {
            if home.is_some_and(|home| home.contains(&slot.name))
                && !variable_owners.contains_key(&slot.name)
            {
                continue;
            }
            let candidates = &variable_owners[&slot.name];
            if candidates.len() != 1 || owners.contains_key(&slot.name) {
                return Err(RuntimeError::new(
                    ErrorKind::Undefined,
                    format!("ambiguous external variable {}", slot.name),
                ));
            }
            pending.push(candidates[0].clone());
        }
        for chunk in unit.functions.iter().chain(std::iter::once(&unit.init)) {
            for op in &chunk.code {
                if let Op::CallExternal { name: symbol, .. } = op {
                    let export = &unit.names[*symbol as usize];
                    let candidates = owners.get(export).ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Undefined,
                            format!("unresolved external {export}"),
                        )
                    })?;
                    if candidates.len() != 1 {
                        return Err(RuntimeError::new(
                            ErrorKind::Undefined,
                            format!("ambiguous export {export} in {}", candidates.join(", ")),
                        ));
                    }
                    pending.push(candidates[0].clone());
                }
            }
        }
        selected.insert(name, unit);
    }
    let modules: Vec<_> = selected
        .keys()
        .map(|name| sources[name].identity.clone())
        .collect();
    let root = selected.keys().position(|name| name == root).unwrap();
    let units: Vec<_> = selected.into_values().collect();
    let mut image = CompiledProgram {
        functions: vec![],
        consts: vec![],
        globals: vec![],
        names: vec![],
        init: empty_init(),
    };
    let mut global_owners = vec![];
    let mut shared_globals = BTreeMap::new();
    let mut global_maps = vec![];
    let mut exported_globals = BTreeMap::new();
    let mut function_bases = vec![];
    let mut constant_bases = vec![];
    let mut name_bases = vec![];
    let mut exports = BTreeMap::new();
    let mut function_owners = vec![];
    let mut initializers = vec![];
    for (module, unit) in units.iter().enumerate() {
        function_bases.push(image.functions.len() as u32);
        constant_bases.push(image.consts.len() as u32);
        name_bases.push(image.names.len() as u32);
        image.consts.extend(unit.consts.clone());
        image.names.extend(unit.names.clone());
        let mut globals = vec![];
        for slot in &unit.globals {
            if slot.external {
                globals.push(u32::MAX);
                continue;
            }
            let idx = if !slot.program_var {
                if let Some(&idx) = shared_globals.get(&slot.name) {
                    idx
                } else {
                    let idx = add_global(&mut image.globals, &mut global_owners, slot, None);
                    shared_globals.insert(slot.name.clone(), idx);
                    idx
                }
            } else {
                add_global(&mut image.globals, &mut global_owners, slot, Some(module))
            };
            if slot.exported {
                exported_globals.insert(slot.name.clone(), idx);
            }
            globals.push(idx);
        }
        global_maps.push(globals);
        for (index, chunk) in unit.functions.iter().enumerate() {
            let mut chunk = chunk.clone();
            if chunk.exported {
                exports.insert(
                    chunk.name.clone(),
                    (module, function_bases[module] + index as u32),
                );
            }
            if module != root {
                chunk.name = format!("{}::{}", modules[module].name, chunk.name);
                chunk.exported = false;
            }
            image.functions.push(chunk);
            function_owners.push(module);
        }
        initializers.push(image.functions.len());
        let mut init = unit.init.clone();
        init.name = format!("{}::<file variables>", modules[module].name);
        image.functions.push(init);
        function_owners.push(module);
    }
    for (module, unit) in units.iter().enumerate() {
        for (index, slot) in unit
            .globals
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.external)
        {
            global_maps[module][index] = *exported_globals
                .get(&slot.name)
                .or_else(|| shared_globals.get(&slot.name))
                .ok_or_else(|| {
                    RuntimeError::new(
                        ErrorKind::Undefined,
                        format!("unresolved external variable {}", slot.name),
                    )
                })?;
        }
    }
    for (index, chunk) in image.functions.iter_mut().enumerate() {
        let module = function_owners[index];
        for op in &mut chunk.code {
            *op = match *op {
                Op::Const(i) => Op::Const(constant_bases[module] + i),
                Op::LoadGlobal(i) => Op::LoadGlobal(global_maps[module][i as usize]),
                Op::StoreGlobal(i) => Op::StoreGlobal(global_maps[module][i as usize]),
                Op::StoreIndexGlobal { slot, n } => Op::StoreIndexGlobal {
                    slot: global_maps[module][slot as usize],
                    n,
                },
                Op::Call { func, argc } => Op::Call {
                    func: function_bases[module] + func,
                    argc,
                },
                Op::CallExternal { name, argc } => {
                    let symbol = &units[module].names[name as usize];
                    let &(_, func) = exports.get(symbol).ok_or_else(|| {
                        RuntimeError::new(
                            ErrorKind::Undefined,
                            format!("unresolved external {symbol}"),
                        )
                    })?;
                    Op::CallLinked { func, argc }
                }
                Op::Unsupported { name, argc } => Op::Unsupported {
                    name: name_bases[module] + name,
                    argc,
                },
                other => other,
            };
        }
    }
    Ok(LinkedProgram {
        program: Rc::new(image),
        modules,
        global_owners,
        function_owners,
        initializers,
        root,
        diagnostics,
    })
}
fn add_global(
    globals: &mut Vec<GlobalSlot>,
    owners: &mut Vec<Option<usize>>,
    slot: &GlobalSlot,
    owner: Option<usize>,
) -> u32 {
    let index = globals.len() as u32;
    globals.push(slot.clone());
    owners.push(owner);
    index
}
fn compile_error(name: &str, diags: &ppl_syntax::Diagnostics) -> RuntimeError {
    RuntimeError::new(
        ErrorKind::InvalidInput,
        format!(
            "{name}: {}",
            diags
                .errors()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ),
    )
}
fn empty_init() -> Chunk {
    Chunk {
        name: "<linked>".into(),
        params: 0,
        locals: 0,
        local_names: vec![],
        code: vec![Op::Zero, Op::Return],
        spans: vec![ppl_syntax::Span::default(); 2],
        exported: false,
        span: ppl_syntax::Span::default(),
    }
}
