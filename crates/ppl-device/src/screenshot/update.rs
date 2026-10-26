//! Validated screenshot envelopes, independent of image decoding and rendering.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Full,
    Patch { x: u16, y: u16 },
}
#[derive(Debug)]
pub struct ScreenshotUpdate {
    pub format: u8,
    pub placement: Placement,
    pub png: Vec<u8>,
    pub timings: ScreenshotTimings,
}
impl ScreenshotUpdate {
    fn validate(raw: &[u8], requested: u8) -> Result<Placement> {
        let bad = || DeviceError::Protocol("invalid FC update envelope".into());
        if raw.len() >= 9 && (raw[8] != requested || !matches!(requested, 4 | 8)) {
            return Err(DeviceError::Refused(
                "unsupported_incremental_format: response does not match requested RGB555 format"
                    .into(),
            ));
        }
        if raw.len() < 46
            || raw[..2] != [0xfc, 1]
            || raw[8] != requested
            || !matches!(requested, 4 | 8)
        {
            return Err(bad());
        }
        let size = u32::from_be_bytes(raw[2..6].try_into().unwrap()) as usize;
        if size.checked_add(6) != Some(raw.len())
            || raw.len() > MAX_REPLY
            || crc16(&raw[8..]) != u16::from_be_bytes([raw[6], raw[7]])
            || raw[13..21] != *b"\x89PNG\r\n\x1a\n"
        {
            return Err(bad());
        }
        let x = u16::from_le_bytes([raw[9], raw[10]]);
        let y = u16::from_le_bytes([raw[11], raw[12]]);
        let placement = if (x, y) == (u16::MAX, u16::MAX) {
            Placement::Full
        } else if x < 320 && y < 240 {
            Placement::Patch { x, y }
        } else {
            return Err(bad());
        };
        Ok(placement)
    }
    pub fn parse(raw: &[u8], requested: u8) -> Result<Self> {
        Ok(Self {
            placement: Self::validate(raw, requested)?,
            format: requested,
            png: raw[13..].to_vec(),
            timings: Default::default(),
        })
    }
    fn parse_owned(mut raw: Vec<u8>, requested: u8) -> Result<Self> {
        let placement = Self::validate(&raw, requested)?;
        raw.drain(..13);
        Ok(Self {
            placement,
            format: requested,
            png: raw,
            timings: Default::default(),
        })
    }
}
/// Explicit opt-in for one experimental request, never a continuous stream.
pub struct IncrementalProbePermit {
    remaining: bool,
}
impl IncrementalProbePermit {
    pub fn acknowledge_experimental_fc04() -> Self {
        Self { remaining: true }
    }
}

/// Explicit opt-in for a bounded visual verification sequence.
pub struct IncrementalVerifyPermit {
    remaining: u32,
}
impl IncrementalVerifyPermit {
    pub fn acknowledge_experimental_fc04(bounded_limit: u32) -> Result<Self> {
        if bounded_limit == 0 || bounded_limit > 50 {
            return Err(DeviceError::Refused(
                "verification limit must be between 1 and 50".into(),
            ));
        }
        Ok(Self {
            remaining: bounded_limit,
        })
    }
    pub fn remaining(&self) -> u32 {
        self.remaining
    }
}

/// Diagnostic-only request and wall-time budget. Never grants normal mirroring.
pub struct BoundedIncrementalPermit {
    remaining: u32,
    deadline: Instant,
}

impl BoundedIncrementalPermit {
    pub fn acknowledge_experimental_fc04(limit: u32, duration: Duration) -> Result<Self> {
        if !(1..=200).contains(&limit) || duration.is_zero() || duration > Duration::from_secs(120)
        {
            return Err(DeviceError::Refused(
                "stream verification requires 1..=200 updates and at most 120 seconds".into(),
            ));
        }
        Ok(Self {
            remaining: limit,
            deadline: Instant::now() + duration,
        })
    }
}

impl ScreenshotSession {
    pub fn verify_incremental_bounded(
        &mut self,
        permit: &mut BoundedIncrementalPermit,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotUpdate> {
        let timeout = permit
            .deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(5));
        if permit.remaining == 0 || timeout.is_zero() {
            return Err(DeviceError::Refused(
                "stream verification budget exhausted".into(),
            ));
        }
        if (self.detected().vendor_id, self.detected().product_id) != (0x03f0, 0x1541) {
            return Err(DeviceError::Refused(
                "stream verification restricted to G1 03f0:1541".into(),
            ));
        }
        // Charge attempts before I/O; an ambiguous failed write is never refunded.
        permit.remaining -= 1;
        self.capture_update_timeout(4, timeout, cancel)
    }

    pub fn probe_incremental(
        &mut self,
        permit: &mut IncrementalProbePermit,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotUpdate> {
        if !std::mem::replace(&mut permit.remaining, false) {
            return Err(DeviceError::Refused("FC04 probe budget exhausted".into()));
        }
        if (self.detected().vendor_id, self.detected().product_id) != (0x03f0, 0x1541) {
            return Err(DeviceError::Refused(
                "probe restricted to G1 03f0:1541".into(),
            ));
        }
        self.capture_update(4, cancel)
    }

    pub fn verify_incremental(
        &mut self,
        permit: &mut IncrementalVerifyPermit,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotUpdate> {
        if permit.remaining == 0 {
            return Err(DeviceError::Refused(
                "FC04 verification budget exhausted".into(),
            ));
        }
        permit.remaining -= 1;
        if (self.detected().vendor_id, self.detected().product_id) != (0x03f0, 0x1541) {
            return Err(DeviceError::Refused(
                "verification restricted to G1 03f0:1541".into(),
            ));
        }
        self.capture_update(4, cancel)
    }

    pub(super) fn capture_update(
        &mut self,
        format: u8,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotUpdate> {
        self.capture_update_timeout(format, Duration::from_secs(5), cancel)
    }

    fn capture_update_timeout(
        &mut self,
        format: u8,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotUpdate> {
        let start = Instant::now();
        let (raw, mut timings) = self.exchange(&[CMD_SCREENSHOT, format], timeout, cancel)?;
        self.stage = "extraction";
        let extract = Instant::now();
        match ScreenshotUpdate::parse_owned(raw, format) {
            Ok(mut update) => {
                timings.extraction = extract.elapsed();
                timings.total = start.elapsed();
                update.timings = timings;
                update.timings.png_bytes = update.png.len();
                Ok(update)
            }
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }
}

/// Structured conceptual capability states for physical incremental mirror support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IncrementalCapabilityStatus {
    pub protocol_parsing: &'static str,
    pub physical_response: &'static str,
    pub physical_visual_correctness: &'static str,
    pub sustained_streaming: &'static str,
    pub production_enablement: &'static str,
}

pub const INCREMENTAL_CAPABILITY: IncrementalCapabilityStatus = IncrementalCapabilityStatus {
    protocol_parsing: "VERIFIED",
    physical_response: "VERIFIED",
    physical_visual_correctness: "UNVERIFIED",
    sustained_streaming: "UNVERIFIED",
    production_enablement: "DISABLED",
};

/// No physical profile has completed changing-screen and recovery acceptance.
pub fn incremental_profiles_available() -> bool {
    false
}

pub fn incremental_hardware_verified(_device: &crate::Detected, _info: &CalcInfo) -> bool {
    false
}

/// A reviewed profile is required for continuous FC04; probes cannot create it.
pub struct VerifiedIncremental {
    _private: (),
}
impl VerifiedIncremental {
    pub fn for_device(device: &crate::Detected, info: &CalcInfo) -> Result<Self> {
        if incremental_hardware_verified(device, info) {
            Ok(Self { _private: () })
        } else {
            Err(DeviceError::Refused(
                "unsupported_capability: incremental screen capture is not hardware verified"
                    .into(),
            ))
        }
    }
}
impl ScreenshotSession {
    pub fn capture_incremental(
        &mut self,
        _permit: &VerifiedIncremental,
        cancel: &AtomicBool,
    ) -> Result<ScreenshotUpdate> {
        self.capture_update(4, cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_stream_permit_cannot_outlive_attempt_or_wall_time_limits() {
        assert!(
            BoundedIncrementalPermit::acknowledge_experimental_fc04(201, Duration::from_secs(1))
                .is_err()
        );
        assert!(
            BoundedIncrementalPermit::acknowledge_experimental_fc04(1, Duration::from_secs(121))
                .is_err()
        );
        let (mut session, writes) =
            super::super::tests::session(vec![super::super::tests::reply(4, 100, 63)]);
        let cancel = AtomicBool::new(false);
        let mut permit =
            BoundedIncrementalPermit::acknowledge_experimental_fc04(1, Duration::from_secs(1))
                .unwrap();
        session
            .verify_incremental_bounded(&mut permit, &cancel)
            .unwrap();
        assert!(
            session
                .verify_incremental_bounded(&mut permit, &cancel)
                .is_err()
        );
        let mut expired =
            BoundedIncrementalPermit::acknowledge_experimental_fc04(1, Duration::from_secs(1))
                .unwrap();
        expired.deadline = Instant::now();
        assert!(
            session
                .verify_incremental_bounded(&mut expired, &cancel)
                .is_err()
        );
        assert_eq!(*writes.lock().unwrap(), vec![vec![0, 0, 0xfc, 4]]);
    }

    #[test]
    fn continuous_request_method_is_exercised_only_with_synthetic_transport() {
        let (mut s, writes) = super::super::tests::session(vec![
            super::super::tests::reply(8, 100, 63),
            super::super::tests::reply(4, 100, 63),
            super::super::tests::reply(4, 100, 63),
        ]);
        let cancel = AtomicBool::new(false);
        s.capture(8, Duration::from_secs(1), &cancel).unwrap();
        let permit = VerifiedIncremental { _private: () }; // tests only; no production constructor bypass
        s.capture_incremental(&permit, &cancel).unwrap();
        s.capture_incremental(&permit, &cancel).unwrap();
        assert_eq!(s.requests_sent(), 3);
        assert_eq!(
            *writes.lock().unwrap(),
            vec![
                vec![0, 0, 0xfc, 8],
                vec![0, 0, 0xfc, 4],
                vec![0, 0, 0xfc, 4]
            ]
        );
    }

    #[test]
    fn verify_permit_bounds_and_budget_exhaustion() {
        assert!(IncrementalVerifyPermit::acknowledge_experimental_fc04(0).is_err());
        assert!(IncrementalVerifyPermit::acknowledge_experimental_fc04(51).is_err());
        let mut permit = IncrementalVerifyPermit::acknowledge_experimental_fc04(2).unwrap();
        assert_eq!(permit.remaining(), 2);

        let (mut s, writes) = super::super::tests::session(vec![
            super::super::tests::reply(4, 100, 63),
            super::super::tests::reply(4, 100, 63),
        ]);
        let cancel = AtomicBool::new(false);
        assert!(s.verify_incremental(&mut permit, &cancel).is_ok());
        assert_eq!(permit.remaining(), 1);
        assert!(s.verify_incremental(&mut permit, &cancel).is_ok());
        assert_eq!(permit.remaining(), 0);
        // Budget exhausted
        let err = s.verify_incremental(&mut permit, &cancel).unwrap_err();
        assert!(matches!(err, DeviceError::Refused(_)));
        assert_eq!(
            *writes.lock().unwrap(),
            vec![vec![0, 0, 0xfc, 4], vec![0, 0, 0xfc, 4]]
        );
    }
}
