//! Exclusive, read-only screenshot stream. Ordinary command/evidence paths are
//! unchanged. Owning `Prime` prevents commands from interleaving with captures.

pub mod update;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::{CMD_GET_INFO, CMD_SCREENSHOT, CalcInfo, DeviceError, Prime, Result, crc16};

const POLL: Duration = Duration::from_millis(40);
const MAX_REPLY: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ScreenshotTimings {
    pub setup: Duration,
    pub write: Duration,
    pub first_response: Duration,
    /// Time inside subsequent HID reads; excludes host assembly work.
    pub transfer: Duration,
    pub assembly: Duration,
    pub extraction: Duration,
    pub total: Duration,
    pub reports: usize,
    pub report_bytes: usize,
    pub png_bytes: usize,
}

pub struct ScreenshotCapture {
    pub png: Vec<u8>,
    pub timings: ScreenshotTimings,
}

pub struct ScreenshotSession {
    prime: Prime,
    synchronized: bool,
    failed: bool,
    report: Vec<u8>,
    stage: &'static str,
    requests_sent: u64,
}

impl ScreenshotSession {
    pub fn new(mut prime: Prime) -> Self {
        // Identity exchanges may already exist. Streaming never accumulates raw
        // reports or traces and never changes the logging policy of other APIs.
        prime.log.clear();
        prime.traces.clear();
        let report = vec![0; prime.report_size];
        Self {
            prime,
            synchronized: false,
            failed: false,
            report,
            stage: "setup",
            requests_sent: 0,
        }
    }

    pub fn requests_sent(&self) -> u64 {
        self.requests_sent
    }

    pub fn detected(&self) -> &crate::Detected {
        &self.prime.detected
    }

    pub fn failure_stage(&self) -> &'static str {
        self.stage
    }

    /// Transfer ownership only after a complete exchange. Failed sessions are
    /// consumed and closed, so a delayed reply cannot become a command result.
    pub fn into_prime(self) -> Result<Prime> {
        if self.failed {
            return Err(DeviceError::Refused(
                "failed screenshot session cannot hand off transport".into(),
            ));
        }
        Ok(self.prime)
    }

    pub fn info(&mut self, cancel: &AtomicBool) -> Result<CalcInfo> {
        let (raw, _) = self.exchange(&[CMD_GET_INFO], Duration::from_secs(5), cancel)?;
        Ok(CalcInfo::parse(&raw))
    }

    pub fn capture(
        &mut self,
        format: u8,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotCapture> {
        if timeout.is_zero() || Instant::now().checked_add(timeout).is_none() {
            return Err(DeviceError::Refused("invalid screenshot deadline".into()));
        }
        if !matches!(format, 8 | 9 | 10 | 11 | 13) {
            return Err(DeviceError::Refused(
                "undocumented screenshot format".into(),
            ));
        }
        let start = Instant::now();
        let (mut raw, mut timings) = self.exchange(&[CMD_SCREENSHOT, format], timeout, cancel)?;
        self.stage = "extraction";
        let extract = Instant::now();
        let valid = raw.len() >= 21
            && raw[8] == format
            && raw[9..13] == [0xff; 4]
            && raw[13..21] == *b"\x89PNG\r\n\x1a\n";
        if !valid {
            self.failed = true;
            return Err(DeviceError::Protocol(
                "screenshot is not a complete image with the requested format".into(),
            ));
        }
        let embedded = u16::from_be_bytes([raw[6], raw[7]]);
        raw[6..8].fill(0);
        if crc16(&raw[6..]) != embedded {
            self.failed = true;
            return Err(DeviceError::Protocol("screenshot CRC mismatch".into()));
        }
        // Transfer the allocation, rather than cloning the compressed image.
        raw.drain(..13);
        timings.png_bytes = raw.len();
        timings.extraction = extract.elapsed();
        timings.total = start.elapsed();
        Ok(ScreenshotCapture { png: raw, timings })
    }

    fn exchange(
        &mut self,
        command: &[u8],
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<(Vec<u8>, ScreenshotTimings)> {
        if self.failed {
            return Err(DeviceError::Refused(
                "failed screenshot session must be reopened".into(),
            ));
        }
        let result = self.exchange_inner(command, timeout, cancel);
        if result.is_err() {
            // A delayed reply must never become the next command's response.
            self.failed = true;
        }
        result
    }

    fn exchange_inner(
        &mut self,
        command: &[u8],
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<(Vec<u8>, ScreenshotTimings)> {
        self.stage = "setup";
        let start = Instant::now();
        let mut timings = ScreenshotTimings::default();
        check_cancel(cancel)?;
        if !self.synchronized {
            // Same initial quiet interval as ordinary commands, bounded and
            // cancellable. Never insert it between completed stream messages.
            let deadline = Instant::now() + Duration::from_millis(250);
            for _ in 0..4096 {
                check_cancel(cancel)?;
                if Instant::now() >= deadline {
                    break;
                }
                if self.prime.dev.read_timeout(&mut self.report, 50)? == 0 {
                    break;
                }
            }
        }
        timings.setup = start.elapsed();
        check_cancel(cancel)?;
        self.stage = "write";
        let write = Instant::now();
        let mut request = vec![0, 0];
        request.extend_from_slice(command);
        if command[0] == CMD_SCREENSHOT {
            self.requests_sent += 1;
        }
        let written = self.prime.dev.write(&request)?;
        if written != request.len() {
            return Err(DeviceError::Hid(
                "short screenshot-session write; not retried".into(),
            ));
        }
        timings.write = write.elapsed();
        let sent = Instant::now();
        let overall = sent
            .checked_add(timeout)
            .ok_or_else(|| DeviceError::Refused("invalid screenshot deadline".into()))?;
        let mut report_deadline = overall;
        let mut data = Vec::new();
        let mut expected = None;
        let mut sequence = 0usize;
        loop {
            // Once submitted, finish this bounded response even on cancellation.
            // Releasing the lock halfway through a reply would leave the next
            // connection exposed to a delayed screenshot. The GUI never waits
            // synchronously for this cleanup; it continues pumping events.
            let remaining = report_deadline
                .min(overall)
                .saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DeviceError::Timeout("screenshot session reply"));
            }
            self.stage = if sequence == 0 {
                "first_response"
            } else {
                "transfer"
            };
            let read = Instant::now();
            // Round up: a sub-millisecond remainder must not become busy polling.
            let ms = remaining.min(POLL).as_millis().max(1) as i32;
            let n = self.prime.dev.read_timeout(&mut self.report, ms)?;
            if sequence > 0 {
                timings.transfer += read.elapsed();
            }
            if n == 0 {
                continue;
            }
            if n > self.report.len() {
                return Err(DeviceError::Protocol("oversized HID report".into()));
            }
            if self.report[0] == 0xff {
                continue;
            }
            self.stage = "assembly";
            let assembly = Instant::now();
            if self.report[0] != (sequence % 255) as u8 {
                return Err(DeviceError::Protocol(
                    "screenshot session report out of sequence".into(),
                ));
            }
            if sequence == 0 {
                timings.first_response = sent.elapsed();
            }
            sequence += 1;
            timings.reports += 1;
            timings.report_bytes += n;
            data.extend_from_slice(&self.report[1..n]);
            if expected.is_none() && data.len() >= 6 {
                if data[0] != command[0] || data[1] != 1 {
                    return Err(DeviceError::Protocol(
                        "unexpected screenshot session reply header".into(),
                    ));
                }
                let size = u32::from_be_bytes(data[2..6].try_into().unwrap()) as usize;
                let size = size
                    .checked_add(6)
                    .filter(|n| *n <= MAX_REPLY)
                    .ok_or_else(|| DeviceError::Protocol("reply exceeds 16 MiB limit".into()))?;
                data.reserve(size.saturating_sub(data.len()));
                expected = Some(size);
            }
            timings.assembly += assembly.elapsed();
            if let Some(size) = expected {
                if data.len() >= size {
                    data.truncate(size);
                    self.synchronized = true;
                    timings.total = start.elapsed();
                    check_cancel(cancel)?;
                    return Ok((data, timings));
                }
            }
            // Short reports are not a substitute for the declared message size.
            report_deadline = Instant::now() + Duration::from_secs(2);
        }
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(DeviceError::Refused("screenshot session cancelled".into()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Detected, Transport};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct Mock {
        replies: VecDeque<Vec<Vec<u8>>>,
        pending: VecDeque<Vec<u8>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl Transport for Mock {
        fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            if let Some(reply) = self.replies.pop_front() {
                self.pending.extend(reply);
            }
            Ok(bytes.len())
        }
        fn read_timeout(&mut self, buffer: &mut [u8], ms: i32) -> Result<usize> {
            if let Some(report) = self.pending.pop_front() {
                if report.is_empty() {
                    std::thread::sleep(Duration::from_millis(1));
                    return Ok(0);
                }
                buffer[..report.len()].copy_from_slice(&report);
                Ok(report.len())
            } else {
                std::thread::sleep(Duration::from_millis(ms.min(2) as u64));
                Ok(0)
            }
        }
    }
    pub(super) fn reply(format: u8, length: usize, chunk: usize) -> Vec<Vec<u8>> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.resize(length, 0x67);
        let mut data = vec![0xfc, 1];
        data.extend_from_slice(&((png.len() + 7) as u32).to_be_bytes());
        data.extend_from_slice(&[0, 0, format, 255, 255, 255, 255]);
        data.extend(png);
        let crc = crc16(&data[6..]);
        data[6..8].copy_from_slice(&crc.to_be_bytes());
        data.chunks(chunk)
            .enumerate()
            .map(|(i, b)| {
                let mut r = vec![(i % 255) as u8];
                r.extend_from_slice(b);
                r
            })
            .collect()
    }
    pub(super) fn session(
        replies: Vec<Vec<Vec<u8>>>,
    ) -> (ScreenshotSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let mock = Mock {
            replies: replies.into(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let detected = Detected {
            vendor_id: 0x03f0,
            product_id: 0x1541,
            model: "mock",
            path: "mock".into(),
            manufacturer: None,
            product: None,
            serial: None,
            release: 0,
            interface: 0,
        };
        (
            ScreenshotSession::new(Prime::with_transport(detected, Box::new(mock))),
            writes,
        )
    }
    #[test]
    fn stream_reuses_connection_wraps_sequence_and_does_not_retain_reports() {
        let (mut s, writes) = session(vec![reply(8, 20_000, 63), reply(8, 100, 63)]);
        let cancel = AtomicBool::new(false);
        let a = s.capture(8, Duration::from_secs(1), &cancel).unwrap();
        assert_eq!(a.png.len(), 20_000);
        assert!(a.timings.reports > 255);
        let b = s.capture(8, Duration::from_secs(1), &cancel).unwrap();
        assert_eq!(b.png.len(), 100);
        assert!(b.timings.setup < Duration::from_millis(1));
        assert!(s.prime.log.is_empty() && s.prime.traces.is_empty());
        assert_eq!(*writes.lock().unwrap(), vec![vec![0, 0, 0xfc, 8]; 2]);
    }
    #[test]
    fn short_and_temporarily_absent_reports_use_declared_size() {
        let mut response = reply(10, 100, 3);
        response.insert(3, Vec::new());
        let (mut s, _) = session(vec![response]);
        assert_eq!(
            s.capture(10, Duration::from_secs(1), &AtomicBool::new(false))
                .unwrap()
                .png
                .len(),
            100
        );
    }
    #[test]
    fn errors_poison_session_and_cannot_consume_another_commands_reply() {
        for kind in 0..5 {
            let mut response = reply(8, 100, 63);
            match kind {
                0 => response[1][0] = 77,
                1 => response[0][9] = 10, // requested-format mismatch
                2 => response[0][7] ^= 1, // CRC
                3 => response[0][1] = 0xfa,
                _ => response[0][3..7].copy_from_slice(&u32::MAX.to_be_bytes()),
            }
            let (mut s, writes) = session(vec![response, reply(8, 100, 63)]);
            assert!(
                s.capture(8, Duration::from_millis(20), &AtomicBool::new(false))
                    .is_err()
            );
            assert!(
                s.capture(8, Duration::from_millis(20), &AtomicBool::new(false))
                    .is_err()
            );
            assert_eq!(writes.lock().unwrap().len(), 1);
        }
    }
    #[test]
    fn cancellation_before_write_and_silent_reply_are_bounded() {
        let (mut s, writes) = session(vec![]);
        assert!(
            s.capture(8, Duration::from_millis(10), &AtomicBool::new(true))
                .is_err()
        );
        assert!(writes.lock().unwrap().is_empty());
        let (mut s, _) = session(vec![]);
        let start = Instant::now();
        assert!(matches!(
            s.capture(8, Duration::from_millis(10), &AtomicBool::new(false)),
            Err(DeviceError::Timeout(_))
        ));
        assert!(start.elapsed() < Duration::from_millis(100));
    }
    #[test]
    fn cancelling_an_inflight_request_consumes_its_reply_before_releasing_the_connection() {
        struct CancelOnWrite {
            inner: Box<dyn Transport>,
            cancel: Arc<AtomicBool>,
        }
        impl Transport for CancelOnWrite {
            fn write(&mut self, bytes: &[u8]) -> Result<usize> {
                let n = self.inner.write(bytes)?;
                self.cancel.store(true, Ordering::Release);
                Ok(n)
            }
            fn read_timeout(&mut self, dst: &mut [u8], ms: i32) -> Result<usize> {
                self.inner.read_timeout(dst, ms)
            }
        }
        let (mut s, writes) = session(vec![reply(8, 1000, 63)]);
        let cancel = Arc::new(AtomicBool::new(false));
        let unused = Mock {
            replies: VecDeque::new(),
            pending: VecDeque::new(),
            writes: writes.clone(),
        };
        let inner = std::mem::replace(&mut s.prime.dev, Box::new(unused));
        s.prime.dev = Box::new(CancelOnWrite {
            inner,
            cancel: cancel.clone(),
        });
        assert!(s.capture(8, Duration::from_secs(1), &cancel).is_err());
        assert!(
            s.synchronized,
            "complete reply consumed even though cancellation arrived during command submission"
        );
        assert_eq!(writes.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::tests::{reply, session};
    use super::*;
    use update::*;
    #[test]
    fn probe_uses_exact_framing_and_cannot_send_twice() {
        let (mut s, writes) = session(vec![reply(8, 100, 63), reply(4, 100, 63)]);
        let cancel = AtomicBool::new(false);
        s.capture(8, Duration::from_secs(1), &cancel).unwrap();
        let mut permit = IncrementalProbePermit::acknowledge_experimental_fc04();
        assert_eq!(
            s.probe_incremental(&mut permit, &cancel).unwrap().placement,
            Placement::Full
        );
        assert!(s.probe_incremental(&mut permit, &cancel).is_err());
        assert_eq!(
            *writes.lock().unwrap(),
            vec![vec![0, 0, 0xfc, 8], vec![0, 0, 0xfc, 4]]
        );
        assert!(VerifiedIncremental::for_device(s.detected(), &CalcInfo::default()).is_err());
    }
    #[test]
    fn malformed_update_poisons_connection_and_consumes_probe_budget() {
        let mut response = reply(4, 100, 63);
        response[1][0] = 44;
        let (mut s, writes) = session(vec![response, reply(8, 100, 63)]);
        let cancel = AtomicBool::new(false);
        let mut permit = IncrementalProbePermit::acknowledge_experimental_fc04();
        assert!(s.probe_incremental(&mut permit, &cancel).is_err());
        assert!(s.capture(8, Duration::from_secs(1), &cancel).is_err());
        assert!(s.probe_incremental(&mut permit, &cancel).is_err());
        assert_eq!(writes.lock().unwrap().len(), 1);
    }
    #[test]
    fn envelope_rejects_crc_length_mixed_sentinel_format_and_bounds() {
        let raw: Vec<_> = reply(4, 100, 63)
            .iter()
            .flat_map(|r| r[1..].iter().copied())
            .collect();
        assert!(ScreenshotUpdate::parse(&raw, 4).is_ok());
        for index in [0, 1, 2, 6, 8, 10, 12, 20] {
            let mut bad = raw.clone();
            bad[index] ^= 1;
            if index == 10 || index == 12 {
                let crc = crc16(&bad[8..]);
                bad[6..8].copy_from_slice(&crc.to_be_bytes());
            }
            assert!(ScreenshotUpdate::parse(&bad, 4).is_err(), "index {index}");
        }
        assert!(ScreenshotUpdate::parse(&raw[..raw.len() - 1], 4).is_err());
    }
}
