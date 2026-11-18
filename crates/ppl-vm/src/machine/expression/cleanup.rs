//! Temporary expression code has stable arena indices while it executes.
//! Nested EXPR may append persistent modules after it; retire only this call's
//! ranges, never truncate or restore away another call's imported module.
use super::*;
use std::ops::Range;

pub(super) struct Temporary {
    pub globals: Range<usize>,
    pub constants: Range<usize>,
    pub functions: Range<usize>,
    pub names: Range<usize>,
}
fn retire<T>(values: &mut Vec<T>, range: Range<usize>, mut empty: impl FnMut() -> T) {
    if range.end == values.len() {
        values.truncate(range.start);
    } else {
        for value in &mut values[range] {
            *value = empty();
        }
    }
}
impl Machine {
    pub(super) fn remove_expression_code(&mut self, temporary: Temporary) {
        // Only slots belonging to this expression can represent newly seen
        // Home names. Appended imported modules own independent private slots.
        for index in temporary.globals.clone() {
            if !self.captured_globals.contains_key(&index)
                && self.setting_slots[index].is_none()
                && self.retain_home_slot(index)
            {
                self.home_variables.insert(
                    self.prog.globals[index].name.clone(),
                    self.globals[index].clone(),
                );
            }
        }
        self.scoped_plot_slots
            .retain(|index| !temporary.globals.contains(index));
        self.legacy_plot_writes
            .retain(|index| !temporary.globals.contains(index));
        self.captured_globals
            .retain(|index, _| !temporary.globals.contains(index));
        self.expression_scopes
            .retain(|index, _| !temporary.functions.contains(index));
        let mut program = (*self.prog).clone();
        retire(&mut program.globals, temporary.globals.clone(), || {
            ppl_compiler::GlobalSlot {
                name: "<retired expression>".into(),
                init: GlobalInit::Zero,
                program_var: true,
                exported: false,
                external: false,
                read_only: true,
            }
        });
        retire(&mut self.globals, temporary.globals.clone(), Value::zero);
        retire(&mut self.setting_slots, temporary.globals.clone(), || None);
        retire(&mut program.consts, temporary.constants.clone(), || {
            Const::Real(Real::ZERO)
        });
        retire(&mut self.consts, temporary.constants, Value::zero);
        retire(&mut program.names, temporary.names, String::new);
        retire(&mut program.functions, temporary.functions.clone(), || {
            ppl_compiler::Chunk {
                name: "<retired expression>".into(),
                params: 0,
                locals: 0,
                local_names: vec![],
                local_scopes: vec![],
                code: vec![Op::Zero, Op::Return],
                spans: vec![Span::default(); 2],
                exported: false,
                span: Span::default(),
            }
        });
        self.prog = Rc::new(program);
        if let Some((image, _)) = &mut self.linked {
            let owner = image.root;
            retire(&mut image.global_owners, temporary.globals, || Some(owner));
            retire(&mut image.function_owners, temporary.functions, || owner);
            image.program = self.prog.clone();
        }
    }
}
