#![cfg(unix)]
use ppl_cas::{CasError, CasSession, CasValue, Options};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

fn helper(code: &str) -> (tempfile::TempDir, Options) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("helper");
    fs::write(&path, format!("#!/usr/bin/env python3\n{code}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    let options = Options {
        helper: path,
        timeout: Duration::from_secs(2),
        max_output_bytes: 4096,
    };
    (dir, options)
}
const GOOD: &str = "import sys,struct,json\nd=sys.stdin.buffer.read()\nr=struct.unpack('<Q',d[8:16])[0]\nprint(json.dumps(dict(version=1,request=r,engine='giac-test',result=dict(type='integer',value='4'),error=None,bindings={'a':'4'})))";
#[test]
fn valid_correlated_reply_commits_the_environment() {
    let (_dir, options) = helper(GOOD);
    let mut session = CasSession::default();
    assert_eq!(
        session
            .evaluate("2+2", &options, &AtomicBool::new(false))
            .unwrap(),
        CasValue::Integer("4".into())
    );
    assert_eq!(session.bindings["a"], "4");
    assert_eq!(session.last_value, Some(CasValue::Integer("4".into())));
}
#[test]
fn invalid_identity_or_shape_never_commits() {
    for script in [
        GOOD.replace("version=1", "version=2"),
        GOOD.replace("request=r", "request=r+1"),
        GOOD.replace("engine='giac-test'", "engine='unrelated'"),
        GOOD.replace(
            "dict(type='integer',value='4')",
            "dict(type='matrix',value=dict(rows=2,cols=2,values=[]))",
        ),
    ] {
        let (_dir, options) = helper(&script);
        let mut session = CasSession::default();
        session.bindings.insert("old".into(), "7".into());
        assert!(matches!(
            session.evaluate("2+2", &options, &AtomicBool::new(false)),
            Err(CasError::Protocol(_))
        ));
        assert_eq!(session.bindings.len(), 1);
        assert_eq!(session.bindings["old"], "7");
        assert!(session.last_value.is_none());
    }
}
#[test]
fn evaluation_error_preserves_existing_context() {
    let (_dir, options) = helper(&GOOD.replace("error=None", "error='invalid argument'"));
    let mut session = CasSession::default();
    session.bindings.insert("old".into(), "7".into());
    assert_eq!(
        session.evaluate("1+", &options, &AtomicBool::new(false)),
        Err(CasError::Evaluation("invalid argument".into()))
    );
    assert_eq!(session.bindings["old"], "7");
}
#[test]
fn cancellation_wakes_a_stalled_helper_and_closes_its_pipes() {
    let (_dir, options) = helper("import time\ntime.sleep(30)");
    let cancel = Arc::new(AtomicBool::new(false));
    let control = cancel.clone();
    let operation =
        std::thread::spawn(move || CasSession::default().evaluate("2+2", &options, &control));
    std::thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    cancel.store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(operation.join().unwrap(), Err(CasError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(1));
}
#[test]
fn timeout_kills_a_helper_that_does_not_read_a_large_request() {
    let (_dir, mut options) = helper("import time\ntime.sleep(30)");
    options.timeout = Duration::from_millis(50);
    let mut session = CasSession::default();
    let started = Instant::now();
    assert_eq!(
        session.evaluate(&"a".repeat(1024 * 1024), &options, &AtomicBool::new(false)),
        Err(CasError::TimedOut)
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}
#[test]
fn output_limit_is_enforced_without_deadlocking_the_writer() {
    let (_dir, options) =
        helper("import sys\nsys.stdin.buffer.read()\nsys.stdout.write('a'*100000)");
    assert!(matches!(
        CasSession::default().evaluate("2+2", &options, &AtomicBool::new(false)),
        Err(CasError::Protocol(_))
    ));
}
#[test]
fn unavailable_and_cancelled_are_explicit() {
    let options = Options {
        helper: "/definitely/missing/ppl-giac-helper".into(),
        ..Default::default()
    };
    let mut session = CasSession::default();
    assert!(matches!(
        session.evaluate("2+2", &options, &AtomicBool::new(false)),
        Err(CasError::Unavailable(_))
    ));
    assert_eq!(
        session.evaluate("2+2", &options, &AtomicBool::new(true)),
        Err(CasError::Cancelled)
    );
}
#[test]
fn invalid_requests_are_rejected_before_spawn() {
    let options = Options::default();
    let mut session = CasSession::default();
    let cancel = AtomicBool::new(false);
    assert!(matches!(
        session.evaluate(" ", &options, &cancel),
        Err(CasError::InvalidRequest(_))
    ));
    session.settings.digits = 0;
    assert!(matches!(
        session.evaluate("2+2", &options, &cancel),
        Err(CasError::InvalidRequest(_))
    ));
    session.settings.digits = 12;
    session.bindings.insert("a;quit()".into(), "5".into());
    assert!(matches!(
        session.evaluate("2+2", &options, &cancel),
        Err(CasError::InvalidRequest(_))
    ));
}

#[test]
fn bridge_rejects_unrequested_home_mutations_without_committing_cas() {
    let script = GOOD.replace("version=1", "version=2").replace(
        "bindings={'a':'4'}",
        "bindings={'a':'4'},home_updates={'unknown':dict(type='integer',value='9')}",
    );
    let (_dir, options) = helper(&script);
    let mut session = CasSession::default();
    session.bindings.insert("old".into(), "7".into());
    let before = serde_json::to_value(&session).unwrap();
    assert!(matches!(
        session.evaluate_with_home(
            "Q+1",
            &Default::default(),
            &options,
            &AtomicBool::new(false)
        ),
        Err(CasError::Protocol(_))
    ));
    assert_eq!(serde_json::to_value(&session).unwrap(), before);
}
