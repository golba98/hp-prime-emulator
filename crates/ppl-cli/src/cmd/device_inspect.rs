//! Conservative screenshot observations. OCR text is evidence from an optional
//! local process, never a structured physical compiler result or navigation proof.
use super::device_control::{base, failure};
use ppl_device::control::Response;
use ppl_graphics::Grob;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq)]
struct Word {
    text: String,
    confidence: f64,
    x: u32,
    y: u32,
}
fn parse_tsv(tsv: &str) -> Result<Vec<Word>, String> {
    if tsv.len() > 1024 * 1024 {
        return Err("OCR output exceeds limit".into());
    }
    let mut words = vec![];
    for row in tsv.lines().skip(1) {
        let fields: Vec<_> = row.splitn(12, '\t').collect();
        if fields.len() != 12 {
            return Err("malformed OCR TSV row".into());
        }
        if fields[0] != "5" || fields[11].trim().is_empty() {
            continue;
        }
        let confidence: f64 = fields[10].parse().map_err(|_| "invalid OCR confidence")?;
        let x = fields[6].parse().map_err(|_| "invalid OCR coordinate")?;
        let y = fields[7].parse().map_err(|_| "invalid OCR coordinate")?;
        if !confidence.is_finite() || !(0.0..=100.0).contains(&confidence) {
            return Err("invalid OCR confidence".into());
        }
        words.push(Word {
            text: fields[11].into(),
            confidence,
            x,
            y,
        });
        if words.len() > 4096 {
            return Err("too many OCR words".into());
        }
    }
    Ok(words)
}
fn dialog_anchor(image: &Grob) -> bool {
    // This is the observed English G1 color/layout profile, not a universal
    // error signature. Text and a second fresh observation are also required.
    let mut fill = 0;
    let mut icon = 0;
    for y in 70..170 {
        for x in 35..285 {
            let c = image.get(x, y).unwrap_or(0);
            if c == 0x84c321 {
                fill += 1;
            }
            let (r, g, b) = ppl_graphics::channels(c);
            if (45..105).contains(&x) && b > 120 && r < 100 && g < 100 {
                icon += 1;
            }
        }
    }
    fill >= 2000 && icon >= 100
}
fn classify(image: &Grob, words: &[Word]) -> Value {
    let text = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut result = json!({"state":"unknown","confidence":null,"confidence_kind":"heuristic_not_probability",
        "text":text,"diagnostic":null,"recognition_profile":"english_g1_observed_layout_v1"});
    let modal = dialog_anchor(image);
    let tokens: Vec<_> = words.iter().map(|w| w.text.to_ascii_lowercase()).collect();
    if modal {
        for i in 0..tokens.len().saturating_sub(5) {
            if tokens[i..i + 4] == ["syntax", "error", "in", "program"]
                && tokens[i + 4].trim_end_matches(':') == "line"
                && words[i..i + 6].iter().all(|w| w.confidence >= 90.)
            {
                let n = &tokens[i + 5];
                if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
                    if let Ok(line) = n.parse::<u32>() {
                        if line > 0 {
                            let message = words[i..i + 6]
                                .iter()
                                .map(|w| w.text.as_str())
                                .collect::<Vec<_>>()
                                .join(" ");
                            result["state"] = json!("syntax_error_dialog");
                            result["confidence"] = json!(
                                words[i..i + 6]
                                    .iter()
                                    .map(|w| w.confidence)
                                    .fold(100., f64::min)
                                    / 100.
                            );
                            result["diagnostic"] = json!({"kind":"syntax_error","message":message,"line":line,"column":null,"source":"physical_screen_ocr","program_attribution":"unverified"});
                            return result;
                        }
                    }
                }
            }
        }
        // A modal with unrecognized text must not inherit the catalog behind it.
        return result;
    }
    for pair in words.windows(2) {
        if pair[0].text == "Program"
            && pair[1].text == "Catalog"
            && pair.iter().all(|w| w.y < 60 && w.confidence >= 90.)
        {
            result["state"] = json!("program_catalog");
            result["confidence"] = json!(pair[0].confidence.min(pair[1].confidence) / 100.);
        }
    }
    result
}
fn ocr(
    image: &Grob,
    dir: &Path,
    index: usize,
    exe: Option<&Path>,
) -> Result<(Vec<Word>, PathBuf), String> {
    let mut enlarged = Grob::new(image.width() * 3, image.height() * 3, 0);
    for y in 0..enlarged.height() {
        for x in 0..enlarged.width() {
            enlarged.pixels_mut()[y * image.width() * 3 + x] =
                image.pixels()[(y / 3) * image.width() + x / 3];
        }
    }
    let input = dir.join(format!("ocr-input-{index}.png"));
    enlarged.save_png(&input).map_err(|e| e.to_string())?;
    let output = dir.join(format!("ocr-{index}"));
    let mut child = Command::new(exe.unwrap_or_else(|| Path::new("tesseract")))
        .arg(&input)
        .arg(&output)
        .args(["-l", "eng", "--psm", "11", "tsv"])
        .env("OMP_THREAD_LIMIT", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("local OCR unavailable: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err("local OCR failed".into());
                }
                break;
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("local OCR timed out or could not be collected".into());
            }
        }
    }
    let path = output.with_extension("tsv");
    let length = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if length > 1024 * 1024 {
        return Err("OCR output exceeds limit".into());
    }
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(&path)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    Ok((parse_tsv(&text)?, path))
}

pub(super) fn inspect(first: Response, second: Response, root: &Path, exe: Option<&Path>) -> Value {
    let mut out = base("ui_inspect");
    out["device"] = first.report["device"].clone();
    let dir = root.join(format!("inspection-{}", ppl_device::fresh_test_name()));
    let result = (|| -> Result<(), String> {
        super::device_control::private_directory(root, true).map_err(|e| e.to_string())?;
        super::device_control::private_directory(&dir, false).map_err(|e| e.to_string())?;
        let mut frames = vec![];
        let mut evidence = vec![];
        let mut observations = vec![];
        let mut decoder = ppl_graphics::PrimeScreenshotDecoder::default();
        for (i, response) in [first, second].into_iter().enumerate() {
            if response.report["device"] != out["device"] || !response.report["error"].is_null() {
                observations.push(json!({"state":"unknown","reason":"second capture failed or calculator identity changed"}));
                continue;
            }
            let raw = dir.join(format!("frame-{i}.raw.png"));
            std::fs::write(&raw, &response.payload).map_err(|e| e.to_string())?;
            evidence.push(raw);
            out["evidence_paths"] = json!(evidence);
            let (image, _) = decoder.decode(&response.payload, Some((320, 240)))?;
            let png = dir.join(format!("frame-{i}.png"));
            image.save_png(&png).map_err(|e| e.to_string())?;
            evidence.push(png.clone());
            out["evidence_paths"] = json!(evidence);
            let observation = match ocr(&image, &dir, i, exe) {
                Ok((words, tsv)) => {
                    evidence.push(tsv);
                    classify(&image, &words)
                }
                Err(reason) => {
                    json!({"state":"unknown","confidence":null,"reason":reason,"diagnostic":null})
                }
            };
            frames.push(json!({"image":png,"raw_sha256":ppl_compat::sha256_hex(&response.payload),"resolution":[320,240],"requested_at_unix_ms":response.report["requested_at_unix_ms"],"completed_at_unix_ms":response.report["completed_at_unix_ms"]}));
            observations.push(observation);
        }
        out["evidence_paths"] = json!(evidence);
        out["screen"] = json!({"state":"unknown","confidence":null,"frames":frames,"observations":observations});
        if observations.len() == 2
            && observations[0]["state"] != "unknown"
            && observations[0]["state"] == observations[1]["state"]
            && observations[0]["diagnostic"] == observations[1]["diagnostic"]
        {
            out["screen"]["state"] = observations[0]["state"].clone();
            out["screen"]["confidence"] = json!(
                observations[0]["confidence"]
                    .as_f64()
                    .unwrap_or(0.)
                    .min(observations[1]["confidence"].as_f64().unwrap_or(0.))
            );
            out["screen"]["confidence_kind"] = json!("heuristic_not_probability");
            if !observations[0]["diagnostic"].is_null() {
                out["diagnostics"] = json!([observations[0]["diagnostic"]]);
                out["status"] = json!("syntax_error");
            } else {
                out["status"] = json!("valid");
            }
        } else {
            failure(
                &mut out,
                "diagnostic_uncertain",
                9,
                "screen state/text could not be verified in two fresh captures; inspect preserved screenshots",
            );
        }
        let report_path = dir.join("inspection.json");
        out["evidence_paths"]
            .as_array_mut()
            .unwrap()
            .push(json!(report_path));
        std::fs::write(
            report_path,
            serde_json::to_vec_pretty(&out).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = result {
        failure(&mut out, "invalid_evidence", 9, e);
        out["artifact_directory"] = json!(dir);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(line: &str) -> Vec<Word> {
        line.split_whitespace()
            .map(|s| Word {
                text: s.into(),
                confidence: 99.,
                x: 300,
                y: 300,
            })
            .collect()
    }
    fn modal() -> Grob {
        let mut g = Grob::screen();
        for y in 95..145 {
            for x in 55..265 {
                g.set(x, y, 0x84c321);
            }
        }
        for y in 105..125 {
            for x in 65..85 {
                g.set(x, y, 0x0000ff);
            }
        }
        g
    }
    #[test]
    fn diagnostics_require_layout_trusted_text_and_actual_numeric_line() {
        for line in [1, 42, 261, 999] {
            let w = words(&format!("Syntax error in program line {line}"));
            let r = classify(&modal(), &w);
            assert_eq!(r["diagnostic"]["line"], line);
            assert!(r["diagnostic"]["column"].is_null());
            assert_eq!(classify(&Grob::screen(), &w)["state"], "unknown");
        }
        for text in [
            "Syntax error in program line O1",
            "Syntax error in program line 0",
            "Program Catalog",
        ] {
            assert_eq!(classify(&modal(), &words(text))["state"], "unknown");
        }
        let mut w = words("Syntax error in program line 261");
        w[5].confidence = 20.;
        assert_eq!(classify(&modal(), &w)["state"], "unknown");
    }
    #[test]
    fn invalid_tsv_and_uncertain_runtime_text_never_become_diagnostics() {
        assert!(parse_tsv("header\nbad row").is_err());
        assert_eq!(
            classify(&modal(), &words("Invalid input"))["state"],
            "unknown"
        );
    }
    #[test]
    fn malformed_capture_preserves_original_evidence_without_claiming_a_state() {
        let root = tempfile::tempdir().unwrap();
        let first = Response {
            report: json!({"device":{"serial_id":"mock"},"error":null}),
            payload: vec![1, 2, 3],
        };
        let second = Response {
            report: json!({"device":{"serial_id":"mock"},"error":null}),
            payload: vec![4, 5, 6],
        };
        let result = inspect(first, second, root.path(), None);
        assert_eq!(result["status"], "invalid_evidence");
        let path = result["evidence_paths"][0].as_str().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), [1, 2, 3]);
        assert_eq!(result["check_executed"], false);
    }
}
