//! Bounded headless FC04 acceptance experiment, separate from production mirroring.
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use clap::Args;
use ppl_device::screenshot::{
    ScreenshotSession,
    update::{BoundedIncrementalPermit, Placement},
};
use ppl_graphics::{PrimeScreenshotDecoder, incremental::IncrementalFramebuffer};
use serde_json::{Value, json};

type Error = Box<dyn std::error::Error>;

#[derive(Args, Debug)]
pub struct StreamOptions {
    /// Separate explicit approval for this bounded physical stream experiment.
    #[arg(long, required = true)]
    pub acknowledge_experimental_fc04: bool,
    /// Previously reviewed, successful genuine changed-screen report.json and fixtures.
    #[arg(long)]
    pub reviewed_visual_report: PathBuf,
    /// Fresh private evidence directory; never overwritten.
    #[arg(long)]
    pub out_dir: PathBuf,
    /// Wall-time ceiling including references and recovery; not a performance target.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=120))]
    pub seconds: u64,
    /// Total FC04 attempts, across all reconnections; never refunded.
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u32).range(1..=200))]
    pub max_updates: u32,
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err("evidence file exceeds limit".into());
    }
    Ok(bytes)
}

/// A report is only a diagnostic prerequisite, never a production capability.
fn review_visual_report(path: &Path, selector: &str) -> Result<Value, Error> {
    let raw = read_bounded(path, 1024 * 1024)?;
    let report: Value = serde_json::from_slice(&raw)?;
    if report["operation"] != "verify_incremental"
        || report["status"] != "visual_verification_passed"
        || report["device"]["serial_id"] != selector
        || report["device"]["vid"] != 0x03f0
        || report["device"]["pid"] != 0x1541
        || report["device"]["firmware_build"] != 15157
        || report["analysis"]["screen_actually_changed"] != true
        || report["analysis"]["any_patch_changed"] != true
        || report["comparison"]["composed_vs_reference"]["mismatching_pixels"] != 0
    {
        return Err("requires reviewed changed-screen success for this exact G1/build; static or edited capability flags grant no production permission".into());
    }
    let directory = path.parent().ok_or("visual report has no parent")?;
    let mut images = Vec::new();
    for (field, name) in [
        ("baseline", "baseline.raw.png"),
        ("reference", "reference.raw.png"),
        ("composed", "composed.png"),
    ] {
        let bytes = read_bounded(&directory.join(name), 16 * 1024 * 1024)?;
        if report[field]["sha256"] != ppl_compat::sha256_hex(&bytes) {
            return Err(format!("reviewed {field} fixture hash mismatch").into());
        }
        images.push(
            PrimeScreenshotDecoder::default()
                .decode(&bytes, Some((320, 240)))?
                .0,
        );
    }
    if images[0] == images[1] || images[1] != images[2] {
        return Err("reviewed fixtures do not demonstrate exact changed-screen reconstruction with the current decoder".into());
    }
    Ok(json!({"sha256":ppl_compat::sha256_hex(&raw),"serial_id":selector}))
}

pub fn run(selector: Option<&str>, options: &StreamOptions, ipc: bool) -> u8 {
    let mut report = json!({
        "schema_version": 1,
        "operation": "verify_incremental_stream",
        "status": "refused",
        "hardware_verified": false,
        "streaming_enabled": false,
        "fc04_attempts": 0,
        "valid_updates": 0,
        "changed_updates": 0,
        "full_captures": [],
        "full_frames": 0,
        "reconnections": 0,
        "capture_errors": 0,
        "checkpoint_mismatches": 0,
        "changed_checkpoints": 0,
        "checkpoints": [],
        "unique_presented_frames_per_second": null,
        "physical_change_to_display_ms": null,
    });
    let mut created = false;
    let result = (|| -> Result<(), Error> {
        let selector = selector.ok_or("requires --device")?;
        if !options.acknowledge_experimental_fc04 || ipc {
            return Err(
                "requires separate experimental acknowledgement and a direct exclusive connection"
                    .into(),
            );
        }
        if !(1..=120).contains(&options.seconds) || !(1..=200).contains(&options.max_updates) {
            return Err("invalid stream experiment bounds".into());
        }
        report["reviewed_visual_report"] =
            review_visual_report(&options.reviewed_visual_report, selector)?;
        if let Some(parent) = options
            .out_dir
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&options.out_dir)?;
        created = true;
        let cancel = AtomicBool::new(false);
        let mut session = open(selector)?;
        let info = session.info(&cancel)?;
        if info.build != Some(15157) {
            return Err("requires firmware build 15157".into());
        }
        let serial = info.serial.ok_or("requires exact serial identity")?;
        report["device"] = json!({"serial_id":selector,"vid":0x03f0,"pid":0x1541,"firmware_build":15157,"boot":info.version});
        println!(
            "Bounded diagnostic: at most {} FC04 attempts / {} seconds, 100 ms minimum request spacing. Keep screens stable at announced checkpoints. No keys are sent. Recovery acceptance requires one controlled manual disconnect/reconnect.",
            options.max_updates, options.seconds
        );
        let mut recover = || -> Result<ScreenshotSession, Error> {
            let mut candidate = open(selector)?;
            let current = candidate.info(&cancel)?;
            if current.serial.as_deref() != Some(serial.as_str())
                || current.build != Some(15157)
                || current.version != info.version
            {
                return Err("reconnected device identity differs".into());
            }
            Ok(candidate)
        };
        observe(session, options, &mut report, &mut recover)?;
        Ok(())
    })();
    if let Err(error) = &result {
        report["error"] = json!(error.to_string());
        report["status"] = json!(if created {
            "stream_verification_failed"
        } else {
            "refused"
        });
    }
    if created
        && let Err(error) = fs::write(
            options.out_dir.join("report.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
    {
        eprintln!("cannot save stream report: {error}");
        return super::common::exit::IO;
    }
    println!("{report}");
    if result.is_ok() && report["status"] == "sustained_verification_passed" {
        super::common::exit::OK
    } else {
        super::common::exit::IO
    }
}

fn open(selector: &str) -> Result<ScreenshotSession, Error> {
    let prime = super::remote::open_selected(Some(selector))?;
    if (prime.detected.vendor_id, prime.detected.product_id) != (0x03f0, 0x1541) {
        return Err("requires G1 03f0:1541".into());
    }
    Ok(ScreenshotSession::new(prime))
}

fn increment(report: &mut Value, key: &str) {
    report[key] = json!(report[key].as_u64().unwrap_or(0) + 1);
}

fn full(
    session: &mut ScreenshotSession,
    deadline: Instant,
    path: &Path,
    cancel: &AtomicBool,
    report: &mut Value,
) -> Result<Vec<u8>, Error> {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(Duration::from_secs(5));
    if timeout.is_zero() {
        return Err("stream wall-time budget exhausted before full reference".into());
    }
    increment(report, "full_attempts");
    let capture = session.capture(8, timeout, cancel)?;
    report["full_captures"].as_array_mut().ok_or("missing full capture records")?.push(json!({"file":path.file_name().map(|p|p.to_string_lossy()),"sha256":ppl_compat::sha256_hex(&capture.png),"timings":capture.timings}));
    fs::write(path, &capture.png)?;
    Ok(capture.png)
}

fn observe(
    mut session: ScreenshotSession,
    options: &StreamOptions,
    report: &mut Value,
    recover: &mut impl FnMut() -> Result<ScreenshotSession, Error>,
) -> Result<(), Error> {
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(options.seconds);
    let mut permit = BoundedIncrementalPermit::acknowledge_experimental_fc04(
        options.max_updates,
        Duration::from_secs(options.seconds),
    )?;
    let mut canvas = IncrementalFramebuffer::default();
    let initial = full(
        &mut session,
        deadline,
        &options.out_dir.join("baseline.raw.png"),
        &cancel,
        report,
    )?;
    increment(report, "full_frames");
    let initial_image = canvas.apply(0, None, &initial)?.image;
    let mut prior_checkpoint = initial_image.clone();
    let mut interval_changed = false;
    let mut ordinal = 1;
    let mut records = File::create(options.out_dir.join("updates.jsonl"))?;
    let mut last_start = Instant::now()
        .checked_sub(Duration::from_millis(100))
        .unwrap();
    let mut recovered_checkpoint = false;
    let mut recovery_attempts = 0;
    for index in 1..=options.max_updates {
        let next = last_start + Duration::from_millis(100);
        if next >= deadline {
            break;
        }
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        if Instant::now() >= deadline {
            break;
        }
        last_start = Instant::now();
        increment(report, "fc04_attempts");
        let update = session.verify_incremental_bounded(&mut permit, &cancel);
        let applied = match update {
            Ok(update) => {
                // Evidence I/O failures abort immediately, without reconnecting USB.
                fs::write(
                    options.out_dir.join(format!("update-{index:03}.raw.png")),
                    &update.png,
                )?;
                let origin = match update.placement {
                    Placement::Full => None,
                    Placement::Patch { x, y } => Some((x, y)),
                };
                match canvas.apply(ordinal, origin, &update.png) {
                    Ok(applied) => {
                        increment(report, "valid_updates");
                        if applied.changed {
                            increment(report, "changed_updates");
                            interval_changed = true;
                        }
                        writeln!(
                            records,
                            "{}",
                            json!({"index":index,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"format":4,"origin":origin,"dimensions":[applied.patch.width(),applied.patch.height()],"changed":applied.changed,"sha256":ppl_compat::sha256_hex(&update.png),"timings":update.timings})
                        )?;
                        Ok(())
                    }
                    Err(error) => Err(ppl_device::DeviceError::Protocol(error)),
                }
            }
            Err(error) => Err(error),
        };
        if let Err(error) = applied {
            increment(report, "capture_errors");
            writeln!(
                records,
                "{}",
                json!({"index":index,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"error":error.to_string(),"failure_stage":session.failure_stage()})
            )?;
            // Drop the poisoned session before attempting a fresh exact identity.
            drop(session);
            canvas.restart();
            recovered_checkpoint = false;
            let mut replacement = None;
            while recovery_attempts < 3 && Instant::now() < deadline {
                recovery_attempts += 1;
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(500)),
                );
                if Instant::now() >= deadline {
                    break;
                }
                if let Ok(candidate) = recover() {
                    replacement = Some(candidate);
                    break;
                }
            }
            session =
                replacement.ok_or_else(|| format!("bounded recovery exhausted after: {error}"))?;
            increment(report, "reconnections");
            let png = full(
                &mut session,
                deadline,
                &options.out_dir.join(format!("recovery-{index:03}.raw.png")),
                &cancel,
                report,
            )?;
            increment(report, "full_frames");
            canvas.apply(0, None, &png)?;
            ordinal = 1;
            interval_changed = false;
            continue;
        }
        ordinal += 1;
        if index % 20 == 0 || index == options.max_updates {
            println!(
                "Checkpoint {index}: keep the physical screen stable during the full reference."
            );
            let composed = canvas.image().ok_or("missing canvas")?.clone();
            composed.save_png(&options.out_dir.join(format!("composed-{index:03}.png")))?;
            let reference = full(
                &mut session,
                deadline,
                &options
                    .out_dir
                    .join(format!("reference-{index:03}.raw.png")),
                &cancel,
                report,
            )?;
            increment(report, "full_frames");
            let decoded = PrimeScreenshotDecoder::default()
                .decode(&reference, Some((320, 240)))?
                .0;
            let mismatches = composed
                .pixels()
                .iter()
                .zip(decoded.pixels())
                .filter(|(a, b)| a != b)
                .count();
            let differs_from_initial = *initial_image != decoded;
            let changed = *prior_checkpoint != decoded && interval_changed;
            if changed {
                increment(report, "changed_checkpoints");
            }
            let after_recovery = report["reconnections"].as_u64().unwrap_or(0) > 0;
            report["checkpoints"].as_array_mut().unwrap().push(json!({"update":index,"mismatching_pixels":mismatches,"differs_from_initial":differs_from_initial,"changed_since_previous":changed,"after_recovery":after_recovery}));
            if mismatches > 0 {
                increment(report, "checkpoint_mismatches");
                return Err("incremental/reference mismatch; no automatic success".into());
            }
            recovered_checkpoint |= report["reconnections"].as_u64().unwrap_or(0) > 0;
            // FC08 can refresh the device cache. Adopt its validated pixels before
            // continuing, and test the following FC04 sequence independently.
            prior_checkpoint = std::sync::Arc::new(decoded);
            interval_changed = false;
            canvas.apply(ordinal, None, &reference)?;
            ordinal += 1;
        }
    }
    report["elapsed_seconds"] = json!(started.elapsed().as_secs_f64());
    report["diagnostic_request_spacing_ms"] = json!(100);
    report["headless"] = json!(true);
    report["recovery_verified"] = json!(recovered_checkpoint);
    report["status"] = json!(if acceptance_complete(report, recovered_checkpoint) {
        "sustained_verification_passed"
    } else {
        "stream_inconclusive"
    });
    Ok(())
}

fn acceptance_complete(report: &Value, recovered_checkpoint: bool) -> bool {
    let Some(checkpoints) = report["checkpoints"].as_array() else {
        return false;
    };
    // A wall-time limit or last-request failure must not promote earlier good
    // checkpoints while leaving the final interval/generation unverified.
    let final_checkpoint = checkpoints.last().and_then(|c| c["update"].as_u64());
    recovered_checkpoint
        && report["changed_checkpoints"].as_u64().unwrap_or(0) >= 2
        && checkpoints.len() >= 3
        && final_checkpoint.is_some()
        && final_checkpoint == report["fc04_attempts"].as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_graphics::Grob;
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    struct Scripted {
        messages: VecDeque<Vec<Vec<u8>>>,
        pending: VecDeque<Vec<u8>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        dropped: Arc<AtomicBool>,
    }
    impl Drop for Scripted {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }
    impl ppl_device::Transport for Scripted {
        fn write(&mut self, request: &[u8]) -> ppl_device::Result<usize> {
            self.writes.lock().unwrap().push(request.to_vec());
            self.pending
                .extend(self.messages.pop_front().ok_or_else(|| {
                    ppl_device::DeviceError::Refused("unexpected test request".into())
                })?);
            Ok(request.len())
        }
        fn read_timeout(&mut self, destination: &mut [u8], _: i32) -> ppl_device::Result<usize> {
            if let Some(packet) = self.pending.pop_front() {
                destination[..packet.len()].copy_from_slice(&packet);
                Ok(packet.len())
            } else {
                Ok(0)
            }
        }
    }
    fn wire(word: u16) -> Vec<u8> {
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, 320, 240);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&word.to_le_bytes().repeat(76800))
            .unwrap();
        png
    }
    fn message(format: u8, png: &[u8]) -> Vec<Vec<u8>> {
        let mut message = vec![0xfc, 1];
        message.extend_from_slice(&((png.len() + 7) as u32).to_be_bytes());
        message.extend_from_slice(&[0, 0, format, 255, 255, 255, 255]);
        message.extend_from_slice(png);
        let crc = ppl_device::crc16(&message[8..]);
        message[6..8].copy_from_slice(&crc.to_be_bytes());
        message
            .chunks(63)
            .enumerate()
            .map(|(index, chunk)| {
                let mut packet = vec![index as u8 % 255];
                packet.extend_from_slice(chunk);
                packet.resize(64, 0);
                packet
            })
            .collect()
    }
    fn session(
        messages: Vec<Vec<Vec<u8>>>,
        writes: &Arc<Mutex<Vec<Vec<u8>>>>,
        dropped: &Arc<AtomicBool>,
    ) -> ScreenshotSession {
        ScreenshotSession::new(ppl_device::Prime::with_transport(
            ppl_device::Detected {
                vendor_id: 0x03f0,
                product_id: 0x1541,
                model: "synthetic",
                path: "not-a-device".into(),
                manufacturer: None,
                product: None,
                serial: None,
                release: 0,
                interface: 0,
            },
            Box::new(Scripted {
                messages: messages.into(),
                pending: VecDeque::new(),
                writes: writes.clone(),
                dropped: dropped.clone(),
            }),
        ))
    }
    fn options(directory: &Path) -> StreamOptions {
        StreamOptions {
            acknowledge_experimental_fc04: true,
            reviewed_visual_report: directory.join("report.json"),
            out_dir: directory.to_owned(),
            seconds: 10,
            max_updates: 3,
        }
    }
    fn report() -> Value {
        json!({"full_captures":[],"checkpoints":[],"hardware_verified":false,"streaming_enabled":false})
    }

    #[test]
    fn acceptance_requires_final_interval_and_latest_recovery_checkpoint() {
        let mut evidence = json!({
            "checkpoints":[{"update":20},{"update":40},{"update":60}],
            "changed_checkpoints":2,"fc04_attempts":60,
        });
        assert!(acceptance_complete(&evidence, true));
        assert!(!acceptance_complete(&evidence, false));
        evidence["fc04_attempts"] = json!(61);
        assert!(!acceptance_complete(&evidence, true));
    }

    #[test]
    fn reviewed_report_requires_actual_changed_current_decoder_pixels_and_hashes() {
        let directory = tempfile::tempdir().unwrap();
        let a = wire(0);
        let b = wire(0x7c00);
        fs::write(directory.path().join("baseline.raw.png"), &a).unwrap();
        fs::write(directory.path().join("reference.raw.png"), &b).unwrap();
        Grob::new(320, 240, 0xff0000)
            .save_png(&directory.path().join("composed.png"))
            .unwrap();
        let composed = fs::read(directory.path().join("composed.png")).unwrap();
        let mut report = json!({"operation":"verify_incremental","status":"visual_verification_passed","device":{"vid":0x03f0,"pid":0x1541,"firmware_build":15157,"serial_id":"test-only"},"analysis":{"screen_actually_changed":true,"any_patch_changed":true},"comparison":{"composed_vs_reference":{"mismatching_pixels":0}},"baseline":{"sha256":ppl_compat::sha256_hex(&a)},"reference":{"sha256":ppl_compat::sha256_hex(&b)},"composed":{"sha256":ppl_compat::sha256_hex(&composed)}});
        let path = directory.path().join("report.json");
        fs::write(&path, report.to_string()).unwrap();
        assert!(review_visual_report(&path, "test-only").is_ok());
        assert!(review_visual_report(&path, "another-device").is_err());
        report["status"] = json!("static_verification_passed");
        fs::write(&path, report.to_string()).unwrap();
        assert!(review_visual_report(&path, "test-only").is_err());
        report["status"] = json!("visual_verification_passed");
        fs::write(&path, report.to_string()).unwrap();
        fs::write(directory.path().join("reference.raw.png"), a).unwrap();
        assert!(review_visual_report(&path, "test-only").is_err());
    }

    #[test]
    fn bounded_sequence_checks_reference_and_never_promotes_static_or_short_stream() {
        let directory = tempfile::tempdir().unwrap();
        let a = wire(0);
        let b = wire(0x7c00);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let source = session(
            vec![
                message(8, &a),
                message(4, &b),
                message(4, &b),
                message(4, &b),
                message(8, &b),
            ],
            &writes,
            &dropped,
        );
        let mut evidence = report();
        observe(
            source,
            &options(directory.path()),
            &mut evidence,
            &mut || panic!("no reconnect expected"),
        )
        .unwrap();
        assert_eq!(evidence["fc04_attempts"], 3);
        assert_eq!(evidence["valid_updates"], 3);
        assert_eq!(evidence["changed_updates"], 1);
        assert_eq!(evidence["checkpoints"][0]["mismatching_pixels"], 0);
        assert_eq!(evidence["status"], "stream_inconclusive");
        assert_eq!(evidence["streaming_enabled"], false);
        assert_eq!(
            writes
                .lock()
                .unwrap()
                .iter()
                .map(|w| w[3])
                .collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 8]
        );
    }

    #[test]
    fn crc_failure_drops_session_before_reconnect_rebaselines_and_retains_global_budget() {
        let directory = tempfile::tempdir().unwrap();
        let a = wire(0);
        let b = wire(0x7c00);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let mut bad = message(4, &b);
        bad[0][7] ^= 1;
        let source = session(vec![message(8, &a), message(4, &b), bad], &writes, &dropped);
        let mut evidence = report();
        let mut reopens = 0;
        observe(
            source,
            &options(directory.path()),
            &mut evidence,
            &mut || {
                assert!(dropped.load(Ordering::SeqCst));
                reopens += 1;
                Ok(session(
                    vec![message(8, &b), message(4, &b), message(8, &b)],
                    &writes,
                    &Arc::new(AtomicBool::new(false)),
                ))
            },
        )
        .unwrap();
        assert_eq!(reopens, 1);
        assert_eq!(evidence["fc04_attempts"], 3);
        assert_eq!(evidence["valid_updates"], 2);
        assert_eq!(evidence["capture_errors"], 1);
        assert_eq!(evidence["recovery_verified"], true);
        assert_eq!(evidence["status"], "stream_inconclusive");
        assert_eq!(
            writes
                .lock()
                .unwrap()
                .iter()
                .map(|w| w[3])
                .collect::<Vec<_>>(),
            vec![8, 4, 4, 8, 4, 8]
        );
    }

    #[test]
    fn checkpoint_mismatch_fails_instead_of_refreshing_over_missing_changes() {
        let directory = tempfile::tempdir().unwrap();
        let a = wire(0);
        let b = wire(0x7c00);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let source = session(
            vec![
                message(8, &a),
                message(4, &b),
                message(4, &b),
                message(4, &b),
                message(8, &a),
            ],
            &writes,
            &dropped,
        );
        let mut evidence = report();
        assert!(
            observe(
                source,
                &options(directory.path()),
                &mut evidence,
                &mut || panic!("mismatch must not reconnect")
            )
            .is_err()
        );
        assert_eq!(evidence["checkpoints"][0]["mismatching_pixels"], 76800);
        assert_eq!(evidence["checkpoint_mismatches"], 1);
        assert_eq!(evidence["streaming_enabled"], false);
    }

    #[test]
    fn synthetic_multi_transition_recovery_acceptance_never_grants_production_permission() {
        let directory = tempfile::tempdir().unwrap();
        let a = wire(0);
        let b = wire(0x7c00);
        let c = wire(0x03e0);
        let d = wire(0x001f);
        let writes = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let mut messages = vec![message(8, &a)];
        messages.extend((0..20).map(|_| message(4, &b)));
        messages.push(message(8, &b));
        messages.extend((0..20).map(|_| message(4, &c)));
        messages.push(message(8, &c));
        let mut bad = message(4, &c);
        bad[0][7] ^= 1;
        messages.push(bad);
        let source = session(messages, &writes, &dropped);
        let mut limits = options(directory.path());
        limits.max_updates = 60;
        limits.seconds = 120;
        let mut evidence = report();
        let mut reopens = 0;
        observe(source, &limits, &mut evidence, &mut || {
            assert!(dropped.load(Ordering::SeqCst));
            reopens += 1;
            let mut messages = vec![message(8, &c)];
            messages.extend((0..19).map(|_| message(4, &d)));
            messages.push(message(8, &d));
            Ok(session(
                messages,
                &writes,
                &Arc::new(AtomicBool::new(false)),
            ))
        })
        .unwrap();
        assert_eq!(reopens, 1);
        assert_eq!(evidence["fc04_attempts"], 60);
        assert_eq!(evidence["valid_updates"], 59);
        assert_eq!(evidence["changed_checkpoints"], 3);
        assert_eq!(evidence["checkpoints"].as_array().unwrap().len(), 3);
        assert_eq!(evidence["status"], "sustained_verification_passed");
        assert_eq!(evidence["hardware_verified"], false);
        assert_eq!(evidence["streaming_enabled"], false);
        assert!(!ppl_device::screenshot::update::incremental_profiles_available());
    }
}
