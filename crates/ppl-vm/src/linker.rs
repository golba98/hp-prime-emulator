//! Link independently compiled Native modules without concatenating source.
use crate::{ErrorKind, RtResult, RuntimeError, SourceIdentity};
use ppl_compiler::{Chunk, CompiledProgram, GlobalSlot, Op};
use std::{collections::BTreeMap, rc::Rc};

#[derive(Clone)]
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
}

/// Build one execution image, sharing calculator slots but isolating file locals.
/// Imports/edits initialize through SessionContext::run_linked; this only links.
pub fn link_sources(
    sources: &BTreeMap<String, ModuleSource>,
    root: &str,
) -> RtResult<LinkedProgram> {
    let mut signatures = BTreeMap::new();
    let mut owners = BTreeMap::<String, Vec<String>>::new();
    for (name, module) in sources {
        let (ast, diags) = ppl_syntax::parse(&module.source);
        // Broken unrelated editor tabs must not block a valid dependency closure.
        if diags.has_errors() {
            continue;
        }
        for item in ast.items {
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
    let options = ppl_compiler::CompileOptions {
        external_functions: signatures,
        ambiguous_externals: owners
            .iter()
            .filter(|(_, names)| names.len() > 1)
            .map(|(name, _)| name.clone())
            .collect(),
        ..Default::default()
    };
    let mut diagnostics = Vec::new();
    let mut selected = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(name) = pending.pop() {
        if selected.contains_key(&name) {
            continue;
        }
        let module = &sources[&name];
        let file = ppl_syntax::SourceFile::new(&name, &module.source);
        let (unit, diags) = ppl_compiler::compile_source(&file, &options);
        let unit = unit.ok_or_else(|| compile_error(&name, &diags))?;
        diagnostics.push((module.identity.clone(), diags));
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
