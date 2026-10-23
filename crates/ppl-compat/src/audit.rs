//! Static side-effect audit of suite cases.
//!
//! A case is compiled exactly as [`crate::run_case`] compiles it and every
//! instruction of every chunk is inspected. Anything that can change
//! calculator state that outlives the test program, block the program, or
//! make the result depend on something other than the source is reported.
//!
//! # What counts as a side effect (named in [`CaseAudit::side_effects`])
//!
//! * `StoreGlobal` / `StoreIndexGlobal` to a global slot that is a calculator
//!   setting (`HAngle`, `HFormat`, `HDigits`, `HComplex`, `Base`, `Bits`,
//!   `Signed`) or any other calculator system variable (`A`–`Z`, `θ`,
//!   `L0`–`L9`, `M0`–`M9`, `Z0`–`Z9`, `G0`–`G9`, `Ans`): named by the slot
//!   name, e.g. `HAngle`, `A`, `L1`.
//! * A call (`Builtin` / `BuiltinRef`) to a side-effecting built-in, named by
//!   the built-in's name (e.g. `PRINT`, `RECT_P`, `RANDOM`). Classification
//!   (see [`classify_builtin`]) is by the command table's category, with
//!   explicit per-name exceptions:
//!   - **graphics**: side-effecting (drawing changes the screen / a GROB),
//!     except the pure helpers `RGB`, `GROBW_P`, `GROBH_P` and the read-only
//!     `GETPIX`/`GETPIX_P` (nondeterministic: they read screen state).
//!   - **io**: side-effecting (terminal `PRINT`, dialogs `MSGBOX`, `INPUT`,
//!     `CHOOSE`, `WAIT`, `GETKEY`, `ISKEYDOWN`, `MOUSE`, `DISP`, `FREEZE`,
//!     `DRAWMENU`, `EDITLIST`, …): they write the screen, block for the
//!     user, or read the keyboard.
//!   - **system**: side-effecting (`STARTAPP`, `STARTVIEW`, `NOTES`,
//!     `AFILES`, `HVARS`, `RECALL`, `EVAL`, …) except `TYPE` (pure).
//!   - **cas**: side-effecting (the CAS has its own persistent state).
//!   - **math / bits / strings / lists / matrices**: pure, except
//!     `RANDOM`, `RANDINT`, `RANDNORM` (advance the persistent generator and
//!     are nondeterministic), `RANDSEED` (reseeds it), `EXPR` (evaluates
//!     arbitrary text, which cannot be audited statically) and `EDITMAT`
//!     (opens an editor).
//!   - **time** (`TICKS`, `TIME`, `DATE`): no side effect, nondeterministic.
//!   - any category this module does not know: side-effecting (fail safe).
//! * `Op::Unsupported` (a command the compiler does not know by name):
//!   `unsupported:<name>`, since nothing is known about what it does.
//! * `Op::Kill` (`KILL`): `KILL`, as it would abort the whole test program.
//!
//! # Nondeterminism ([`CaseAudit::nondeterministic`])
//!
//! Set by the random and time built-ins, by `GETPIX`, and by any *read* of a
//! calculator system variable other than a setting (its value depends on
//! what the user left there). Reads of settings are not flagged: the device
//! program records `HAngle`/`HFormat`/`HDigits` in its header.

use std::collections::BTreeSet;

use ppl_compiler::builtins::{BuiltinSpec, spec};
use ppl_compiler::{CompiledProgram, GlobalInit, Op};
use serde::{Deserialize, Serialize};

use crate::Case;

/// Name of the side effect reported for terminal output.
pub const SIDE_EFFECT_PRINT: &str = "PRINT";
/// Name of the side effect reported for `KILL`.
pub const SIDE_EFFECT_KILL: &str = "KILL";

/// Result of [`audit_case`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseAudit {
    /// Persistent or externally visible effects, by name (see module docs).
    pub side_effects: BTreeSet<String>,
    /// The result may differ between runs or devices for the same source.
    pub nondeterministic: bool,
}

/// How a built-in behaves for audit purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinEffect {
    pub side_effect: bool,
    pub nondeterministic: bool,
}

const PURE: BuiltinEffect = BuiltinEffect {
    side_effect: false,
    nondeterministic: false,
};
const EFFECT: BuiltinEffect = BuiltinEffect {
    side_effect: true,
    nondeterministic: false,
};
const READS_STATE: BuiltinEffect = BuiltinEffect {
    side_effect: false,
    nondeterministic: true,
};

/// Classifies a built-in (see the module documentation for the rules).
pub fn classify_builtin(s: &BuiltinSpec) -> BuiltinEffect {
    match s.name {
        "RGB" | "GROBW_P" | "GROBH_P" | "TYPE" => return PURE,
        "GETPIX" | "GETPIX_P" | "TICKS" | "TIME" | "DATE" => return READS_STATE,
        "RANDOM" | "RANDINT" | "RANDNORM" => {
            return BuiltinEffect {
                side_effect: true,
                nondeterministic: true,
            };
        }
        "RANDSEED" | "EXPR" | "EDITMAT" | "EDITLIST" => return EFFECT,
        _ => {}
    }
    // Matched by name so that a category added to the table later falls
    // through to the fail-safe arm instead of breaking the build.
    match s.category.as_str() {
        "math" | "bits" | "strings" | "lists" | "matrices" => PURE,
        "time" => READS_STATE,
        _ => EFFECT,
    }
}

/// Compiles a case the way [`crate::run_case`] does and audits it.
/// A compile error is returned as `Err` with the first diagnostic.
pub fn audit_case(c: &Case) -> Result<CaseAudit, String> {
    let file =
        ppl_syntax::SourceFile::new(format!("<audit {}>", c.id), crate::case_wrapper_source(c));
    let (prog, diags) =
        ppl_compiler::compile_source(&file, &ppl_compiler::CompileOptions::default());
    match prog {
        Some(p) => Ok(audit_program(&p)),
        None => Err(diags
            .errors()
            .next()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| "compile failed".into())),
    }
}

/// Audits every chunk (including the file-level initialiser) of a program.
pub fn audit_program(prog: &CompiledProgram) -> CaseAudit {
    let mut a = CaseAudit::default();
    for chunk in std::iter::once(&prog.init).chain(prog.functions.iter()) {
        for op in &chunk.code {
            match *op {
                Op::StoreGlobal(g) | Op::StoreIndexGlobal { slot: g, .. } => {
                    let Some(slot) = prog.globals.get(g as usize) else {
                        continue;
                    };
                    if matches!(slot.init, GlobalInit::Setting(_)) || !slot.program_var {
                        a.side_effects.insert(slot.name.clone());
                    }
                }
                Op::LoadGlobal(g) => {
                    let Some(slot) = prog.globals.get(g as usize) else {
                        continue;
                    };
                    if !slot.program_var && !matches!(slot.init, GlobalInit::Setting(_)) {
                        a.nondeterministic = true;
                    }
                }
                Op::Builtin { id, .. } | Op::BuiltinRef { id, .. } => {
                    let s = spec(id);
                    let e = classify_builtin(s);
                    if e.side_effect {
                        a.side_effects.insert(s.name.to_string());
                    }
                    a.nondeterministic |= e.nondeterministic;
                }
                Op::Unsupported { name, .. } => {
                    let n = prog
                        .names
                        .get(name as usize)
                        .map(String::as_str)
                        .unwrap_or("?");
                    a.side_effects.insert(format!("unsupported:{n}"));
                }
                Op::Kill => {
                    a.side_effects.insert(SIDE_EFFECT_KILL.into());
                }
                _ => {}
            }
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(body: &str) -> Case {
        toml::from_str(&format!("id = \"t\"\nbody = '''\n{body}\n'''\n")).unwrap()
    }

    fn effects(body: &str) -> Vec<String> {
        audit_case(&case(body))
            .unwrap()
            .side_effects
            .into_iter()
            .collect()
    }

    #[test]
    fn pure_cases_have_no_effects() {
        assert!(effects("RETURN 1+2;").is_empty());
        assert!(effects("LOCAL L:={1,2}; L(0):=3; RETURN SIZE(L);").is_empty());
        assert!(effects("RETURN STRING(TYPE(\"a\"));").is_empty());
        assert!(!audit_case(&case("RETURN 1;")).unwrap().nondeterministic);
    }

    #[test]
    fn settings_and_system_variables() {
        assert_eq!(effects("HAngle:=1; RETURN 0;"), ["HAngle"]);
        assert_eq!(effects("A:=5; RETURN A;"), ["A"]);
        assert_eq!(effects("L1(1):=5; RETURN 0;"), ["L1"]);
        // Reading a user variable is not a side effect but is nondeterministic.
        let a = audit_case(&case("RETURN A;")).unwrap();
        assert!(a.side_effects.is_empty() && a.nondeterministic);
        // Reading a setting is neither.
        let a = audit_case(&case("RETURN HAngle;")).unwrap();
        assert!(a.side_effects.is_empty() && !a.nondeterministic);
    }

    #[test]
    fn builtins_by_category() {
        assert_eq!(effects("PRINT(1); RETURN 0;"), [SIDE_EFFECT_PRINT]);
        assert_eq!(effects("RECT_P(); RETURN 0;"), ["RECT_P"]);
        assert_eq!(effects("RETURN RGB(1,2,3);"), Vec::<String>::new());
        let a = audit_case(&case("RETURN RANDOM();")).unwrap();
        assert!(a.side_effects.contains("RANDOM") && a.nondeterministic);
        let a = audit_case(&case("RETURN TICKS();")).unwrap();
        assert!(a.side_effects.is_empty() && a.nondeterministic);
        assert_eq!(effects("RETURN NOTES(\"x\");"), ["NOTES"]);
    }

    #[test]
    fn compile_error_is_err() {
        assert!(audit_case(&case("RETURN (;")).is_err());
    }
}
