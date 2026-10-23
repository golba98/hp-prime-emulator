//! Window smoke test for `ppl gui` on a private Xvfb server.
//!
//! Ignored by default. Run with:
//! `PPL_GUI_SMOKE=1 cargo test -p ppl-cli --test gui_smoke -- --ignored --nocapture`
//! Needs `Xvfb` and ImageMagick `import`/`magick` on PATH. Never touches the
//! user's real display: `DISPLAY` points at the private server and
//! `WAYLAND_DISPLAY` is removed.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PPL: &str = env!("CARGO_BIN_EXE_ppl");

struct Xvfb {
    child: Child,
    display: String,
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        // SIGTERM lets Xvfb remove its socket and lock file; SIGKILL is the
        // fallback.
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_xvfb() -> Xvfb {
    let n = (90..400)
        .find(|n| {
            !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
                && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
        })
        .expect("a free X display number");
    let display = format!(":{n}");
    let child = Command::new("Xvfb")
        .args([&display, "-screen", "0", "1024x768x24", "-nolisten", "tcp"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start Xvfb (is it installed?)");
    let xvfb = Xvfb { child, display };
    let socket = format!("/tmp/.X11-unix/X{n}");
    let t0 = Instant::now();
    while !Path::new(&socket).exists() {
        assert!(t0.elapsed() < Duration::from_secs(10), "Xvfb did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    xvfb
}

fn tmp(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("ppl-gui-smoke");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{}_{name}", std::process::id()))
}

fn spawn_gui(x: &Xvfb, args: &[&str]) -> Child {
    Command::new(PPL)
        .arg("gui")
        .args(args)
        .env_remove("WAYLAND_DISPLAY")
        .env("DISPLAY", &x.display)
        .env("WINIT_UNIX_BACKEND", "x11")
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ppl gui")
}

/// Waits for exit; returns (exit code, stderr, time waited).
fn wait_exit(mut child: Child, timeout: Duration) -> (Option<i32>, String, Duration) {
    let t0 = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            let out = child.wait_with_output().unwrap();
            return (
                out.status.code(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
                t0.elapsed(),
            );
        }
        if t0.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("ppl gui did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn magick_info(png: &Path, format: &str) -> String {
    let out = Command::new("magick")
        .arg(png)
        .args(["-format", format, "info:"])
        .output()
        .expect("run ImageMagick `magick`");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn enabled() -> bool {
    if std::env::var_os("PPL_GUI_SMOKE").is_none() {
        eprintln!("skipping: set PPL_GUI_SMOKE=1 to run the Xvfb smoke test");
        return false;
    }
    true
}

#[test]
#[ignore = "needs Xvfb; run with PPL_GUI_SMOKE=1"]
fn gui_renders_and_closes_promptly() {
    if !enabled() {
        return;
    }
    let x = start_xvfb();
    let prog = tmp("box.ppl");
    std::fs::write(
        &prog,
        "EXPORT BOX()\nBEGIN\n  RECT_P(40,40,280,200,RGB(255,0,0),RGB(0,0,255));\n  \
         TEXTOUT_P(\"SMOKE\",120,110);\n  WAIT(0);\n  RETURN 1;\nEND;\n",
    )
    .unwrap();

    // 1. Render, capture, then close while the program blocks in WAIT(0).
    let exit_after = Duration::from_secs(4);
    let started = Instant::now();
    let child = spawn_gui(&x, &[prog.to_str().unwrap(), "--exit-after", "4"]);
    std::thread::sleep(Duration::from_millis(1500));
    let shot = tmp("capture.png");
    let st = Command::new("import")
        .args(["-display", &x.display, "-window", "root"])
        .arg(&shot)
        .status()
        .expect("run ImageMagick `import`");
    assert!(st.success(), "import failed");
    let colors: usize = magick_info(&shot, "%k").parse().unwrap_or(0);
    // Window at (0,0) without a WM, scaled 2x: (150,150) is inside the
    // blue rectangle (screen 40..280 x 40..200 -> window 80..560 x 80..400).
    let px = magick_info(&shot, "%[pixel:p{150,150}]");
    eprintln!(
        "capture {}: {colors} colours, p(150,150) = {px}",
        shot.display()
    );
    assert!(colors >= 3, "capture looks blank ({colors} colours)");
    assert!(
        px == "blue" || px.contains("(0,0,255)"),
        "expected the blue rectangle at (150,150), got {px}"
    );

    let (code, stderr, _) = wait_exit(child, Duration::from_secs(15));
    let total = started.elapsed();
    eprintln!("closed after {total:?}: exit {code:?}, stderr {stderr:?}");
    assert_eq!(
        code,
        Some(4),
        "user cancel exits with code 4; stderr: {stderr}"
    );
    assert!(stderr.contains("cancelled"), "{stderr}");
    assert!(
        total < exit_after + Duration::from_secs(3),
        "close took {total:?} (hang?)"
    );

    // 2. --exit-on-finish closes the window once the program returns.
    let quick = tmp("quick.ppl");
    std::fs::write(
        &quick,
        "EXPORT QUICK()\nBEGIN\n  RECT_P(0,0,10,10,RGB(255,0,0));\n  RETURN 7;\nEND;\n",
    )
    .unwrap();
    let child = spawn_gui(&x, &[quick.to_str().unwrap(), "--exit-on-finish"]);
    let (code, stderr, took) = wait_exit(child, Duration::from_secs(15));
    eprintln!("exit-on-finish: exit {code:?} after {took:?}, stderr {stderr:?}");
    assert_eq!(code, Some(0), "{stderr}");
    assert!(stderr.contains("QUICK returned 7"), "{stderr}");

    // 3. SIGTERM ends a blocked GUI process promptly.
    let child = spawn_gui(&x, &[prog.to_str().unwrap()]);
    std::thread::sleep(Duration::from_millis(1000));
    let pid = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .unwrap()
            .success()
    );
    let (code, _, took) = wait_exit(child, Duration::from_secs(5));
    eprintln!("SIGTERM: exit {code:?} after {took:?}");
}
