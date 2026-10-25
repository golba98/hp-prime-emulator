use std::sync::atomic::AtomicBool;
use std::time::Instant;

use super::{MirrorCapture, UpdateDiagnostics};
use ppl_device::screenshot::ScreenshotSession;
use ppl_device::screenshot::update::{Placement, VerifiedIncremental};
use ppl_device::{Detected, DeviceError, Prime};
use ppl_graphics::PrimeScreenshotDecoder;
use ppl_graphics::incremental::IncrementalFramebuffer;

use super::{MirrorQuality, MirrorSource, SCREENSHOT_READ_TIMEOUT, SourceFrame};

pub struct UsbMirrorSource {
    session: Option<ScreenshotSession>,
    expected_serial: Option<String>,
    expected_firmware: Option<(Option<u32>, Option<String>)>,
    serial_id: Option<String>,
    device_name: String,
    quality: MirrorQuality,
    decoder: PrimeScreenshotDecoder,
    stage: &'static str,
    capture_mode: MirrorCapture,
    framebuffer: IncrementalFramebuffer,
    ordinal: u64,
    prior_requests: u64,
    permit: Option<VerifiedIncremental>,
    terminal: bool,
    experimental_fc04: bool,
}

impl UsbMirrorSource {
    /// No USB I/O here: initialization/identity run on the capture worker.
    pub fn new(prime: Prime) -> Self {
        Self {
            device_name: prime.detected.model.to_string(),
            session: Some(ScreenshotSession::new(prime)),
            expected_serial: None,
            expected_firmware: None,
            serial_id: None,
            quality: MirrorQuality::Full,
            decoder: PrimeScreenshotDecoder::default(),
            stage: "identity",
            capture_mode: MirrorCapture::Full,
            framebuffer: Default::default(),
            ordinal: 0,
            prior_requests: 0,
            permit: None,
            terminal: false,
            experimental_fc04: false,
        }
    }

    pub fn with_experimental_fc04(mut self, acknowledged: bool) -> Self {
        self.experimental_fc04 = acknowledged;
        self
    }

    pub fn with_capture(mut self, mode: MirrorCapture) -> Result<Self, DeviceError> {
        if mode == MirrorCapture::Incremental && self.quality != MirrorQuality::Full {
            return Err(DeviceError::Refused(
                "incremental requires full quality".into(),
            ));
        }
        // No profile is approved yet without explicit experimental acknowledgement.
        if mode == MirrorCapture::Incremental
            && !self.experimental_fc04
            && !ppl_device::screenshot::update::incremental_profiles_available()
        {
            return Err(DeviceError::Refused(
                "unsupported_capability: incremental hardware capture is unverified".into(),
            ));
        }
        self.capture_mode = mode;
        Ok(self)
    }

    pub fn with_quality(mut self, quality: MirrorQuality) -> Self {
        self.quality = quality;
        self
    }

    pub fn from_detected(detected: Detected) -> Result<Self, DeviceError> {
        Prime::open(detected).map(Self::new)
    }
}

impl MirrorSource for UsbMirrorSource {
    fn capture_frame(&mut self) -> Result<ppl_graphics::Grob, DeviceError> {
        self.capture(&AtomicBool::new(false))
            .map(|frame| std::sync::Arc::unwrap_or_clone(frame.grob))
    }

    fn capture(&mut self, cancel: &AtomicBool) -> Result<SourceFrame, DeviceError> {
        let result = (|| {
            let session = self.session.as_mut().ok_or(DeviceError::NotFound)?;
            if self.expected_serial.is_none() {
                let info = session.info(cancel)?;
                if self.capture_mode == MirrorCapture::Incremental {
                    self.permit = Some(VerifiedIncremental::for_device_experimental(
                        session.detected(),
                        &info,
                        self.experimental_fc04,
                    )?);
                }
                self.expected_firmware = Some((info.build, info.version.clone()));
                self.serial_id = info.serial_fingerprint();
                self.expected_serial = Some(info.serial.ok_or_else(|| {
                    DeviceError::Refused("cannot establish mirror device identity".into())
                })?);
            }
            let requested = Instant::now();
            self.stage = "usb";
            if self.capture_mode == MirrorCapture::Incremental {
                let baseline = self.ordinal == 0;
                let (png, timings, origin, format) = if baseline {
                    let capture = session.capture(8, SCREENSHOT_READ_TIMEOUT, cancel)?;
                    (capture.png, capture.timings, None, 8)
                } else {
                    let permit = self.permit.as_ref().ok_or_else(|| {
                        DeviceError::Refused("incremental hardware permit missing".into())
                    })?;
                    let update = session.capture_incremental(permit, cancel)?;
                    let origin = match update.placement {
                        Placement::Full => None,
                        Placement::Patch { x, y } => Some((x, y)),
                    };
                    (update.png, update.timings, origin, 4)
                };
                if cancel.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(DeviceError::Refused("capture cancelled".into()));
                }
                self.stage = "composition";
                let applied =
                    self.framebuffer
                        .apply(self.ordinal, origin, &png)
                        .map_err(|error| {
                            if error == "incremental RGB565 requires grayscale16 PNG" {
                                DeviceError::Refused(format!(
                                    "unsupported_incremental_encoding: {error}"
                                ))
                            } else {
                                DeviceError::Protocol(error)
                            }
                        })?;
                self.ordinal += 1;
                let dimensions = (
                    u32::from_be_bytes(png[16..20].try_into().unwrap()) as usize,
                    u32::from_be_bytes(png[20..24].try_into().unwrap()) as usize,
                );
                return Ok(SourceFrame {
                    grob: applied.image,
                    requested,
                    completed: Instant::now(),
                    usb: timings,
                    decode: applied.decode,
                    raw: Some(png),
                    update: Some(UpdateDiagnostics {
                        format,
                        origin,
                        dimensions,
                        baseline,
                        changed: applied.changed,
                        composition: applied.composition,
                    }),
                });
            }
            let capture =
                match session.capture(self.quality.format(), SCREENSHOT_READ_TIMEOUT, cancel) {
                    Ok(capture) => capture,
                    Err(e) => {
                        self.stage = session.failure_stage();
                        return Err(e);
                    }
                };
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(DeviceError::Refused(
                    "capture cancelled before decode".into(),
                ));
            }
            let (grob, decode) = self
                .decoder
                .decode(&capture.png, Some(self.quality.dimensions()))
                .map_err(|e| {
                    self.stage = self.decoder.failure_stage();
                    DeviceError::Protocol(format!("screenshot decode failed: {e}"))
                })?;
            Ok(SourceFrame {
                grob: std::sync::Arc::new(grob),
                requested,
                completed: Instant::now(),
                usb: capture.timings,
                decode,
                raw: Some(capture.png),
                update: None,
            })
        })();
        if let Err(error) = &result {
            self.terminal = matches!(error,DeviceError::Refused(message) if message.starts_with("unsupported_"));
            if self.stage == "usb" {
                self.stage = self.session.as_ref().map_or("usb", |s| s.failure_stage());
            }
            // Failed sessions are never reused; no commands follow an incomplete
            // response. Release the disconnected handle before backoff.
            self.framebuffer.invalidate();
            self.prior_requests += self.session.as_ref().map_or(0, |s| s.requests_sent());
            self.session = None;
        }
        result
    }

    fn screenshot_requests(&self) -> Option<u64> {
        Some(self.prior_requests + self.session.as_ref().map_or(0, |s| s.requests_sent()))
    }

    fn recovery_attempt_limit(&self) -> Option<usize> {
        (self.capture_mode == MirrorCapture::Incremental).then_some(if self.terminal {
            0
        } else {
            3
        })
    }

    fn failure_stage(&self) -> &'static str {
        self.stage
    }

    fn device_name(&self) -> &str {
        &self.device_name
    }
    fn device_serial(&self) -> Option<&str> {
        self.serial_id.as_deref()
    }

    fn try_reconnect(&mut self) -> Result<bool, DeviceError> {
        self.reconnect(&AtomicBool::new(false))
    }

    fn reconnect(&mut self, cancel: &AtomicBool) -> Result<bool, DeviceError> {
        self.session = None;
        let Some(expected) = &self.expected_serial else {
            return Err(DeviceError::Refused(
                "automatic reconnect requires established identity; restart mirror".into(),
            ));
        };
        let mut matched = None;
        for detected in ppl_device::detect()? {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(false);
            }
            let Ok(prime) = Prime::open(detected) else {
                continue;
            };
            let mut session = ScreenshotSession::new(prime);
            if matches_original(&mut session, expected, cancel) {
                if self.capture_mode == MirrorCapture::Incremental {
                    let info = session.info(cancel)?;
                    if self.expected_firmware.as_ref() != Some(&(info.build, info.version.clone()))
                    {
                        continue;
                    }
                    self.permit = Some(VerifiedIncremental::for_device_experimental(
                        session.detected(),
                        &info,
                        self.experimental_fc04,
                    )?);
                }
                if matched.is_some() {
                    return Err(DeviceError::Refused(
                        "multiple calculators share the original mirror identity".into(),
                    ));
                }
                matched = Some(session);
            }
        }
        self.framebuffer.restart();
        self.ordinal = 0;
        self.session = matched;
        Ok(self.session.is_some())
    }
}

// Compare actual serials in memory, never hashes or USB paths. Only the
// pre-existing display fingerprint is exposed, and raw identity is never logged.
fn matches_original(session: &mut ScreenshotSession, expected: &str, cancel: &AtomicBool) -> bool {
    session.info(cancel).ok().and_then(|i| i.serial).as_deref() == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Identity {
        serial: &'static str,
        pending: VecDeque<Vec<u8>>,
    }
    impl ppl_device::Transport for Identity {
        fn write(&mut self, request: &[u8]) -> Result<usize, DeviceError> {
            assert_eq!(request, &[0, 0, 0xfa]); // identity only, no other commands
            let mut raw = vec![0xfa, 1, 0, 0, 0, 64];
            raw.resize(70, 0);
            raw[54..64].copy_from_slice(self.serial.as_bytes());
            self.pending = raw
                .chunks(63)
                .enumerate()
                .map(|(n, b)| {
                    let mut r = vec![n as u8];
                    r.extend_from_slice(b);
                    r
                })
                .collect();
            Ok(request.len())
        }
        fn read_timeout(&mut self, dst: &mut [u8], _: i32) -> Result<usize, DeviceError> {
            if let Some(r) = self.pending.pop_front() {
                dst[..r.len()].copy_from_slice(&r);
                Ok(r.len())
            } else {
                Ok(0)
            }
        }
    }
    fn prime(serial: &'static str) -> Prime {
        let d = Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "mock",
            path: "same-path".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        Prime::with_transport(
            d,
            Box::new(Identity {
                serial,
                pending: VecDeque::new(),
            }),
        )
    }
    #[test]
    fn reconnect_matches_exact_identity_not_a_reused_device_path() {
        let mut wrong = ScreenshotSession::new(prime("2222222222"));
        assert!(!matches_original(
            &mut wrong,
            "1111111111",
            &AtomicBool::new(false)
        ));
        let mut right = ScreenshotSession::new(prime("1111111111"));
        assert!(matches_original(
            &mut right,
            "1111111111",
            &AtomicBool::new(false)
        ));
        let mut unidentified = UsbMirrorSource::new(prime("1111111111"));
        assert!(unidentified.try_reconnect().is_err()); // refuses before enumeration
    }
}
