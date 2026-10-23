//! Integration tests for `ppl debug`, driving the real binary with a piped
//! stdin command script.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const PPL: &str = env!("CARGO_BIN_EXE_ppl");

/// Lines are numbered for the assertions below.
const PROG: &str = "\
EXPORT GV:=5;
SQR2(X)
BEGIN
  LOCAL Y;
  Y:=X*X;
  RETURN Y;
END;

EXPORT MAIN()
BEGIN
  LOCAL A,B;
  A:=3;
  B:=SQR2(A);
  PRINT(B);
  PRINT(GV+A);
  RETURN B+1;
END;
";

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn temp_program(src: &str, ext: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ppl-debug-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!(
        "prog_{}_{}.{ext}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&path, src).unwrap();
    path
}

/// Runs `ppl <args>` feeding `chunks` to stdin (with `delay` between
/// chunks), then closes stdin. Fails if the process outlives `timeout`.
fn run_ppl(args: &[&str], chunks: &[&str], delay: Duration, timeout: Duration) -> Out {
    let mut child = Command::new(PPL)
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ppl");
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    {
        let mut stdin = child.stdin.take().unwrap();
        for (i, c) in chunks.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(delay);
            }
            // The process may already have exited; that is not an error here.
            let _ = stdin.write_all(c.as_bytes());
            let _ = stdin.flush();
        }
    } // stdin closed -> EOF
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("`ppl {}` did not exit within {timeout:?}", args.join(" "));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Out {
        code: status.code(),
        stdout: out_t.join().unwrap(),
        stderr: err_t.join().unwrap(),
    }
}

fn debug(file: &Path, extra: &[&str], script: &str) -> Out {
    let mut args = vec!["debug", file.to_str().unwrap()];
    args.extend_from_slice(extra);
    run_ppl(&args, &[script], Duration::ZERO, Duration::from_secs(20))
}

/// The `[STOP] ...` header lines, in order.
fn stops(out: &Out) -> Vec<String> {
    out.stdout
        .lines()
        .filter(|l| l.starts_with("[STOP]"))
        .map(|l| l.to_string())
        .collect()
}

fn assert_contains(hay: &str, needle: &str) {
    assert!(
        hay.contains(needle),
        "expected to find {needle:?} in:\n-----\n{hay}\n-----"
    );
}

#[test]
fn breakpoint_from_command_line_stops_at_right_line() {
    let f = temp_program(PROG, "ppl");
    let out = debug(&f, &["-b", "5", "--run"], "p X\nl\nbt\nc\n");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_contains(&out.stdout, "Breakpoint set at line 5");
    assert_contains(&out.stdout, "Breakpoint hit at line 5");
    let s = stops(&out);
    assert_eq!(s.len(), 1, "{s:?}");
    assert!(
        s[0].starts_with("[STOP] SQR2() at ") && s[0].ends_with(":5:3 (frame depth 2)"),
        "{s:?}"
    );
    assert_contains(&out.stdout, "=>    5 |   Y:=X*X;");
    assert_contains(&out.stdout, "  X = 3\n");
    assert_contains(&out.stdout, "  Y = 0\n");
    assert_contains(&out.stdout, "  #0 SQR2() at line 5:3\n");
    assert_contains(&out.stdout, "  #1 MAIN() at line 13:");
    assert_contains(&out.stdout, "Program exited normally with value: 10");
}

#[test]
fn breakpoint_set_at_prompt_and_continue() {
    let f = temp_program(PROG, "ppl");
    let out = debug(&f, &[], "b 15\nb\nc\np B\nc\n");
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_contains(&out.stdout, "Breakpoint set at line 15");
    assert_contains(&out.stdout, "Active breakpoints: {15}");
    let s = stops(&out);
    assert_eq!(s.len(), 2, "{s:?}");
    assert!(
        s[0].contains(":11:3 (frame depth 1)"),
        "first statement: {s:?}"
    );
    assert!(
        s[1].contains("MAIN() at") && s[1].contains(":15:3"),
        "{s:?}"
    );
    assert_contains(&out.stdout, "  B = 9\n");
    assert_contains(&out.stdout, "Program exited normally with value: 10");
}

#[test]
fn step_enters_calls_next_steps_over_finish_returns() {
    let f = temp_program(PROG, "ppl");
    // step x3 reaches the call on line 13, step enters SQR2 (line 4).
    let out = debug(&f, &[], "s\ns\ns\nfin\nq\n");
    let s = stops(&out);
    let lines: Vec<&str> = s
        .iter()
        .map(|l| l.rsplit(':').nth(1).unwrap_or(""))
        .collect();
    assert_eq!(lines, ["11", "12", "13", "4", "14"], "{s:?}");
    assert!(s[3].starts_with("[STOP] SQR2()") && s[3].ends_with("(frame depth 2)"));
    // `finish` returns to MAIN at the statement after the call.
    assert!(s[4].starts_with("[STOP] MAIN()") && s[4].ends_with("(frame depth 1)"));
    assert_eq!(out.code, Some(4));
    assert_contains(&out.stdout, "Debugger quit; program aborted.");

    // `next` over the call never stops inside SQR2.
    let out = debug(&f, &[], "n\nn\nn\nn\nq\n");
    let s = stops(&out);
    assert!(s.iter().all(|l| l.starts_with("[STOP] MAIN()")), "{s:?}");
    let lines: Vec<&str> = s
        .iter()
        .map(|l| l.rsplit(':').nth(1).unwrap_or(""))
        .collect();
    assert_eq!(lines, ["11", "12", "13", "14", "15"], "{s:?}");
}

#[test]
fn next_still_stops_at_breakpoints_inside_calls() {
    let f = temp_program(PROG, "ppl");
    let out = debug(&f, &["-b", "6"], "n\nn\nn\nq\n");
    let s = stops(&out);
    assert!(
        s[3].starts_with("[STOP] SQR2()") && s[3].contains(":6:3"),
        "{s:?}"
    );
}

#[test]
fn frames_locals_globals_and_print() {
    let f = temp_program(PROG, "ppl");
    let out = debug(
        &f,
        &["-b", "5", "--run"],
        "l\nup\nl\np A\np X\ndown\np X\np GV\ng\np NOPE\nc\n",
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    let o = &out.stdout;
    assert_contains(o, "Frame #1 MAIN() at line 13:");
    assert_contains(o, "  A = 3\n  B = 0\n");
    assert_contains(o, "Variable `X` not found in current scope or globals.");
    assert_contains(o, "Frame #0 SQR2() at line 5:3");
    assert_contains(o, "  X = 3\n");
    assert_contains(o, "  GV = 5\n");
    assert_contains(o, "Variable `NOPE` not found");
    // `up` past the outermost frame is refused.
    let out = debug(&f, &["-b", "5", "--run"], "up\nup\nc\n");
    assert_contains(&out.stdout, "Already at the outermost frame");
}

#[test]
fn runtime_error_is_reported_with_line() {
    let src = "EXPORT E1()\nBEGIN\n  LOCAL Z:=0;\n  RETURN 1/Z;\nEND;\n";
    let f = temp_program(src, "ppl");
    let out = debug(&f, &[], "c\n");
    assert_eq!(out.code, Some(3));
    assert_contains(&out.stderr, "Execution terminated: Error: Division by 0");
    let name = f.to_str().unwrap();
    assert_contains(&out.stderr, &format!("--> {name}:4:10 in E1   RETURN 1/Z;"));
}

#[test]
fn eof_on_stdin_quits() {
    let f = temp_program(PROG, "ppl");
    let out = debug(&f, &[], "");
    assert_eq!(out.code, Some(4));
    assert_eq!(stops(&out).len(), 1);
    assert_contains(&out.stdout, "End of input: quitting debugger.");
    assert_contains(&out.stdout, "Debugger quit; program aborted.");
    // A partial script ending mid-session also terminates.
    let out = debug(&f, &[], "s\nl\n");
    assert_eq!(out.code, Some(4));
    assert_eq!(stops(&out).len(), 2);
}

#[test]
fn unknown_command_and_help() {
    let f = temp_program(PROG, "ppl");
    let out = debug(&f, &[], "frobnicate\nh\nc\n");
    assert_eq!(out.code, Some(0));
    assert_contains(
        &out.stdout,
        "Unknown command `frobnicate`. Type `h` for help.",
    );
    assert_contains(&out.stdout, "n, next");
    assert_contains(&out.stdout, "fin, finish, out");
}

/// Program output under the debugger (`c` straight away) is exactly the
/// output of `ppl run`: the debugger must not change semantics.
#[test]
fn print_output_matches_ppl_run() {
    let src = "\
EXPORT OUTP()
BEGIN
  LOCAL I,S:=0,L:={};
  FOR I FROM 1 TO 5 DO
    S:=S+I^2;
    L(I):=S;
    PRINT(\"i=\"+I+\" s=\"+S);
  END;
  PRINT(L);
  PRINT(STRING(1/3));
  IF S>50 THEN PRINT(\"big\"); ELSE PRINT(\"small\"); END;
  RETURN S;
END;
";
    for ext in ["ppl", "hpprgm"] {
        let f = temp_program(src, ext);
        let file = f.to_str().unwrap();
        let run = run_ppl(&["run", file], &[], Duration::ZERO, Duration::from_secs(20));
        assert_eq!(run.code, Some(0), "run stderr: {}", run.stderr);
        assert!(run.stdout.lines().count() >= 8, "{}", run.stdout);

        let dbg = debug(&f, &[], "c\n");
        assert_eq!(dbg.code, Some(0), "debug stderr: {}", dbg.stderr);
        let after_prompt = dbg.stdout.rsplit_once("(ppldb) ").expect("one prompt").1;
        let (program_out, tail) = after_prompt
            .split_once("\nProgram exited normally with value: ")
            .expect("normal exit");
        assert_eq!(program_out, run.stdout);
        assert_eq!(tail.trim(), "55");
    }
}

/// Time spent at the prompt does not count toward a wall-clock limit unless
/// one is requested with --max-time.
#[test]
fn prompt_time_does_not_hit_time_limit_by_default() {
    let src = "\
EXPORT SLOW()
BEGIN
  LOCAL I,S:=0;
  FOR I FROM 1 TO 3000 DO S:=S+1; END;
  RETURN S;
END;
";
    let f = temp_program(src, "ppl");
    let file = f.to_str().unwrap();
    let pause = Duration::from_millis(1300);
    let out = run_ppl(
        &["debug", file],
        &["", "c\n"],
        pause,
        Duration::from_secs(20),
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_contains(&out.stdout, "Program exited normally with value: 3000");

    // With an explicit limit the same pause is charged to the program.
    let out = run_ppl(
        &["debug", file, "--max-time", "1"],
        &["", "c\n"],
        pause,
        Duration::from_secs(20),
    );
    assert_eq!(out.code, Some(4), "stdout: {}", out.stdout);
    assert_contains(&out.stderr, "Resource limit exceeded");
}

#[test]
fn scripted_input_choose_and_keys() {
    let src = "\
EXPORT ASK()
BEGIN
  LOCAL A:=0,C:=0,K;
  INPUT(A);
  CHOOSE(C,\"Pick\",\"x\",\"y\",\"z\");
  K:=GETKEY;
  PRINT(A+C*100+K*1000);
  RETURN 0;
END;
";
    let f = temp_program(src, "ppl");
    let out = debug(
        &f,
        &["--input", "7", "--choose", "3", "--keys", "30"],
        "c\n",
    );
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_contains(&out.stdout, "30307\n");
}
