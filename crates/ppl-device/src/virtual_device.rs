//! Real firmware USB communication through the local emulator's virtual wire.
//! The original guest firmware produces every protocol response. No HID API,
//! device enumeration, physical connection or host protocol mock is used.
use crate::{Detected, DeviceError, Prime, Result, transport::Transport};
use ppl_engine::{
    Cancellation,
    firmware_options::Options,
    live::LiveSession,
    session::{Command, Event},
    usb::{Link, Operation, Status},
};
use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};

pub struct VirtualTransport {
    session: LiveSession,
    cancel: Cancellation,
    request: u64,
    pending: VecDeque<Vec<u8>>,
}
impl VirtualTransport {
    pub fn connect(options: &Options, profile: &Path) -> Result<Self> {
        let cancel = Cancellation::default();
        let session =
            LiveSession::spawn(options, profile, &cancel).map_err(DeviceError::Protocol)?;
        let mut transport = Self {
            session,
            cancel,
            request: 0,
            pending: VecDeque::new(),
        };
        transport.operation(
            Operation::Connect {
                link: Link::Computer,
            },
            Instant::now() + Duration::from_secs(10),
        )?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let (bytes, status) = transport.operation(Operation::Read { host: false }, deadline)?;
            transport.retain(bytes)?;
            if status.peripheral_enabled && status.peripheral_callback {
                break;
            }
            if Instant::now() >= deadline {
                return Err(DeviceError::Timeout("virtual firmware USB initialization"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Registering the OS callback precedes the application's protocol
        // subscription. Only a real FF/59 exchange establishes readiness.
        // Retry this read-only liveness command; never retry file mutations.
        transport.await_protocol_ready(Instant::now() + Duration::from_secs(15))?;
        Ok(transport)
    }
    fn await_protocol_ready(&mut self, deadline: Instant) -> Result<()> {
        let report = Prime::frame(&[crate::CMD_CHECK_READY], 64).pop().unwrap();
        let mut reply = [0u8; 64];
        while Instant::now() < deadline {
            self.write(&report)?;
            let window = Instant::now() + Duration::from_millis(250);
            loop {
                let remaining = window
                    .min(deadline)
                    .saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let count = self.read_timeout(
                    &mut reply,
                    remaining.as_millis().min(i32::MAX as u128) as i32,
                )?;
                if count >= 2 && reply[0] == 0 && reply[1] == crate::READY_REPLY {
                    return Ok(());
                }
                if count == 0 {
                    break;
                }
            }
        }
        Err(DeviceError::Timeout(
            "virtual firmware protocol readiness (FF/59)",
        ))
    }
    fn operation(
        &mut self,
        operation: Operation,
        deadline: Instant,
    ) -> Result<(Option<Vec<u8>>, Status)> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| DeviceError::Protocol("virtual USB request counter exhausted".into()))?;
        let request = self.request;
        self.session
            .send(&Command::Usb { request, operation })
            .map_err(DeviceError::Protocol)?;
        loop {
            match self
                .session
                .event(deadline, &self.cancel)
                .map_err(DeviceError::Protocol)?
            {
                Event::Usb {
                    request: actual,
                    success,
                    message,
                    bytes,
                    status,
                } => {
                    if actual != request {
                        return Err(DeviceError::Protocol(
                            "virtual USB reply identity mismatch".into(),
                        ));
                    }
                    return if success {
                        Ok((bytes, status))
                    } else {
                        Err(DeviceError::Protocol(message))
                    };
                }
                Event::Error { message } | Event::Closed { message } => {
                    return Err(DeviceError::Protocol(message));
                }
                _ => {}
            }
        }
    }
    fn retain(&mut self, bytes: Option<Vec<u8>>) -> Result<()> {
        if let Some(bytes) = bytes {
            if bytes.is_empty() || bytes.len() % 64 != 0 {
                return Err(DeviceError::Protocol(
                    "firmware USB transmission is not a sequence of 64-byte G1 reports".into(),
                ));
            }
            self.pending
                .extend(bytes.chunks_exact(64).map(<[u8]>::to_vec));
        }
        Ok(())
    }
}
impl Transport for VirtualTransport {
    fn write(&mut self, report: &[u8]) -> Result<usize> {
        if report.len() < 2 || report.len() > 65 || report[0] != 0 {
            return Err(DeviceError::Protocol(
                "G1 output reports require report ID zero and 1..=64 wire bytes".into(),
            ));
        }
        // USB interrupt endpoints deliver fixed-size reports to the driver.
        // The host HID report-ID prefix is not part of the wire payload.
        let mut bytes = vec![0; 64];
        bytes[..report.len() - 1].copy_from_slice(&report[1..]);
        self.operation(
            Operation::Write { host: false, bytes },
            Instant::now() + Duration::from_secs(5),
        )?;
        Ok(report.len())
    }
    fn read_timeout(&mut self, output: &mut [u8], timeout_ms: i32) -> Result<usize> {
        if output.len() < 64 {
            return Err(DeviceError::Protocol(
                "G1 input buffer must hold 64 bytes".into(),
            ));
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.max(0) as u64);
        loop {
            if let Some(bytes) = self.pending.pop_front() {
                output[..bytes.len()].copy_from_slice(&bytes);
                return Ok(bytes.len());
            }
            let (bytes, _) = self.operation(
                Operation::Read { host: false },
                Instant::now() + Duration::from_secs(5),
            )?;
            self.retain(bytes)?;
            if !self.pending.is_empty() {
                continue;
            }
            if Instant::now() >= deadline {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
impl Drop for VirtualTransport {
    fn drop(&mut self) {
        // Uses the same saved firmware OFF path as the GUI; LiveSession's Drop
        // still bounds helper teardown if the firmware cannot switch off.
        let _ = self
            .session
            .power_down(Duration::from_secs(5), &self.cancel);
    }
}
impl Prime {
    pub fn open_virtual(options: &Options, profile: &Path) -> Result<Self> {
        let transport = VirtualTransport::connect(options, profile)?;
        let detected = Detected {
            vendor_id: crate::transport::VID_HP,
            product_id: 0x1541,
            model: "Virtual HP Prime G1 (local firmware)",
            path: format!("virtual:{}", profile.display()),
            manufacturer: Some("Local firmware emulator".into()),
            product: Some("Virtual G1 endpoint".into()),
            serial: None,
            release: 0,
            interface: 0,
        };
        Ok(Self::with_transport(detected, Box::new(transport)))
    }
}
