//! Explicitly acknowledged single-request probe and visual-change verification.
use ppl_device::screenshot::{
    ScreenshotSession,
    update::{IncrementalProbePermit, IncrementalVerifyPermit, Placement},
};
use ppl_graphics::incremental::IncrementalFramebuffer;
use serde_json::{Value, json};
use std::{
    io::BufRead,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(super) fn run(selector: Option<&str>, acknowledged: bool, directory: &Path, ipc: bool) -> u8 {
    let mut report = json!({"schema_version":1,"operation":"probe_incremental","status":"refused","hardware_verified":false,"streaming_enabled":false,"fc04_attempts":0,"timestamp_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()});
    let mut created = false;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if !acknowledged || selector.is_none() || ipc {
            return Err("requires --device, --acknowledge-experimental-fc04, and a direct exclusive connection".into());
        }
        if let Some(parent) = directory.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.recursive(false).create(directory)?;
        created = true;
        let prime = super::remote::open_selected(selector)?;
        if (prime.detected.vendor_id, prime.detected.product_id) != (0x03f0, 0x1541) {
            return Err("probe restricted to 03f0:1541".into());
        }
        let cancel = AtomicBool::new(false);
        let mut session = ScreenshotSession::new(prime);
        let info = session.info(&cancel)?;
        if info.build != Some(15157) || info.serial.is_none() {
            return Err("probe requires identified firmware build 15157".into());
        }
        report["device"] = json!({"vid":0x03f0,"pid":0x1541,"firmware_build":info.build,"boot":info.version,"serial_id":ppl_compat::serial_fingerprint(info.serial.as_deref().unwrap())});
        capture_probe(&mut session, directory, &mut report)?;
        Ok(())
    })();
    let success = result.is_ok();
    if let Err(e) = result {
        report["error"] = json!(e.to_string());
        report["status"] = json!(if report["fc04_attempts"] == 1 {
            "probe_failed"
        } else {
            "refused"
        });
    }
    if created && let Err(e) = write_report(directory, &report) {
        eprintln!("error saving probe report: {e}");
        println!("{report}");
        return super::common::exit::IO;
    }
    println!("{report}");
    if success {
        super::common::exit::OK
    } else {
        super::common::exit::IO
    }
}

pub(super) fn run_verify(
    selector: Option<&str>,
    acknowledged: bool,
    directory: &Path,
    updates: u32,
    is_static: bool,
    ipc: bool,
) -> u8 {
    let mut report = json!({
        "schema_version": 1,
        "operation": "verify_incremental",
        "status": "refused",
        "scenario": if is_static { "static" } else { "changed" },
        "hardware_verified": false,
        "streaming_enabled": false,
        "fc04_attempts": 0,
        "timestamp_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
    });
    let mut created = false;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if !acknowledged || selector.is_none() || ipc {
            return Err("requires --device, --acknowledge-experimental-fc04, and a direct exclusive connection".into());
        }
        if updates == 0 || updates > 50 {
            return Err("updates must be between 1 and 50".into());
        }
        if let Some(parent) = directory.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.recursive(false).create(directory)?;
        created = true;
        let prime = super::remote::open_selected(selector)?;
        if (prime.detected.vendor_id, prime.detected.product_id) != (0x03f0, 0x1541) {
            return Err("verification restricted to G1 03f0:1541".into());
        }
        let cancel = AtomicBool::new(false);
        let mut session = ScreenshotSession::new(prime);
        let info = session.info(&cancel)?;
        if info.build != Some(15157) || info.serial.is_none() {
            return Err("verification requires identified firmware build 15157".into());
        }
        report["device"] = json!({
            "vid": 0x03f0,
            "pid": 0x1541,
            "firmware_build": info.build,
            "boot": info.version,
            "serial_id": ppl_compat::serial_fingerprint(info.serial.as_deref().unwrap()),
        });
        capture_verify(&mut session, directory, updates, is_static, &mut report)?;
        Ok(())
    })();
    let success = result.is_ok();
    if let Err(e) = result {
        report["error"] = json!(e.to_string());
        if report["status"] == "refused" {
            report["status"] = json!(if report["fc04_attempts"].as_u64().unwrap_or(0) > 0 {
                "verification_failed"
            } else {
                "refused"
            });
        }
    }
    if created && let Err(e) = write_report(directory, &report) {
        eprintln!("error saving verification report: {e}");
        println!("{report}");
        return super::common::exit::IO;
    }
    println!("{report}");
    let passed = matches!(
        report["status"].as_str(),
        Some("visual_verification_passed" | "static_verification_passed")
    );
    if success && passed {
        super::common::exit::OK
    } else {
        super::common::exit::IO
    }
}

fn write_report(directory: &Path, report: &Value) -> std::io::Result<()> {
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(report)?,
    )
}

fn capture_probe(
    session: &mut ScreenshotSession,
    directory: &Path,
    report: &mut Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let cancel = AtomicBool::new(false);
    let baseline_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let baseline = session.capture(8, Duration::from_secs(5), &cancel)?;
    std::fs::write(directory.join("baseline.raw.png"), &baseline.png)?;
    let mut canvas = IncrementalFramebuffer::default();
    let initial = canvas.apply(0, None, &baseline.png)?;
    initial.image.save_png(&directory.join("baseline.png"))?;
    report["baseline"] = json!({"request_unix_ms":baseline_timestamp,"sha256":ppl_compat::sha256_hex(&baseline.png),"timings":baseline.timings});
    // Persist baseline provenance before the experimental request.
    write_report(directory, report)?;
    report["fc04_attempts"] = json!(1);
    report["request_hidapi_hex"] = json!("0000fc04");
    let mut permit = IncrementalProbePermit::acknowledge_experimental_fc04();
    let update = match session.probe_incremental(&mut permit, &cancel) {
        Ok(update) => update,
        Err(error) => {
            report["response_class"] = json!(match &error {
                ppl_device::DeviceError::Timeout(_) => "silent_or_incomplete",
                ppl_device::DeviceError::Protocol(_) => "unexpected_or_invalid",
                _ => "transport_failure",
            });
            return Err(error.into());
        }
    };
    std::fs::write(directory.join("update.raw.png"), &update.png)?;
    let origin = match update.placement {
        Placement::Full => None,
        Placement::Patch { x, y } => Some((x, y)),
    };
    report["response_class"] = json!(if origin.is_none() {
        "full_origin"
    } else {
        "coordinate_update"
    });
    report["response_format"] = json!(update.format);
    let applied = canvas.apply(1, origin, &update.png)?;
    applied.image.save_png(&directory.join("composed.png"))?;
    report["status"] = json!("valid_response_unverified_visual_result");
    report["update"] = json!({"width":u32::from_be_bytes(update.png[16..20].try_into().unwrap()),"height":u32::from_be_bytes(update.png[20..24].try_into().unwrap()),"origin":origin,"sha256":ppl_compat::sha256_hex(&update.png),"changed":applied.changed,"timings":update.timings,"composition_ms":applied.composition.as_secs_f64()*1000.0});
    Ok(())
}

fn capture_verify(
    session: &mut ScreenshotSession,
    directory: &Path,
    updates: u32,
    is_static: bool,
    report: &mut Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdin = std::io::stdin().lock();
    let reader: Option<&mut dyn BufRead> = if is_static { None } else { Some(&mut stdin) };
    capture_verify_inner(session, directory, updates, reader, report)
}

fn capture_verify_inner(
    session: &mut ScreenshotSession,
    directory: &Path,
    updates: u32,
    mut prompt_reader: Option<&mut dyn BufRead>,
    report: &mut Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let cancel = AtomicBool::new(false);
    let baseline_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let baseline = session.capture(8, Duration::from_secs(5), &cancel)?;
    std::fs::write(directory.join("baseline.raw.png"), &baseline.png)?;
    let mut canvas = IncrementalFramebuffer::default();
    let initial = canvas.apply(0, None, &baseline.png)?;
    initial.image.save_png(&directory.join("baseline.png"))?;
    report["baseline"] = json!({
        "request_unix_ms": baseline_timestamp,
        "sha256": ppl_compat::sha256_hex(&baseline.png),
        "timings": baseline.timings,
    });
    write_report(directory, report)?;

    let is_static = prompt_reader.is_none();
    if let Some(reader) = &mut prompt_reader {
        println!("Baseline captured successfully.");
        println!(
            "Please change the physical calculator screen now (e.g. press Apps, Symb, or open Program Catalog)."
        );
        println!(
            "Press Enter when the screen has changed to capture incremental updates and verify..."
        );
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(
                "manual screen-change confirmation reached EOF; no FC04 requests sent".into(),
            );
        }
    } else {
        println!("Running static screen verification (no screen change expected)...");
    }

    let mut permit = IncrementalVerifyPermit::acknowledge_experimental_fc04(updates)?;
    let mut updates_list = Vec::new();
    let mut any_changed = false;
    let mut fc04_count = 0u64;
    report["request_hidapi_hex"] = json!("0000fc04");

    for i in 1..=updates {
        fc04_count += 1;
        report["fc04_attempts"] = json!(fc04_count);
        let update = match session.verify_incremental(&mut permit, &cancel) {
            Ok(u) => u,
            Err(e) => {
                report["error"] = json!(format!("update {i} failed: {e}"));
                return Err(e.into());
            }
        };
        std::fs::write(
            directory.join(format!("update-{:02}.raw.png", i)),
            &update.png,
        )?;
        let origin = match update.placement {
            Placement::Full => None,
            Placement::Patch { x, y } => Some((x, y)),
        };
        let applied = canvas.apply(i as u64, origin, &update.png)?;
        applied
            .patch
            .save_png(&directory.join(format!("update-{:02}-patch.png", i)))?;
        if applied.changed {
            any_changed = true;
        }
        let w = applied.patch.width();
        let h = applied.patch.height();
        updates_list.push(json!({
            "index": i,
            "format": update.format,
            "placement": if origin.is_none() { "full" } else { "patch" },
            "origin": origin,
            "width": w,
            "height": h,
            "changed": applied.changed,
            "sha256": ppl_compat::sha256_hex(&update.png),
            "timings": update.timings,
            "composition_ms": applied.composition.as_secs_f64() * 1000.0,
        }));
    }

    report["incremental_sequence"] = json!({
        "updates_requested": updates,
        "updates_completed": fc04_count,
        "any_changed": any_changed,
        "updates": updates_list,
    });

    let composed_grob = canvas
        .image()
        .ok_or("missing composed framebuffer")?
        .clone();
    composed_grob.save_png(&directory.join("composed.png"))?;
    let composed_bytes = std::fs::read(directory.join("composed.png"))?;
    report["composed"] = json!({
        "sha256": ppl_compat::sha256_hex(&composed_bytes),
        "dimensions": [composed_grob.width(), composed_grob.height()],
    });

    println!("Capturing independent full-frame reference...");
    let ref_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let reference = session.capture(8, Duration::from_secs(5), &cancel)?;
    std::fs::write(directory.join("reference.raw.png"), &reference.png)?;
    let (ref_grob, _ref_decode) =
        ppl_graphics::PrimeScreenshotDecoder::default().decode(&reference.png, Some((320, 240)))?;
    ref_grob.save_png(&directory.join("reference.png"))?;
    report["reference"] = json!({
        "request_unix_ms": ref_timestamp,
        "sha256": ppl_compat::sha256_hex(&reference.png),
        "timings": reference.timings,
    });

    let comp_vs_ref = ppl_graphics::compare_screens_detailed(&composed_grob, &ref_grob)
        .ok_or("dimensions mismatch on composed vs reference")?;
    let base_vs_ref = ppl_graphics::compare_screens_detailed(&initial.image, &ref_grob)
        .ok_or("dimensions mismatch on baseline vs reference")?;
    let base_vs_comp = ppl_graphics::compare_screens_detailed(&initial.image, &composed_grob)
        .ok_or("dimensions mismatch on baseline vs composed")?;

    report["comparison"] = json!({
        "composed_vs_reference": diff_to_json(&comp_vs_ref),
        "baseline_vs_reference": diff_to_json(&base_vs_ref),
        "baseline_vs_composed": diff_to_json(&base_vs_comp),
    });

    let screen_actually_changed = base_vs_ref.mismatching_pixels > 0;
    let reconstruction_exact = comp_vs_ref.mismatching_pixels == 0;

    let (status, diag) = if is_static {
        if comp_vs_ref.mismatching_pixels == 0 && base_vs_ref.mismatching_pixels == 0 {
            (
                "static_verification_passed",
                "Static screen: composed and reference exactly match baseline with 0 mismatches.",
            )
        } else {
            (
                "static_verification_failed",
                "Static screen: unexpected pixel changes detected during static verification.",
            )
        }
    } else if screen_actually_changed && reconstruction_exact {
        (
            "visual_verification_passed",
            "Screen change verified: composed incremental framebuffer exactly matches physical reference.",
        )
    } else if !screen_actually_changed {
        (
            "screen_did_not_change",
            "Screen did not change: baseline and reference frames are identical.",
        )
    } else {
        (
            "reconstruction_mismatch",
            "Reconstruction mismatch: composed incremental framebuffer does not match physical reference.",
        )
    };

    let mut possible_causes = Vec::new();
    if !screen_actually_changed {
        possible_causes.push("operator_did_not_change_screen_or_input_not_registered");
    } else if !any_changed {
        possible_causes.push("fc04_updates_missed_screen_change_or_change_occurred_after_updates");
    } else if comp_vs_ref.mismatching_pixels > 0 {
        if comp_vs_ref.mismatching_pixels < base_vs_ref.mismatching_pixels {
            possible_causes.push("incomplete_patch_sequence_more_updates_needed");
            possible_causes.push("screen_animation_or_cursor_blink_during_reference_capture");
        } else {
            possible_causes.push("patch_coordinate_or_pixel_corruption");
            possible_causes.push("color_conversion_or_palette_mismatch");
        }
    }

    report["status"] = json!(status);
    report["analysis"] = json!({
        "screen_actually_changed": screen_actually_changed,
        "reconstruction_exact": reconstruction_exact,
        "any_patch_changed": any_changed,
        "diagnosis": diag,
        "possible_causes": possible_causes,
    });

    Ok(())
}

fn diff_to_json(d: &ppl_graphics::ScreenDiffDetailed) -> Value {
    json!({
        "compared_pixels": d.total_pixels,
        "matching_pixels": d.matching_pixels,
        "mismatching_pixels": d.mismatching_pixels,
        "mismatch_percentage": d.mismatch_percentage,
        "first_mismatch": d.first_mismatch.map(|(x, y)| json!([x, y])),
        "mismatch_bounds": d.mismatch_bounds.map(|[min_x, min_y, max_x, max_y]| json!({
            "min_x": min_x,
            "min_y": min_y,
            "max_x": max_x,
            "max_y": max_y,
            "width": max_x.saturating_sub(min_x) + 1,
            "height": max_y.saturating_sub(min_y) + 1,
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };
    struct Fake {
        replies: VecDeque<Vec<Vec<u8>>>,
        pending: VecDeque<Vec<u8>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl ppl_device::Transport for Fake {
        fn write(&mut self, request: &[u8]) -> ppl_device::Result<usize> {
            self.writes.lock().unwrap().push(request.to_vec());
            self.pending.extend(self.replies.pop_front().unwrap());
            Ok(request.len())
        }
        fn read_timeout(&mut self, out: &mut [u8], _: i32) -> ppl_device::Result<usize> {
            if let Some(reply) = self.pending.pop_front() {
                out[..reply.len()].copy_from_slice(&reply);
                Ok(reply.len())
            } else {
                Ok(0)
            }
        }
    }
    fn response(format: u8, origin: (u16, u16), png: &[u8]) -> Vec<Vec<u8>> {
        let mut raw = vec![0xfc, 1];
        raw.extend_from_slice(&((png.len() + 7) as u32).to_be_bytes());
        raw.extend_from_slice(&[0, 0, format]);
        raw.extend_from_slice(&origin.0.to_le_bytes());
        raw.extend_from_slice(&origin.1.to_le_bytes());
        raw.extend_from_slice(png);
        let crc = ppl_device::crc16(&raw[8..]);
        raw[6..8].copy_from_slice(&crc.to_be_bytes());
        raw.chunks(63)
            .enumerate()
            .map(|(i, b)| {
                let mut r = vec![(i % 255) as u8];
                r.extend_from_slice(b);
                r
            })
            .collect()
    }
    #[test]
    fn bounded_probe_composes_synthetic_bottom_right_patch_and_preserves_baseline_on_failure() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        // Independent PNG fixture: raw scanline [filter=0, RGB555 red LE=00 7c].
        let patch: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            16, 0, 0, 0, 0, 106, 238, 71, 22, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 168,
            1, 0, 0, 127, 0, 125, 136, 91, 148, 9, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ];
        for corrupt in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let writes = Arc::new(Mutex::new(Vec::new()));
            let mut delta = response(4, (319, 239), patch);
            if corrupt {
                delta[0][7] ^= 1;
            }
            let transport = Fake {
                replies: vec![response(8, (65535, 65535), baseline), delta].into(),
                pending: VecDeque::new(),
                writes: writes.clone(),
            };
            let detected = ppl_device::Detected {
                vendor_id: 0x03f0,
                product_id: 0x1541,
                model: "synthetic",
                path: "not-physical".into(),
                manufacturer: None,
                product: None,
                serial: None,
                release: 0,
                interface: 0,
            };
            let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
                detected,
                Box::new(transport),
            ));
            let mut report = json!({"hardware_verified":false,"streaming_enabled":false});
            let result = capture_probe(&mut session, directory.path(), &mut report);
            assert_eq!(
                *writes.lock().unwrap(),
                vec![vec![0, 0, 0xfc, 8], vec![0, 0, 0xfc, 4]]
            );
            assert!(directory.path().join("baseline.raw.png").is_file());
            assert!(!report["hardware_verified"].as_bool().unwrap());
            if corrupt {
                assert!(result.is_err());
                assert!(!directory.path().join("composed.png").exists());
            } else {
                result.unwrap();
                let (mut expected, _) = ppl_graphics::PrimeScreenshotDecoder::default()
                    .decode(baseline, Some((320, 240)))
                    .unwrap();
                expected.pixels_mut()[76799] = 0xff0000;
                let actual = ppl_graphics::Grob::read_png(
                    &std::fs::read(directory.path().join("composed.png")).unwrap(),
                )
                .unwrap();
                assert_eq!(actual.pixels(), expected.pixels());
            }
        }
    }

    fn encode_gray16(w: usize, h: usize, pixels: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, w as u32, h as u32);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Sixteen);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(
                &pixels
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        bytes
    }

    #[test]
    fn manual_confirmation_eof_refuses_before_fc04() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = Fake {
            replies: vec![response(8, (65535, 65535), baseline)].into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = ppl_device::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "synthetic",
            path: "not-physical".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
            detected,
            Box::new(transport),
        ));
        let directory = tempfile::tempdir().unwrap();
        let mut prompt = std::io::Cursor::new(Vec::<u8>::new());
        let mut report = json!({"fc04_attempts":0});
        let error = capture_verify_inner(
            &mut session,
            directory.path(),
            5,
            Some(&mut prompt),
            &mut report,
        )
        .unwrap_err();
        assert!(error.to_string().contains("confirmation reached EOF"));
        assert_eq!(*writes.lock().unwrap(), vec![vec![0, 0, 0xfc, 8]]);
        assert_eq!(report["fc04_attempts"], 0);
    }

    #[test]
    fn static_screen_verification_passes_with_zero_mismatches() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        let patch = encode_gray16(1, 1, &[0x0000]); // black pixel matching (0,0)
        let directory = tempfile::tempdir().unwrap();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = Fake {
            replies: vec![
                response(8, (65535, 65535), baseline),
                response(4, (0, 0), &patch),
                response(4, (0, 0), &patch),
                response(8, (65535, 65535), baseline),
            ]
            .into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = ppl_device::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "synthetic",
            path: "not-physical".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
            detected,
            Box::new(transport),
        ));
        let mut report = json!({
            "schema_version": 1,
            "operation": "verify_incremental",
            "scenario": "static",
            "hardware_verified": false,
            "streaming_enabled": false,
        });
        let result = capture_verify_inner(&mut session, directory.path(), 2, None, &mut report);
        assert!(result.is_ok());
        assert_eq!(report["status"], "static_verification_passed");
        assert_eq!(
            report["comparison"]["composed_vs_reference"]["mismatching_pixels"],
            0
        );
        assert_eq!(
            report["comparison"]["baseline_vs_reference"]["mismatching_pixels"],
            0
        );
        assert_eq!(
            report["analysis"]["screen_actually_changed"].as_bool(),
            Some(false)
        );
        assert_eq!(
            report["analysis"]["reconstruction_exact"].as_bool(),
            Some(true)
        );
        assert_eq!(
            *writes.lock().unwrap(),
            vec![
                vec![0, 0, 0xfc, 8],
                vec![0, 0, 0xfc, 4],
                vec![0, 0, 0xfc, 4],
                vec![0, 0, 0xfc, 8],
            ]
        );
        assert!(directory.path().join("baseline.raw.png").is_file());
        assert!(directory.path().join("baseline.png").is_file());
        assert!(directory.path().join("update-01.raw.png").is_file());
        assert!(directory.path().join("update-01-patch.png").is_file());
        assert!(directory.path().join("update-02.raw.png").is_file());
        assert!(directory.path().join("update-02-patch.png").is_file());
        assert!(directory.path().join("composed.png").is_file());
        assert!(directory.path().join("reference.raw.png").is_file());
        assert!(directory.path().join("reference.png").is_file());
        assert!(directory.path().join("report.json").is_file());
    }

    #[test]
    fn changed_screen_verification_detects_change_and_exact_match() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        let patch = encode_gray16(1, 1, &[0x7c00]); // Red pixel at (319, 239)
        // Reference frame: 320x240 with pixel (319, 239) modified to 0x7c00
        let (mut ref_grob, _) = ppl_graphics::PrimeScreenshotDecoder::default()
            .decode(baseline, Some((320, 240)))
            .unwrap();
        ref_grob.set(319, 239, 0x00FF0000);
        let mut ref_pixels = vec![0u16; 76800];
        // decode baseline samples to u16
        for y in 0..240 {
            for x in 0..320 {
                let px = ref_grob.pixels()[y * 320 + x];
                let r = ((px >> 16) & 0xFF) as u16 >> 3;
                let g = ((px >> 8) & 0xFF) as u16 >> 3;
                let b = (px & 0xFF) as u16 >> 3;
                ref_pixels[y * 320 + x] = (r << 10) | (g << 5) | b;
            }
        }
        let reference_png = encode_gray16(320, 240, &ref_pixels);

        let directory = tempfile::tempdir().unwrap();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = Fake {
            replies: vec![
                response(8, (65535, 65535), baseline),
                response(4, (319, 239), &patch),
                response(8, (65535, 65535), &reference_png),
            ]
            .into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = ppl_device::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "synthetic",
            path: "not-physical".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
            detected,
            Box::new(transport),
        ));
        let mut report = json!({
            "schema_version": 1,
            "operation": "verify_incremental",
            "scenario": "changed",
            "hardware_verified": false,
            "streaming_enabled": false,
        });
        let mut stdin_sim = std::io::Cursor::new(b"\n");
        let result = capture_verify_inner(
            &mut session,
            directory.path(),
            1,
            Some(&mut stdin_sim),
            &mut report,
        );
        assert!(result.is_ok());
        assert_eq!(report["status"], "visual_verification_passed");
        assert_eq!(
            report["comparison"]["composed_vs_reference"]["mismatching_pixels"],
            0
        );
        assert_eq!(
            report["comparison"]["baseline_vs_reference"]["mismatching_pixels"],
            1
        );
        assert_eq!(
            report["analysis"]["screen_actually_changed"].as_bool(),
            Some(true)
        );
        assert_eq!(
            report["analysis"]["reconstruction_exact"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn changed_screen_verification_detects_reconstruction_mismatch() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        let patch = encode_gray16(1, 1, &[0x7c00]); // Red pixel at (319, 239)
        // Reference frame has blue pixel instead of red at (319, 239)
        let (mut ref_grob, _) = ppl_graphics::PrimeScreenshotDecoder::default()
            .decode(baseline, Some((320, 240)))
            .unwrap();
        ref_grob.set(319, 239, 0x000000FF);
        let mut ref_pixels = vec![0u16; 76800];
        for y in 0..240 {
            for x in 0..320 {
                let px = ref_grob.pixels()[y * 320 + x];
                let r = ((px >> 16) & 0xFF) as u16 >> 3;
                let g = ((px >> 8) & 0xFF) as u16 >> 3;
                let b = (px & 0xFF) as u16 >> 3;
                ref_pixels[y * 320 + x] = (r << 10) | (g << 5) | b;
            }
        }
        let reference_png = encode_gray16(320, 240, &ref_pixels);

        let directory = tempfile::tempdir().unwrap();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = Fake {
            replies: vec![
                response(8, (65535, 65535), baseline),
                response(4, (319, 239), &patch),
                response(8, (65535, 65535), &reference_png),
            ]
            .into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = ppl_device::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "synthetic",
            path: "not-physical".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
            detected,
            Box::new(transport),
        ));
        let mut report = json!({
            "schema_version": 1,
            "operation": "verify_incremental",
            "scenario": "changed",
            "hardware_verified": false,
            "streaming_enabled": false,
        });
        let mut stdin_sim = std::io::Cursor::new(b"\n");
        let result = capture_verify_inner(
            &mut session,
            directory.path(),
            1,
            Some(&mut stdin_sim),
            &mut report,
        );
        assert!(result.is_ok());
        assert_eq!(report["status"], "reconstruction_mismatch");
        assert_eq!(
            report["comparison"]["composed_vs_reference"]["mismatching_pixels"],
            1
        );
        assert_eq!(
            report["comparison"]["composed_vs_reference"]["first_mismatch"],
            json!([319, 239])
        );
        assert_eq!(
            report["analysis"]["possible_causes"],
            json!([
                "patch_coordinate_or_pixel_corruption",
                "color_conversion_or_palette_mismatch"
            ])
        );
    }

    #[test]
    fn changed_screen_verification_fails_closed_when_screen_did_not_change() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        let patch = encode_gray16(1, 1, &[0x0000]);
        let directory = tempfile::tempdir().unwrap();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = Fake {
            replies: vec![
                response(8, (65535, 65535), baseline),
                response(4, (0, 0), &patch),
                response(8, (65535, 65535), baseline),
            ]
            .into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = ppl_device::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "synthetic",
            path: "not-physical".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
            detected,
            Box::new(transport),
        ));
        let mut report = json!({
            "schema_version": 1,
            "operation": "verify_incremental",
            "scenario": "changed",
            "hardware_verified": false,
            "streaming_enabled": false,
        });
        let mut stdin_sim = std::io::Cursor::new(b"\n");
        let result = capture_verify_inner(
            &mut session,
            directory.path(),
            1,
            Some(&mut stdin_sim),
            &mut report,
        );
        assert!(result.is_ok());
        assert_eq!(report["status"], "screen_did_not_change");
        assert_eq!(
            report["comparison"]["baseline_vs_reference"]["mismatching_pixels"],
            0
        );
        assert_eq!(
            report["analysis"]["possible_causes"],
            json!(["operator_did_not_change_screen_or_input_not_registered"])
        );
    }

    #[test]
    fn multi_patch_sequence_with_overlapping_updates_passes_verification() {
        let baseline = include_bytes!("../../../../tests/fixtures/prime-rgb555-le.png");
        // Patch 1: 10x10 green (0x03e0) at (10, 20)
        let patch1 = encode_gray16(10, 10, &[0x03e0; 100]);
        // Patch 2: 10x10 blue (0x001f) at (15, 25) - overlaps (15..20, 25..30)
        let patch2 = encode_gray16(10, 10, &[0x001f; 100]);
        // Patch 3: 1x1 unchanged at (0, 0)
        let patch3 = encode_gray16(1, 1, &[0x0000]);

        // Build expected reference frame
        let (mut ref_grob, _) = ppl_graphics::PrimeScreenshotDecoder::default()
            .decode(baseline, Some((320, 240)))
            .unwrap();
        // apply patch 1: (10..20, 20..30) green (0x0000FF00)
        for y in 20..30 {
            for x in 10..20 {
                ref_grob.set(x, y, 0x0000FF00);
            }
        }
        // apply patch 2: (15..25, 25..35) blue (0x000000FF)
        for y in 25..35 {
            for x in 15..25 {
                ref_grob.set(x, y, 0x000000FF);
            }
        }
        let mut ref_pixels = vec![0u16; 76800];
        for y in 0..240 {
            for x in 0..320 {
                let px = ref_grob.pixels()[y * 320 + x];
                let r = ((px >> 16) & 0xFF) as u16 >> 3;
                let g = ((px >> 8) & 0xFF) as u16 >> 3;
                let b = (px & 0xFF) as u16 >> 3;
                ref_pixels[y * 320 + x] = (r << 10) | (g << 5) | b;
            }
        }
        let reference_png = encode_gray16(320, 240, &ref_pixels);

        let directory = tempfile::tempdir().unwrap();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let transport = Fake {
            replies: vec![
                response(8, (65535, 65535), baseline),
                response(4, (10, 20), &patch1),
                response(4, (15, 25), &patch2),
                response(4, (0, 0), &patch3),
                response(8, (65535, 65535), &reference_png),
            ]
            .into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = ppl_device::Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "synthetic",
            path: "not-physical".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        let mut session = ScreenshotSession::new(ppl_device::Prime::with_transport(
            detected,
            Box::new(transport),
        ));
        let mut report = json!({
            "schema_version": 1,
            "operation": "verify_incremental",
            "scenario": "changed",
            "hardware_verified": false,
            "streaming_enabled": false,
        });
        let mut stdin_sim = std::io::Cursor::new(b"\n");
        let result = capture_verify_inner(
            &mut session,
            directory.path(),
            3,
            Some(&mut stdin_sim),
            &mut report,
        );
        assert!(result.is_ok());
        assert_eq!(report["status"], "visual_verification_passed");
        assert_eq!(
            report["comparison"]["composed_vs_reference"]["mismatching_pixels"],
            0
        );
        assert!(
            report["comparison"]["baseline_vs_reference"]["mismatching_pixels"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(report["composed"]["dimensions"], json!([320, 240]));
        assert!(report["composed"]["sha256"].is_string());
        assert_eq!(report["request_hidapi_hex"], "0000fc04");
        assert_eq!(
            report["incremental_sequence"]["updates"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }
}
