//! Shared workload generators for the `ppl-bench` criterion benchmarks.
//!
//! Every PPL workload here is checked by the unit tests at the bottom of
//! this file, so the benchmarks always measure programs that compile and
//! produce the expected result.

use ppl_compiler::{CompileOptions, CompiledProgram, compile_source};
use ppl_graphics::{Grob, WHITE, rgb};
use ppl_syntax::SourceFile;
use ppl_vm::{HeadlessHost, Limits, Machine, Settings, Value};
use std::path::PathBuf;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Front-end inputs
// ---------------------------------------------------------------------------

/// Path to `examples/FIREWORKS.hpprgm` in the workspace.
pub fn fireworks_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/FIREWORKS.hpprgm")
}

/// Loads `examples/FIREWORKS.hpprgm` through the `ppl-syntax` source loader
/// (which handles UTF-8, UTF-16 and container headers).
pub fn fireworks_source() -> SourceFile {
    let (file, _enc) = ppl_syntax::read_source_file(&fireworks_path())
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", fireworks_path().display()));
    file
}

/// Default size of the synthetic front-end input.
pub const SYNTHETIC_BYTES: usize = 100 * 1024;

fn synthetic_function(i: usize) -> String {
    format!(
        r#"// synthetic function {i}: loops, branches, literals and calls
ZZS_F{i:05}(a, b)
BEGIN
  LOCAL k, s := 0;
  LOCAL h := #1Fh;
  LOCAL lst := {{1, 2.5, -3, "txt{i}"}};
  LOCAL mat := [[1, 2], [3, 4]];
  FOR k FROM 1 TO a STEP 1 DO
    IF k MOD 3 == 0 AND b > 0 THEN
      s := s + k * 1.5E-3 / (b + 1);
    ELSE
      s := s - SQRT(ABS(k - b)) + 0.25;
    END;
  END;
  WHILE s > 1000 DO
    s := s / 2;
  END;
  REPEAT
    s := s + 1;
  UNTIL s >= 0;
  lst(1) := s;
  RETURN s + SIZE(lst) + mat(2, 1);
END;

"#
    )
}

/// Generates a valid PPL program of at least `target_bytes` bytes made of
/// many small functions plus an exported `ZZS_MAIN()` entry point.
pub fn synthetic_source_text(target_bytes: usize) -> String {
    let main = "EXPORT ZZS_MAIN()\nBEGIN\n  RETURN ZZS_F00000(10, 2);\nEND;\n";
    let mut out = String::with_capacity(target_bytes + 1024);
    let mut i = 0;
    while out.len() + main.len() < target_bytes || i == 0 {
        out.push_str(&synthetic_function(i));
        i += 1;
    }
    out.push_str(main);
    out
}

/// The synthetic ~100 KB source as a [`SourceFile`].
pub fn synthetic_source() -> SourceFile {
    SourceFile::new("<synthetic>", synthetic_source_text(SYNTHETIC_BYTES))
}

/// Both front-end inputs, labelled for benchmark ids.
pub fn frontend_inputs() -> Vec<(&'static str, SourceFile)> {
    vec![
        ("fireworks", fireworks_source()),
        ("synthetic_100k", synthetic_source()),
    ]
}

// ---------------------------------------------------------------------------
// Compile / run helpers
// ---------------------------------------------------------------------------

/// Limits with no step or wall-clock budget, so benches never time out.
pub fn bench_limits() -> Limits {
    Limits {
        max_steps: None,
        max_time: None,
        ..Limits::default()
    }
}

/// Compiles `src`, panicking with the first diagnostic on failure.
pub fn compile_program(name: &str, src: &str) -> Rc<CompiledProgram> {
    let file = SourceFile::new(name, src.to_string());
    let (prog, diags) = compile_source(&file, &CompileOptions::default());
    if let Some(err) = diags.errors().next() {
        panic!("{name}: compile error: {}", err.message);
    }
    Rc::new(prog.unwrap_or_else(|| panic!("{name}: no program produced")))
}

/// A fresh headless machine with [`bench_limits`].
pub fn new_machine(prog: &Rc<CompiledProgram>) -> Machine {
    Machine::new(
        Rc::clone(prog),
        Box::new(HeadlessHost::new()),
        Settings::default(),
        bench_limits(),
    )
}

/// One VM workload: a self-contained program, its entry point and an
/// optional setup function that is run once before measuring.
#[derive(Clone, Copy, Debug)]
pub struct Workload {
    pub name: &'static str,
    pub source: &'static str,
    pub entry: &'static str,
    pub setup: Option<&'static str>,
}

impl Workload {
    /// Compiles the program, runs `setup` (if any) and returns a machine
    /// ready for repeated `call_function(entry)` calls.
    pub fn prepare(&self) -> Machine {
        let prog = compile_program(self.name, self.source);
        let mut m = new_machine(&prog);
        if let Some(setup) = self.setup {
            m.call_function(setup, Vec::new())
                .unwrap_or_else(|e| panic!("{}: setup failed: {e}", self.name));
        }
        m
    }

    /// Runs the entry point once on `m`.
    pub fn run(&self, m: &mut Machine) -> Value {
        m.call_function(self.entry, Vec::new())
            .unwrap_or_else(|e| panic!("{}: runtime error: {e}", self.name))
    }
}

/// Formats a value the way the calculator would display it.
pub fn display(m: &Machine, v: &Value) -> String {
    v.display(m.settings.format())
}

// ---------------------------------------------------------------------------
// VM workloads
// ---------------------------------------------------------------------------

/// Number of iterations in [`FOR_SUM`].
pub const FOR_SUM_N: u64 = 100_000;

/// 100 000-iteration FOR loop accumulating a sum.
pub const FOR_SUM: Workload = Workload {
    name: "for_sum_100k",
    source: r#"
EXPORT ZZB_LOOP()
BEGIN
  LOCAL i;
  LOCAL s := 0;
  FOR i FROM 1 TO 100000 DO
    s := s + i;
  END;
  RETURN s;
END;
"#,
    entry: "ZZB_LOOP",
    setup: None,
};

/// Argument of the recursive Fibonacci workload.
pub const FIB_N: u32 = 20;

/// Number of `ZZB_FIBR` calls made by fib([`FIB_N`]) (2·fib(n+1) − 1).
pub const FIB_CALLS: u64 = 21_891;

/// Recursive fib(20): [`FIB_CALLS`] user-function calls.
pub const FIB: Workload = Workload {
    name: "fib_20",
    source: r#"
ZZB_FIBR(n)
BEGIN
  IF n < 2 THEN
    RETURN n;
  END;
  RETURN ZZB_FIBR(n - 1) + ZZB_FIBR(n - 2);
END;

EXPORT ZZB_FIB()
BEGIN
  RETURN ZZB_FIBR(20);
END;
"#,
    entry: "ZZB_FIB",
    setup: None,
};

/// Iterations of the Real arithmetic chain.
pub const REAL_CHAIN_N: u64 = 10_000;

/// Real (12-digit BCD) mul / div / sqrt chain: Σ √i·1.37/3.11·2.5.
///
/// `SQRT` is applied to the integer loop counter on purpose: at the time of
/// writing `Real::sqrt` overflows its `u128` intermediate for 12-digit
/// mantissas with an odd exponent (e.g. `SQRT(5.12345678901)`), panicking in
/// debug builds and returning a wrong value in release builds. See the
/// ignored `sqrt_full_mantissa_odd_exponent` test below.
pub const REAL_CHAIN: Workload = Workload {
    name: "real_chain_10k",
    source: r#"
EXPORT ZZB_REAL()
BEGIN
  LOCAL i;
  LOCAL s := 0;
  FOR i FROM 1 TO 10000 DO
    s := s + SQRT(i) * 1.37 / 3.11 * 2.5;
  END;
  RETURN s;
END;
"#,
    entry: "ZZB_REAL",
    setup: None,
};

/// All VM workloads.
pub const VM_WORKLOADS: [Workload; 3] = [FOR_SUM, FIB, REAL_CHAIN];

// ---------------------------------------------------------------------------
// Collection workloads
// ---------------------------------------------------------------------------

/// Length of the MAKELIST and append workloads.
pub const LIST_N: u64 = 10_000;

/// MAKELIST of 10 000 items; returns the last element (20 000).
pub const MAKELIST_10K: Workload = Workload {
    name: "makelist_10k",
    source: r#"
EXPORT ZZB_MKL()
BEGIN
  LOCAL k;
  LOCAL l := MAKELIST(k * 2, k, 1, 10000);
  RETURN l(SIZE(l));
END;
"#,
    entry: "ZZB_MKL",
    setup: None,
};

/// 10 000 `L(0) := i` appends; returns SIZE(L) + L(SIZE(L)) (20 000).
pub const LIST_APPEND_10K: Workload = Workload {
    name: "list_append_10k",
    source: r#"
EXPORT ZZB_APPEND()
BEGIN
  LOCAL i;
  LOCAL l := {};
  FOR i FROM 1 TO 10000 DO
    l(0) := i;
  END;
  RETURN SIZE(l) + l(SIZE(l));
END;
"#,
    entry: "ZZB_APPEND",
    setup: None,
};

/// Matrix dimension for the matrix workloads.
pub const MAT_N: usize = 20;

const MATRIX_SOURCE: &str = r#"
LOCAL zzm_a, zzm_b, zzm_d;

EXPORT ZZB_MATSETUP()
BEGIN
  LOCAL i, j;
  zzm_a := MAKEMAT(0, 20, 20);
  zzm_b := MAKEMAT(0, 20, 20);
  zzm_d := MAKEMAT(0, 20, 20);
  FOR i FROM 1 TO 20 DO
    FOR j FROM 1 TO 20 DO
      zzm_a(i, j) := i + j;
      zzm_b(i, j) := i - j;
      zzm_d(i, j) := (i == j) * 40 + 1 / (i + j);
    END;
  END;
  RETURN 0;
END;

EXPORT ZZB_MATMUL()
BEGIN
  LOCAL c := zzm_a * zzm_b;
  RETURN c(20, 20);
END;

EXPORT ZZB_DET()
BEGIN
  RETURN DET(zzm_d);
END;
"#;

/// 20×20 matrix multiply (matrices built once in setup); returns C(20,20).
pub const MATMUL_20: Workload = Workload {
    name: "matmul_20x20",
    source: MATRIX_SOURCE,
    entry: "ZZB_MATMUL",
    setup: Some("ZZB_MATSETUP"),
};

/// DET of a 20×20 diagonally dominant matrix (built once in setup).
pub const DET_20: Workload = Workload {
    name: "det_20x20",
    source: MATRIX_SOURCE,
    entry: "ZZB_DET",
    setup: Some("ZZB_MATSETUP"),
};

/// All collection workloads.
pub const COLLECTION_WORKLOADS: [Workload; 4] = [MAKELIST_10K, LIST_APPEND_10K, MATMUL_20, DET_20];

// ---------------------------------------------------------------------------
// Graphics workloads
// ---------------------------------------------------------------------------

/// Number of primitives per graphics batch.
pub const GFX_BATCH: usize = 200;

/// 200 LINE_P calls across the screen.
pub const PPL_LINES: Workload = Workload {
    name: "ppl_line_p_x200",
    source: r#"
EXPORT ZZB_LINES()
BEGIN
  LOCAL i;
  FOR i FROM 0 TO 199 DO
    LINE_P(i MOD 320, 0, 319 - (i MOD 320), 239, RGB(i, 0, 255 - i));
  END;
  RETURN 0;
END;
"#,
    entry: "ZZB_LINES",
    setup: None,
};

/// 200 FILLCIRCLE_P calls (radius 4..23).
pub const PPL_FILLCIRCLES: Workload = Workload {
    name: "ppl_fillcircle_p_x200",
    source: r#"
EXPORT ZZB_CIRCLES()
BEGIN
  LOCAL i;
  FOR i FROM 0 TO 199 DO
    FILLCIRCLE_P((i * 37) MOD 320, (i * 23) MOD 240, 4 + (i MOD 20), RGB(255 - i, i, 128));
  END;
  RETURN 0;
END;
"#,
    entry: "ZZB_CIRCLES",
    setup: None,
};

/// 200 TEXTOUT_P calls.
pub const PPL_TEXTOUT: Workload = Workload {
    name: "ppl_textout_p_x200",
    source: r#"
EXPORT ZZB_TEXT()
BEGIN
  LOCAL i;
  FOR i FROM 0 TO 199 DO
    TEXTOUT_P("Hello PPL " + i, (i * 13) MOD 280, (i * 11) MOD 228, 2, RGB(0, 0, i));
  END;
  RETURN 0;
END;
"#,
    entry: "ZZB_TEXT",
    setup: None,
};

/// All PPL graphics workloads.
pub const GRAPHICS_WORKLOADS: [Workload; 3] = [PPL_LINES, PPL_FILLCIRCLES, PPL_TEXTOUT];

/// Direct `Grob::line` batch matching [`PPL_LINES`].
pub fn grob_lines(g: &mut Grob) {
    for i in 0..GFX_BATCH as i64 {
        let x = i % 320;
        g.line(x, 0, 319 - x, 239, rgb(i as u8, 0, 255 - i as u8));
    }
}

/// Direct `Grob::fill_circle` batch matching [`PPL_FILLCIRCLES`].
pub fn grob_fill_circles(g: &mut Grob) {
    for i in 0..GFX_BATCH as i64 {
        g.fill_circle(
            (i * 37) % 320,
            (i * 23) % 240,
            4 + i % 20,
            rgb(255 - i as u8, i as u8, 128),
        );
    }
}

/// Direct `text::draw_text` batch matching [`PPL_TEXTOUT`].
pub fn grob_text(g: &mut Grob) {
    let px = ppl_graphics::text::font_px(2, ppl_graphics::text::DEFAULT_PX);
    for i in 0..GFX_BATCH as i64 {
        let s = format!("Hello PPL {i}");
        ppl_graphics::text::draw_text(
            g,
            &s,
            (i * 13) % 280,
            (i * 11) % 228,
            px,
            rgb(0, 0, i as u8),
            None,
            None,
        );
    }
}

/// Number of non-white pixels in `g` (used to check drawing happened).
pub fn ink(g: &Grob) -> usize {
    g.pixels().iter().filter(|&&p| p != WHITE).count()
}

/// A deterministic, non-uniform 320×240 frame for blit benchmarks.
pub fn test_frame() -> Grob {
    let mut g = Grob::screen();
    grob_fill_circles(&mut g);
    grob_lines(&mut g);
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(w: &Workload) -> (Machine, Value) {
        let mut m = w.prepare();
        let v = w.run(&mut m);
        (m, v)
    }

    fn real(v: &Value) -> f64 {
        v.as_real().expect("real result").to_f64()
    }

    #[test]
    fn fireworks_loads_and_compiles() {
        let f = fireworks_source();
        assert!(f.text.contains("FIREWORKS"));
        compile_program("fireworks", &f.text);
    }

    #[test]
    fn synthetic_source_compiles_and_runs() {
        let text = synthetic_source_text(SYNTHETIC_BYTES);
        assert!(text.len() >= SYNTHETIC_BYTES);
        assert!(text.len() < SYNTHETIC_BYTES + 2048);
        let prog = compile_program("synthetic", &text);
        let mut m = new_machine(&prog);
        let v = m.call_function("ZZS_MAIN", Vec::new()).unwrap();
        assert!(v.as_real().is_some());
    }

    #[test]
    fn for_sum() {
        let (m, v) = run(&FOR_SUM);
        let n = FOR_SUM_N;
        assert_eq!(display(&m, &v), (n * (n + 1) / 2).to_string());
    }

    #[test]
    fn fib() {
        fn f(n: u32) -> u64 {
            if n < 2 { n as u64 } else { f(n - 1) + f(n - 2) }
        }
        let (m, v) = run(&FIB);
        assert_eq!(display(&m, &v), f(FIB_N).to_string());
        assert_eq!(FIB_CALLS, 2 * f(FIB_N + 1) - 1);
    }

    #[test]
    fn real_chain() {
        let (_, v) = run(&REAL_CHAIN);
        let expect: f64 = (1..=REAL_CHAIN_N)
            .map(|i| (i as f64).sqrt() * 1.37 / 3.11 * 2.5)
            .sum();
        let got = real(&v);
        assert!(
            ((got - expect) / expect).abs() < 1e-9,
            "got {got}, expected {expect}"
        );
    }

    /// Regression check for the `Real::sqrt` u128 overflow (ppl-math).
    #[test]
    fn sqrt_full_mantissa_odd_exponent() {
        let prog = compile_program(
            "sqrt",
            "EXPORT ZZB_SQ()\nBEGIN\n  RETURN SQRT(5.12345678901);\nEND;\n",
        );
        let mut m = new_machine(&prog);
        let v = m.call_function("ZZB_SQ", Vec::new()).unwrap();
        let got = real(&v);
        assert!((got - 5.12345678901f64.sqrt()).abs() < 1e-10, "got {got}");
    }

    #[test]
    fn makelist() {
        let (m, v) = run(&MAKELIST_10K);
        assert_eq!(display(&m, &v), (LIST_N * 2).to_string());
    }

    #[test]
    fn list_append() {
        let (m, v) = run(&LIST_APPEND_10K);
        assert_eq!(display(&m, &v), (LIST_N * 2).to_string());
    }

    #[test]
    fn matmul() {
        let (m, v) = run(&MATMUL_20);
        let n = MAT_N as i64;
        let expect: i64 = (1..=n).map(|k| (n + k) * (k - n)).sum();
        assert_eq!(display(&m, &v), expect.to_string());
        // repeated calls give the same answer (machine reuse in benches)
        let mut m = m;
        let again = MATMUL_20.run(&mut m);
        assert_eq!(display(&m, &again), expect.to_string());
    }

    #[test]
    fn det() {
        let n = MAT_N;
        let mut a: Vec<Vec<f64>> = (1..=n)
            .map(|i| {
                (1..=n)
                    .map(|j| if i == j { 40.0 } else { 0.0 } + 1.0 / (i + j) as f64)
                    .collect()
            })
            .collect();
        let mut expect = 1.0;
        for c in 0..n {
            expect *= a[c][c];
            let pivot = a[c].clone();
            for row in a.iter_mut().skip(c + 1) {
                let f = row[c] / pivot[c];
                for (x, p) in row.iter_mut().zip(&pivot).skip(c) {
                    *x -= f * p;
                }
            }
        }
        let (_, v) = run(&DET_20);
        let got = real(&v);
        assert!(
            ((got - expect) / expect).abs() < 1e-9,
            "got {got}, expected {expect}"
        );
    }

    #[test]
    fn ppl_graphics_draw() {
        for w in GRAPHICS_WORKLOADS {
            let (m, v) = run(&w);
            assert_eq!(real(&v), 0.0, "{}", w.name);
            assert!(ink(m.screen()) > 1000, "{} drew too little", w.name);
        }
    }

    #[test]
    fn grob_direct_draw() {
        for f in [grob_lines, grob_fill_circles, grob_text] {
            let mut g = Grob::screen();
            f(&mut g);
            assert!(ink(&g) > 1000);
        }
    }
}
