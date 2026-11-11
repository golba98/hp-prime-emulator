//! App definitions shared by symbolic entry, plot and numeric views.
//! Sampling executes native PPL on a worker; egui never evaluates user code.
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

pub type CurveSamples = Vec<Option<[f64; 2]>>;
pub type PlotSamples = Vec<CurveSamples>;
pub type PlotResult = Result<PlotSamples, String>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphKind {
    Function,
    Parametric,
    Polar,
    Sequence,
    Implicit,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Curve {
    pub expression: String,
    pub second: String,
    pub enabled: bool,
    #[serde(default)]
    pub sequence: Option<SequenceDefinition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceDefinition {
    pub start: i64,
    pub initial: Vec<String>,
    #[serde(default)]
    pub offset: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotWindow {
    pub xmin: f64,
    pub xmax: f64,
    pub ymin: f64,
    pub ymax: f64,
    pub tmin: f64,
    pub tmax: f64,
    pub axes: bool,
}
impl Default for PlotWindow {
    fn default() -> Self {
        Self {
            xmin: -10.,
            xmax: 10.,
            ymin: -10.,
            ymax: 10.,
            tmin: 0.,
            tmax: std::f64::consts::TAU,
            axes: true,
        }
    }
}
impl PlotWindow {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.xmin, self.xmax, self.ymin, self.ymax, self.tmin, self.tmax,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || self.xmin >= self.xmax
            || self.ymin >= self.ymax
            || self.tmin >= self.tmax
        {
            return Err("Plot bounds must be finite and increasing".into());
        }
        Ok(())
    }
    pub fn zoom(&mut self, factor: f64) {
        if !factor.is_finite() || factor <= 0. {
            return;
        }
        let x = (self.xmin + self.xmax) / 2.;
        let y = (self.ymin + self.ymax) / 2.;
        let dx = (self.xmax - self.xmin) * factor / 2.;
        let dy = (self.ymax - self.ymin) * factor / 2.;
        if dx.is_finite() && dy.is_finite() && dx > f64::EPSILON && dy > f64::EPSILON {
            self.xmin = x - dx;
            self.xmax = x + dx;
            self.ymin = y - dy;
            self.ymax = y + dy;
        }
    }
    pub fn pan(&mut self, x: f64, y: f64) {
        let dx = x * (self.xmax - self.xmin);
        let dy = y * (self.ymax - self.ymin);
        if dx.is_finite() && dy.is_finite() {
            self.xmin += dx;
            self.xmax += dx;
            self.ymin += dy;
            self.ymax += dy;
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphApp {
    pub kind: GraphKind,
    pub curves: Vec<Curve>,
    pub window: PlotWindow,
    pub numeric_start: f64,
    pub numeric_step: f64,
    #[serde(default)]
    pub numeric_y: f64,
}
impl GraphApp {
    pub fn new(kind: GraphKind) -> Self {
        Self {
            kind,
            curves: vec![Curve {
                expression: String::new(),
                second: String::new(),
                enabled: true,
                sequence: None,
            }],
            window: Default::default(),
            numeric_start: if kind == GraphKind::Sequence { 1. } else { 0. },
            numeric_step: 1.,
            numeric_y: 0.,
        }
    }
    fn resolved_context(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: &AtomicBool,
    ) -> Result<ppl_vm::SessionSnapshot, String> {
        let independent: &[&str] = match self.kind {
            GraphKind::Function => &["X"],
            GraphKind::Parametric => &["T"],
            GraphKind::Polar => &["θ"],
            GraphKind::Sequence => &["N"],
            GraphKind::Implicit => &["X", "Y", "x", "y"],
        };
        let mut expressions = Vec::new();
        for curve in self
            .curves
            .iter()
            .filter(|curve| curve.enabled && !curve.expression.trim().is_empty())
        {
            expressions.push(curve.expression.as_str());
            if self.kind == GraphKind::Parametric {
                expressions.push(curve.second.as_str());
            }
            if self.kind == GraphKind::Sequence
                && let Some(config) = &curve.sequence
            {
                expressions.extend(config.initial.iter().map(String::as_str));
            }
        }
        super::native_graph_context::resolve(snapshot, &expressions, independent, cancel)
    }
    /// Every curve evaluates against the same immutable calculator revision.
    pub fn sample(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        count: usize,
        cancel: Arc<AtomicBool>,
    ) -> PlotResult {
        self.window.validate()?;
        let memory = self.resolved_context(snapshot, &cancel)?;
        let snapshot = &memory;
        if self.kind == GraphKind::Implicit {
            return self.implicit_sample(snapshot, count, cancel);
        }
        let count = count.clamp(2, 4096);
        let mut curves = Vec::new();
        let parameter = matches!(self.kind, GraphKind::Parametric | GraphKind::Polar);
        let (lo, hi) = if parameter {
            (self.window.tmin, self.window.tmax)
        } else {
            (self.window.xmin, self.window.xmax)
        };
        let positions: Vec<_> = if self.kind == GraphKind::Sequence {
            let (lo, hi) = (lo.ceil(), hi.floor());
            if lo > hi {
                Vec::new()
            } else {
                let count = ((hi - lo + 1.).min(count as f64)) as usize;
                (0..count)
                    .map(|i| {
                        if count == 1 {
                            lo
                        } else {
                            lo + (i as f64 * (hi - lo) / (count - 1) as f64).round()
                        }
                    })
                    .collect()
            }
        } else {
            (0..count)
                .map(|i| lo + (hi - lo) * i as f64 / (count - 1) as f64)
                .collect()
        };
        for (index, definition) in self
            .curves
            .iter()
            .enumerate()
            .filter(|(_, c)| c.enabled && !c.expression.trim().is_empty())
        {
            check_cancel(&cancel)?;
            let mut evaluator =
                CurveEvaluator::new(self.kind, index, definition, snapshot, cancel.clone())?;
            let mut second = if self.kind == GraphKind::Parametric {
                Some(Evaluator::new(
                    GraphKind::Parametric,
                    &definition.second,
                    snapshot,
                    cancel.clone(),
                    None,
                )?)
            } else {
                None
            };
            let mut points = Vec::with_capacity(positions.len());
            for &t in &positions {
                check_cancel(&cancel)?;
                let first = evaluator.at(t)?;
                let point = match self.kind {
                    GraphKind::Parametric => match (first, second.as_mut().unwrap().at(t)?) {
                        (Some(x), Some(y)) => Some([x, y]),
                        _ => None,
                    },
                    GraphKind::Polar => first.and_then(|r| {
                        let angle = snapshot
                            .settings
                            .to_radians(ppl_math::Real::from_f64(t).ok()?);
                        Some([r * angle.cos(), r * angle.sin()])
                    }),
                    _ => first.map(|y| [t, y]),
                };
                points.push(point.filter(|p| p.iter().all(|v| v.is_finite())));
            }
            curves.push(points);
        }
        Ok(curves)
    }
    /// Function/sequence: X(or N), values. Polar: theta, radii. Parametric:
    /// T followed by X(T),Y(T) for each enabled definition, in display order.
    pub fn table(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        rows: usize,
    ) -> Result<Vec<Vec<Option<f64>>>, String> {
        self.table_with_cancel(snapshot, rows, Arc::new(AtomicBool::new(false)))
    }
    pub fn table_with_cancel(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        rows: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<Vec<Option<f64>>>, String> {
        check_cancel(&cancel)?;
        if !self.numeric_start.is_finite()
            || !self.numeric_step.is_finite()
            || self.numeric_step == 0.
        {
            return Err("Numeric start/step must be finite; step cannot be zero".into());
        }
        if self.kind == GraphKind::Sequence
            && (self.numeric_start.fract() != 0. || self.numeric_step.fract() != 0.)
        {
            return Err("Sequence table start and step must be integers".into());
        }
        let memory = self.resolved_context(snapshot, &cancel)?;
        let snapshot = &memory;
        if self.kind == GraphKind::Implicit {
            return self.implicit_table(snapshot, rows, cancel);
        }
        let mut functions = Vec::new();
        for (index, c) in self
            .curves
            .iter()
            .enumerate()
            .filter(|(_, c)| c.enabled && !c.expression.trim().is_empty())
        {
            functions.push((
                CurveEvaluator::new(self.kind, index, c, snapshot, cancel.clone())?,
                if self.kind == GraphKind::Parametric {
                    Some(Evaluator::new(
                        GraphKind::Parametric,
                        &c.second,
                        snapshot,
                        cancel.clone(),
                        None,
                    )?)
                } else {
                    None
                },
            ));
        }
        let mut table = Vec::new();
        for i in 0..rows.min(1000) {
            check_cancel(&cancel)?;
            let x = self.numeric_start + self.numeric_step * i as f64;
            if !x.is_finite() {
                return Err("Numeric table coordinate overflow".into());
            }
            let mut row = vec![Some(x)];
            for (first, second) in &mut functions {
                row.push(first.at(x)?);
                if let Some(second) = second {
                    row.push(second.at(x)?);
                }
            }
            table.push(row);
        }
        Ok(table)
    }
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Plot cancelled".into())
    } else {
        Ok(())
    }
}
fn numeric(result: ppl_vm::RtResult<ppl_vm::Value>) -> Result<Option<f64>, String> {
    if matches!(&result,Ok(ppl_vm::Value::Complex(c)) if !c.im.is_zero()) {
        return Ok(None);
    }
    match result.and_then(|v| v.expect_real("Graph")) {
        Ok(v) => Ok(Some(v.to_f64()).filter(|v| v.is_finite())),
        Err(e)
            if matches!(
                e.kind,
                ppl_vm::ErrorKind::DivisionByZero
                    | ppl_vm::ErrorKind::Overflow
                    | ppl_vm::ErrorKind::NonReal
                    | ppl_vm::ErrorKind::Undefined
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e.to_string()),
    }
}
/// A compiled and linked private machine, seeded through typed PPL parameters.
/// This uses real VM global slots; Home values are never pasted into formula text.
struct Evaluator {
    machine: ppl_vm::Machine,
    entry: String,
    sequence_append: Option<String>,
    sequence_reset: Option<String>,
    implicit: bool,
}
impl Evaluator {
    fn new(
        kind: GraphKind,
        expression: &str,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: Arc<AtomicBool>,
        sequence: Option<&str>,
    ) -> Result<Self, String> {
        Self::build(kind, expression, snapshot, cancel, sequence)
    }
    fn implicit(
        expression: &str,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        Self::build(GraphKind::Implicit, expression, snapshot, cancel, None)
    }
    fn build(
        kind: GraphKind,
        expression: &str,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: Arc<AtomicBool>,
        sequence: Option<&str>,
    ) -> Result<Self, String> {
        use std::collections::BTreeMap;
        let implicit = kind == GraphKind::Implicit;
        let mut prefix = "__SIDE_GRAPH".to_string();
        while snapshot.programs.values().any(|s| s.contains(&prefix))
            || snapshot.globals.keys().any(|s| s.starts_with(&prefix))
        {
            prefix.push('_');
        }
        let entry = format!("{prefix}_AT");
        let mut source = String::new();
        let namespace = super::native_graph_context::namespace(snapshot);
        for name in namespace
            .iter()
            .filter(|n| !ppl_compiler::is_system_variable(n) && Some(n.as_str()) != sequence)
        {
            if !valid_name(name) {
                return Err(format!("Invalid calculator variable name {name:?}"));
            }
            source.push_str(&format!("EXPORT {name};\n"));
        }
        let mut seeds = Vec::new();
        let mut statements = String::new();
        let mut args = Vec::new();
        let mut seed_index = 0;
        let mut flush =
            |source: &mut String, statements: &mut String, args: &mut Vec<ppl_vm::Value>| {
                if args.is_empty() {
                    return;
                }
                let name = format!("{prefix}_SEED{seed_index}");
                seed_index += 1;
                let params = (0..args.len())
                    .map(|i| format!("{prefix}_P{i}"))
                    .collect::<Vec<_>>()
                    .join(",");
                source.push_str(&format!(
                    "EXPORT {name}({params}) BEGIN {statements} RETURN 0; END;\n"
                ));
                seeds.push((name, std::mem::take(args)));
                statements.clear();
            };
        for (name, value) in &snapshot.globals {
            let bytes = name.as_bytes();
            // G0-G9 are read-only graphics references, not assignable value slots.
            if bytes.len() == 2 && bytes[0] == b'G' && bytes[1].is_ascii_digit()
                || Some(name.as_str()) == sequence
            {
                continue;
            }
            if args.len() == 64 {
                flush(&mut source, &mut statements, &mut args);
            }
            statements.push_str(&format!("{name}:={prefix}_P{};", args.len()));
            args.push(value.to_value());
        }
        flush(&mut source, &mut statements, &mut args);
        let mut sequence_append = None;
        let mut sequence_reset = None;
        if let Some(name) = sequence {
            let values = format!("{prefix}_VALUES");
            let start = format!("{prefix}_START");
            let append = format!("{prefix}_APPEND");
            let reset = format!("{prefix}_RESET");
            source.push_str(&format!("LOCAL {values}:={{}}; LOCAL {start}:=0; EXPORT {name}(K) BEGIN RETURN {values}(K-{start}+1); END; EXPORT {reset}(V,K) BEGIN {values}:=V; {start}:=K; RETURN 0; END; EXPORT {append}(V) BEGIN {values}:=CONCAT({values},{{V}}); RETURN V; END;\n"));
            sequence_append = Some(append);
            sequence_reset = Some(reset);
        }
        // Only this app's independent variables are function parameters.
        // Other calculator variables keep the real seeded Home values. In
        // particular Function X+T must not silently become 2*X.
        let parameters = match kind {
            GraphKind::Function => "X",
            GraphKind::Parametric => "T",
            GraphKind::Polar => "θ",
            GraphKind::Sequence => "N",
            GraphKind::Implicit => "X,Y,x,y",
        };
        source.push_str(&format!(
            "EXPORT {entry}({parameters}) BEGIN RETURN {expression}; END;"
        ));
        let mut catalog: BTreeMap<_, _> = snapshot
            .programs
            .iter()
            .map(|(name, source)| {
                (
                    name.clone(),
                    ppl_vm::ModuleSource {
                        identity: ppl_vm::SourceIdentity::new(name, source),
                        source: source.clone(),
                    },
                )
            })
            .collect();
        let root = "<side-graph>";
        catalog.insert(
            root.into(),
            ppl_vm::ModuleSource {
                identity: ppl_vm::SourceIdentity::new(root, &source),
                source,
            },
        );
        let image = ppl_vm::link_sources_with_base(&catalog, root, snapshot.settings.int_base)
            .map_err(|e| e.to_string())?;
        let host = GraphHost(ppl_vm::HeadlessHost {
            interrupt: Some(cancel),
            ..Default::default()
        });
        let mut machine = ppl_vm::Machine::new_linked(
            image,
            Box::new(host),
            snapshot.settings.clone(),
            ppl_vm::Limits {
                max_steps: Some(if implicit { 50_000_000 } else { 5_000_000 }),
                max_time: Some(std::time::Duration::from_secs(3)),
                ..Default::default()
            },
        );
        machine.active_app = snapshot.active_app.clone();
        machine.app_settings = snapshot.app_settings.clone();
        machine.app_files = snapshot.app_files.clone();
        machine.cas_context = snapshot.cas.clone();
        machine.home_variables = snapshot
            .globals
            .iter()
            .map(|(name, value)| (name.clone(), value.to_value()))
            .collect();
        machine
            .seed_object_stores(&snapshot.notes, &snapshot.programs)
            .map_err(|e| e.to_string())?;
        for (name, args) in seeds {
            machine
                .call_function(&name, args)
                .map_err(|e| e.to_string())?;
        }
        machine
            .initialize_dependencies()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            machine,
            entry,
            sequence_append,
            sequence_reset,
            implicit,
        })
    }
    fn at_value(&mut self, x: f64) -> ppl_vm::RtResult<ppl_vm::Value> {
        self.at_xy_value(x, 0.)
    }
    fn at_xy_value(&mut self, x: f64, y: f64) -> ppl_vm::RtResult<ppl_vm::Value> {
        let x = ppl_vm::Value::Real(ppl_math::Real::from_f64(x)?);
        let args = if self.implicit {
            let y = ppl_vm::Value::Real(ppl_math::Real::from_f64(y)?);
            vec![x.clone(), y.clone(), x, y]
        } else {
            vec![x]
        };
        self.machine.call_function(&self.entry, args)
    }

    fn at(&mut self, x: f64) -> Result<Option<f64>, String> {
        numeric(self.at_value(x))
    }
}
struct GraphHost(ppl_vm::HeadlessHost);
impl ppl_vm::Host for GraphHost {
    fn print(&mut self, text: &str) {
        ppl_vm::Host::print(&mut self.0, text);
    }
    fn poll_key(&mut self) -> ppl_vm::RtResult<Option<i64>> {
        ppl_vm::Host::poll_key(&mut self.0)
    }
    fn is_key_down(&mut self, key: i64) -> ppl_vm::RtResult<bool> {
        ppl_vm::Host::is_key_down(&mut self.0, key)
    }
    fn wait_key(&mut self, t: Option<f64>) -> ppl_vm::RtResult<Option<i64>> {
        ppl_vm::Host::wait_key(&mut self.0, t)
    }
    fn sleep(&mut self, t: f64) -> ppl_vm::RtResult<()> {
        ppl_vm::Host::sleep(&mut self.0, t)
    }
    fn touch(&mut self) -> ppl_vm::RtResult<Option<ppl_vm::host::Touch>> {
        ppl_vm::Host::touch(&mut self.0)
    }
    fn dialog(
        &mut self,
        d: &ppl_vm::host::Dialog,
        g: &ppl_graphics::Grob,
    ) -> ppl_vm::RtResult<ppl_vm::host::DialogResult> {
        ppl_vm::Host::dialog(&mut self.0, d, g)
    }
    fn now_ms(&mut self) -> f64 {
        ppl_vm::Host::now_ms(&mut self.0)
    }
    fn local_time(&mut self) -> (i64, i64, i64, i64, i64, f64) {
        ppl_vm::Host::local_time(&mut self.0)
    }
    fn interrupt_flag(&self) -> Option<Arc<AtomicBool>> {
        self.0.interrupt.clone()
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_alphabetic() || i > 0 && c.is_ascii_digit())
}
enum CurveEvaluator {
    Plain(Evaluator),
    ExplicitSequence {
        evaluator: Evaluator,
        start: i64,
        offset: u8,
    },
    Sequence {
        evaluator: Evaluator,
        start: i64,
        offset: u8,
        values: Vec<ppl_vm::Value>,
    },
}
impl CurveEvaluator {
    fn new(
        kind: GraphKind,
        index: usize,
        curve: &Curve,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if kind != GraphKind::Sequence {
            return Ok(Self::Plain(Evaluator::new(
                kind,
                &curve.expression,
                snapshot,
                cancel,
                None,
            )?));
        }
        let name = format!("U{}", index + 1);
        let config = curve.sequence.clone().unwrap_or(SequenceDefinition {
            start: 1,
            initial: vec![],
            offset: 0,
        });
        if config.start < 0
            || config.start > 999_999_995_904
            || config.initial.len() > 2
            || config.offset > 2
        {
            return Err("Sequence start must be non-negative, with at most two initial terms and offset 0, 1 or 2".into());
        }
        let order = recurrence_order(&curve.expression, &name, config.offset)?;
        if order > config.initial.len() {
            return Err(format!("{name} requires {order} initial term(s)"));
        }
        if order == 0 && config.initial.is_empty() {
            return Ok(Self::ExplicitSequence {
                evaluator: Evaluator::new(
                    GraphKind::Sequence,
                    &curve.expression,
                    snapshot,
                    cancel,
                    None,
                )?,
                start: config.start,
                offset: config.offset,
            });
        }
        let mut values = Vec::new();
        for (initial, expression) in config.initial.iter().enumerate() {
            let mut evaluator = Evaluator::new(
                GraphKind::Sequence,
                expression,
                snapshot,
                cancel.clone(),
                None,
            )?;
            let value = evaluator
                .at_value((config.start + initial as i64) as f64)
                .map_err(|e| e.to_string())?;
            value
                .expect_real("Sequence initial term")
                .map_err(|e| e.to_string())?;
            values.push(value);
        }
        let mut evaluator = Evaluator::new(
            GraphKind::Sequence,
            &curve.expression,
            snapshot,
            cancel,
            Some(&name),
        )?;
        evaluator
            .machine
            .call_function(
                evaluator.sequence_reset.as_ref().unwrap(),
                vec![
                    ppl_vm::Value::list(values.clone()),
                    ppl_vm::Value::from_i64(config.start),
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(Self::Sequence {
            evaluator,
            start: config.start,
            offset: config.offset,
            values,
        })
    }
    fn at(&mut self, x: f64) -> Result<Option<f64>, String> {
        match self {
            Self::Plain(e) => e.at(x),
            Self::ExplicitSequence {
                evaluator,
                start,
                offset,
            } => {
                if x.fract() != 0. || x < *start as f64 {
                    return Ok(None);
                }
                evaluator.at(x - *offset as f64)
            }
            Self::Sequence {
                evaluator,
                start,
                offset,
                values,
            } => {
                if x.fract() != 0. || x < *start as f64 {
                    return Ok(None);
                }
                if x > *start as f64 + 4095. {
                    return Err("Sequence request exceeds the 4096-term operation budget".into());
                }
                let index = (x - *start as f64) as usize;
                while values.len() <= index {
                    let n = *start as f64 + values.len() as f64 - *offset as f64;
                    let value = evaluator.at_value(n).map_err(|e| e.to_string())?;
                    value
                        .expect_real("Sequence term")
                        .map_err(|e| e.to_string())?;
                    evaluator
                        .machine
                        .call_function(
                            evaluator.sequence_append.as_ref().unwrap(),
                            vec![value.clone()],
                        )
                        .map_err(|e| e.to_string())?;
                    values.push(value);
                }
                numeric(Ok(values[index].clone()))
            }
        }
    }
}
fn recurrence_order(expression: &str, name: &str, offset: u8) -> Result<usize, String> {
    use ppl_syntax::ast::{BinOp, Expr, ExprKind as E};
    fn shift(e: &Expr) -> Option<i64> {
        match &e.kind {
            E::Ident(n) if n == "N" => Some(0),
            E::Binary(op, a, b) if matches!(a.kind,E::Ident(ref n)if n=="N") => {
                let E::Number(v) = &b.kind else {
                    return None;
                };
                let n = v.parse::<i64>().ok()?;
                match op {
                    BinOp::Add => Some(n),
                    BinOp::Sub => Some(-n),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    fn walk(e: &Expr, name: &str, offset: u8, order: &mut usize) -> Result<(), String> {
        match &e.kind {
            E::Call(c, args) if matches!(c.kind,E::Ident(ref n)if n==name) => {
                if args.len() != 1 {
                    return Err("A sequence reference needs exactly one index".into());
                }
                let lag = offset as i64
                    - shift(&args[0]).ok_or(
                        "Sequence recurrence references must use N, N-1, N-2 or N+1 as appropriate",
                    )?;
                if !(1..=2).contains(&lag) {
                    return Err(
                        "A sequence recurrence may reference only the preceding one or two terms"
                            .into(),
                    );
                }
                *order = (*order).max(lag as usize);
            }
            E::Unary(_, e) | E::Postfix(_, e) | E::Quoted(e) => walk(e, name, offset, order)?,
            E::Binary(_, a, b) | E::Assign(a, b) | E::Store(a, b) | E::ComplexPair(a, b) => {
                walk(a, name, offset, order)?;
                walk(b, name, offset, order)?;
            }
            E::Call(_, args) | E::Index(_, args) | E::List(args) | E::Bracket(args) => {
                for e in args {
                    walk(e, name, offset, order)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let (ast, diags) = ppl_syntax::parse_expression(expression);
    if diags.has_errors() {
        return Err(diags.errors().next().unwrap().message.clone());
    }
    let mut order = 0;
    walk(&ast, name, offset, &mut order)?;
    Ok(order)
}
pub type TableResult = Result<Vec<Vec<Option<f64>>>, String>;
pub struct TableJob {
    rx: mpsc::Receiver<TableResult>,
    cancel: Arc<AtomicBool>,
}
impl TableJob {
    pub fn start(app: GraphApp, snapshot: ppl_vm::SessionSnapshot) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            let _ = tx.send(app.table_with_cancel(&snapshot, 50, flag));
        });
        Self { rx, cancel }
    }
    pub fn poll(&self) -> Option<TableResult> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Calculator plot worker exited without a result".into()))
            }
        }
    }
}
impl Drop for TableJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
pub struct PlotJob {
    rx: mpsc::Receiver<PlotResult>,
    cancel: Arc<AtomicBool>,
}
impl PlotJob {
    pub fn start(app: GraphApp, snapshot: ppl_vm::SessionSnapshot) -> Self {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            let _ = tx.send(app.sample(&snapshot, 640, flag));
        });
        Self { rx, cancel }
    }
    pub fn poll(&self) -> Option<PlotResult> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("Calculator plot worker exited without a result".into()))
            }
        }
    }
}
impl Drop for PlotJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> ppl_vm::SessionSnapshot {
        ppl_vm::SessionContext::default().snapshot()
    }
    fn cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a}!={b}");
    }
    #[test]
    fn graph_and_table_seed_actual_home_values_and_new_revisions() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves[0].expression = "A*X".into();
        app.window.xmin = 0.;
        app.window.xmax = 2.;
        app.numeric_start = 2.;
        let mut context = ppl_vm::SessionContext::default();
        context
            .evaluate_home(
                "A:=2",
                Box::new(ppl_vm::HeadlessHost::default()),
                Default::default(),
            )
            .result
            .unwrap();
        let first = context.snapshot();
        assert_eq!(
            app.sample(&first, 3, cancel()).unwrap()[0][2],
            Some([2., 4.])
        );
        assert_eq!(app.table(&first, 1).unwrap()[0], vec![Some(2.), Some(4.)]);
        context
            .evaluate_home(
                "A:=3",
                Box::new(ppl_vm::HeadlessHost::default()),
                Default::default(),
            )
            .result
            .unwrap();
        let second = context.snapshot();
        assert_eq!(
            app.sample(&second, 3, cancel()).unwrap()[0][2],
            Some([2., 6.])
        );
        assert_eq!(app.table(&second, 1).unwrap()[0][1], Some(6.));
        assert_eq!(app.table(&first, 1).unwrap()[0][1], Some(4.));
    }
    #[test]
    fn custom_variables_and_installed_named_functions_share_real_globals() {
        let mut memory = snapshot();
        memory.globals.insert(
            "A".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(2)),
        );
        memory.globals.insert(
            "custom".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(5)),
        );
        memory.programs.insert(
            "LIB".into(),
            "LOCAL factor:=3; EXPORT SCALE(Q) BEGIN RETURN factor*Q+A+custom; END;".into(),
        );
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves[0].expression = "SCALE(X)".into();
        app.numeric_start = 2.;
        assert_eq!(app.table(&memory, 1).unwrap()[0][1], Some(13.));
        app.curves[0].expression = "custom*X".into();
        assert_eq!(app.table(&memory, 1).unwrap()[0][1], Some(10.));
        memory.notes.insert("NOTE".into(), "actual note".into());
        app.curves[0].expression = "SIZE(Programs())+SIZE(Notes())".into();
        assert_eq!(app.table(&memory, 1).unwrap()[0][1], Some(2.));
    }
    #[test]
    fn installed_module_initializes_once_per_sampling_operation() {
        let mut m = snapshot();
        m.globals.insert(
            "A".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(2)),
        );
        m.programs.insert(
            "COUNTER".into(),
            "LOCAL count:=A; EXPORT SERIES(Q) BEGIN count:=count+1; RETURN count; END;".into(),
        );
        let mut a = GraphApp::new(GraphKind::Function);
        a.curves[0].expression = "SERIES(X)".into();
        assert_eq!(
            a.table(&m, 3).unwrap(),
            vec![
                vec![Some(0.), Some(3.)],
                vec![Some(1.), Some(4.)],
                vec![Some(2.), Some(5.)]
            ]
        );
        assert_eq!(
            m.globals["A"],
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(2))
        );
    }
    #[test]
    fn bare_integer_literals_in_curves_and_libraries_follow_home_base() {
        let mut m = snapshot();
        m.settings.int_base = ppl_math::IntBase::Dec;
        let mut a = GraphApp::new(GraphKind::Function);
        a.curves[0].expression = "#10+X".into();
        assert_eq!(a.table(&m, 1).unwrap()[0][1], Some(10.));
        m.settings.int_base = ppl_math::IntBase::Hex;
        assert_eq!(a.table(&m, 1).unwrap()[0][1], Some(16.));
        m.settings.int_base = ppl_math::IntBase::Dec;
        a.curves[0].expression = "#10h+X".into();
        assert_eq!(a.table(&m, 1).unwrap()[0][1], Some(16.));
        m.programs.insert(
            "BASED".into(),
            "EXPORT BASED(Q) BEGIN RETURN #10+Q; END;".into(),
        );
        a.curves[0].expression = "BASED(X)".into();
        assert_eq!(a.table(&m, 1).unwrap()[0][1], Some(10.));
        m.settings.int_base = ppl_math::IntBase::Hex;
        assert_eq!(a.table(&m, 1).unwrap()[0][1], Some(16.));
    }
    #[test]
    fn parametric_table_has_both_coordinates_and_polar_table_has_radius() {
        let mut a = GraphApp::new(GraphKind::Parametric);
        a.curves[0].expression = "COS(T)".into();
        a.curves[0].second = "SIN(T)".into();
        let m = snapshot();
        assert_eq!(
            a.table(&m, 1).unwrap()[0],
            vec![Some(0.), Some(1.), Some(0.)]
        );
        assert_eq!(a.sample(&m, 3, cancel()).unwrap()[0][0], Some([1., 0.]));
        a.kind = GraphKind::Polar;
        a.curves[0].expression = "2".into();
        assert_eq!(a.table(&m, 1).unwrap()[0], vec![Some(0.), Some(2.)]);
        let mut m = m;
        m.settings.angle = ppl_vm::AngleMode::Degrees;
        a.window.tmax = 90.;
        let end = a.sample(&m, 3, cancel()).unwrap()[0][2].unwrap();
        near(end[0], 0.);
        near(end[1], 2.);
    }
    #[test]
    fn domain_holes_recover_but_program_input_is_a_real_error() {
        let mut a = GraphApp::new(GraphKind::Function);
        a.window.xmin = -1.;
        a.window.xmax = 1.;
        a.curves[0].expression = "1/X".into();
        let p = a.sample(&snapshot(), 3, cancel()).unwrap();
        assert_eq!(p[0], vec![Some([-1., -1.]), None, Some([1., 1.])]);
        a.curves[0].expression = "WAIT(0)".into();
        assert!(
            a.sample(&snapshot(), 3, cancel())
                .unwrap_err()
                .contains("script")
        );
    }
    #[test]
    fn fibonacci_backward_and_forward_offset_recurrence_share_plot_and_table() {
        let mut a = GraphApp::new(GraphKind::Sequence);
        a.numeric_start = 1.;
        a.window.xmin = 1.;
        a.window.xmax = 10.;
        a.curves[0].sequence = Some(SequenceDefinition {
            start: 1,
            initial: vec!["1".into(), "1".into()],
            offset: 0,
        });
        a.curves[0].expression = "U1(N-1)+U1(N-2)".into();
        let table = a.table(&snapshot(), 10).unwrap();
        let expected = [1., 1., 2., 3., 5., 8., 13., 21., 34., 55.];
        for (i, &value) in expected.iter().enumerate() {
            assert_eq!(table[i], vec![Some(i as f64 + 1.), Some(value)]);
        }
        let points = a.sample(&snapshot(), 640, cancel()).unwrap();
        assert_eq!(points[0].len(), 10);
        assert_eq!(points[0][9], Some([10., 55.]));
        a.curves[0].sequence.as_mut().unwrap().offset = 2;
        a.curves[0].expression = "U1(N)+U1(N+1)".into();
        assert_eq!(a.table(&snapshot(), 10).unwrap(), table);
    }
    #[test]
    fn sequence_initial_terms_can_read_home_and_invalid_recursion_is_rejected() {
        let mut m = snapshot();
        m.globals.insert(
            "A".into(),
            ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(3)),
        );
        let mut a = GraphApp::new(GraphKind::Sequence);
        a.numeric_start = 0.;
        a.curves[0].expression = "U1(N-1)*2".into();
        a.curves[0].sequence = Some(SequenceDefinition {
            start: 0,
            initial: vec!["A".into()],
            offset: 0,
        });
        assert_eq!(
            a.table(&m, 3).unwrap(),
            vec![
                vec![Some(0.), Some(3.)],
                vec![Some(1.), Some(6.)],
                vec![Some(2.), Some(12.)]
            ]
        );
        a.curves[0].expression = "U1(N)".into();
        assert!(a.table(&m, 3).is_err());
        a.curves[0].expression = "U1(N-2)".into();
        assert!(a.table(&m, 3).is_err());
    }
    #[test]
    fn complex_enabled_home_still_leaves_nonreal_graph_holes() {
        let mut m = snapshot();
        m.settings.complex_results = true;
        let mut a = GraphApp::new(GraphKind::Function);
        a.window.xmin = -1.;
        a.window.xmax = 1.;
        a.curves[0].expression = "SQRT(X)".into();
        assert_eq!(
            a.sample(&m, 3, cancel()).unwrap()[0],
            vec![None, Some([0., 0.]), Some([1., 1.])]
        );
    }
    #[test]
    fn invalid_windows_cancel_and_domain_coordinates_are_bounded() {
        let mut a = GraphApp::new(GraphKind::Function);
        a.window.xmin = a.window.xmax;
        assert!(a.sample(&snapshot(), 5, cancel()).is_err());
        a.window = Default::default();
        a.curves[0].expression = "X".into();
        assert!(
            a.sample(&snapshot(), 5, Arc::new(AtomicBool::new(true)))
                .is_err()
        );
    }
    #[test]
    fn cancel_reaches_installed_function_loop_inside_vm() {
        let mut m = snapshot();
        m.programs.insert(
            "BUSY".into(),
            "EXPORT BUSY(Q) BEGIN REPEAT Q:=Q+1; UNTIL 0; END;".into(),
        );
        let mut a = GraphApp::new(GraphKind::Function);
        a.curves[0].expression = "BUSY(X)".into();
        let flag = cancel();
        let stop = flag.clone();
        let worker = std::thread::spawn(move || a.sample(&m, 10, flag));
        std::thread::sleep(std::time::Duration::from_millis(20));
        stop.store(true, Ordering::Release);
        assert!(worker.join().unwrap().unwrap_err().contains("interrupted"));
    }
}

#[derive(Clone, Copy)]
enum Relation {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}
struct ImplicitRelation {
    evaluator: Evaluator,
    relation: Relation,
}
#[derive(Clone, Copy)]
struct SurfaceValue {
    number: f64,
    sign: i8,
}
impl ImplicitRelation {
    fn new(
        expression: &str,
        snapshot: &ppl_vm::SessionSnapshot,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        use ppl_syntax::ast::{BinOp as B, ExprKind as E};
        let (ast, diags) = ppl_syntax::parse_expression(expression);
        if diags.has_errors() {
            return Err(diags.errors().next().unwrap().message.clone());
        }
        let (relation, source) = match ast.kind {
            E::Binary(op, left, right)
                if matches!(op, B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge) =>
            {
                let lhs = &expression[left.span.start as usize..left.span.end as usize];
                let rhs = &expression[right.span.start as usize..right.span.end as usize];
                let relation = match op {
                    B::Eq => Relation::Equal,
                    B::Ne => Relation::NotEqual,
                    B::Lt => Relation::Less,
                    B::Le => Relation::LessEqual,
                    B::Gt => Relation::Greater,
                    _ => Relation::GreaterEqual,
                };
                (relation, format!("({lhs})-({rhs})"))
            }
            E::Binary(B::And | B::Or, _, _) => {
                return Err("Enter individual implicit relations as separate definitions".into());
            }
            _ => (Relation::Equal, expression.to_string()),
        };
        Ok(Self {
            evaluator: Evaluator::implicit(&source, snapshot, cancel)?,
            relation,
        })
    }
    fn value(&mut self, x: f64, y: f64) -> Result<Option<SurfaceValue>, String> {
        let result = self.evaluator.at_xy_value(x, y);
        if matches!(&result,Ok(ppl_vm::Value::Complex(c)) if !c.im.is_zero()) {
            return Ok(None);
        }
        match result.and_then(|v| v.expect_real("Implicit relation")) {
            Ok(real) => {
                let sign = if real.is_zero() {
                    0
                } else if real.is_negative() {
                    -1
                } else {
                    1
                };
                let number = real.to_f64();
                let number = if !number.is_finite() {
                    if sign < 0 { -f64::MAX } else { f64::MAX }
                } else if number == 0. && sign != 0 {
                    sign as f64 * f64::MIN_POSITIVE
                } else {
                    number
                };
                Ok(Some(SurfaceValue { number, sign }))
            }
            Err(e)
                if matches!(
                    e.kind,
                    ppl_vm::ErrorKind::DivisionByZero
                        | ppl_vm::ErrorKind::Overflow
                        | ppl_vm::ErrorKind::NonReal
                        | ppl_vm::ErrorKind::Undefined
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.to_string()),
        }
    }
    fn truth(&self, value: SurfaceValue) -> bool {
        match self.relation {
            Relation::Equal => value.sign == 0,
            Relation::NotEqual => value.sign != 0,
            Relation::Less => value.sign < 0,
            Relation::LessEqual => value.sign <= 0,
            Relation::Greater => value.sign > 0,
            Relation::GreaterEqual => value.sign >= 0,
        }
    }
}
impl GraphApp {
    pub fn implicit_truth(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        curve: usize,
        x: f64,
        y: f64,
    ) -> Result<Option<bool>, String> {
        if !x.is_finite() || !y.is_finite() {
            return Err("Implicit coordinates must be finite".into());
        }
        let definition = self
            .curves
            .get(curve)
            .ok_or("No implicit relation selected")?;
        let mut relation = ImplicitRelation::new(
            &definition.expression,
            snapshot,
            Arc::new(AtomicBool::new(false)),
        )?;
        let value = relation.value(x, y)?;
        Ok(value.map(|v| relation.truth(v)))
    }
    fn implicit_table(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        rows: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<Vec<Option<f64>>>, String> {
        if !self.numeric_y.is_finite() {
            return Err("Numeric Y must be finite".into());
        }
        let mut definitions = self
            .curves
            .iter()
            .filter(|c| c.enabled && !c.expression.trim().is_empty())
            .map(|c| ImplicitRelation::new(&c.expression, snapshot, cancel.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for row in 0..rows.min(1000) {
            check_cancel(&cancel)?;
            let x = self.numeric_start + row as f64 * self.numeric_step;
            if !x.is_finite() {
                return Err("Numeric X overflow".into());
            }
            let mut values = vec![Some(x), Some(self.numeric_y)];
            for relation in &mut definitions {
                let value = relation.value(x, self.numeric_y)?;
                values.push(value.map(|v| if relation.truth(v) { 1. } else { 0. }));
            }
            out.push(values);
        }
        Ok(out)
    }
    fn implicit_sample(
        &self,
        snapshot: &ppl_vm::SessionSnapshot,
        count: usize,
        cancel: Arc<AtomicBool>,
    ) -> PlotResult {
        let nx = count.clamp(16, 320);
        let ny = (nx * 3 / 4).max(12);
        let mut curves = Vec::new();
        for curve in self
            .curves
            .iter()
            .filter(|c| c.enabled && !c.expression.trim().is_empty())
        {
            check_cancel(&cancel)?;
            let mut relation = ImplicitRelation::new(&curve.expression, snapshot, cancel.clone())?;
            let samples = if matches!(relation.relation, Relation::Equal) {
                contours(&mut relation, &self.window, nx, ny, &cancel)?
            } else {
                region_runs(&mut relation, &self.window, nx, ny, &cancel)?
            };
            curves.push(samples);
        }
        Ok(curves)
    }
}
fn region_runs(
    relation: &mut ImplicitRelation,
    window: &PlotWindow,
    nx: usize,
    ny: usize,
    cancel: &AtomicBool,
) -> Result<CurveSamples, String> {
    let dx = (window.xmax - window.xmin) / nx as f64;
    let dy = (window.ymax - window.ymin) / ny as f64;
    let mut segments = Vec::new();
    for row in 0..ny {
        check_cancel(cancel)?;
        let y = window.ymax - (row as f64 + 0.5) * dy;
        let mut start = None;
        for col in 0..nx {
            let x = window.xmin + (col as f64 + 0.5) * dx;
            let inside = relation.value(x, y)?.is_some_and(|v| relation.truth(v));
            if inside {
                start.get_or_insert(col);
            } else if let Some(first) = start.take() {
                segments.extend([
                    Some([window.xmin + first as f64 * dx, y]),
                    Some([window.xmin + col as f64 * dx, y]),
                    None,
                ]);
            }
        }
        if let Some(first) = start {
            segments.extend([
                Some([window.xmin + first as f64 * dx, y]),
                Some([window.xmax, y]),
                None,
            ]);
        }
    }
    Ok(segments)
}
fn contours(
    relation: &mut ImplicitRelation,
    window: &PlotWindow,
    nx: usize,
    ny: usize,
    cancel: &AtomicBool,
) -> Result<CurveSamples, String> {
    let dx = (window.xmax - window.xmin) / nx as f64;
    let dy = (window.ymax - window.ymin) / ny as f64;
    let mut grid = Vec::with_capacity((nx + 1) * (ny + 1));
    for row in 0..=ny {
        check_cancel(cancel)?;
        let y = window.ymin + row as f64 * dy;
        for col in 0..=nx {
            grid.push(relation.value(window.xmin + col as f64 * dx, y)?);
        }
    }
    let mut output = Vec::new();
    for row in 0..ny {
        check_cancel(cancel)?;
        for col in 0..nx {
            let values = [
                grid[row * (nx + 1) + col],
                grid[row * (nx + 1) + col + 1],
                grid[(row + 1) * (nx + 1) + col + 1],
                grid[(row + 1) * (nx + 1) + col],
            ];
            let [Some(a), Some(b), Some(c), Some(d)] = values else {
                continue;
            };
            let values = [a, b, c, d];
            let x = window.xmin + col as f64 * dx;
            let y = window.ymin + row as f64 * dy;
            let points = [[x, y], [x + dx, y], [x + dx, y + dy], [x, y + dy]];
            let mut crossings = Vec::new();
            for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
                let (va, vb) = (values[a], values[b]);
                if va.sign == 0 && vb.sign == 0 {
                    output.extend([Some(points[a]), Some(points[b]), None]);
                    continue;
                }
                if va.sign != 0 && vb.sign != 0 && va.sign == vb.sign {
                    continue;
                }
                let scale = va.number.abs().max(vb.number.abs());
                let t = if va.sign == 0 {
                    0.
                } else if vb.sign == 0 {
                    1.
                } else {
                    (va.number / scale) / ((va.number / scale) - (vb.number / scale))
                };
                let point = [
                    points[a][0] + (points[b][0] - points[a][0]) * t,
                    points[a][1] + (points[b][1] - points[a][1]) * t,
                ];
                // A sign jump across a pole is not a zero contour. Check the
                // interpolated crossing before joining the cell's segments.
                if va.sign != 0 && vb.sign != 0 {
                    let Some(candidate) = relation.value(point[0], point[1])? else {
                        continue;
                    };
                    if candidate.sign != 0
                        && candidate.number.abs() >= va.number.abs().min(vb.number.abs())
                    {
                        continue;
                    }
                }
                if !crossings.contains(&point) {
                    crossings.push(point);
                }
            }
            match crossings.len() {
                2 => output.extend([Some(crossings[0]), Some(crossings[1]), None]),
                4 => {
                    let center = values.iter().map(|v| v.number / 4.).sum::<f64>();
                    if (center >= 0.) == (values[0].sign >= 0) {
                        output.extend([
                            Some(crossings[0]),
                            Some(crossings[1]),
                            None,
                            Some(crossings[2]),
                            Some(crossings[3]),
                            None,
                        ]);
                    } else {
                        output.extend([
                            Some(crossings[0]),
                            Some(crossings[3]),
                            None,
                            Some(crossings[1]),
                            Some(crossings[2]),
                            None,
                        ]);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(output)
}
#[cfg(test)]
mod implicit_tests {
    use super::*;
    fn memory() -> ppl_vm::SessionSnapshot {
        ppl_vm::SessionContext::default().snapshot()
    }
    #[test]
    fn numeric_implicit_relations_use_both_coordinates_and_home_memory() {
        let mut a = GraphApp::new(GraphKind::Implicit);
        a.curves[0].expression = "X^2+Y^2=A".into();
        let mut m = memory();
        m.globals
            .insert("A".into(), ppl_vm::PortableValue::Real(ppl_math::Real::ONE));
        assert_eq!(a.implicit_truth(&m, 0, 1., 0.).unwrap(), Some(true));
        assert_eq!(a.implicit_truth(&m, 0, 1., 1.).unwrap(), Some(false));
        a.numeric_start = 0.;
        a.numeric_step = 1.;
        a.numeric_y = 1.;
        assert_eq!(
            a.table(&m, 2).unwrap(),
            vec![
                vec![Some(0.), Some(1.), Some(1.)],
                vec![Some(1.), Some(1.), Some(0.)]
            ]
        );
    }
    #[test]
    fn circle_is_real_two_dimensional_contour() {
        let mut a = GraphApp::new(GraphKind::Implicit);
        a.window.xmin = -1.5;
        a.window.xmax = 1.5;
        a.window.ymin = -1.5;
        a.window.ymax = 1.5;
        a.curves[0].expression = "x^2+y^2=1".into();
        let points = a
            .sample(&memory(), 64, Arc::new(AtomicBool::new(false)))
            .unwrap();
        let points: Vec<_> = points[0].iter().flatten().collect();
        assert!(points.len() > 50);
        for p in points {
            assert!((p[0] * p[0] + p[1] * p[1] - 1.).abs() < 0.004, "{p:?}");
        }
    }
    #[test]
    fn inequality_returns_filled_horizontal_runs_and_cancel_is_honored() {
        let mut a = GraphApp::new(GraphKind::Implicit);
        a.window.xmin = -1.;
        a.window.xmax = 1.;
        a.window.ymin = -1.;
        a.window.ymax = 1.;
        a.curves[0].expression = "Y<X".into();
        assert_eq!(a.implicit_truth(&memory(), 0, 0.5, 0.).unwrap(), Some(true));
        assert_eq!(
            a.implicit_truth(&memory(), 0, 0., 0.5).unwrap(),
            Some(false)
        );
        let runs = a
            .sample(&memory(), 32, Arc::new(AtomicBool::new(false)))
            .unwrap();
        assert!(!runs[0].is_empty());
        for triple in runs[0].chunks(3) {
            let a = triple[0].unwrap();
            let b = triple[1].unwrap();
            assert_eq!(a[1], b[1]);
            assert!(a[0] < b[0]);
            assert!(b[0] > a[1]);
            assert_eq!(triple[2], None);
        }
        assert!(
            a.sample(&memory(), 320, Arc::new(AtomicBool::new(true)))
                .is_err()
        );
    }
    #[test]
    fn a_sign_change_across_a_pole_is_not_drawn_as_a_zero_curve() {
        let mut a = GraphApp::new(GraphKind::Implicit);
        a.window.xmin = -1.;
        a.window.xmax = 1.;
        a.window.ymin = -1.;
        a.window.ymax = 1.;
        a.curves[0].expression = "1/(X-0.123)=0".into();
        assert!(
            a.sample(&memory(), 64, Arc::new(AtomicBool::new(false)))
                .unwrap()[0]
                .is_empty()
        );
    }
    #[test]
    fn underflow_does_not_turn_false_equation_true() {
        let mut a = GraphApp::new(GraphKind::Implicit);
        a.curves[0].expression = "1E-400=0".into();
        assert_eq!(a.implicit_truth(&memory(), 0, 0., 0.).unwrap(), Some(false));
    }
}

#[cfg(test)]
mod independent_scope_tests {
    use super::*;
    fn memory() -> ppl_vm::SessionSnapshot {
        let mut memory = ppl_vm::SessionContext::default().snapshot();
        for (name, value) in [("X", 99), ("Y", 88), ("T", 7), ("N", 11), ("θ", 13)] {
            memory.globals.insert(
                name.into(),
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(value)),
            );
        }
        memory
    }
    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }
    #[test]
    fn function_binds_x_while_preserving_home_t_n_and_theta() {
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves[0].expression = "X+T+N+θ".into();
        app.numeric_start = 2.;
        assert_eq!(
            app.table(&memory(), 1).unwrap()[0],
            vec![Some(2.), Some(33.)]
        );
        app.window.xmin = 1.;
        app.window.xmax = 3.;
        assert_eq!(
            app.sample(&memory(), 3, flag()).unwrap()[0],
            vec![Some([1., 32.]), Some([2., 33.]), Some([3., 34.])]
        );
    }
    #[test]
    fn parametric_both_coordinates_bind_t_and_preserve_home_x_n() {
        let mut app = GraphApp::new(GraphKind::Parametric);
        app.curves[0].expression = "T+X".into();
        app.curves[0].second = "T+N+θ".into();
        app.numeric_start = 2.;
        assert_eq!(
            app.table(&memory(), 1).unwrap()[0],
            vec![Some(2.), Some(101.), Some(26.)]
        );
        app.window.tmin = 1.;
        app.window.tmax = 3.;
        assert_eq!(
            app.sample(&memory(), 3, flag()).unwrap()[0][1],
            Some([101., 26.])
        );
    }
    #[test]
    fn polar_binds_theta_but_keeps_home_x_n_t() {
        let mut app = GraphApp::new(GraphKind::Polar);
        app.curves[0].expression = "θ+X+N+T".into();
        app.numeric_start = 2.;
        assert_eq!(
            app.table(&memory(), 1).unwrap()[0],
            vec![Some(2.), Some(119.)]
        );
        app.window.tmin = 0.;
        app.window.tmax = 1.;
        assert_eq!(
            app.sample(&memory(), 2, flag()).unwrap()[0][0],
            Some([117., 0.])
        );
    }
    #[test]
    fn sequence_explicit_initial_and_recurrent_forms_bind_only_n() {
        let mut app = GraphApp::new(GraphKind::Sequence);
        app.numeric_start = 1.;
        app.curves[0].expression = "N+X+T".into();
        assert_eq!(
            app.table(&memory(), 2).unwrap(),
            vec![vec![Some(1.), Some(107.)], vec![Some(2.), Some(108.)]]
        );
        app.curves[0].expression = "U1(N-1)+N+X+T".into();
        app.curves[0].sequence = Some(SequenceDefinition {
            start: 1,
            initial: vec!["N+X+T".into()],
            offset: 0,
        });
        assert_eq!(
            app.table(&memory(), 2).unwrap(),
            vec![vec![Some(1.), Some(107.)], vec![Some(2.), Some(215.)]]
        );
    }
    #[test]
    fn implicit_xy_aliases_are_coordinates_and_other_home_variables_stay_known() {
        let mut app = GraphApp::new(GraphKind::Implicit);
        app.curves[0].expression = "X+x+Y+y+T+N+θ=41".into();
        assert_eq!(
            app.implicit_truth(&memory(), 0, 2., 3.).unwrap(),
            Some(true)
        );
        assert_eq!(
            app.implicit_truth(&memory(), 0, 2., 4.).unwrap(),
            Some(false)
        );
        assert!(
            app.sample(&memory(), 32, Arc::new(AtomicBool::new(true)))
                .is_err()
        );
    }
}

#[cfg(all(test, feature = "giac-integration"))]
mod cas_context_tests {
    use super::*;
    fn memory() -> ppl_vm::SessionSnapshot {
        let mut snapshot = ppl_vm::SessionContext::default().snapshot();
        for (name, value) in [("T", 7), ("X", 99), ("N", 11)] {
            snapshot.globals.insert(
                name.into(),
                ppl_vm::PortableValue::Real(ppl_math::Real::from_i64(value)),
            );
        }
        snapshot.cas.bindings.insert("q".into(), "4".into());
        snapshot.cas.bindings.insert("x".into(), "999".into());
        snapshot.cas.last_value = Some(ppl_cas::CasValue::Integer("1234".into()));
        snapshot
    }
    #[test]
    fn function_samples_and_table_share_known_cas_and_home_values() {
        let snapshot = memory();
        let before = serde_json::to_value(&snapshot.cas).unwrap();
        let mut app = GraphApp::new(GraphKind::Function);
        app.curves[0].expression = "X+q+T".into();
        app.numeric_start = 2.;
        app.window.xmin = 1.;
        app.window.xmax = 3.;
        assert_eq!(
            app.table(&snapshot, 2).unwrap(),
            vec![vec![Some(2.), Some(13.)], vec![Some(3.), Some(14.)]]
        );
        assert_eq!(
            app.sample(&snapshot, 3, Arc::new(AtomicBool::new(false)))
                .unwrap()[0],
            vec![Some([1., 12.]), Some([2., 13.]), Some([3., 14.])]
        );
        assert_eq!(serde_json::to_value(&snapshot.cas).unwrap(), before);
        assert!(!snapshot.globals.contains_key("q"));
    }
    #[test]
    fn parametric_sequence_and_implicit_share_cas_without_shadowing_parameters() {
        let snapshot = memory();
        let mut parametric = GraphApp::new(GraphKind::Parametric);
        parametric.numeric_start = 2.;
        parametric.curves[0].expression = "T+q+X".into();
        parametric.curves[0].second = "T+q+N".into();
        assert_eq!(
            parametric.table(&snapshot, 1).unwrap()[0],
            vec![Some(2.), Some(105.), Some(17.)]
        );
        let mut sequence = GraphApp::new(GraphKind::Sequence);
        sequence.curves[0].expression = "N+q+X+T".into();
        assert_eq!(
            sequence.table(&snapshot, 1).unwrap()[0],
            vec![Some(1.), Some(111.)]
        );
        let mut implicit = GraphApp::new(GraphKind::Implicit);
        implicit.numeric_start = 2.;
        implicit.numeric_y = 3.;
        implicit.curves[0].expression = "X+Y+x-x+q+T=16".into();
        assert_eq!(
            implicit.table(&snapshot, 1).unwrap()[0],
            vec![Some(2.), Some(3.), Some(1.)]
        );
    }
    #[test]
    fn graph_batch_can_resolve_cas_collections_for_native_indexing() {
        let mut snapshot = memory();
        snapshot
            .cas
            .bindings
            .insert("items".into(), "[1,2,3]".into());
        let mut app = GraphApp::new(GraphKind::Function);
        app.numeric_start = 2.;
        app.curves[0].expression = "items(2)*X".into();
        assert_eq!(
            app.table(&snapshot, 1).unwrap()[0],
            vec![Some(2.), Some(4.)]
        );
    }
}
