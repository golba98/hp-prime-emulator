//! Direct firmware Check: the original G1 2.4.2 compiler, reached without
//! driving the calculator's UI.
//!
//! The firmware's `Programs(name):=source` (`0x30B43FDC`) sets a program's
//! source and compiles it with the Program Editor Check's arguments. This
//! plan makes the same firmware calls, in the same order, as host guest
//! calls on the UI thread (`guest_call`), then reads the compiler's result
//! record as the Check handler (`0x308F2C60`) does and resolves the Check
//! dialog's message through the firmware's resource lookup. Every step runs
//! guest code; the host only passes the UTF-16 source in a guest-heap buffer
//! and reads results. See `docs/emulator/M9_DIRECT_COMPILER_REPORT.md`.
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::{
    compiler::{self, CheckObservation, RawPosition},
    guest_call::{Arg, Call, CallPlan, Returned, Step},
    memory::GuestMemory,
    sha256,
};

/// `operator new(size)`.
const NEW: u32 = 0x3080_0124;
/// Clears a program's source items (`ctx+0x314`) or its second list
/// (`ctx+0x31C`), as `Programs():=` does with `r1 = 0`.
const CLEAR_SOURCES: u32 = 0x306a_ea88;
const CLEAR_SECOND: u32 = 0x306a_e930;
/// Copies a NUL-terminated UTF-16 string into a new heap buffer.
const COPY_TEXT: u32 = 0x306b_c954;
/// Initializes a 0x48-byte source item `(item, name, 0, text)`; the item
/// owns `text` (stored at `item+0x40`).
const ITEM_INIT: u32 = 0x306a_dfd8;
/// Appends `*slot` to a pointer vector.
const PUSH: u32 = 0x306a_ea00;
/// The item name `Programs():=` passes: UTF-16 `"Main"`.
const ITEM_NAME: u32 = 0x30b4_4328;
/// Resource id → UTF-16 text (the Check dialog's lookup).
const RESOURCE_TEXT: u32 = 0x306c_3cd8;
/// Check handler's `mov r0, #<success resource>` and its literal holding
/// the error resource; read from the guest, not assumed.
const SUCCESS_ID_INSN: u32 = 0x308f_2ca8;
const ERROR_ID_LITERAL: u32 = 0x308f_2d04;
/// SHA-256 of the firmware code these calls rely on: `Programs():=`'s
/// set-and-compile sequence, the compiler prologue, the Check handler's
/// compile call and resource choice, the item name and the lookup.
const CODE_RANGES: [(u32, u32); 6] = [
    (0x30b4_4018, 0x30b4_407c),
    (0x306b_b740, 0x306b_b760),
    (0x308f_2c90, 0x308f_2cac),
    (0x308f_2d04, 0x308f_2d08),
    (0x30b4_4328, 0x30b4_4332),
    (0x306c_3cd8, 0x306c_3ce0),
];
const CODE_SHA256: &str = "1baf6b54b9502e7cacb0beeb1ca07ec9f2206d472d71a44bb141ad53318c997b";

/// What a direct Check produced, before validation.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Outcome {
    pub error: Option<String>,
    pub return_value: Option<u32>,
    pub record: Option<[u32; 3]>,
    pub record_va: Option<u32>,
    pub text_va: Option<u32>,
    pub message: Option<String>,
    /// (step, guest-call milliseconds) in order.
    pub calls: Vec<(String, f64)>,
    pub heap_before: Option<(usize, usize, usize)>,
    pub heap_after: Option<(usize, usize, usize)>,
}

/// Checks that the guest's code is the code these calls were traced on.
pub fn code_matches(mem: &GuestMemory) -> Result<(), String> {
    let mut bytes = Vec::new();
    for (start, end) in CODE_RANGES {
        bytes.extend(
            mem.read_bytes_alloc(start, (end - start) as usize)
                .map_err(|e| e.to_string())?,
        );
    }
    if sha256::hex(&bytes) != CODE_SHA256 {
        return Err("the firmware's compiler code differs from the traced G1 2.4.2 image".into());
    }
    Ok(())
}

/// Decodes an ARM data-processing immediate (`mov r0, #imm`).
fn arm_immediate(insn: u32) -> u32 {
    (insn & 0xff).rotate_right(((insn >> 8) & 0xf) * 2)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    ClearSources,
    ClearSecond,
    NewItem,
    CopyText,
    InitItem,
    Push,
    Compile,
    Message,
    Finished,
}

pub struct Plan {
    ctx: u32,
    text_va: u32,
    stage: Stage,
    item: u32,
    copy: u32,
    started: Option<Instant>,
    out: Arc<Mutex<Option<Outcome>>>,
    o: Outcome,
}

impl Plan {
    /// A plan to set program context `ctx`'s source to the NUL-terminated
    /// UTF-16 text at `text_va` and compile it. The result lands in `out`.
    pub fn new(ctx: u32, text_va: u32, out: Arc<Mutex<Option<Outcome>>>) -> Plan {
        Plan {
            ctx,
            text_va,
            stage: Stage::ClearSources,
            item: 0,
            copy: 0,
            started: None,
            out,
            o: Outcome {
                text_va: Some(text_va),
                ..Default::default()
            },
        }
    }

    fn fail(&mut self, e: impl Into<String>) -> Step {
        self.o.error = Some(e.into());
        self.finish()
    }

    fn finish(&mut self) -> Step {
        self.stage = Stage::Finished;
        *self.out.lock().unwrap() = Some(std::mem::take(&mut self.o));
        Step::Done
    }

    fn call(&mut self, stage: Stage, call: Call) -> Step {
        self.stage = stage;
        Step::Call(call)
    }
}

fn v(x: u32) -> Arg {
    Arg::Value(x)
}

impl CallPlan for Plan {
    fn next(&mut self, mem: &GuestMemory, last: Option<Returned>) -> Step {
        if let Some(r) = &last {
            self.o
                .calls
                .push((format!("{:?}", self.stage), r.elapsed_ms));
        }
        let ctx = self.ctx;
        match (self.stage, last) {
            (Stage::ClearSources, None) => {
                self.started = Some(Instant::now());
                if let Err(e) = code_matches(mem) {
                    return self.fail(e);
                }
                // `Programs():=` refuses a program in use (`ctx+0x24` bit 0).
                match mem.read_u32(ctx + 0x24) {
                    Ok(flags) if flags & 1 == 0 => {}
                    Ok(_) => return self.fail("the check program is in use"),
                    Err(e) => return self.fail(e.to_string()),
                }
                self.o.heap_before = Some(mem.heap_stats());
                self.call(
                    Stage::ClearSources,
                    Call::new(CLEAR_SOURCES, [v(ctx + 0x314), v(0), v(0), v(0)]),
                )
            }
            (Stage::ClearSources, Some(_)) => self.call(
                Stage::ClearSecond,
                Call::new(CLEAR_SECOND, [v(ctx + 0x31c), v(0), v(0), v(0)]),
            ),
            (Stage::ClearSecond, Some(_)) => {
                self.call(Stage::NewItem, Call::new(NEW, [v(0x48), v(0), v(0), v(0)]))
            }
            (Stage::NewItem, Some(r)) => {
                if r.r0 == 0 {
                    return self.fail("source item allocation failed");
                }
                self.item = r.r0;
                self.call(
                    Stage::CopyText,
                    Call::new(COPY_TEXT, [v(self.text_va), v(0), v(0), v(0)]),
                )
            }
            (Stage::CopyText, Some(r)) => {
                if r.r0 == 0 {
                    return self.fail("source text copy failed");
                }
                self.copy = r.r0;
                self.call(
                    Stage::InitItem,
                    Call::new(ITEM_INIT, [v(self.item), v(ITEM_NAME), v(0), v(self.copy)]),
                )
            }
            (Stage::InitItem, Some(_)) => self.call(
                Stage::Push,
                Call::new(PUSH, [v(ctx + 0x314), Arg::Frame(0), v(0), v(0)])
                    .with_frame(vec![self.item]),
            ),
            (Stage::Push, Some(_)) => self.call(
                Stage::Compile,
                // Check's arguments; the record starts at -1 as in Check.
                Call::new(compiler::COMPILE_ENTRY, [v(ctx), v(1), v(0), Arg::Frame(0)])
                    .with_frame(vec![u32::MAX, 0, 0]),
            ),
            (Stage::Compile, Some(r)) => {
                self.o.return_value = Some(r.r0);
                self.o.record = Some([r.frame[0], r.frame[1], r.frame[2]]);
                self.o.record_va = Some(r.frame_va);
                self.o.heap_after = Some(mem.heap_stats());
                // The Check handler's dialog: success resource on 1; on 0,
                // the error resource unless the record has no source (-1).
                let id = match (r.r0, r.frame[0]) {
                    (1, _) => mem.read_u32(SUCCESS_ID_INSN).map(arm_immediate),
                    (0, idx) if idx != u32::MAX => mem.read_u32(ERROR_ID_LITERAL),
                    _ => return self.finish(),
                };
                match id {
                    Ok(id) => self.call(
                        Stage::Message,
                        Call::new(RESOURCE_TEXT, [v(id), v(0), v(0), v(0)]),
                    ),
                    Err(e) => self.fail(e.to_string()),
                }
            }
            (Stage::Message, Some(r)) => {
                match compiler::read_utf16(mem, r.r0, 4096) {
                    Ok(text) => self.o.message = Some(text),
                    Err(e) => return self.fail(format!("message: {e}")),
                }
                let total = self.started.map(|t| t.elapsed().as_secs_f64() * 1000.0);
                self.o.calls.push(("Total".into(), total.unwrap_or(0.0)));
                self.finish()
            }
            (stage, _) => self.fail(format!("guest call sequence broke at {stage:?}")),
        }
    }
}

/// Validates a direct Check against the compiler observation and returns
/// it as a Check observation (`firmware_diagnostic` input), or no verdict.
pub fn validate(
    source: &str,
    ctx: u32,
    outcome: &Outcome,
    trace: &compiler::CompilerTrace,
) -> Result<CheckObservation, String> {
    if let Some(e) = &outcome.error {
        return Err(e.clone());
    }
    if trace.dropped != 0 {
        return Err("compiler observations were dropped".into());
    }
    let direct: Vec<_> = trace
        .checks
        .iter()
        .filter(|c| c.kind.as_deref() == Some("direct"))
        .collect();
    let [c] = direct.as_slice() else {
        return Err("expected exactly one direct compiler entry".into());
    };
    if !c.errors.is_empty() {
        return Err(format!("guest observation errors: {:?}", c.errors));
    }
    if c.program_va != Some(ctx) || c.result_va != outcome.record_va || c.cpsr & 0x20 != 0 {
        return Err("compiler context, record or ARM state was not the requested one".into());
    }
    if c.sources.len() != 1
        || c.parser_returns.len() != 1
        || c.sources[0].source_index != 0
        || c.sources[0].sha256 != sha256::hex(source.as_bytes())
        || c.sources[0].utf16_units != source.encode_utf16().count()
    {
        return Err("compiler did not observe exactly the requested source".into());
    }
    let (Some(rv), Some(record)) = (outcome.return_value, outcome.record) else {
        return Err("missing compiler return".into());
    };
    if !matches!(rv, 0 | 1) || outcome.message.as_ref().is_none_or(|m| m.is_empty()) {
        return Err("missing or unrecognized compiler return/message".into());
    }
    let mut obs = (*c).clone();
    obs.return_value = Some(rv);
    obs.message = outcome.message.clone();
    if rv == 0 {
        obs.raw_position = Some(RawPosition {
            source_index: record[0] as i32,
            line: record[1] as i32,
            offset_utf16: record[2] as i32,
        });
    }
    Ok(obs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{CompilerTrace, ObservedSource};

    const SRC: &str = "EXPORT T()\nBEGIN\n RETURN (1;\nEND;\n";

    fn observed(source: &str) -> CheckObservation {
        CheckObservation {
            kind: Some("direct".into()),
            compiler_entered: true,
            program_va: Some(0x2045_c870),
            result_va: Some(0x2000_0100),
            sources: vec![ObservedSource {
                source_index: 0,
                address: 0x2000_2000,
                sha256: sha256::hex(source.as_bytes()),
                utf16_units: source.encode_utf16().count(),
            }],
            parser_returns: vec![22],
            ..Default::default()
        }
    }

    fn outcome() -> Outcome {
        Outcome {
            return_value: Some(0),
            record: Some([0, 3, 18]),
            record_va: Some(0x2000_0100),
            message: Some("Error: Syntax Error".into()),
            ..Default::default()
        }
    }

    fn trace(checks: Vec<CheckObservation>) -> CompilerTrace {
        CompilerTrace { checks, dropped: 0 }
    }

    #[test]
    fn a_matching_direct_compile_yields_the_guest_position() {
        let obs = validate(SRC, 0x2045_c870, &outcome(), &trace(vec![observed(SRC)])).unwrap();
        assert_eq!(obs.return_value, Some(0));
        let p = obs.raw_position.unwrap();
        assert_eq!((p.source_index, p.line, p.offset_utf16), (0, 3, 18));
        assert_eq!(obs.message.as_deref(), Some("Error: Syntax Error"));
    }

    #[test]
    fn mismatched_source_context_or_repeated_entry_is_no_verdict() {
        let ok = outcome();
        let other = observed("EXPORT T()\nBEGIN\nEND;\n");
        assert!(validate(SRC, 0x2045_c870, &ok, &trace(vec![other])).is_err());
        assert!(validate(SRC, 0x1234_5678, &ok, &trace(vec![observed(SRC)])).is_err());
        let twice = trace(vec![observed(SRC), observed(SRC)]);
        assert!(validate(SRC, 0x2045_c870, &ok, &twice).is_err());
        assert!(validate(SRC, 0x2045_c870, &ok, &trace(vec![])).is_err());
        let mut no_message = outcome();
        no_message.message = None;
        assert!(validate(SRC, 0x2045_c870, &no_message, &trace(vec![observed(SRC)])).is_err());
        let mut failed = outcome();
        failed.error = Some("the check program is in use".into());
        assert!(validate(SRC, 0x2045_c870, &failed, &trace(vec![observed(SRC)])).is_err());
    }

    #[test]
    fn arm_immediates_decode_like_the_check_handler() {
        // 0x308F2CA8: mov r0, #0x410
        assert_eq!(arm_immediate(0xe3a0_0e41), 0x410);
        assert_eq!(arm_immediate(0xe3a0_0001), 1);
    }
}
