//! Semantic analysis and bytecode generation in one pass over the AST.
//!
//! Name resolution order for an identifier:
//! 1. `LOCAL` variables and parameters in enclosing blocks of the function,
//! 2. file-level `LOCAL`/`EXPORT` variables,
//! 3. user functions,
//! 4. calculator variables (`A`–`Z`, `L0`–`L9`, `M0`–`M9`, `Z0`–`Z9`,
//!    `G0`–`G9`, settings such as `HAngle`),
//! 5. built-in commands.
//!
//! Anything else is an "undeclared identifier" error, which matches the
//! Prime rejecting programs that use undefined names (unverified for every
//! context).

use std::collections::HashMap;

use ppl_math::{Complex, HpInt, IntBase, Real};
use ppl_syntax::ast::*;
use ppl_syntax::{Category as DiagCat, Diagnostic, Diagnostics, Span};

use crate::builtins::{self, Builtin, Status};
use crate::bytecode::*;
use crate::catalog::{self, CatalogKind};

fn stddev_numeric_entry(expr: &Expr, integral_only: bool) -> bool {
    match &expr.kind {
        ExprKind::Number(source) => {
            !integral_only || Real::parse(source).is_some_and(Real::is_integer)
        }
        ExprKind::Unary(UnOp::Neg, operand) => stddev_numeric_entry(operand, integral_only),
        ExprKind::Binary(BinOp::Add | BinOp::Sub, left, right) => {
            stddev_numeric_entry(left, integral_only) && stddev_numeric_entry(right, integral_only)
        }
        _ => false,
    }
}

fn stddev_cell_has_call(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Call(..) => true,
        ExprKind::ComplexPair(real, imaginary) => {
            stddev_cell_has_call(real) || stddev_cell_has_call(imaginary)
        }
        _ => false,
    }
}

fn stddev_cell_has_division(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Binary(BinOp::Div, ..) => true,
        ExprKind::Binary(_, left, right) | ExprKind::ComplexPair(left, right) => {
            stddev_cell_has_division(left) || stddev_cell_has_division(right)
        }
        ExprKind::Unary(_, operand) => stddev_cell_has_division(operand),
        ExprKind::Call(_, arguments) => arguments.iter().any(stddev_cell_has_division),
        _ => false,
    }
}

fn stddev_literal_zero_quotient(left: &Expr, right: &Expr) -> bool {
    stddev_literal_real(left).is_some() && stddev_literal_real(right).is_some_and(Real::is_zero)
}

fn stddev_matrix_cells(rows: &[Expr]) -> Option<(usize, Vec<&Expr>)> {
    let (first, remaining) = rows.split_first()?;
    let ExprKind::Bracket(first) = &first.kind else {
        return None;
    };
    let cols = first.len();
    let mut cells = Vec::from_iter(first);
    for row in remaining {
        let ExprKind::Bracket(row) = &row.kind else {
            return None;
        };
        if row.len() != cols {
            return None;
        }
        cells.extend(row);
    }
    Some((cols, cells))
}

fn source_encoded_matrix(operand: &Expr) -> bool {
    let ExprKind::Bracket(rows) = &operand.kind else {
        return false;
    };
    !rows.is_empty()
        && rows
            .iter()
            .all(|row| matches!(row.kind, ExprKind::Bracket(_)))
        && rows.iter().any(|row| {
            matches!(&row.kind, ExprKind::Bracket(cells)
                if cells.iter().any(|cell| !matches!(cell.kind, ExprKind::Number(_))))
        })
}

fn source_encoded_vector(operand: &Expr) -> Option<&[Expr]> {
    let ExprKind::Bracket(entries) = &operand.kind else {
        return None;
    };
    (entries
        .iter()
        .any(|entry| !matches!(entry.kind, ExprKind::Number(_)))
        && !entries
            .iter()
            .any(|entry| matches!(entry.kind, ExprKind::Bracket(_))))
    .then_some(entries)
}

fn stddev_diagnostic_expression(
    expr: &Expr,
    depth: usize,
    canonicalize_zero_subtraction: bool,
    allow_pairs: bool,
) -> Option<Expr> {
    if depth > 128 {
        return None;
    }
    let normalize = |expr: &Expr| {
        stddev_diagnostic_expression(expr, depth + 1, canonicalize_zero_subtraction, allow_pairs)
    };
    let operand = |expr: &Expr| normalize(expr).map(Box::new);
    let items = |items: &[Expr]| items.iter().map(normalize).collect::<Option<_>>();
    let kind = match &expr.kind {
        ExprKind::Number(number) => {
            let number = Real::parse(number)?.to_string();
            let number = match number.split_once('E') {
                Some((significand, exponent)) if !exponent.starts_with('-') => {
                    format!("{significand}E+{exponent}")
                }
                _ => number,
            };
            ExprKind::Number(number)
        }
        ExprKind::Ident(name) => ExprKind::Ident(name.clone()),
        ExprKind::Bracket(entries) => ExprKind::Bracket(items(entries)?),
        ExprKind::ComplexPair(real, imaginary) if allow_pairs => {
            ExprKind::ComplexPair(operand(real)?, operand(imaginary)?)
        }
        ExprKind::Unary(UnOp::Neg, value) => ExprKind::Unary(UnOp::Neg, operand(value)?),
        ExprKind::Binary(BinOp::Div, left, right) if allow_pairs => {
            if stddev_literal_zero_quotient(left, right) {
                ExprKind::Ident("undef".into())
            } else {
                ExprKind::Binary(BinOp::Div, operand(left)?, operand(right)?)
            }
        }
        ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), left, right) => {
            let right = operand(right)?;
            let op = if canonicalize_zero_subtraction
                && *op == BinOp::Sub
                && matches!(&right.kind, ExprKind::Number(number) if number == "0")
            {
                BinOp::Add
            } else {
                *op
            };
            ExprKind::Binary(op, operand(left)?, right)
        }
        ExprKind::Call(callee, args) if matches!(callee.kind, ExprKind::Ident(_)) => {
            if args.is_empty() {
                callee.kind.clone()
            } else {
                ExprKind::Call(operand(callee)?, items(args)?)
            }
        }
        _ => return None,
    };
    Some(Expr {
        kind,
        span: expr.span,
    })
}

fn stddev_matrix_operand_source(
    expr: &Expr,
    encoded: bool,
    canonicalize_zero_subtraction: bool,
    allow_pairs: bool,
) -> Option<String> {
    let expr = stddev_diagnostic_expression(expr, 0, canonicalize_zero_subtraction, allow_pairs)?;
    if !encoded {
        return crate::expression_source::ppl_diagnostic_expression_source(&expr).ok();
    }
    let ExprKind::Bracket(rows) = &expr.kind else {
        return None;
    };
    let (cols, cells) = stddev_matrix_cells(rows)?;
    let cells = cells
        .into_iter()
        .map(crate::expression_source::ppl_diagnostic_expression_source)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(format!(
        "ListToMat(EVALLIST({{1,{},{cols},{}}}))",
        rows.len(),
        cells.join(",")
    ))
}

fn stddev_encoded_vector_source(
    entries: &[Expr],
    canonicalize_zero_subtraction: bool,
    allow_pairs: bool,
) -> Option<String> {
    let cells = entries
        .iter()
        .map(|entry| {
            let entry =
                stddev_diagnostic_expression(entry, 0, canonicalize_zero_subtraction, allow_pairs)?;
            crate::expression_source::ppl_diagnostic_expression_source(&entry).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    Some(format!(
        "ListToMat(EVALLIST({{0,1,{},{}}}))",
        entries.len(),
        cells.join(",")
    ))
}

fn stddev_bad_argument(data: &str, weights: &str) -> String {
    format!("stddev({data},{weights}) \n Error: Bad Argument Value")
}

enum StddevMatrixDiagnostic {
    EncodedOperands(String),
    RowMismatch(String),
}

fn stddev_matrix_diagnostic(args: &[Expr]) -> Option<StddevMatrixDiagnostic> {
    let [data, weights] = args else {
        return None;
    };
    let data_encoded = source_encoded_matrix(data);
    let weights_encoded = source_encoded_matrix(weights);
    let both_encoded = data_encoded && weights_encoded;
    if both_encoded {
        let cell_count = |operand: &Expr| {
            let ExprKind::Bracket(rows) = &operand.kind else {
                return None;
            };
            let (_, cells) = stddev_matrix_cells(rows)?;
            Some(cells.len())
        };
        // The two encoded carriers include three shape fields each. Unequal
        // carrier lengths select the diagnostic, even when both are matrices.
        if cell_count(data)? == cell_count(weights)? {
            return None;
        }
    }
    let encoded = data_encoded || weights_encoded;
    let matrix_operand = |expr: &Expr| match &expr.kind {
        ExprKind::Bracket(rows) => stddev_matrix_cells(rows).is_some(),
        ExprKind::Ident(_)
        | ExprKind::Call(_, _)
        | ExprKind::Unary(UnOp::Neg, _)
        | ExprKind::Binary(BinOp::Add, _, _) => true,
        _ => false,
    };
    if !encoded && !args.iter().all(matrix_operand) {
        return None;
    }
    let data = stddev_matrix_operand_source(data, data_encoded, both_encoded, false)?;
    let weights = stddev_matrix_operand_source(weights, weights_encoded, both_encoded, false)?;
    let message = stddev_bad_argument(&data, &weights);
    Some(if encoded {
        StddevMatrixDiagnostic::EncodedOperands(message)
    } else {
        StddevMatrixDiagnostic::RowMismatch(message)
    })
}

fn stddev_literal_real(expr: &Expr) -> Option<Real> {
    match &expr.kind {
        ExprKind::Number(source) => Real::parse(source),
        ExprKind::Unary(UnOp::Neg, operand) => match &operand.kind {
            ExprKind::Number(source) => Some(Real::parse(source)?.neg()),
            _ => None,
        },
        _ => None,
    }
}

fn is_app_variable(name: &str) -> bool {
    // App qualifiers are case sensitive. Additional app namespaces require
    // their own verified variable contract; ordinary dotted functions and CAS
    // continue through the existing resolution paths.
    name.strip_prefix("Geometry.")
        .is_some_and(crate::geometry::is_variable)
        || name
            .strip_prefix("Spreadsheet.")
            .is_some_and(|field| !field.is_empty() && !field.contains('.'))
        || name.strip_prefix("Function.").is_some_and(is_function_slot)
        || name.strip_prefix("Graph_3D.").is_some_and(|field| {
            is_graph3d_slot(field)
                || is_plot_variable(field)
                || matches!(field, "Zmin" | "Zmax" | "Ztick")
                || matches!(
                    field,
                    "NumXStart"
                        | "NumYStart"
                        | "NumXStep"
                        | "NumYStep"
                        | "NumXZoom"
                        | "NumYZoom"
                        | "NumType"
                        | "NumIndep"
                )
        })
        || [
            "Function.",
            "Parametric.",
            "Polar.",
            "Sequence.",
            "Statistics_1Var.",
            "Statistics_2Var.",
        ]
        .iter()
        .any(|prefix| name.strip_prefix(prefix).is_some_and(is_plot_variable))
        || name == "Function.Cursor"
        || name.strip_prefix("Statistics_1Var.").is_some_and(|field| {
            is_statistics_slot(field)
                || is_statistics_column(field)
                || matches!(field, "Xzoom" | "Yzoom" | "Cursor")
        })
}

fn is_plot_variable(name: &str) -> bool {
    matches!(
        name,
        "Xmin"
            | "Xmax"
            | "Ymin"
            | "Ymax"
            | "Xtick"
            | "Ytick"
            | "Axes"
            | "GridDots"
            | "GridLines"
            | "Labels"
            | "Hmin"
            | "Hmax"
            | "Hwidth"
    )
}

fn is_statistics_column(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 2 && bytes[0] == b'D' && bytes[1].is_ascii_digit()
}

fn app_access_name(name: &str) -> String {
    if is_graph3d_slot(name) {
        return format!("Graph 3D.{name}");
    }
    for (prefix, app) in [
        ("Graph_3D.", "Graph 3D"),
        ("Statistics_1Var.", "Statistics 1Var"),
        ("Statistics_2Var.", "Statistics 2Var"),
    ] {
        if let Some(field) = name.strip_prefix(prefix) {
            return format!("{app}.{field}");
        }
    }
    name.to_owned()
}

fn is_function_slot(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 2 && bytes[0] == b'F' && bytes[1].is_ascii_digit()
}

fn is_graph3d_slot(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 3 && bytes.starts_with(b"FZ") && bytes[2].is_ascii_digit()
}

fn is_statistics_two_slot(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 2 && bytes[0] == b'S' && (b'1'..=b'5').contains(&bytes[1])
}

fn is_statistics_slot(name: &str) -> bool {
    let bytes = name.as_bytes();
    (bytes.len() == 2 && bytes[0] == b'H' && (b'1'..=b'5').contains(&bytes[1]))
        || matches!(
            name,
            "NbItem"
                | "MinVal"
                | "Q₁"
                | "MedVal"
                | "Q₃"
                | "MaxVal"
                | "ΣX"
                | "ΣX2"
                | "MeanX"
                | "sX"
                | "σX"
                | "serrX"
                | "ssX"
        )
}

#[derive(Clone, Debug)]
pub struct CompileOptions {
    /// Base used for `#123` literals without a suffix (Home setting).
    pub default_int_base: IntBase,
    /// Signed width code for interactive Home/EXPR literals. Ordinary PPL
    /// compilation retains its32-bit unsigned default independently of the
    /// running program's later Bits/Signed assignments.
    pub default_int_width: i64,
    /// Exported functions installed in the caller-owned module catalog.
    pub external_functions: std::collections::BTreeMap<String, usize>,
    pub external_variables: std::collections::BTreeSet<String>,
    pub ambiguous_externals: std::collections::BTreeSet<String>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            default_int_base: IntBase::Hex,
            default_int_width: 32,
            external_functions: Default::default(),
            external_variables: Default::default(),
            ambiguous_externals: Default::default(),
        }
    }
}

pub fn compile(program: &Program, opts: &CompileOptions) -> (Option<CompiledProgram>, Diagnostics) {
    compile_with_source(program, opts, None)
}

pub(crate) fn compile_with_source(
    program: &Program,
    opts: &CompileOptions,
    source: Option<&str>,
) -> (Option<CompiledProgram>, Diagnostics) {
    let mut c = Compiler::new(opts.clone(), &program.punctuation, source);
    c.collect(program);
    c.compile_items(program);
    let has_errors = c.diags.has_errors();
    let mut diags = std::mem::take(&mut c.diags);
    diags.sort();
    if has_errors {
        return (None, diags);
    }
    let prog = CompiledProgram {
        functions: c
            .chunks
            .into_iter()
            .map(|c| c.expect("every function compiled"))
            .collect(),
        consts: c.consts,
        globals: c.globals,
        names: c.names,
        init: c.init.expect("init chunk"),
    };
    (Some(prog), diags)
}

#[derive(Clone, Copy, Debug)]
enum VarRef {
    Local(u16),
    Global(u32),
    App(u32),
}

struct FnInfo {
    name: String,
    params: usize,
    def_start: u32,
    span: Span,
}

struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    try_depth: usize,
}

struct LocalInfo {
    span: Span,
    used: bool,
    name: String,
    hidden: bool,
    scope: std::ops::Range<u32>,
}

struct Compiler<'a> {
    opts: CompileOptions,
    punctuation: &'a [Punctuation],
    source: Option<&'a str>,
    diags: Diagnostics,
    consts: Vec<Const>,
    globals: Vec<GlobalSlot>,
    global_map: HashMap<String, u32>,
    fns: Vec<FnInfo>,
    fn_map: HashMap<String, u32>,
    forward_decls: HashMap<String, u32>,
    chunks: Vec<Option<Chunk>>,
    names: Vec<String>,
    init: Option<Chunk>,
    // per-function state
    cur: Chunk,
    scopes: Vec<HashMap<String, u16>>,
    locals: Vec<LocalInfo>,
    loops: Vec<LoopCtx>,
    try_depth: usize,
    in_function: bool,
    /// Nesting depth of arguments to calls the emulator cannot run (CAS
    /// and catalog commands). Undeclared names there are symbolic CAS
    /// variables on the calculator, not program errors.
    cas_arg_depth: u32,
}

fn empty_chunk(name: &str, span: Span) -> Chunk {
    Chunk {
        name: name.to_string(),
        params: 0,
        locals: 0,
        local_names: Vec::new(),
        local_scopes: Vec::new(),
        code: Vec::new(),
        stddev_metadata: Vec::new(),
        spans: Vec::new(),
        exported: false,
        span,
    }
}

/// True for calculator variables that always exist (`A`–`Z`, `θ`, `L0`–`L9`,
/// `M0`–`M9`, `Z0`–`Z9`, `G0`–`G9`, statistics `C0`–`C9`/`D0`–`D9`,
/// `Ans`, mode settings). Case-sensitive,
/// exactly as name resolution in the compiler.
pub fn is_system_variable(name: &str) -> bool {
    system_global(name).is_some()
}

/// Calculator variables that always exist.
fn system_global(name: &str) -> Option<(GlobalInit, bool)> {
    let plot_default = match name.to_ascii_lowercase().as_str() {
        "xmin" | "ymin" => Some("-10"),
        "xmax" | "ymax" => Some("10"),
        "xtick" | "ytick" | "hwidth" => Some("1"),
        "xzoom" | "yzoom" => Some("2"),
        "cursor" => Some("0"),
        "hmin" | "axes" | "griddots" | "gridlines" | "labels" => Some("0"),
        "hmax" => Some("24"),
        "tmin" => Some("0"),
        "nmin" => Some("1"),
        "nmax" => Some("100"),
        "tmax" => Some("6.28318530718"),
        _ => None,
    };
    if let Some(value) = plot_default {
        return Some((GlobalInit::Real(Real::parse(value).unwrap()), false));
    }

    let b = name.as_bytes();
    if b.len() == 1 && b[0].is_ascii_uppercase() || name == "θ" {
        return Some((GlobalInit::Zero, false));
    }
    if b.len() == 2 && b[1].is_ascii_digit() {
        let n = b[1] - b'0';
        return match b[0] {
            b'L' | b'C' | b'D' => Some((GlobalInit::EmptyList, false)),
            b'M' => Some((GlobalInit::ZeroMatrix, false)),
            b'Z' => Some((GlobalInit::ComplexZero, false)),
            b'G' => Some((GlobalInit::Grob(n), true)),
            _ => None,
        };
    }
    let setting = match name.to_ascii_lowercase().as_str() {
        "hangle" => Setting::HAngle,
        "hformat" => Setting::HFormat,
        "hseparator" => Setting::HSeparator,
        "hdigits" => Setting::HDigits,
        "hcomplex" => Setting::HComplex,
        "aangle" => Setting::AAngle,
        "aformat" => Setting::AFormat,
        "adigits" => Setting::ADigits,
        "acomplex" => Setting::AComplex,
        "numstart" => Setting::NumStart,
        "numstep" => Setting::NumStep,
        "numzoom" => Setting::NumZoom,
        "numtype" => Setting::NumType,
        "numindep" => Setting::NumIndep,
        "base" => Setting::Base,
        "bits" => Setting::Bits,
        "signed" => Setting::Signed,
        "ans" => return Some((GlobalInit::Zero, false)),
        _ => return None,
    };
    Some((GlobalInit::Setting(setting), false))
}

fn canonical_system_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "xmin" => "Xmin".into(),
        "xmax" => "Xmax".into(),
        "ymin" => "Ymin".into(),
        "ymax" => "Ymax".into(),
        "xtick" => "Xtick".into(),
        "ytick" => "Ytick".into(),
        "xzoom" => "Xzoom".into(),
        "yzoom" => "Yzoom".into(),
        "axes" => "Axes".into(),
        "griddots" => "GridDots".into(),
        "gridlines" => "GridLines".into(),
        "labels" => "Labels".into(),
        "cursor" => "Cursor".into(),
        "hmin" => "Hmin".into(),
        "hmax" => "Hmax".into(),
        "hwidth" => "Hwidth".into(),
        "numstart" => "NumStart".into(),
        "numstep" => "NumStep".into(),
        "numzoom" => "NumZoom".into(),
        "numtype" => "NumType".into(),
        "numindep" => "NumIndep".into(),
        "nmin" => "Nmin".into(),
        "nmax" => "Nmax".into(),
        "tmin" => "Tmin".into(),
        "tmax" => "Tmax".into(),

        "hangle" => "HAngle".into(),
        "hformat" => "HFormat".into(),
        "hseparator" => "HSeparator".into(),
        "hdigits" => "HDigits".into(),
        "hcomplex" => "HComplex".into(),
        "aangle" => "AAngle".into(),
        "aformat" => "AFormat".into(),
        "adigits" => "ADigits".into(),
        "acomplex" => "AComplex".into(),
        "base" => "Base".into(),
        "bits" => "Bits".into(),
        "signed" => "Signed".into(),
        "ans" => "Ans".into(),
        _ => name.to_string(),
    }
}

impl<'a> Compiler<'a> {
    fn new(opts: CompileOptions, punctuation: &'a [Punctuation], source: Option<&'a str>) -> Self {
        Compiler {
            opts,
            punctuation,
            source,
            diags: Diagnostics::default(),
            consts: Vec::new(),
            globals: Vec::new(),
            global_map: HashMap::new(),
            fns: Vec::new(),
            fn_map: HashMap::new(),
            forward_decls: HashMap::new(),
            chunks: Vec::new(),
            names: Vec::new(),
            init: None,
            cur: empty_chunk("", Span::default()),
            scopes: Vec::new(),
            locals: Vec::new(),
            loops: Vec::new(),
            try_depth: 0,
            in_function: false,
            cas_arg_depth: 0,
        }
    }

    fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::error(DiagCat::Semantic, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    fn warn(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::warning(DiagCat::Semantic, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    /// Valid HP Prime source the emulator cannot run faithfully.
    fn compat(
        &mut self,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> &mut Diagnostic {
        self.diags.push(Diagnostic::compat(code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    /// Informational compatibility note (the emulator substitutes behaviour).
    fn compat_note(
        &mut self,
        code: &'static str,
        span: Span,
        msg: impl Into<String>,
    ) -> &mut Diagnostic {
        self.diags
            .push(Diagnostic::note(DiagCat::Compatibility, code, span, msg));
        self.diags.items.last_mut().unwrap()
    }

    /// Compiles `args` and a call that raises "unsupported" if executed.
    fn unsupported_call(&mut self, name: &str, args: &[Expr], span: Span) {
        self.cas_arg_depth += 1;
        for a in args {
            self.expr(a);
        }
        self.cas_arg_depth -= 1;
        self.unsupported(name, args.len(), span);
    }

    // ---- declarations ----------------------------------------------------

    fn collect(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Function(f) => {
                    if let Some(&prev) = self.fn_map.get(&f.name.name) {
                        let prev_span = self.fns[prev as usize].span;
                        self.error(
                            "duplicate-function",
                            f.name.span,
                            format!("function `{}` is defined twice", f.name.name),
                        )
                        .hint = Some(format!("first definition at byte {}", prev_span.start));
                        continue;
                    }
                    if builtins::lookup(&f.name.name).is_some()
                        || system_global(&f.name.name).is_some()
                    {
                        self.error(
                            "reserved-name",
                            f.name.span,
                            format!(
                                "`{}` is a built-in name and cannot be redefined",
                                f.name.name
                            ),
                        );
                    }
                    self.check_params(&f.params);
                    let id = self.fns.len() as u32;
                    self.fn_map.insert(f.name.name.clone(), id);
                    self.fns.push(FnInfo {
                        name: f.name.name.clone(),
                        params: f.params.len(),
                        def_start: f.span.start,
                        span: f.span,
                    });
                    self.chunks.push(None);
                }
                Item::ForwardDecl { name, span, .. } => {
                    self.forward_decls
                        .entry(name.name.clone())
                        .or_insert(span.start);
                }
                Item::Globals { vars, exported, .. } => {
                    for v in vars {
                        if self.global_map.contains_key(&v.name.name) {
                            self.warn(
                                "duplicate-global",
                                v.name.span,
                                format!("variable `{}` is declared twice", v.name.name),
                            );
                            continue;
                        }
                        if system_global(&v.name.name).is_some() {
                            self.error(
                                "reserved-name",
                                v.name.span,
                                format!(
                                    "`{}` is a calculator variable and cannot be redeclared",
                                    v.name.name
                                ),
                            );
                            continue;
                        }
                        let id = self.globals.len() as u32;
                        self.globals.push(GlobalSlot {
                            name: v.name.name.clone(),
                            init: GlobalInit::Zero,
                            program_var: true,
                            exported: *exported,
                            external: false,
                            read_only: false,
                        });
                        self.global_map.insert(v.name.name.clone(), id);
                    }
                }
                Item::CasBlock { span, .. } => {
                    self.compat(
                        "cas-block-unsupported",
                        *span,
                        "`#cas` sections are not emulated; functions defined there are unavailable",
                    );
                }
            }
        }
        for (name, &pos) in &self.forward_decls.clone() {
            if !self.fn_map.contains_key(name) {
                self.warn(
                    "forward-without-definition",
                    Span {
                        start: pos,
                        end: pos,
                    },
                    format!("`{name}` is declared but never defined"),
                );
            }
        }
    }

    fn check_params(&mut self, params: &[Ident]) {
        let mut seen = Vec::<&str>::new();
        for p in params {
            if seen.contains(&p.name.as_str()) {
                self.error(
                    "duplicate-parameter",
                    p.span,
                    format!("parameter `{}` appears twice", p.name),
                );
            }
            seen.push(&p.name);
            if let Some(b) = builtins::lookup(&p.name) {
                self.error(
                    "reserved-name",
                    p.span,
                    format!(
                        "parameter `{}` shadows the built-in command {}",
                        p.name, b.name
                    ),
                );
            }
        }
    }

    fn compile_items(&mut self, program: &Program) {
        // File-level initialisers run first, in source order.
        self.begin_chunk("<init>", Span::default(), &[]);
        for item in &program.items {
            if let Item::Globals { vars, .. } = item {
                for v in vars {
                    let Some(&slot) = self.global_map.get(&v.name.name) else {
                        continue;
                    };
                    if let Some(init) = &v.init {
                        self.expr(init);
                        self.cur.emit(Op::StoreGlobal(slot), v.name.span);
                    }
                }
            }
        }
        self.cur.emit(Op::Zero, Span::default());
        self.cur.emit(Op::Return, Span::default());
        self.init = Some(self.end_chunk(false));

        for item in &program.items {
            if let Item::Function(f) = item {
                let Some(&id) = self.fn_map.get(&f.name.name) else {
                    continue;
                };
                if self.chunks[id as usize].is_some()
                    || self.fns[id as usize].def_start != f.span.start
                {
                    continue;
                }
                let chunk = self.function(f);
                self.chunks[id as usize] = Some(chunk);
            }
        }
    }

    fn begin_chunk(&mut self, name: &str, span: Span, params: &[Ident]) {
        self.cur = empty_chunk(name, span);
        self.scopes = vec![HashMap::new()];
        self.locals.clear();
        self.loops.clear();
        self.try_depth = 0;
        for p in params {
            let slot = self.new_local(&p.name, p.span, false);
            self.locals[slot as usize].used = true;
        }
        self.cur.params = params.len() as u8;
    }

    fn end_chunk(&mut self, exported: bool) -> Chunk {
        let mut chunk = std::mem::replace(&mut self.cur, empty_chunk("", Span::default()));
        chunk.locals = self.locals.len() as u16;
        chunk.local_names = self.locals.iter().map(|l| l.name.clone()).collect();
        chunk.local_scopes = self
            .locals
            .iter()
            .map(|local| local.scope.start..local.scope.end.min(chunk.code.len() as u32))
            .collect();
        chunk.exported = exported;
        let unused: Vec<(String, Span)> = self
            .locals
            .iter()
            .filter(|l| !l.used && !l.hidden)
            .map(|l| (l.name.clone(), l.span))
            .collect();
        for (name, span) in unused {
            self.warn(
                "unused-local",
                span,
                format!("LOCAL `{name}` is declared but never used"),
            );
        }
        chunk
    }

    fn function(&mut self, f: &FunctionDef) -> Chunk {
        self.in_function = true;
        self.begin_chunk(&f.name.name, f.span, &f.params);
        self.block(&f.body);
        self.cur.emit(Op::Zero, f.span);
        self.cur.emit(Op::Return, f.span);
        self.in_function = false;
        self.end_chunk(f.exported)
    }

    fn new_local(&mut self, name: &str, span: Span, hidden: bool) -> u16 {
        let slot = self.locals.len() as u16;
        self.locals.push(LocalInfo {
            span,
            used: hidden,
            name: name.to_string(),
            hidden,
            scope: self.cur.here()..u32::MAX,
        });
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), slot);
        slot
    }

    fn hidden_local(&mut self, tag: &str) -> u16 {
        let name = format!("<{tag}{}>", self.locals.len());
        self.new_local(&name, Span::default(), true)
    }

    // ---- name resolution -------------------------------------------------

    fn resolve_var(&mut self, name: &str) -> Option<VarRef> {
        for scope in self.scopes.iter().rev() {
            if let Some(&slot) = scope.get(name) {
                self.locals[slot as usize].used = true;
                return Some(VarRef::Local(slot));
            }
        }
        if let Some(&g) = self.global_map.get(name) {
            return Some(VarRef::Global(g));
        }
        if self.is_app_access(name) {
            let index = self.names.len() as u32;
            self.names.push(app_access_name(name));
            return Some(VarRef::App(index));
        }
        if self.opts.external_variables.contains(name) {
            let id = self.globals.len() as u32;
            self.globals.push(GlobalSlot {
                name: name.to_owned(),
                init: GlobalInit::Zero,
                program_var: true,
                exported: false,
                external: true,
                read_only: false,
            });
            self.global_map.insert(name.to_owned(), id);
            return Some(VarRef::Global(id));
        }
        let (init, read_only) = system_global(name)?;
        let canon = canonical_system_name(name);
        if let Some(&g) = self.global_map.get(&canon) {
            return Some(VarRef::Global(g));
        }
        let id = self.globals.len() as u32;
        self.globals.push(GlobalSlot {
            name: canon.clone(),
            init,
            program_var: false,
            exported: false,
            external: false,
            read_only,
        });
        self.global_map.insert(canon, id);
        Some(VarRef::Global(id))
    }

    fn load(&mut self, v: VarRef, span: Span) {
        match v {
            VarRef::Local(s) => self.cur.emit(Op::LoadLocal(s), span),
            VarRef::Global(g) => self.cur.emit(Op::LoadGlobal(g), span),
            VarRef::App(name) => self.cur.emit(
                Op::AppVariable {
                    name,
                    argc: 0,
                    store: false,
                },
                span,
            ),
        };
    }

    fn store(&mut self, v: VarRef, span: Span) {
        if let VarRef::Global(g) = v {
            if self.globals[g as usize].read_only {
                let name = self.globals[g as usize].name.clone();
                self.error("read-only", span, format!("`{name}` cannot be assigned"))
                    .hint = Some("use DIMGROB_P or BLIT_P to change a graphics variable".into());
            }
        }
        match v {
            VarRef::Local(s) => self.cur.emit(Op::StoreLocal(s), span),
            VarRef::Global(g) => self.cur.emit(Op::StoreGlobal(g), span),
            VarRef::App(name) => {
                self.cur.emit(
                    Op::AppVariable {
                        name,
                        argc: 0,
                        store: true,
                    },
                    span,
                );
                self.cur.emit(Op::Pop, span)
            }
        };
    }

    fn constant_index(&mut self, c: Const) -> u32 {
        let idx = match self.consts.iter().position(|k| *k == c) {
            Some(i) => i,
            None => {
                self.consts.push(c);
                self.consts.len() - 1
            }
        };
        idx as u32
    }

    fn constant(&mut self, c: Const, span: Span) {
        let idx = self.constant_index(c);
        self.cur.emit(Op::Const(idx), span);
    }

    fn check_call_order(&mut self, id: u32, span: Span) {
        let info = &self.fns[id as usize];
        if info.def_start > span.start && self.in_function {
            let declared = self
                .forward_decls
                .get(&info.name)
                .is_some_and(|&p| p < span.start);
            if !declared {
                let name = info.name.clone();
                self.diags.push(
                    Diagnostic::warning(
                        DiagCat::Semantic,
                        "call-before-definition",
                        span,
                        format!("`{name}` is called before it is defined"),
                    )
                    .with_hint(format!(
                        "add a forward declaration `{name}(…);` above the caller"
                    ))
                    .unverified(),
                );
            }
        }
    }

    fn check_local_call(&mut self, name: &str, id: u32, argc: usize, span: Span) {
        let params = self.fns[id as usize].params;
        if params != argc {
            self.error(
                "wrong-arg-count",
                span,
                format!("`{name}` expects {params} argument(s), got {argc}"),
            );
        }
        self.check_call_order(id, span);
    }

    // ---- statements ------------------------------------------------------

    fn block(&mut self, stmts: &[Stmt]) {
        self.scopes.push(HashMap::new());
        let mut terminated: Option<Span> = None;
        for s in stmts {
            if let Some(t) = terminated.take() {
                self.warn("unreachable-code", s.span, "unreachable code")
                    .hint = Some(format!(
                    "control never reaches here after the statement at byte {}",
                    t.start
                ));
            }
            self.stmt(s);
            if matches!(
                s.kind,
                StmtKind::Return(_) | StmtKind::Break(_) | StmtKind::Continue
            ) {
                terminated = Some(s.span);
            }
        }
        self.end_scope();
    }

    fn end_scope(&mut self) {
        for slot in self.scopes.pop().expect("lexical scope").into_values() {
            self.locals[slot as usize].scope.end = self.cur.here();
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        self.cur.emit(Op::Stmt, s.span);
        match &s.kind {
            StmtKind::Expr(e) => match &e.kind {
                ExprKind::Assign(t, v) => self.assign(t, v, false, e.span),
                ExprKind::Store(v, t) => self.assign(t, v, false, e.span),
                _ => {
                    self.expr(e);
                    self.cur.emit(Op::Pop, e.span);
                }
            },
            StmtKind::Local(vars) => {
                for v in vars {
                    if v.name.name.is_empty() {
                        continue;
                    }
                    if let Some(b) = builtins::lookup(&v.name.name) {
                        self.warn(
                            "shadows-builtin",
                            v.name.span,
                            format!(
                                "LOCAL `{}` hides the built-in command {}",
                                v.name.name, b.name
                            ),
                        );
                    }
                    if self.scopes.last().unwrap().contains_key(&v.name.name) {
                        self.warn(
                            "duplicate-local",
                            v.name.span,
                            format!("`{}` is already declared in this block", v.name.name),
                        );
                    }
                    // Evaluate the initialiser before the name is visible.
                    match &v.init {
                        Some(init) => self.expr(init),
                        None => {
                            self.cur.emit(Op::Zero, v.name.span);
                        }
                    }
                    let slot = self.new_local(&v.name.name, v.name.span, false);
                    self.cur.emit(Op::StoreLocal(slot), v.name.span);
                }
            }
            StmtKind::Block(body) => self.block(body),
            StmtKind::If {
                cond,
                then_body,
                else_body,
            } => {
                self.expr(cond);
                let jf = self.cur.emit(Op::JumpIfFalse(0), cond.span);
                self.block(then_body);
                match else_body {
                    Some(eb) => {
                        let jend = self.cur.emit(Op::Jump(0), s.span);
                        let here = self.cur.here();
                        self.cur.patch_jump(jf, here);
                        self.block(eb);
                        let here = self.cur.here();
                        self.cur.patch_jump(jend, here);
                    }
                    None => {
                        let here = self.cur.here();
                        self.cur.patch_jump(jf, here);
                    }
                }
            }
            StmtKind::Case { arms, default } => {
                let mut ends = Vec::new();
                for (cond, body) in arms {
                    self.expr(cond);
                    let jf = self.cur.emit(Op::JumpIfFalse(0), cond.span);
                    self.block(body);
                    ends.push(self.cur.emit(Op::Jump(0), cond.span));
                    let here = self.cur.here();
                    self.cur.patch_jump(jf, here);
                }
                if let Some(d) = default {
                    self.block(d);
                }
                let here = self.cur.here();
                for j in ends {
                    self.cur.patch_jump(j, here);
                }
            }
            StmtKind::IfErr {
                body,
                handler,
                else_body,
            } => {
                let t = self.cur.emit(Op::TryBegin(0), s.span);
                self.try_depth += 1;
                self.block(body);
                self.try_depth -= 1;
                self.cur.emit(Op::TryEnd, s.span);
                if let Some(eb) = else_body {
                    self.block(eb);
                }
                let jend = self.cur.emit(Op::Jump(0), s.span);
                let here = self.cur.here();
                self.cur.patch_jump(t, here);
                self.block(handler);
                let here = self.cur.here();
                self.cur.patch_jump(jend, here);
            }
            StmtKind::For {
                var,
                from,
                to,
                step,
                down,
                body,
            } => self.for_loop(var, from, to, step.as_ref(), *down, body, s.span),
            StmtKind::While { cond, body } => {
                let top = self.cur.here();
                self.expr(cond);
                let jf = self.cur.emit(Op::JumpIfFalse(0), cond.span);
                self.loops.push(LoopCtx {
                    breaks: Vec::new(),
                    continues: Vec::new(),
                    try_depth: self.try_depth,
                });
                self.block(body);
                self.cur.emit(Op::Jump(top), s.span);
                let end = self.cur.here();
                self.cur.patch_jump(jf, end);
                self.finish_loop(top, end);
            }
            StmtKind::Repeat { body, until } => {
                let top = self.cur.here();
                self.loops.push(LoopCtx {
                    breaks: Vec::new(),
                    continues: Vec::new(),
                    try_depth: self.try_depth,
                });
                self.block(body);
                let cont = self.cur.here();
                self.expr(until);
                self.cur.emit(Op::JumpIfFalse(top), until.span);
                let end = self.cur.here();
                self.finish_loop(cont, end);
            }
            StmtKind::Return(e) => {
                if !self.in_function {
                    self.error(
                        "return-outside-function",
                        s.span,
                        "RETURN outside a function",
                    );
                }
                match e {
                    Some(e) => self.expr(e),
                    None => {
                        self.cur.emit(Op::Zero, s.span);
                    }
                }
                self.cur.emit(Op::Return, s.span);
            }
            StmtKind::Break(levels) => {
                if levels.is_some() {
                    self.warn(
                        "break-levels",
                        s.span,
                        "BREAK with a level count is treated as a single-level BREAK",
                    )
                    .unverified = true;
                }
                self.loop_jump(true, s.span);
            }
            StmtKind::Continue => self.loop_jump(false, s.span),
            StmtKind::Kill => {
                self.cur.emit(Op::Kill, s.span);
            }
        }
    }

    fn loop_jump(&mut self, is_break: bool, span: Span) {
        let Some(ctx) = self.loops.last() else {
            let what = if is_break { "BREAK" } else { "CONTINUE" };
            self.error("outside-loop", span, format!("{what} outside a loop"));
            return;
        };
        for _ in ctx.try_depth..self.try_depth {
            self.cur.emit(Op::TryEnd, span);
        }
        let j = self.cur.emit(Op::Jump(0), span);
        let ctx = self.loops.last_mut().unwrap();
        if is_break {
            ctx.breaks.push(j)
        } else {
            ctx.continues.push(j)
        }
    }

    fn finish_loop(&mut self, continue_target: u32, end: u32) {
        let ctx = self.loops.pop().unwrap();
        for j in ctx.breaks {
            self.cur.patch_jump(j, end);
        }
        for j in ctx.continues {
            self.cur.patch_jump(j, continue_target);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn for_loop(
        &mut self,
        var: &Ident,
        from: &Expr,
        to: &Expr,
        step: Option<&Expr>,
        down: bool,
        body: &[Stmt],
        span: Span,
    ) {
        let Some(v) = self.resolve_var(&var.name) else {
            self.error(
                "undeclared-identifier",
                var.span,
                format!("undeclared variable `{}` used as FOR counter", var.name),
            )
            .hint = Some(format!("declare it first: LOCAL {};", var.name));
            return;
        };
        self.scopes.push(HashMap::new());
        let to_slot = self.hidden_local("to");
        let step_slot = self.hidden_local("step");
        self.expr(from);
        self.store(v, var.span);
        self.expr(to);
        self.cur.emit(Op::StoreLocal(to_slot), to.span);
        match step {
            Some(s) => self.expr(s),
            None => self.constant(Const::Real(Real::ONE), span),
        }
        self.cur.emit(Op::StoreLocal(step_slot), span);
        let top = self.cur.here();
        self.load(v, var.span);
        self.cur.emit(Op::LoadLocal(to_slot), span);
        self.cur.emit(Op::ForTest { down }, span);
        let jf = self.cur.emit(Op::JumpIfFalse(0), span);
        self.loops.push(LoopCtx {
            breaks: Vec::new(),
            continues: Vec::new(),
            try_depth: self.try_depth,
        });
        self.block(body);
        let cont = self.cur.here();
        self.load(v, var.span);
        self.cur.emit(Op::LoadLocal(step_slot), span);
        self.cur
            .emit(Op::Bin(if down { BinOp::Sub } else { BinOp::Add }), span);
        self.store(v, var.span);
        self.cur.emit(Op::Jump(top), span);
        let end = self.cur.here();
        self.cur.patch_jump(jf, end);
        self.finish_loop(cont, end);
        self.end_scope();
    }

    // ---- assignment ------------------------------------------------------

    fn assign(&mut self, target: &Expr, value: &Expr, want_value: bool, span: Span) {
        match &target.kind {
            ExprKind::Ident(name) => {
                if let Some(variable) = name
                    .strip_prefix("CAS.")
                    .filter(|name| !name.is_empty() && !name.contains('.'))
                {
                    let index = self.names.len() as u32;
                    self.names.push(variable.to_owned());
                    self.expr(value);
                    if want_value {
                        self.cur.emit(Op::Dup, span);
                    }
                    self.cur.emit(
                        Op::CasVariable {
                            name: index,
                            store: true,
                        },
                        target.span,
                    );
                    return;
                }
                if self.is_app_access(name) {
                    self.app_store_value(name, value);
                    self.app_variable(name, 0, true, target.span);
                    if !want_value {
                        self.cur.emit(Op::Pop, span);
                    }
                    return;
                }
                let Some(v) = self.resolve_var(name) else {
                    let msg = if self.fn_map.contains_key(name) || builtins::lookup(name).is_some()
                    {
                        format!("cannot assign to function `{name}`")
                    } else {
                        format!("undeclared variable `{name}`")
                    };
                    self.error("undeclared-identifier", target.span, msg).hint =
                        Some(format!("declare it first: LOCAL {name};"));
                    self.expr(value);
                    if !want_value {
                        self.cur.emit(Op::Pop, span);
                    }
                    return;
                };
                self.expr(value);
                if want_value {
                    self.cur.emit(Op::Dup, span);
                }
                self.store(v, target.span);
                if want_value
                    && crate::geometry::is_plot_field(name)
                    && let VarRef::Global(slot) = v
                {
                    self.cur.emit(Op::GeometryAssignmentResult(slot), span);
                }
            }
            ExprKind::Call(base, idx) | ExprKind::Index(base, idx) => {
                let ExprKind::Ident(name) = &base.kind else {
                    self.error(
                        "invalid-assignment-target",
                        target.span,
                        "only variables and their elements can be assigned",
                    );
                    return;
                };
                // Indexed system variables are assignable on the Prime
                // (`Notes("X"):=…`, `Programs("X"):=…`; hardware-verified by
                // the result-note writers of campaign runs). They resolve to
                // built-ins here, not variables, so they get their own store
                // operations.
                if idx.len() == 2
                    && builtins::lookup_case_insensitive(name)
                        .is_some_and(|spec| spec.builtin == Builtin::Hvars)
                {
                    self.expr(&idx[0]);
                    self.expr(&idx[1]);
                    self.expr(value);
                    self.cur.emit(Op::StoreHomeParameters, target.span);
                    if !want_value {
                        self.cur.emit(Op::Pop, span);
                    }
                    return;
                }
                if idx.len() == 1 {
                    let sys_store =
                        builtins::lookup_case_insensitive(name).and_then(|s| match s.builtin {
                            Builtin::Notes => Some(Op::StoreNote),
                            Builtin::Programs => Some(Op::StoreProgram),
                            Builtin::Hvars => Some(Op::StoreHomeVariable),
                            Builtin::Afiles => Some(Op::StoreAppFile),
                            _ => None,
                        });
                    if let Some(op) = sys_store {
                        self.expr(&idx[0]);
                        self.expr(value);
                        self.cur.emit(op, target.span);
                        if !want_value {
                            self.cur.emit(Op::Pop, span);
                        }
                        return;
                    }
                }
                if self.is_app_access(name) {
                    if idx.len() > 255 {
                        self.error(
                            "wrong-arg-count",
                            target.span,
                            "Too many app variable indices",
                        );
                        return;
                    }
                    for index in idx {
                        self.expr(index);
                    }
                    self.app_store_value(name, value);
                    self.app_variable(name, idx.len() as u8, true, target.span);
                    if !want_value {
                        self.cur.emit(Op::Pop, span);
                    }
                    return;
                }
                let Some(v) = self.resolve_var(name) else {
                    self.error(
                        "invalid-assignment-target",
                        target.span,
                        format!("`{name}` is not a variable"),
                    )
                    .hint = Some(format!("declare it first: LOCAL {name};"));
                    return;
                };
                if idx.is_empty() || idx.len() > 255 {
                    self.error(
                        "invalid-assignment-target",
                        target.span,
                        "an element assignment needs at least one index",
                    );
                    return;
                }
                if let VarRef::Local(slot) = v
                    && matches!(idx.as_slice(), [Expr { kind: ExprKind::Number(index), .. }] if index == "0")
                {
                    self.expr(value);
                    self.cur.emit(Op::StoreIndexZeroLocal(slot), target.span);
                    if !want_value {
                        self.cur.emit(Op::Pop, span);
                    }
                    return;
                }
                for i in idx {
                    self.expr(i);
                }
                self.expr(value);
                let n = idx.len() as u8;
                match v {
                    VarRef::Local(slot) => {
                        self.cur.emit(Op::StoreIndexLocal { slot, n }, target.span)
                    }
                    VarRef::Global(slot) => {
                        if self.globals[slot as usize].read_only {
                            self.error(
                                "read-only",
                                target.span,
                                format!("`{name}` cannot be assigned"),
                            );
                        }
                        self.cur.emit(Op::StoreIndexGlobal { slot, n }, target.span)
                    }
                    VarRef::App(name) => self.cur.emit(
                        Op::AppVariable {
                            name,
                            argc: n,
                            store: true,
                        },
                        target.span,
                    ),
                };
                if !want_value {
                    self.cur.emit(Op::Pop, span);
                }
            }
            _ => {
                self.error(
                    "invalid-assignment-target",
                    target.span,
                    "L-value must be a variable or an element of a variable",
                )
                .hint =
                    Some("the left side of := (or right side of ▶) must name a variable".into());
                self.expr(value);
                if !want_value {
                    self.cur.emit(Op::Pop, span);
                }
            }
        }
    }

    // ---- expressions -----------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        let span = e.span;
        match &e.kind {
            ExprKind::Number(text) => match Real::parse(text) {
                Some(r) => self.constant(
                    Const::Number {
                        source: text.clone(),
                        legacy: r,
                    },
                    span,
                ),
                None => {
                    self.error(
                        "number-out-of-range",
                        span,
                        format!("number `{text}` is out of range"),
                    );
                    self.cur.emit(Op::Zero, span);
                }
            },
            ExprKind::HashInt(text) => match HpInt::parse_literal_with_defaults(
                text,
                self.opts.default_int_base,
                self.opts.default_int_width,
            ) {
                Some(i) => self.constant(Const::Int(i), span),
                None => {
                    self.error(
                        "malformed-integer",
                        span,
                        format!("malformed integer literal `#{text}`"),
                    )
                    .hint = Some("integers look like #FFh, #1010b, #777o or #255d".into());
                    self.cur.emit(Op::Zero, span);
                }
            },
            ExprKind::Str(s) => self.constant(Const::Str(s.clone()), span),
            ExprKind::Ident(name) => self.ident(name, span),
            ExprKind::List(items) => {
                for i in items {
                    self.expr(i);
                }
                self.cur.emit(Op::MakeList(items.len() as u32), span);
            }
            ExprKind::Bracket(items) => self.bracket(items, span),
            ExprKind::ComplexPair(a, b) => {
                self.expr(a);
                self.expr(b);
                self.cur.emit(Op::MakeComplex, span);
            }
            ExprKind::Unary(op, x) => {
                self.expr(x);
                self.cur.emit(
                    match op {
                        UnOp::Neg => Op::Neg,
                        UnOp::Not => Op::Not,
                        UnOp::Sqrt => Op::Sqrt,
                        UnOp::Sexagesimal => Op::Sexagesimal,
                    },
                    span,
                );
            }
            ExprKind::Postfix(op, x) => {
                self.expr(x);
                self.cur.emit(
                    match op {
                        PostOp::Factorial => Op::Factorial,
                        PostOp::Square => Op::Square,
                        PostOp::Cube => Op::Cube,
                    },
                    span,
                );
            }
            ExprKind::Binary(op, l, r) => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    self.expr(l);
                    let jump = self.cur.emit(
                        Op::LogicJump {
                            or: *op == BinOp::Or,
                            target: 0,
                        },
                        span,
                    );
                    self.expr(r);
                    self.cur.emit(Op::Bin(*op), span);
                    self.cur.patch_jump(jump, self.cur.here());
                    return;
                }
                // Hardware-verified (2026-10-23, G1 build 15157): the
                // calculator's compiler rejects real literals as DIV
                // operands with a "Syntax error" when the program is run
                // (`3.5 DIV 1E-11` → error at that line; `7 DIV -3` from
                // the verified core suite is fine). DIV is the
                // integer-quotient operator.
                if *op == BinOp::IntDiv {
                    let real_lit = |e: &Expr| {
                        matches!(&e.kind, ExprKind::Number(s)
                            if s.contains('.') || s.to_ascii_uppercase().contains('E'))
                    };
                    if real_lit(l) || real_lit(r) {
                        self.error(
                            "div-requires-integers",
                            span,
                            "DIV requires integer operands; real literals are a syntax error on the calculator",
                        )
                        .hint = Some(
                            "use IP() to take the integer part of a real, or divide with /".into(),
                        );
                    }
                }
                self.expr(l);
                self.expr(r);
                self.cur.emit(Op::Bin(*op), span);
            }
            ExprKind::Call(callee, args) => self.call(callee, args, span),
            ExprKind::Index(base, args) => {
                self.expr(base);
                for a in args {
                    self.expr(a);
                }
                self.cur.emit(Op::Index(args.len().min(255) as u8), span);
            }
            ExprKind::Assign(t, v) => self.assign(t, v, true, span),
            ExprKind::Store(v, t) => self.assign(t, v, true, span),
            ExprKind::Quoted(inner) => {
                self.constant(Const::Symbolic(crate::symbolic::source(inner)), span);
            }
            ExprKind::Error => {
                self.cur.emit(Op::Zero, span);
            }
        }
    }

    fn bracket(&mut self, items: &[Expr], span: Span) {
        let is_matrix =
            !items.is_empty() && items.iter().all(|i| matches!(i.kind, ExprKind::Bracket(_)));
        if !is_matrix {
            for i in items {
                self.expr(i);
            }
            self.cur.emit(Op::MakeVector(items.len() as u32), span);
            return;
        }
        let mut cols = None;
        for row in items {
            let ExprKind::Bracket(cells) = &row.kind else {
                unreachable!()
            };
            match cols {
                None => cols = Some(cells.len()),
                Some(c) if c != cells.len() => {
                    self.error(
                        "invalid-dimension",
                        row.span,
                        format!("matrix row has {} elements, expected {c}", cells.len()),
                    );
                }
                _ => {}
            }
            for c in cells {
                self.expr(c);
            }
        }
        // Rows of unequal length were reported; pad so code generation stays sane.
        let cols = cols.unwrap_or(0) as u32;
        self.cur.emit(
            Op::MakeMatrix {
                rows: items.len() as u32,
                cols,
            },
            span,
        );
    }

    /// Bare function/analysis slots and results follow declared PPL names.
    /// Their app is selected at execution, including in a qualified outer call.
    fn is_app_access(&self, name: &str) -> bool {
        is_app_variable(name)
            || ((is_function_slot(name)
                || is_graph3d_slot(name)
                || matches!(name, "Zmin" | "Zmax" | "Ztick")
                || is_statistics_slot(name)
                || is_statistics_two_slot(name)
                || (crate::geometry::is_variable(name)
                    && system_global(name).is_none()
                    && builtins::lookup_case_insensitive(name).is_none()))
                && !self.scopes.iter().any(|scope| scope.contains_key(name))
                && !self.global_map.contains_key(name)
                && !self.fn_map.contains_key(name)
                && !self.opts.external_variables.contains(name)
                && !self.opts.external_functions.contains_key(name)
                && !self.opts.ambiguous_externals.contains(name))
    }

    fn app_store_value(&mut self, name: &str, value: &Expr) {
        // A quoted worksheet formula executes as PPL when its cell recalculates.
        // CAS syntax such as list[...] would change a list literal into an index.
        if name.starts_with("Spreadsheet.")
            && let ExprKind::Quoted(inner) = &value.kind
            && let Ok(source) = crate::ppl_expression_source(inner)
        {
            self.constant(Const::Symbolic(source), value.span);
        } else {
            self.expr(value);
        }
    }

    fn app_variable(&mut self, name: &str, argc: u8, store: bool, span: Span) {
        let index = self.names.len() as u32;
        self.names.push(app_access_name(name));
        self.cur.emit(
            Op::AppVariable {
                name: index,
                argc,
                store,
            },
            span,
        );
    }

    fn ident(&mut self, name: &str, span: Span) {
        if self.is_app_access(name) {
            self.app_variable(name, 0, false, span);
            return;
        }
        if let Some(variable) = name
            .strip_prefix("CAS.")
            .filter(|name| !name.is_empty() && !name.contains('.'))
        {
            let index = self.names.len() as u32;
            self.names.push(variable.to_owned());
            self.cur.emit(
                Op::CasVariable {
                    name: index,
                    store: false,
                },
                span,
            );
            return;
        }
        match name {
            "π" => return self.constant(Const::Pi, span),
            "ℯ" => {
                return self.constant(
                    Const::Real(Real::parse("2.71828182846").unwrap().decimal()),
                    span,
                );
            }
            "ⅈ" | "\u{E003}" => {
                return self.constant(Const::Complex(Complex::new(Real::ZERO, Real::ONE)), span);
            }
            "∞" => {
                self.compat("unsupported-value", span, "∞ is not emulated")
                    .hint = Some("evaluating it raises an \"unsupported\" error".into());
                self.unsupported("∞", 0, span);
                return;
            }
            _ => {}
        }
        if let Some(v) = self.resolve_var(name) {
            return self.load(v, span);
        }
        if let Some(&id) = self.fn_map.get(name) {
            let params = self.fns[id as usize].params;
            if params != 0 {
                self.error(
                    "wrong-arg-count",
                    span,
                    format!("`{name}` expects {params} argument(s), got 0"),
                );
            }
            self.check_call_order(id, span);
            self.cur.emit(Op::Call { func: id, argc: 0 }, span);
            return;
        }
        if self.opts.ambiguous_externals.contains(name) {
            self.error(
                "ambiguous-external",
                span,
                format!("ambiguous external export {name}"),
            );
            self.cur.emit(Op::Zero, span);
            return;
        }
        if let Some(&params) = self.opts.external_functions.get(name) {
            if params != 0 {
                self.error(
                    "wrong-arg-count",
                    span,
                    format!("{name} expects {params} argument(s), got 0"),
                );
            }
            let idx = self.names.len() as u32;
            self.names.push(name.into());
            self.cur.emit(Op::CallExternal { name: idx, argc: 0 }, span);
            return;
        }
        // ASCII i is the firmware's imaginary constant only after ordinary
        // local/file/function resolution. A declared local or function i wins.
        if name == "i" {
            return self.constant(Const::Complex(Complex::new(Real::ZERO, Real::ONE)), span);
        }
        if name.eq_ignore_ascii_case("AFiles") {
            return self.builtin(Builtin::Afiles, &[], span);
        }
        if let Some(spec) = builtins::lookup(name) {
            if spec.accepts(0) && spec.ref_args == 0 {
                return self.builtin(spec.builtin, &[], span);
            }
            self.error(
                "wrong-arg-count",
                span,
                format!("{} needs arguments ({})", spec.name, spec.arity_text()),
            );
            self.cur.emit(Op::Zero, span);
            return;
        }
        if self.cas_arg_depth > 0 {
            // `solve(x^2-4=0, x)`: `x` is a CAS symbol, not a program
            // variable, and the whole call is already reported as
            // unsupported, so no diagnostic of its own.
            self.cur.emit(Op::Zero, span);
            return;
        }
        if let Some(entry) = catalog::lookup(name).filter(|e| e.kind == CatalogKind::Command) {
            // Documented zero-argument commands and constants (MAXREAL,
            // VERSION, AFiles, ...).
            self.compat(
                "unsupported-command",
                span,
                format!(
                    "{} is an HP Prime {} that is not emulated",
                    entry.name,
                    entry.kind.label()
                ),
            )
            .hint = Some(format!(
                "evaluating it raises an \"unsupported\" error ({})",
                catalog::SOURCE
            ));
            self.unsupported(entry.name, 0, span);
            return;
        }
        let mut d = Diagnostic::error(
            DiagCat::Semantic,
            "undeclared-identifier",
            span,
            format!("undeclared variable `{name}`"),
        )
        .with_hint(format!(
            "declare it with LOCAL {name}; or use a calculator variable such as A–Z"
        ));
        if let Some(spec) = builtins::lookup_case_insensitive(name) {
            d = d.with_hint(format!("did you mean the command {}?", spec.name));
        } else if let Some(entry) = catalog::lookup(name) {
            d = d.with_hint(format!(
                "`{}` is also a {}; call it with its arguments if that was intended",
                entry.name,
                entry.kind.label()
            ));
        }
        self.diags.push(d);
        self.cur.emit(Op::Zero, span);
    }

    fn arity_location(&self, callee: &Expr, min: u8, max: u8, argc: usize) -> Option<Span> {
        // Search only an erroneous call's existing tokens. Valid calls need
        // no lookup, allocation, extra lexer pass or per-call metadata.
        let start = self
            .punctuation
            .partition_point(|t| t.span.start < callee.span.end);
        if self.punctuation.get(start)?.kind != PunctuationKind::Open {
            return None;
        }
        let mut depth = 1usize;
        let mut commas = 0usize;
        for token in &self.punctuation[start + 1..] {
            match token.kind {
                PunctuationKind::Open => depth += 1,
                PunctuationKind::Close => {
                    depth -= 1;
                    if depth == 0 {
                        return (argc < usize::from(min)).then_some(token.span);
                    }
                }
                PunctuationKind::Comma if depth == 1 => {
                    commas += 1;
                    if argc > usize::from(max) && commas == usize::from(max) {
                        return Some(token.span);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn geometry_argument(&mut self, expression: &Expr, captures: &mut u8) -> Expr {
        use ExprKind as E;
        let mut result = expression.clone();
        if let E::Call(callee, args) | E::Index(callee, args) = &expression.kind {
            let native = matches!(&callee.kind, E::Ident(name) if crate::geometry::is_function_call(name, args.len())
                && !self.fn_map.contains_key(name) && !self.opts.external_functions.contains_key(name)
                && !self.scopes.iter().any(|scope| scope.contains_key(name)));
            if !native {
                if *captures >= 250 {
                    self.error(
                        "invalid-geometry-expression",
                        expression.span,
                        "Too many Geometry argument calls",
                    );
                    return result;
                }
                let name = format!("__GeometryArgument{}", *captures);
                *captures += 1;
                self.constant(Const::Str(name.clone()), expression.span);
                self.expr(expression);
                self.cur.emit(Op::MakeList(2), expression.span);
                result.kind = E::Ident(name);
                return result;
            }
        }
        match &mut result.kind {
            E::Call(_, args) | E::Index(_, args) | E::List(args) | E::Bracket(args) => {
                for arg in args {
                    *arg = self.geometry_argument(arg, captures);
                }
            }
            E::Unary(_, e) | E::Postfix(_, e) | E::Quoted(e) => {
                **e = self.geometry_argument(e, captures)
            }
            E::Binary(_, a, b) | E::ComplexPair(a, b) => {
                **a = self.geometry_argument(a, captures);
                **b = self.geometry_argument(b, captures);
            }
            E::Assign(..) | E::Store(..) => {
                if *captures >= 250 {
                    self.error(
                        "invalid-geometry-expression",
                        expression.span,
                        "Too many Geometry argument calls",
                    );
                    return result;
                }
                let name = format!("__GeometryArgument{}", *captures);
                *captures += 1;
                self.constant(Const::Str(name.clone()), expression.span);
                self.expr(expression);
                self.cur.emit(Op::MakeList(2), expression.span);
                result.kind = E::Ident(name);
            }
            _ => {}
        }
        result
    }
    fn call(&mut self, callee: &Expr, args: &[Expr], span: Span) {
        let ExprKind::Ident(name) = &callee.kind else {
            // `L(1)(2)`: index the result.
            self.expr(callee);
            for a in args {
                self.expr(a);
            }
            self.cur.emit(Op::Index(args.len().min(255) as u8), span);
            return;
        };
        if self.is_app_access(name) {
            if args.len() > 255 {
                self.error("wrong-arg-count", span, "Too many app variable indices");
                return;
            }
            for arg in args {
                self.expr(arg);
            }
            self.app_variable(name, args.len() as u8, false, span);
            return;
        }
        if let Some(v) = self.resolve_var(name) {
            self.load(v, callee.span);
            if args.is_empty() {
                return;
            }
            for a in args {
                self.expr(a);
            }
            self.cur.emit(Op::Index(args.len().min(255) as u8), span);
            return;
        }
        if let Some(&id) = self.fn_map.get(name) {
            self.check_local_call(name, id, args.len(), span);
            for a in args {
                self.expr(a);
            }
            self.cur.emit(
                Op::Call {
                    func: id,
                    argc: args.len().min(255) as u8,
                },
                span,
            );
            return;
        }
        if self.opts.ambiguous_externals.contains(name) {
            self.error(
                "ambiguous-external",
                span,
                format!("ambiguous external export {name}"),
            );
            self.cur.emit(Op::Zero, span);
            return;
        }
        if let Some(&params) = self.opts.external_functions.get(name) {
            if params != args.len() {
                self.error(
                    "wrong-arg-count",
                    span,
                    format!("{name} expects {params} argument(s), got {}", args.len()),
                );
            }
            for a in args {
                self.expr(a);
            }
            let idx = self.names.len() as u32;
            self.names.push(name.into());
            self.cur.emit(
                Op::CallExternal {
                    name: idx,
                    argc: args.len().min(255) as u8,
                },
                span,
            );
            return;
        }
        if crate::geometry::is_function_call(name, args.len()) {
            let mut captures = 0;
            let args = args
                .iter()
                .map(|arg| self.geometry_argument(arg, &mut captures))
                .collect();
            let expression = Expr {
                kind: ExprKind::Call(Box::new(callee.clone()), args),
                span,
            };
            match crate::ppl_expression_source(&expression) {
                Ok(source) => self.constant(Const::Symbolic(source), span),
                Err(error) => {
                    self.error("invalid-geometry-expression", span, error.to_string());
                    self.cur.emit(Op::Zero, span);
                }
            }
            self.app_variable("Geometry.__Expression", captures + 1, false, span);
            return;
        }
        if let Some(function) = name.strip_prefix("CAS.") {
            let source = format!(
                "{}({})",
                function,
                args.iter()
                    .map(|a| match &a.kind {
                        ExprKind::Str(s) => s.clone(),
                        _ => crate::symbolic::source(a),
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            );
            self.constant(Const::Symbolic(source), span);
            self.cur.emit(
                Op::Builtin {
                    id: Builtin::CasCall,
                    argc: 1,
                },
                span,
            );
            return;
        }
        if name.contains('.') {
            // `CAS.solve(...)`, `Finance.TvmFV(...)`: documented forms for
            // reaching the CAS and app functions from a program.
            self.compat(
                "cas-unsupported",
                callee.span,
                format!("`{name}` is a CAS/app function and is not emulated"),
            )
            .hint = Some("running this call raises an \"unsupported\" error".into());
            self.unsupported_call(name, args, span);
            return;
        }
        let spec = match builtins::lookup(name) {
            Some(s) => Some(s),
            None => {
                let ci = builtins::lookup_case_insensitive(name);
                if let Some(s) = ci {
                    if s.status != Status::Unsupported {
                        let d = self.compat_note(
                            "lowercase-command",
                            callee.span,
                            if name == "stddev" {
                                "Native implements only some forms of the calculator's lowercase stddev command".into()
                            } else if name == "append" {
                                "Native supports selected collection forms of lowercase append (APPEND)".into()
                            } else {
                                format!(
                                    "`{name}` calls the CAS command on the calculator; \
                                     the emulator runs the Home command {} instead",
                                    s.name
                                )
                            },
                        );
                        d.hint = Some(
                            "results can differ where the CAS and Home versions differ".into(),
                        );
                        d.unverified = true;
                    }
                }
                ci
            }
        };
        let Some(spec) = spec else {
            if let Some(entry) = catalog::lookup_case_insensitive(name) {
                if entry.kind == CatalogKind::Cas {
                    let source = format!(
                        "{}({})",
                        entry.name,
                        args.iter()
                            .map(crate::symbolic::source)
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    self.constant(Const::Symbolic(source), span);
                    self.cur.emit(
                        Op::Builtin {
                            id: Builtin::CasCall,
                            argc: 1,
                        },
                        span,
                    );
                    return;
                }
                let d = self.compat(
                    "unsupported-command",
                    callee.span,
                    format!(
                        "{} is an HP Prime {} that is not emulated",
                        entry.name,
                        entry.kind.label()
                    ),
                );
                d.hint = Some(format!(
                    "running this call raises an \"unsupported\" error ({})",
                    catalog::SOURCE
                ));
                if entry.name != name {
                    d.message = format!(
                        "`{name}` matches the documented {} {} (spelling differs in case) and is not emulated",
                        entry.kind.label(),
                        entry.name
                    );
                    d.unverified = true;
                }
                self.unsupported_call(entry.name, args, span);
                return;
            }
            let known_upper = builtins::lookup_case_insensitive(name).is_some();
            let mut d = Diagnostic::error(
                DiagCat::Semantic,
                "unknown-function",
                callee.span,
                format!("unknown function `{name}`"),
            );
            if known_upper {
                d = d.with_hint(format!("did you mean {}?", name.to_uppercase()));
            }
            self.diags.push(d);
            self.cur.emit(Op::Zero, span);
            return;
        };
        if !spec.accepts(args.len()) {
            // Firmware stops at the close for missing arguments, or at the
            // comma introducing the first excess argument. Token spans keep
            // this exact even with whitespace, comments and nested calls.
            let at = self.arity_location(callee, spec.min, spec.max, args.len());
            self.error(
                "wrong-arg-count",
                at.unwrap_or(span),
                format!(
                    "{} expects {} argument(s), got {}",
                    spec.name,
                    spec.arity_text(),
                    args.len()
                ),
            );
        }
        if spec.status == Status::Unsupported {
            self.compat(
                "unsupported-command",
                callee.span,
                format!("{} is not emulated yet", spec.name),
            )
            .hint = Some(match spec.builtin {
                Builtin::Eval => "Use G1 Firmware mode for symbolic evaluation; Native values do not model CAS expressions",
                Builtin::Startapp => "Use G1 Firmware mode to launch calculator applications and their views",
                _ => "running this call raises an \"unsupported\" error",
            }.into());
        }
        match spec.builtin {
            Builtin::Do1VStats | Builtin::Do2VStats | Builtin::SetSample | Builtin::SetFreq
                if spec.accepts(args.len()) => {
                // Firmware stores the argument syntax, including local names,
                // without evaluating its list arithmetic or scalar frequency.
                for (index, argument) in args.iter().enumerate() {
                    let captured = crate::ppl_expression_source(argument).map(|canonical| {
                        // Parentheses distinguish a scalar frequency expression from
                        // a literal on the original calculator. AST printing erases them.
                        if spec.builtin == Builtin::SetFreq && index == 1 {
                            self.source
                                .and_then(|source| source.get(argument.span.start as usize..argument.span.end as usize))
                                .map(str::to_owned)
                                .unwrap_or(canonical)
                        } else {
                            canonical
                        }
                    });
                    match captured {
                        Ok(source) => self.constant(Const::Symbolic(source), argument.span),
                        Err(error) => {
                            self.error("invalid-app-expression", argument.span, error.to_string());
                            self.cur.emit(Op::Zero, argument.span);
                        }
                    }
                }
                self.cur.emit(Op::Builtin { id: spec.builtin, argc: args.len() as u8 }, span);
            }
            Builtin::CasCall if args.len() == 1 => {
                let expression = &args[0];
                // Lists and declared PPL functions are ordinary evaluated
                // arguments. Algebra syntax is quoted: do not run assignments
                // embedded in it before the CAS type check.
                let variable = matches!(&expression.kind, ExprKind::Ident(name)
                    if self.resolve_var(name).is_some() || name.starts_with("CAS.") || self.is_app_access(name));
                let native_call = |e: &Expr| match &e.kind {
                    ExprKind::Ident(name) => self.is_app_access(name),
                    ExprKind::Call(callee, _) => matches!(&callee.kind, ExprKind::Ident(name)
                        if self.is_app_access(name) || self.fn_map.contains_key(name)
                            || self.opts.external_functions.contains_key(name)),
                    _ => false,
                };
                let native = matches!(&expression.kind, ExprKind::Str(_) | ExprKind::List(_))
                    || (matches!(&expression.kind, ExprKind::Bracket(_) | ExprKind::ComplexPair(_, _))
                        && !crate::symbolic::contains_assignment(expression))
                    || variable
                    || native_call(expression);
                let assignment = !native && crate::symbolic::contains_assignment(expression);
                let source = matches!(&expression.kind, ExprKind::Str(_));
                if native {
                    self.expr(expression);
                } else if assignment {
                    self.constant(Const::Symbolic(crate::symbolic::source(expression)), span);
                } else {
                    let (template, marker, operands) = crate::symbolic::dynamic_source(expression, native_call);
                    if operands.is_empty() {
                        self.constant(Const::Symbolic(template), span);
                    } else if operands.len() > u8::MAX as usize {
                        self.error("too-many-cas-operands", span, "CAS expression has more than255 Native call operands");
                        self.cur.emit(Op::Zero, span);
                    } else {
                        for operand in &operands { self.expr(operand); }
                        self.constant(Const::Str(template), span);
                        self.constant(Const::Str(marker), span);
                        self.cur.emit(Op::CasCompose { argc: operands.len() as u8 }, span);
                    }
                }
                self.cur.emit(Op::CasCall { assignment, source }, span);
            }
            Builtin::Makelist if args.len() >= 4 => self.makelist(args, span),
            Builtin::Append if name == "append" && args.len() == 2 => {
                let base = &args[0];
                let encoded_base =
                    source_encoded_matrix(base) || source_encoded_vector(base).is_some();
                self.expr(base);
                let payload = &args[1];
                if let ExprKind::Binary(BinOp::Div, numerator, denominator) = &payload.kind {
                    let numerator_call = self.append_callback_operand(numerator, 0);
                    let denominator_call =
                        self.append_callback_operand(denominator, 1 + u8::from(numerator_call));
                    let literal_operands = matches!(numerator.kind, ExprKind::Number(_))
                        && matches!(denominator.kind, ExprKind::Number(_));
                    self.cur.emit(
                        Op::AppendQuotient {
                            literal_operands,
                            callback_operands: [numerator_call, denominator_call],
                            encoded_base,
                        },
                        payload.span,
                    );
                } else {
                    let callback_payload = self.append_callback_operand(payload, 0);
                    let literal_payload = matches!(payload.kind, ExprKind::Str(_));
                    self.cur.emit(
                        Op::AppendCollection {
                            literal_payload,
                            callback_payload,
                            encoded_base,
                        },
                        span,
                    );
                }
            }
            Builtin::Stddev if name == "stddev" => {
                if self.stddev_inline_vector_pair(args, span)
                    || self.weighted_collection_stddev(args, span)
                    || self.stddev_mixed_vector(args, span)
                {
                    return;
                }
                let matrix_diagnostic = stddev_matrix_diagnostic(args);
                if let Some(StddevMatrixDiagnostic::EncodedOperands(diagnostic)) = matrix_diagnostic {
                    for arg in args {
                        self.expr(arg);
                    }
                    self.constant(Const::Str(diagnostic), span);
                    self.cur.emit(Op::StddevMatrixDiagnostic, span);
                } else {
                    // In a two-argument call, encoded vectors have separate
                    // argument-shape rules. Keep their established dispatch.
                    if matches!(args, [data, weights]
                        if source_encoded_vector(data).is_some()
                            || source_encoded_vector(weights).is_some())
                    {
                        self.builtin(spec.builtin, args, span);
                    } else {
                        for arg in args {
                            self.expr(arg);
                        }
                        let row_mismatch_diagnostic =
                            if let Some(StddevMatrixDiagnostic::RowMismatch(diagnostic)) = matrix_diagnostic {
                                self.constant(Const::Str(diagnostic), span);
                                true
                            } else {
                                false
                            };
                        let list_sources = self.stddev_list_sources(args);
                        self.cur.emit(Op::StddevExact {
                            argc: args.len() as u8,
                            encoded_vector: args.first().and_then(source_encoded_vector).is_some(),
                            // Encoded inline matrices have a separate argument path.
                            evaluated_matrices: matches!(args, [data, weights]
                                if !source_encoded_matrix(data) && !source_encoded_matrix(weights)),
                            row_mismatch_diagnostic,
                            list_sources,
                        }, span);
                    }
                }
            }
            // Firmware EDITMAT writes back bare M0–M9 names, even when
            // locally declared. Other local names are value expressions.
            Builtin::Editmat | Builtin::Editlist
                if args.first().is_some_and(|arg| {
                    matches!(&arg.kind, ExprKind::Ident(name) if name.len() == 2
                    && name.starts_with(if spec.builtin == Builtin::Editmat { 'M' } else { 'L' }) && name.as_bytes()[1].is_ascii_digit())
                }) =>
            {
                self.ref_builtin(spec.builtin, args, span);
            }
            Builtin::LineP | Builtin::Line | Builtin::TriangleP | Builtin::Triangle => {
                self.graphics_builtin(spec.builtin, args, span);
            }
            _ if spec.ref_args > 0 => self.ref_builtin(spec.builtin, args, span),
            _ => {
                self.builtin(spec.builtin, args, span);
            }
        }
    }

    fn stddev_list_sources(&mut self, args: &[Expr]) -> Option<std::num::NonZeroU32> {
        let [data, weights] = args else {
            return None;
        };
        let source = |expression: &Expr| match &expression.kind {
            ExprKind::List(_) => Some(None),
            ExprKind::Ident(name) => Some(Some(name.clone())),
            _ => None,
        };
        let sources = [source(data)?, source(weights)?];
        Some(
            self.cur
                .add_stddev_metadata(StddevMetadata::ListSources(sources)),
        )
    }

    fn builtin(&mut self, id: Builtin, args: &[Expr], span: Span) {
        for a in args {
            self.expr(a);
        }
        self.cur.emit(
            Op::Builtin {
                id,
                argc: args.len().min(255) as u8,
            },
            span,
        );
    }

    fn append_callback_operand(&mut self, expression: &Expr, preceding: u8) -> bool {
        if let ExprKind::Call(callee, args) = &expression.kind
            && let ExprKind::Ident(name) = &callee.kind
            && let Some(id) = self.fn_map.get(name).copied()
            && !self.is_app_access(name)
            && self.resolve_var(name).is_none()
        {
            let span = expression.span;
            self.check_local_call(name, id, args.len(), span);
            for (index, arg) in args.iter().enumerate() {
                if let ExprKind::Binary(BinOp::Div, numerator, denominator) = &arg.kind {
                    self.expr(numerator);
                    self.expr(denominator);
                    self.cur.emit(
                        Op::AppendCallbackArgumentQuotient {
                            preceding: u16::from(preceding) + index.min(255) as u16,
                        },
                        arg.span,
                    );
                } else {
                    self.expr(arg);
                }
            }
            let argc = args.len().min(255) as u8;
            let handler = self.cur.emit(
                Op::TryAppendCallback {
                    handler: 0,
                    argc,
                    preceding,
                },
                span,
            );
            self.cur.emit(Op::Call { func: id, argc }, span);
            self.cur.emit(Op::TryEnd, span);
            self.cur.emit(Op::Zero, span);
            // The VM has already supplied the operand and status on failure.
            self.cur.patch_jump(handler, self.cur.here());
            return true;
        }
        self.expr(expression);
        false
    }

    fn stddev_callback_source(&mut self, expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::ComplexPair(real, imaginary) => {
                self.stddev_callback_source(real) && self.stddev_callback_source(imaginary)
            }
            ExprKind::Call(callee, args) => {
                matches!(&callee.kind, ExprKind::Ident(name)
                    if self.fn_map.contains_key(name)
                        && !self.is_app_access(name)
                        && self.resolve_var(name).is_none())
                    && args.iter().all(|arg| stddev_numeric_entry(arg, false))
            }
            _ => stddev_numeric_entry(expression, false),
        }
    }

    fn stddev_inline_source(&mut self, expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::Unary(UnOp::Neg, value) => self.stddev_inline_source(value),
            ExprKind::Binary(BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, left, right) => {
                self.stddev_inline_source(left) && self.stddev_inline_source(right)
            }
            ExprKind::Call(callee, args) => {
                matches!(&callee.kind, ExprKind::Ident(name)
                    if self.fn_map.contains_key(name)
                        && !self.is_app_access(name)
                        && self.resolve_var(name).is_none())
                    && args.iter().all(|arg| self.stddev_inline_source(arg))
            }
            _ => stddev_numeric_entry(expression, false),
        }
    }

    fn stddev_inline_observation(&mut self, expression: &Expr) -> bool {
        match &expression.kind {
            ExprKind::ComplexPair(real, imaginary) => {
                self.stddev_inline_source(real) && self.stddev_inline_source(imaginary)
            }
            _ => self.stddev_inline_source(expression),
        }
    }

    fn stddev_callback_diagnostic(
        &mut self,
        data: &Expr,
        weights: &Expr,
        observations: &[&Expr],
        weight_entries: &[&Expr],
        encoded_weights: bool,
    ) -> Option<String> {
        let entries = || observations.iter().chain(weight_entries).copied();
        if !entries().any(stddev_cell_has_call)
            || !entries().all(|entry| self.stddev_callback_source(entry))
        {
            return None;
        }
        let data = stddev_matrix_operand_source(data, true, encoded_weights, true)?;
        let weights =
            stddev_matrix_operand_source(weights, encoded_weights, encoded_weights, true)?;
        Some(stddev_bad_argument(&data, &weights))
    }

    fn stddev_callback_cell(&mut self, expression: &Expr, failure: Option<u16>) {
        let span = expression.span;
        let handler =
            stddev_cell_has_call(expression).then(|| self.cur.emit(Op::TryStddevCell(0), span));
        if let ExprKind::ComplexPair(real, imaginary) = &expression.kind {
            self.stddev_callback_cell(real, failure);
            self.stddev_callback_cell(imaginary, failure);
            self.cur.emit(Op::MakeComplex, span);
        } else {
            self.expr(expression);
        }
        self.finish_stddev_cell_guard(handler, failure, span);
    }

    fn finish_stddev_cell_guard(
        &mut self,
        handler: Option<usize>,
        failure: Option<u16>,
        span: Span,
    ) {
        if let Some(handler) = handler {
            self.cur.emit(Op::TryEnd, span);
            let end = self.cur.emit(Op::Jump(0), span);
            self.cur.patch_jump(handler, self.cur.here());
            if let Some(failure) = failure {
                self.constant(Const::Real(Real::ONE), span);
                self.cur.emit(Op::StoreLocal(failure), span);
            }
            // Keep evaluating the source rows with a numeric placeholder.
            self.cur.emit(Op::Zero, span);
            self.cur.patch_jump(end, self.cur.here());
        }
    }

    fn stddev_inline_cell(&mut self, expression: &Expr, failure: u16) {
        let span = expression.span;
        let handler = match expression.kind {
            ExprKind::Call(..) | ExprKind::ComplexPair(..) => {
                Some(self.cur.emit(Op::TryStddevCell(0), span))
            }
            _ => None,
        };
        match &expression.kind {
            ExprKind::Binary(op, left, right) => {
                self.stddev_inline_cell(left, failure);
                self.stddev_inline_cell(right, failure);
                self.cur.emit(
                    Op::StddevCellBin {
                        op: *op,
                        literal_zero_quotient: *op == BinOp::Div
                            && stddev_literal_zero_quotient(left, right),
                    },
                    span,
                );
            }
            ExprKind::Unary(UnOp::Neg, value) => {
                self.stddev_inline_cell(value, failure);
                self.cur.emit(Op::StddevCellNeg, span);
            }
            ExprKind::ComplexPair(real, imaginary) => {
                self.stddev_inline_cell(real, failure);
                self.stddev_inline_cell(imaginary, failure);
                self.cur.emit(Op::StddevCellPair, span);
            }
            ExprKind::Call(callee, args) => {
                let ExprKind::Ident(name) = &callee.kind else {
                    unreachable!("validated stddev cell callback");
                };
                let id = self.fn_map[name];
                self.check_local_call(name, id, args.len(), span);
                for arg in args {
                    self.stddev_inline_cell(arg, failure);
                }
                self.cur.emit(
                    Op::Call {
                        func: id,
                        argc: args.len().min(255) as u8,
                    },
                    span,
                );
            }
            _ => self.expr(expression),
        }
        self.finish_stddev_cell_guard(handler, Some(failure), span);
    }

    fn stddev_mixed_vector(&mut self, args: &[Expr], span: Span) -> bool {
        let [data, weights] = args else {
            return false;
        };
        let (encoded_first, entries, ordinary) =
            match (source_encoded_vector(data), source_encoded_vector(weights)) {
                (Some(entries), None) => (true, entries, weights),
                (None, Some(entries)) => (false, entries, data),
                _ => return false,
            };
        if matches!(&ordinary.kind, ExprKind::Bracket(entries)
            if entries.iter().any(|entry| matches!(entry.kind, ExprKind::Bracket(_))))
            || !entries
                .iter()
                .all(|entry| self.stddev_callback_source(entry))
        {
            return false;
        }
        let allow_pairs = matches!(ordinary.kind, ExprKind::Bracket(_) | ExprKind::List(_));
        let Some(encoded_source) = stddev_encoded_vector_source(entries, false, allow_pairs) else {
            return false;
        };
        let evaluated_list = matches!(ordinary.kind, ExprKind::List(_));
        let ordinary_source = if evaluated_list {
            String::new()
        } else {
            let Some(ordinary) = stddev_diagnostic_expression(ordinary, 0, false, false) else {
                return false;
            };
            let Ok(source) = crate::expression_source::ppl_diagnostic_expression_source(&ordinary)
            else {
                return false;
            };
            source
        };
        // The brace list is constructed before the encoded vector's cells
        // are evaluated. References and function calls keep argument order.
        let ordinary_first = evaluated_list || !encoded_first;
        if ordinary_first {
            self.expr(ordinary);
        }
        for entry in entries {
            self.stddev_callback_cell(entry, None);
            self.cur.emit(Op::Pop, entry.span);
        }
        if !ordinary_first {
            self.expr(ordinary);
        }
        self.constant(Const::Str(encoded_source), span);
        self.constant(Const::Str(ordinary_source), span);
        self.cur.emit(
            Op::StddevMixedVectorDiagnostic {
                encoded_first,
                evaluated_list,
            },
            span,
        );
        true
    }

    fn stddev_inline_vector_pair(&mut self, args: &[Expr], span: Span) -> bool {
        let [data, weights] = args else {
            return false;
        };
        let (Some(observations), Some(frequencies)) =
            (source_encoded_vector(data), source_encoded_vector(weights))
        else {
            return false;
        };
        if !observations
            .iter()
            .all(|entry| self.stddev_inline_observation(entry))
            || !frequencies
                .iter()
                .all(|entry| self.stddev_inline_source(entry))
        {
            return false;
        }
        let (Some(data_source), Some(weight_source)) = (
            stddev_encoded_vector_source(observations, true, true),
            stddev_encoded_vector_source(frequencies, true, true),
        ) else {
            return false;
        };
        self.emit_stddev_inline_collection(
            [data, weights],
            [
                &observations.iter().collect::<Vec<_>>(),
                &frequencies.iter().collect::<Vec<_>>(),
            ],
            [[0, 1, observations.len()], [0, 1, frequencies.len()]],
            true,
            stddev_bad_argument(&data_source, &weight_source),
            span,
        );
        true
    }

    fn stddev_inline_matrix(
        &mut self,
        operands: [&Expr; 2],
        entries: [&[&Expr]; 2],
        shapes: [[usize; 2]; 2],
        span: Span,
    ) -> bool {
        let [observations, weights] = entries;
        if !observations
            .iter()
            .chain(weights)
            .any(|entry| stddev_cell_has_division(entry))
            || !observations
                .iter()
                .all(|entry| self.stddev_inline_observation(entry))
            || !weights.iter().all(|entry| self.stddev_inline_source(entry))
        {
            return false;
        }
        let encoded_weights = weights
            .iter()
            .any(|entry| !matches!(entry.kind, ExprKind::Number(_)));
        let Some(data_source) =
            stddev_matrix_operand_source(operands[0], true, encoded_weights, true)
        else {
            return false;
        };
        let Some(weights_source) =
            stddev_matrix_operand_source(operands[1], encoded_weights, encoded_weights, true)
        else {
            return false;
        };
        self.emit_stddev_inline_collection(
            operands,
            entries,
            shapes.map(|[rows, cols]| [1, rows, cols]),
            encoded_weights,
            stddev_bad_argument(&data_source, &weights_source),
            span,
        );
        true
    }

    fn emit_stddev_inline_collection(
        &mut self,
        operands: [&Expr; 2],
        entries: [&[&Expr]; 2],
        shapes: [[usize; 3]; 2],
        encoded_weights: bool,
        diagnostic: String,
        span: Span,
    ) {
        let failure = self.hidden_local("stddev-cell");
        self.cur.emit(Op::Zero, span);
        self.cur.emit(Op::StoreLocal(failure), span);
        for (operand, cells) in operands.into_iter().zip(entries) {
            for entry in cells {
                self.stddev_inline_cell(entry, failure);
            }
            self.cur
                .emit(Op::MakeList(cells.len() as u32), operand.span);
        }
        self.cur.emit(Op::LoadLocal(failure), span);
        self.constant(Const::Str(diagnostic), span);
        let shapes = self.cur.add_stddev_metadata(StddevMetadata::InlineShapes(
            shapes.map(|shape| shape.map(|dimension| dimension as u32)),
        ));
        self.cur.emit(
            Op::StddevInlineCollection {
                shapes,
                encoded_weights,
            },
            span,
        );
    }

    fn weighted_collection_stddev(&mut self, args: &[Expr], span: Span) -> bool {
        let [data, weights] = args else {
            return false;
        };
        let ExprKind::Bracket(data_entries) = &data.kind else {
            return false;
        };
        let weights_kind = &weights.kind;
        let (observations, weight_entries, [data_shape, weight_shape]) =
            if let Some((cols, observations)) = stddev_matrix_cells(data_entries) {
                let ExprKind::Bracket(weight_rows) = weights_kind else {
                    return false;
                };
                let Some((weight_cols, weight_entries)) = stddev_matrix_cells(weight_rows) else {
                    return false;
                };
                if observations
                    .iter()
                    .all(|entry| matches!(entry.kind, ExprKind::Number(_)))
                {
                    return false;
                }
                (
                    observations,
                    weight_entries,
                    [
                        Some([1, data_entries.len(), cols]),
                        Some([1, weight_rows.len(), weight_cols]),
                    ],
                )
            } else {
                let encoded = data_entries
                    .iter()
                    .any(|entry| !matches!(entry.kind, ExprKind::Number(_)));
                let weight_entries = match weights_kind {
                    ExprKind::List(entries) if !encoded => entries,
                    ExprKind::Bracket(entries) if encoded => entries,
                    _ => return false,
                };
                (
                    data_entries.iter().collect(),
                    weight_entries.iter().collect(),
                    [encoded.then_some([0, 1, data_entries.len()]); 2],
                )
            };
        if observations.len() != weight_entries.len() {
            return false;
        }
        if let [
            Some([1, data_rows, data_cols]),
            Some([1, weight_rows, weight_cols]),
        ] = [data_shape, weight_shape]
            && self.stddev_inline_matrix(
                [data, weights],
                [&observations, &weight_entries],
                [[data_rows, data_cols], [weight_rows, weight_cols]],
                span,
            )
        {
            return true;
        }
        // A known nonzero weight on a nonreal cell selects the numeric
        // complex path. Zero-weight complex cells can leave an exact series.
        let complex_matrix = matches!(data_shape, Some([1, _, _]))
            && observations
                .iter()
                .zip(&weight_entries)
                .any(|(entry, weight)| {
                    matches!(&entry.kind, ExprKind::ComplexPair(_, imaginary)
                    if stddev_literal_real(imaginary).is_some_and(|value| !value.is_zero()))
                        && stddev_literal_real(weight).is_some_and(|value| !value.is_zero())
                });
        if !observations.iter().all(|entry| {
            stddev_numeric_entry(entry, false)
                || (complex_matrix
                    && matches!(&entry.kind, ExprKind::ComplexPair(real, imaginary)
                        if stddev_numeric_entry(real, false)
                            && stddev_numeric_entry(imaginary, false)))
        }) {
            let number_or_call = |entry: &Expr| {
                stddev_numeric_entry(entry, false)
                    || matches!(&entry.kind, ExprKind::Call(callee, _)
                        if matches!(callee.kind, ExprKind::Ident(_)))
            };
            if let (Some(data_shape @ [1, _, _]), Some(weight_shape @ [1, _, _])) =
                (data_shape, weight_shape)
                && weight_entries.iter().all(|entry| number_or_call(entry))
                && observations
                    .iter()
                    .any(|entry| matches!(entry.kind, ExprKind::ComplexPair(..)))
                && observations.iter().all(|entry| match &entry.kind {
                    ExprKind::ComplexPair(real, imaginary) => {
                        number_or_call(real) && number_or_call(imaginary)
                    }
                    _ => number_or_call(entry),
                })
            {
                let encoded_weights = weight_entries
                    .iter()
                    .any(|entry| !matches!(entry.kind, ExprKind::Number(_)));
                if let Some(diagnostic) = self.stddev_callback_diagnostic(
                    data,
                    weights,
                    &observations,
                    &weight_entries,
                    encoded_weights,
                ) {
                    let failure = self.hidden_local("stddev-cell");
                    self.cur.emit(Op::Zero, span);
                    self.cur.emit(Op::StoreLocal(failure), span);
                    for (operand, [_, rows, cols], entries) in [
                        (data, data_shape, observations),
                        (weights, weight_shape, weight_entries),
                    ] {
                        for entry in entries {
                            self.stddev_callback_cell(entry, Some(failure));
                        }
                        self.cur.emit(
                            Op::MakeMatrix {
                                rows: rows as u32,
                                cols: cols as u32,
                            },
                            operand.span,
                        );
                    }
                    let success = encoded_weights.then(|| {
                        self.cur.emit(Op::LoadLocal(failure), span);
                        self.cur.emit(Op::JumpIfFalse(0), span)
                    });
                    // Ordinary weights also select the mixed-operand diagnostic
                    // after the source cells execute.
                    self.constant(Const::Str(diagnostic), span);
                    self.cur.emit(Op::StddevMatrixDiagnostic, span);
                    if let Some(success) = success {
                        let end = self.cur.emit(Op::Jump(0), span);
                        self.cur.patch_jump(success, self.cur.here());
                        self.cur.emit(Op::StddevEncodedComplexMatrix, span);
                        self.cur.patch_jump(end, self.cur.here());
                    }
                    return true;
                }
                if encoded_weights {
                    for operand in args {
                        self.expr(operand);
                    }
                    self.cur.emit(Op::StddevEncodedComplexMatrix, span);
                    return true;
                }
            }
            return false;
        }
        // The encoded matrices pair their row-major cells and each operand's
        // own shape fields as weighted observations.
        let same_shape = data_shape == weight_shape;
        let try_exact_matrix = matches!(data_shape, Some([1, _, _]))
            && weight_entries
                .iter()
                .any(|entry| !matches!(entry.kind, ExprKind::Number(_)))
            && weight_entries
                .iter()
                .all(|entry| stddev_numeric_entry(entry, same_shape));
        if (!same_shape || complex_matrix) && !try_exact_matrix {
            return false;
        }
        if !try_exact_matrix {
            let Some(frequencies) = weight_entries
                .iter()
                .map(|entry| stddev_literal_real(entry))
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            if !frequencies.iter().any(|value| value.is_negative()) {
                return false;
            }
            let shape_weight = weight_shape.map_or(0, |shape| shape.into_iter().sum::<usize>());
            let fractional = frequencies.iter().any(|value| !value.is_integer());
            let zero_total = frequencies
                .iter()
                .try_fold(shape_weight as i128, |total, value| {
                    total.checked_add(value.to_i128()?)
                })
                == Some(0);
            if !fractional && !zero_total {
                return false;
            }
        }

        for (operand, entries, shape) in [
            (data, observations, data_shape),
            (weights, weight_entries, weight_shape),
        ] {
            if let Some(shape) = shape {
                for dimension in shape {
                    self.constant(Const::Real(Real::from_u64(dimension as u64)), operand.span);
                }
            }
            let count = entries.len() + shape.map_or(0, |fields| fields.len());
            for entry in entries {
                self.expr(entry);
            }
            self.cur.emit(Op::MakeList(count as u32), operand.span);
        }
        let op = if try_exact_matrix {
            Op::StddevExact {
                argc: 2,
                encoded_vector: false,
                evaluated_matrices: false,
                row_mismatch_diagnostic: false,
                list_sources: None,
            }
        } else {
            Op::Builtin {
                id: Builtin::Stddev,
                argc: 2,
            }
        };
        self.cur.emit(op, span);
        true
    }

    fn unsupported(&mut self, name: &str, argc: usize, span: Span) {
        let idx = match self.names.iter().position(|n| n == name) {
            Some(i) => i,
            None => {
                self.names.push(name.to_string());
                self.names.len() - 1
            }
        };
        self.cur.emit(
            Op::Unsupported {
                name: idx as u32,
                argc: argc.min(255) as u8,
            },
            span,
        );
    }

    /// Graphics overloads choose an optional z-string at runtime. Preserve
    /// candidate variable arguments without mistaking a coordinate/color for
    /// a mandatory reference. Identity tokens keep aliases coherent on writeback.
    fn graphics_builtin(&mut self, id: Builtin, args: &[Expr], span: Span) {
        let mut positions = vec![args.len().saturating_sub(1)];
        if matches!(id, Builtin::TriangleP | Builtin::Triangle) && args.len() >= 4 {
            positions.push(args.len() - 4);
        }
        positions.sort_unstable();
        positions.dedup();
        let mut targets = Vec::new();
        for position in positions {
            if let Some(expr) = args.get(position)
                && let ExprKind::Ident(name) = &expr.kind
                && let Some(target) = self.resolve_var(name)
                && !matches!(target, VarRef::Global(slot) if self.globals[slot as usize].read_only)
            {
                targets.push((position, target, expr.span));
            }
        }
        if targets.is_empty() {
            self.builtin(id, args, span);
            return;
        }
        for argument in args {
            self.expr(argument);
        }
        for &(position, target, at) in &targets {
            self.constant(Const::Real(Real::from_i64(position as i64)), at);
            self.constant(
                Const::Str(match target {
                    VarRef::Local(slot) => format!("L{slot}"),
                    VarRef::Global(slot) => format!("G{slot}"),
                    VarRef::App(slot) => format!("A:{}", self.names[slot as usize]),
                }),
                at,
            );
            self.cur.emit(Op::MakeList(2), at);
        }
        self.cur.emit(Op::MakeList(targets.len() as u32), span);
        self.cur.emit(
            Op::BuiltinRef {
                id,
                argc: (args.len() + 1).min(255) as u8,
                refs: targets.len() as u8,
            },
            span,
        );
        for &(_, target, at) in targets.iter().rev() {
            self.store(target, at);
        }
    }

    /// Commands whose first argument is a variable they write to.
    fn ref_builtin(&mut self, id: Builtin, args: &[Expr], span: Span) {
        let Some(first) = args.first() else {
            self.builtin(id, args, span);
            return;
        };
        let resolve = |c: &mut Compiler, e: &Expr| -> Option<(VarRef, Span)> {
            match &e.kind {
                ExprKind::Ident(n) => match c.resolve_var(n) {
                    Some(v) => Some((v, e.span)),
                    None => {
                        c.error(
                            "undeclared-identifier",
                            e.span,
                            format!("undeclared variable `{n}`"),
                        )
                        .hint = Some(format!("declare it first: LOCAL {n};"));
                        None
                    }
                },
                _ => {
                    c.error(
                        "expected-variable",
                        e.span,
                        "this argument must be a variable name",
                    );
                    None
                }
            }
        };
        let mut targets: Vec<(VarRef, Span)> = Vec::new();
        // INPUT only: each field's allowed-types expression (`[2]` in
        // `{{A,[2]}}`), aligned with `targets`; `None` is an omitted spec.
        let mut types: Vec<Option<&Expr>> = Vec::new();
        let mut positions: Vec<Option<&Expr>> = Vec::new();
        match &first.kind {
            ExprKind::List(items) if id == Builtin::Input => {
                for it in items {
                    match &it.kind {
                        // {{A,[0]}}: variable plus allowed-type list.
                        ExprKind::List(inner) if !inner.is_empty() => {
                            if let Some(t) = resolve(self, &inner[0]) {
                                targets.push(t);
                                types.push(inner.get(1));
                                positions.push(inner.get(2));
                            }
                            if let Some(t) = inner.get(1) {
                                self.check_input_types(t);
                            }
                            if inner.len() > 3 {
                                self.error(
                                    "input-field-arity",
                                    inner[3].span.to(inner[inner.len() - 1].span),
                                    "INPUT fields accept a variable, type specification and optional position",
                                );
                            }
                        }
                        _ => {
                            if let Some(t) = resolve(self, it) {
                                targets.push(t);
                                types.push(None);
                                positions.push(None);
                            }
                        }
                    }
                }
            }
            _ => {
                if let Some(t) = resolve(self, first) {
                    targets.push(t);
                    types.push(None);
                    positions.push(None);
                }
            }
        }
        for (v, s) in &targets {
            self.load(*v, *s);
        }
        self.cur
            .emit(Op::MakeList(targets.len() as u32), first.span);
        let mut argc = args.len();
        if id == Builtin::Input {
            // INPUT receives optional descriptors as a hidden second argument.
            // Preserve omission separately from explicit scalar controls: zero
            // is a valid descriptor value, not an unrestricted edit field.
            for t in &types {
                match t {
                    Some(e) => {
                        self.expr(e);
                        self.cur.emit(Op::MakeList(1), e.span);
                    }
                    None => {
                        self.cur.emit(Op::MakeList(0), first.span);
                    }
                }
            }
            self.cur.emit(Op::MakeList(types.len() as u32), first.span);
            // Preserve scalar versus list syntax: a list-valued scalar initial
            // value must not be confused with a list of field defaults.
            self.constant(
                Const::Real(if matches!(first.kind, ExprKind::List(_)) {
                    Real::ZERO
                } else {
                    Real::ONE
                }),
                first.span,
            );
            // Preserve position expressions rather than silently discarding them.
            for position in &positions {
                match position {
                    Some(e) => {
                        self.expr(e);
                        self.cur.emit(Op::MakeList(1), e.span);
                    }
                    None => {
                        self.cur.emit(Op::MakeList(0), first.span);
                    }
                }
            }
            self.cur
                .emit(Op::MakeList(positions.len() as u32), first.span);
            argc += 3;
        }
        for a in &args[1..] {
            self.expr(a);
        }
        if id == Builtin::Editmat && args.len() == 1 {
            if let ExprKind::Ident(name) = &first.kind {
                self.constant(Const::Str(name.clone()), first.span);
                argc += 1;
            }
        }
        let refs = targets.len().min(255) as u8;
        self.cur.emit(
            Op::BuiltinRef {
                id,
                argc: argc.min(255) as u8,
                refs,
            },
            span,
        );
        // An app assignment has observable effects (for example freezing a
        // worksheet formula). Cancelling INPUT/CHOOSE must not write even an
        // unchanged value back into that app. Keep the original result below
        // the updates and discard all updates on cancellation.
        let cancelled = targets
            .iter()
            .any(|(v, _)| matches!(v, VarRef::App(_)))
            .then(|| {
                self.cur.emit(Op::DupFrom(refs), span);
                self.cur.emit(Op::JumpIfFalse(0), span)
            });
        // Stack: result, new values…; store them back in reverse order.
        for (v, s) in targets.iter().rev() {
            self.store(*v, *s);
        }
        if let Some(cancelled) = cancelled {
            let end = self.cur.emit(Op::Jump(0), span);
            self.cur.patch_jump(cancelled, self.cur.here());
            for _ in 0..refs {
                self.cur.emit(Op::Pop, span);
            }
            self.cur.patch_jump(end, self.cur.here());
        }
    }

    /// The installed G1 accepts integral filters -1..9 (including 5 and 7),
    /// and rejects fractional/14.x descriptors. Other builds need controls.
    fn check_input_types(&mut self, e: &Expr) {
        let ExprKind::Bracket(items) = &e.kind else {
            return;
        };
        for it in items {
            let code = match &it.kind {
                ExprKind::Number(n) => n.parse::<f64>().ok(),
                ExprKind::Unary(UnOp::Neg, inner) => match &inner.kind {
                    ExprKind::Number(n) => n.parse::<f64>().ok().map(|v| -v),
                    _ => None,
                },
                _ => None,
            };
            let Some(code) = code else { continue };
            if !(-1.0..=9.0).contains(&code) || code.fract() != 0.0 {
                let d = self.warn(
                    "input-unknown-type",
                    it.span,
                    format!(
                        "{code} is not an INPUT type code accepted by the installed G1 profile"
                    ),
                );
                d.hint = Some("use -1 (any) or an integral G1 TYPE filter from 0 through 9".into());
                d.unverified = true;
            }
        }
    }

    /// `MAKELIST(expr, var, start, end[, step])` evaluates `expr` once per
    /// value of `var`, so it compiles to a loop.
    fn makelist(&mut self, args: &[Expr], span: Span) {
        let ExprKind::Ident(var_name) = &args[1].kind else {
            self.error(
                "expected-variable",
                args[1].span,
                "MAKELIST's second argument must be a variable name",
            );
            self.cur.emit(Op::Zero, span);
            return;
        };
        self.scopes.push(HashMap::new());
        let v = match self.resolve_var(var_name) {
            Some(v) => v,
            // An undeclared name acts as a temporary bound variable.
            None => VarRef::Local(self.new_local(var_name, args[1].span, true)),
        };
        let saved = self.hidden_local("saved");
        let acc = self.hidden_local("acc");
        let end = self.hidden_local("end");
        let step = self.hidden_local("step");
        self.load(v, span);
        self.cur.emit(Op::StoreLocal(saved), span);
        self.cur.emit(Op::MakeList(0), span);
        self.cur.emit(Op::StoreLocal(acc), span);
        self.expr(&args[2]);
        self.store(v, args[1].span);
        self.expr(&args[3]);
        self.cur.emit(Op::StoreLocal(end), span);
        match args.get(4) {
            Some(s) => self.expr(s),
            None => self.constant(Const::Real(Real::ONE), span),
        }
        self.cur.emit(Op::StoreLocal(step), span);
        let top = self.cur.here();
        self.load(v, span);
        self.cur.emit(Op::LoadLocal(end), span);
        self.cur.emit(Op::ForTest { down: false }, span);
        let jf = self.cur.emit(Op::JumpIfFalse(0), span);
        self.expr(&args[0]);
        self.cur.emit(Op::AppendLocal(acc), span);
        self.load(v, span);
        self.cur.emit(Op::LoadLocal(step), span);
        self.cur.emit(Op::Bin(BinOp::Add), span);
        self.store(v, args[1].span);
        self.cur.emit(Op::Jump(top), span);
        let here = self.cur.here();
        self.cur.patch_jump(jf, here);
        self.cur.emit(Op::LoadLocal(saved), span);
        self.store(v, args[1].span);
        self.cur.emit(Op::LoadLocal(acc), span);
        self.end_scope();
    }
}

#[cfg(test)]
mod div_tests {
    //! DIV is the integer-quotient operator; real literals as operands are
    //! a compile-time syntax error on the G1 (hardware-verified 2026-10-23:
    //! `3.5 DIV 1E-11` reported "Syntax error" at its line when the
    //! program was run, while integer DIV from the verified core suite
    //! compiles).

    use crate::*;

    fn has_error(src: &str) -> bool {
        let file = ppl_syntax::SourceFile::new("<t>", src.to_string());
        let (_, d) = compile_source(&file, &CompileOptions::default());
        d.errors().any(|e| e.code == "div-requires-integers")
    }

    #[test]
    fn real_literal_div_is_rejected() {
        assert!(has_error("EXPORT T() BEGIN RETURN 3.5 DIV 1E-11; END;"));
        assert!(has_error("EXPORT T() BEGIN RETURN 3 DIV 2.5; END;"));
    }

    #[test]
    fn integer_div_is_accepted() {
        assert!(!has_error("EXPORT T() BEGIN RETURN 7 DIV -3; END;"));
        assert!(!has_error("EXPORT T() BEGIN RETURN -42 DIV 7; END;"));
    }
}
