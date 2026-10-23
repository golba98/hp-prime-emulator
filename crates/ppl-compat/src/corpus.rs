//! Deterministic, seeded generation of PPL compatibility cases.
//!
//! [`generate_corpus`] turns a seed and a case count into a [`Suite`]: every
//! case is built from a category template over deterministic operand pools
//! and is **validated** — it must compile through the real compiler
//! ([`crate::audit::audit_case`]) and be safe for a device program (pure, no
//! undeclared side effects; error-risk categories are generated `isolate`).
//! Cases that fail validation are dropped and counted, never shipped.
//!
//! The suite's groups are **batches**: at most [`crate::MAX_CASES_PER_PROGRAM`]
//! non-isolated cases of one category per group, so one group = one device
//! program (`ZZT_…` note channel); every isolated case gets its own group.
//! The campaign orchestrator walks the groups.
//!
//! Reproducibility: the same `(count, seed, categories)` always yields the
//! same suite, byte for byte. [`GeneratedCorpus::sha256`] is the SHA-256 of
//! the serialized TOML.

use serde::Serialize;

use crate::audit::audit_case;
use crate::sha256::sha256_hex;
use crate::{Case, Group, Suite};

/// Every category name, in generation order.
pub const CATEGORIES: &[&str] = &[
    "num_basic",
    "num_precision",
    "types_format",
    "control_flow",
    "scoping",
    "strings",
    "lists",
    "matrices",
    "complex",
    "errors",
];

/// Deterministic xorshift64* generator. Not for cryptography; it exists so
/// no `rand` dependency is needed and every platform reproduces the exact
/// same sequence.
#[derive(Clone, Debug)]
pub struct SeedRng(u64);

impl SeedRng {
    pub fn new(seed: u64) -> SeedRng {
        SeedRng(seed.wrapping_mul(0x2545F4914F6CDD1D).max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform integer in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i64
    }

    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[(self.next_u64() % v.len() as u64) as usize]
    }

    pub fn chance(&mut self, pct: u64) -> bool {
        self.next_u64() % 100 < pct
    }
}

/// Result of [`generate_corpus`].
#[derive(Clone, Debug, Serialize)]
pub struct GeneratedCorpus {
    pub suite: Suite,
    pub seed: u64,
    pub count: usize,
    /// Cases dropped because they did not compile or were unsafe.
    pub rejected: usize,
    /// SHA-256 of the TOML returned by [`GeneratedCorpus::toml`].
    pub sha256: String,
    /// Cases per category (accepted).
    pub per_category: Vec<(String, usize)>,
}

impl GeneratedCorpus {
    /// Serializes the suite as TOML, reproducible for the same seed.
    pub fn toml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Generated PPL compatibility corpus (ppl test generate).\n\
             # Deterministic: seed {seed}, count {count}, rejected {rejected}.\n\n",
            seed = self.seed,
            count = self.count,
            rejected = self.rejected
        ));
        out.push_str(&format!(
            "suite = \"{}\"\n",
            self.suite.suite.replace('"', "")
        ));
        out.push_str("description = \"");
        out.push_str(
            &self
                .suite
                .description
                .replace('\\', "\\\\")
                .replace('"', "\\\""),
        );
        out.push_str("\"\n\n");
        for g in &self.suite.groups {
            out.push_str("[[group]]\n");
            out.push_str(&format!("id = \"{}\"\n", g.id));
            out.push_str("description = \"\"\n\n");
        }
        for c in &self.suite.cases {
            out.push_str("[[case]]\n");
            out.push_str(&format!("id = \"{}\"\n", c.id));
            out.push_str(&format!(
                "group = \"{}\"\n",
                c.group.as_deref().unwrap_or("")
            ));
            if let Some(e) = &c.expr {
                out.push_str(&format!("expr = \"{}\"\n", escape_toml(e)));
            }
            if let Some(b) = &c.body {
                out.push_str(&format!("body = \"\"\"{}\n\"\"\"\n", b.replace("\r", "")));
            }
            if c.isolate {
                out.push_str("isolate = true\n");
            }
            if let Some(r) = &c.risk {
                out.push_str(&format!("risk = \"{}\"\n", escape_toml(r)));
            }
            out.push('\n');
        }
        out
    }
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Interesting numeric operand pools (12-digit decimal semantics).
const INTS: &[i64] = &[
    0, 1, 2, 3, 5, 7, 9, 10, 42, 99, 255, 1000, -1, -2, -7, -42, -99,
];
const REALS: &[&str] = &[
    "0.5",
    "1.5",
    "2.5",
    "3.5",
    "-0.5",
    "-1.5",
    "-2.5",
    "0.1",
    "0.7",
    "1.25",
    "1.35",
    "3.14159",
    "2.71828",
    "-7.7",
    "0.000001",
    "1E-5",
    "1E-11",
    "1E499",
    "1E-499",
    "1.23456789012",
];
/// Operands safe for *arithmetic*: no exponent-boundary values, which on
/// the G1 can raise errors that `IFERR` does not survive (see the
/// `num_boundary_isolated` convention in `tests/compat/cases/core.toml`).
const REALS_SAFE: &[&str] = &[
    "0.5",
    "1.5",
    "2.5",
    "3.5",
    "-0.5",
    "-1.5",
    "-2.5",
    "0.1",
    "0.7",
    "1.25",
    "1.35",
    "3.14159",
    "2.71828",
    "-7.7",
    "0.000001",
    "1E-5",
    "1E-11",
    "1.23456789012",
];
const TIES: &[&str] = &[
    "0.5", "1.5", "2.5", "3.5", "4.5", "-0.5", "-1.5", "-2.5", "-3.5",
];
const STRS: &[&str] = &[
    "hello",
    "HP Prime",
    "PPL",
    "",
    "a",
    "Hello, World!",
    "0123456789",
    "CamelCase",
    "with space",
    "HP-Prime G1",
    "tab\tsep",
];

/// Builds one case body/expression for `category` from `rng`.
fn make_case(category: &str, rng: &mut SeedRng, n: usize) -> Case {
    let id = format!("{category}_{n:05}");
    let mut c = Case {
        id: id.clone(),
        expr: None,
        body: None,
        note: Some("generated".into()),
        known_divergence: None,
        group: Some(category.into()),
        isolate: false,
        mutates: Vec::new(),
        risk: None,
    };
    match category {
        "num_basic" => {
            // No exponent-boundary operands here: arithmetic on them can
            // raise errors IFERR does not survive on the G1 (the
            // `num_boundary_isolated` convention in core.toml). Boundary
            // coverage lives in `num_precision` (literals/rounding are
            // safe) and in the isolated `errors` category.
            //
            // Negative operands are parenthesized: hardware observation
            // (2026-10-23) suggests the G1's compiler rejects unary minus
            // juxtaposed after a binary operator (`0.7 * -7`, `1.25 -
            // -7.7`) — two note-channel batches containing such forms
            // would not run at all, while sibling batches without them
            // ran fine. `p()` keeps every generated expression
            // unambiguous on any parser.
            let neg_in_parens = |v: &str| {
                if v.starts_with('-') {
                    format!("({v})")
                } else {
                    v.to_string()
                }
            };
            let a = rng.pick(REALS_SAFE);
            let b = rng.pick(REALS_SAFE);
            let i = rng.pick(INTS);
            let j = rng.pick(INTS);
            let (pa, pb) = (neg_in_parens(a), neg_in_parens(b));
            let (si, sj) = (i.to_string(), j.to_string());
            let (pi, pj) = (neg_in_parens(&si), neg_in_parens(&sj));
            c.expr = Some(match rng.range(0, 9) {
                0 => format!("{pa} + {pb}"),
                1 => format!("{pa} - {pb}"),
                2 => format!("{pa} * {pb}"),
                3 => format!("{pa} / {pb}"),
                4 => format!("{}^{}", a.trim_start_matches('-'), rng.range(2, 9)),
                5 => format!("{pa} MOD {pb}"),
                // DIV does not exist as a PPL operator on the G1: the
                // calculator reports a syntax error at any DIV line
                // (hardware evidence 2026-10-23, and the pre-existing
                // core.toml risk note "DIV operator existence unverified
                // on G1"). IP(a/b) is the safe integer-quotient analogue.
                6 => format!("IP({pa} / {pb})"),
                7 => format!("SQRT({})", a.trim_start_matches('-')),
                8 => format!("{pi} + {pa} * {pj}"),
                _ => format!("-({pa} + {pb})"),
            });
        }
        "num_precision" => {
            let x = rng.pick(TIES);
            c.expr = Some(match rng.range(0, 7) {
                0 => format!("ROUND({x}, {})", rng.range(-2, 3)),
                1 => format!("ROUND({x}, {})", rng.range(0, 2)),
                2 => format!("TRUNCATE({x}, {})", rng.range(0, 2)),
                3 => format!("CEILING({x})"),
                4 => format!("FLOOR({x})"),
                5 => format!("IP({x}) + FP({x})"),
                // 13-digit literal normalization: the digit after the
                // 12-digit mantissa decides round-to-nearest-even.
                _ => {
                    let mut digits = String::from("1.2345678901");
                    for _ in 0..3 {
                        digits.push((b'0' + rng.range(0, 9) as u8) as char);
                    }
                    digits
                }
            });
        }
        "types_format" => {
            c.expr = Some(match rng.range(0, 8) {
                0 => format!("TYPE({})", rng.pick(REALS)),
                1 => format!("TYPE({})", rng.pick(INTS)),
                2 => "TYPE(#FFh)".into(),
                3 => format!("TYPE(({}, {}))", rng.range(1, 9), rng.range(1, 9)),
                4 => format!("TYPE([{}, {}])", rng.range(1, 9), rng.range(1, 9)),
                5 => format!("TYPE({{{}, {}}})", rng.range(1, 9), rng.range(1, 9)),
                6 => format!("TYPE(\"{}\")", rng.pick(STRS)),
                _ => format!(
                    "STRING({}) + \"|\" + STRING({})",
                    rng.pick(INTS),
                    rng.pick(REALS)
                ),
            });
        }
        "control_flow" => {
            let a = rng.range(1, 6);
            let b = a + rng.range(0, 6);
            c.body = Some(match rng.range(0, 7) {
                0 => format!("LOCAL i, s:=0; FOR i FROM {a} TO {b} DO s:=s+i; END; RETURN s;"),
                1 => {
                    format!("LOCAL i, s:=0; FOR i FROM {a} TO {b} STEP 2 DO s:=s+i; END; RETURN s;")
                }
                2 => format!("LOCAL i:=0, s:=0; WHILE i<{b} DO s:=s+i; i:=i+1; END; RETURN s;"),
                3 => format!("LOCAL i:=0, s:=0; REPEAT s:=s+i; i:=i+1; UNTIL i>={b}; RETURN s;"),
                4 => format!(
                    "LOCAL i, s:=0; FOR i FROM 1 TO 10 DO IF i={a} THEN CONTINUE; END; IF i={b} THEN BREAK; END; s:=s+i; END; RETURN s;"
                ),
                5 => format!("IF {a}<{b} THEN RETURN {a}; ELSE RETURN {b}; END;"),
                _ => format!("LOCAL i; FOR i FROM {b} TO {a} DO RETURN i; END; RETURN -1;"),
            });
        }
        "scoping" => {
            let a = rng.range(1, 9);
            let half = a / 2;
            c.body = Some(match rng.range(0, 5) {
                0 => format!("LOCAL x:={a}; IF x>{half} THEN LOCAL y:=x*2; x:=x+y; END; RETURN x;"),
                1 => {
                    format!("LOCAL a:={a}, b:=0; FOR a FROM 1 TO 3 DO b:=b+a; END; RETURN a*10+b;")
                }
                2 => format!("LOCAL z:={a}; z:=z+1; IFTE(z>0, z, -z); RETURN z;"),
                3 => format!(
                    "LOCAL p:={a}; IF p MOD 2==0 THEN p:=p/2; ELSE p:=p*3+1; END; RETURN p;"
                ),
                _ => format!(
                    "LOCAL i, j, t:=0; FOR i FROM 1 TO {a} DO FOR j FROM 1 TO 2 DO t:=t+i*j; END; END; RETURN t;"
                ),
            });
        }
        "strings" => {
            let s = rng.pick(STRS);
            let t = rng.pick(STRS);
            c.expr = Some(match rng.range(0, 10) {
                0 => format!("\"{s}\" + \"{t}\""),
                1 => format!("SIZE(\"{s}\")"),
                2 => format!("LEFT(\"{s}\", {})", rng.range(0, 4)),
                3 => format!("RIGHT(\"{s}\", {})", rng.range(0, 4)),
                4 => format!("MID(\"{s}\", {}, {})", rng.range(1, 4), rng.range(0, 4)),
                5 => format!("POS(\"{s}\", \"{}\")", rng.pick(STRS)),
                6 => format!("INSTRING(\"{s}\", \"{}\")", t),
                7 => format!("UPPER(\"{s}\") + \"|\" + LOWER(\"{s}\")"),
                8 => format!("ASC(\"{}\")", if s.is_empty() { "A" } else { &s[..1] }),
                _ => format!("REPLACE(\"{s}\", \"{}\", \"{}\")", t, rng.pick(STRS)),
            });
        }
        "lists" => {
            let a = rng.range(1, 9);
            let b = rng.range(1, 9);
            let lit = format!("{{{a}, {}, {}}}", b, a + b);
            c.expr = Some(match rng.range(0, 9) {
                0 => lit.clone(),
                1 => format!("SIZE({lit})"),
                2 => format!("{lit}({})", rng.range(1, 3)),
                3 => format!("CONCAT({lit}, {{{a}}})"),
                4 => format!("SORT({lit})"),
                5 => format!("REVERSE({lit})"),
                6 => format!("DELTALIST({lit}) + PILIST({lit})"),
                7 => format!("MEAN({lit}) + MEDIAN({lit})"),
                _ => format!("MIN({lit}) + MAX({lit}) + POS({lit}, {b})"),
            });
        }
        "matrices" => {
            let a = rng.range(1, 9);
            let b = rng.range(1, 9);
            let m = format!("[[{a}, {b}], [{b}, {a}]]");
            c.expr = Some(match rng.range(0, 7) {
                0 => m.clone(),
                1 => format!("DET({m})"),
                2 => format!("TRN({m})"),
                3 => format!("{m} + {m}"),
                4 => format!("{m} * {m}"),
                5 => format!("INV({m})"),
                _ => format!("SIZE({m}) + {m}(1,1)"),
            });
        }
        "complex" => {
            let a = rng.range(1, 9);
            let b = rng.range(1, 9);
            let z = format!("({a}, {b})");
            let w = format!("({b}, -{a})");
            c.expr = Some(match rng.range(0, 8) {
                0 => z.clone(),
                1 => format!("{z} + {w}"),
                2 => format!("{z} * {w}"),
                3 => format!("RE({z}) + IM({z})"),
                4 => format!("ABS({z})"),
                5 => format!("ARG({z})"),
                6 => format!("CONJ({z})"),
                _ => format!("TYPE({z}) + SIGN({z})"),
            });
        }
        "errors" => {
            // Error-risk cases follow the repo convention: isolate (one
            // device program each) so a risky case never affects others.
            c.isolate = true;
            c.risk = Some("expected runtime error (caught by IFERR on the device)".into());
            let a = rng.pick(INTS).abs().max(1);
            c.expr = Some(match rng.range(0, 5) {
                0 => "1/0".into(),
                1 => format!("{a}/0"),
                2 => "1E499*10".into(),
                3 => format!("{{1,2}}({})", rng.range(5, 9)),
                _ => format!("\"abc\"({})", rng.range(5, 9)),
            });
        }
        _ => unreachable!("unknown category {category}"),
    }
    c
}

/// Generates `count` validated cases spread over `categories`.
///
/// Validation is the compile audit: a case is accepted only if it compiles
/// and its only side effects are declared (`errors` cases declare none —
/// they must stay pure). Generation is capped at `4 * count + 64` attempts
/// to stay finite; the number of dropped cases is reported.
pub fn generate_corpus(
    count: usize,
    seed: u64,
    categories: &[String],
) -> Result<GeneratedCorpus, String> {
    let cats: Vec<String> = if categories.is_empty() {
        CATEGORIES.iter().map(|s| s.to_string()).collect()
    } else {
        for c in categories {
            if !CATEGORIES.contains(&c.as_str()) {
                return Err(format!(
                    "unknown category `{c}` (available: {})",
                    CATEGORIES.join(", ")
                ));
            }
        }
        categories.to_vec()
    };
    if count == 0 {
        return Err("count must be at least 1".into());
    }
    let mut rng = SeedRng::new(seed);
    let mut cases: Vec<Case> = Vec::new();
    let mut rejected = 0usize;
    let mut per_category: Vec<(String, usize)> = cats.iter().map(|c| (c.clone(), 0)).collect();
    let mut n = 0usize;
    let attempts = 4 * count + 64;
    while cases.len() < count && n < attempts {
        let cat_idx = (n % cats.len()).min(cats.len() - 1);
        let cat = cats[cat_idx].clone();
        let c = make_case(&cat, &mut rng, n);
        n += 1;
        match audit_case(&c) {
            Ok(a) if a.side_effects.is_empty() && !a.nondeterministic => {
                per_category[cat_idx].1 += 1;
                cases.push(c);
            }
            Ok(a) => {
                rejected += 1;
                let why = if !a.side_effects.is_empty() {
                    format!("side effects {:?}", a.side_effects)
                } else {
                    "nondeterministic".into()
                };
                // Errors category must never carry side effects; other
                // categories may not be nondeterministic either.
                eprintln!("generate: dropped {}: {}", c.id, why);
            }
            Err(_) => {
                rejected += 1;
            }
        }
    }
    if cases.len() < count {
        return Err(format!(
            "only {} of {} cases could be generated and validated \
             ({} attempts, {rejected} rejected); widen the categories or seed",
            cases.len(),
            count,
            n
        ));
    }
    // Batch the cases: groups of at most MAX_CASES_PER_PROGRAM per category;
    // isolated cases get a group of their own.
    let mut group_ids: Vec<String> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let cat = c.group.clone().unwrap_or_default();
        let gid = if c.isolate {
            format!("{cat}_iso{i:05}")
        } else {
            let n = cases[..i]
                .iter()
                .zip(&group_ids)
                .filter(|(c2, g)| !c2.isolate && g.starts_with(&format!("{cat}_b")))
                .count();
            format!("{cat}_b{}", n / crate::MAX_CASES_PER_PROGRAM + 1)
        };
        if !groups.iter().any(|g| g.id == gid) {
            groups.push(Group {
                id: gid.clone(),
                description: String::new(),
            });
        }
        group_ids.push(gid);
    }
    for (c, gid) in cases.iter_mut().zip(&group_ids) {
        c.group = Some(gid.clone());
    }
    let cases_len = cases.len();
    let suite = Suite {
        suite: format!("gen_{seed:08x}"),
        description: format!(
            "Deterministic generated corpus (seed {seed}, {} cases, categories: {})",
            cases.len(),
            cats.join(", ")
        ),
        groups,
        cases,
    };
    let corpus = GeneratedCorpus {
        suite,
        seed,
        count: cases_len,
        rejected,
        per_category,
        sha256: String::new(),
    };
    let mut c2 = corpus.clone();
    let toml_text = c2.toml();
    c2.sha256 = sha256_hex(toml_text.as_bytes());
    Ok(c2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(count: usize, seed: u64) -> GeneratedCorpus {
        generate_corpus(count, seed, &[]).unwrap()
    }

    #[test]
    fn reproducible_for_same_seed() {
        let a = make(50, 42);
        let b = make(50, 42);
        assert_eq!(a.toml(), b.toml());
        assert_eq!(a.sha256, b.sha256);
    }

    #[test]
    fn different_seed_differs() {
        let a = make(50, 1);
        let b = make(50, 2);
        assert_ne!(a.toml(), b.toml());
    }

    #[test]
    fn all_cases_compile_and_are_safe() {
        let c = make(200, 7);
        assert_eq!(c.count, 200);
        for case in &c.suite.cases {
            let a = audit_case(case).unwrap_or_else(|e| panic!("{}: {e}", case.id));
            assert!(
                a.side_effects.is_empty(),
                "{} has side effects {:?}",
                case.id,
                a.side_effects
            );
            assert!(!a.nondeterministic, "{} is nondeterministic", case.id);
        }
    }

    #[test]
    fn suite_loads_through_the_standard_loader() {
        // round-trip: the generated TOML must load with load_suite.
        let c = make(120, 0xABCD);
        let text = c.toml();
        let path = std::env::temp_dir().join(format!("zzt-corpus-{}.toml", std::process::id()));
        std::fs::write(&path, &text).unwrap();
        let suite = crate::load_suite(&path).unwrap();
        assert_eq!(suite.cases.len(), 120);
        let _ = std::fs::remove_file(&path);
        let reloaded: Vec<String> = suite.cases.iter().map(|c| c.id.clone()).collect();
        assert_eq!(reloaded.len(), c.suite.cases.len());
    }

    #[test]
    fn groups_fit_one_device_program_each() {
        let c = make(300, 99);
        let by_group: std::collections::BTreeMap<String, usize> = c
            .suite
            .cases
            .iter()
            .map(|x| (x.group.clone().unwrap_or_default(), ()))
            .fold(std::collections::BTreeMap::new(), |mut m, (g, _)| {
                *m.entry(g).or_insert(0) += 1;
                m
            });
        for (g, n) in by_group {
            if g.contains("_iso") {
                assert_eq!(n, 1, "isolated group {g} has {n} cases");
            } else {
                assert!(n <= crate::MAX_CASES_PER_PROGRAM, "group {g} has {n} cases");
            }
        }
    }

    #[test]
    fn every_group_can_become_a_device_program() {
        let c = make(120, 5);
        for g in &c.suite.groups {
            let cases = crate::select_cases(&c.suite, Some(&g.id), None).unwrap();
            let name = "ZZT_TG1";
            let gprog = crate::generate_device_program(
                &c.suite,
                &cases,
                name,
                &crate::Channel::Note {
                    note: "ZZT_TG1R".into(),
                },
            );
            assert!(
                gprog.is_ok(),
                "group {} -> {:?}",
                g.id,
                gprog.err().map(|e| e.join("; "))
            );
        }
    }

    #[test]
    fn emulator_runs_generated_batches() {
        let c = make(60, 1234);
        for g in &c.suite.groups {
            let cases = crate::select_cases(&c.suite, Some(&g.id), None).unwrap();
            let name = "ZZT_EMU";
            let gprog = crate::generate_device_program(
                &c.suite,
                &cases,
                name,
                &crate::Channel::Note {
                    note: "ZZT_EMUR".into(),
                },
            )
            .unwrap();
            // every generated case must run to completion in the emulator
            // (a runtime error is a legitimate differential result; a
            // compile error is not — generation validated compilation)
            for case in &cases {
                if let crate::Outcome::CompileError { detail } = crate::run_case(case) {
                    panic!("case {} does not compile: {}", case.id, detail)
                }
            }
            let _ = gprog;
        }
    }

    #[test]
    fn unknown_category_is_rejected() {
        assert!(generate_corpus(10, 1, &["nope".into()]).is_err());
    }

    #[test]
    fn categories_can_be_restricted() {
        let c = generate_corpus(30, 1, &["strings".into(), "complex".into()]).unwrap();
        assert_eq!(c.count, 30);
        assert!(
            c.suite
                .cases
                .iter()
                .all(|x| x.id.starts_with("strings_") || x.id.starts_with("complex_"))
        );
    }
}
