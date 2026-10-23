//! USB HID transport: device enumeration and raw report I/O.

use hidapi::{HidApi, HidDevice};

use super::Result;

pub const VID_HP: u16 = 0x03F0;

/// Known HP Prime USB product IDs.
pub const KNOWN_PIDS: &[(u16, &str)] = &[
    (0x0441, "HP Prime G1 (firmware before build 8151)"),
    (0x1541, "HP Prime G1 (firmware build 8151 or later)"),
    (0x2441, "HP Prime G2"),
];

pub const CMD_CHECK_READY: u8 = 0xFF;
pub const CMD_GET_INFO: u8 = 0xFA;
pub const CMD_SCREENSHOT: u8 = 0xFC;
pub const CMD_REQ_FILE: u8 = 0xF8;
pub const CMD_FILE: u8 = 0xF7;
pub const CMD_KEY: u8 = 0xEC;

/// The byte a ready G1 sends in reply to `FF` (observed on build 15157).
pub const READY_REPLY: u8 = 0x59;

/// Raw report I/O. Implemented for [`hidapi::HidDevice`]; tests use a mock.
pub trait Transport: Send {
    /// Writes one output report (byte 0 is the HID report id). Returns the
    /// number of bytes written.
    fn write(&mut self, report: &[u8]) -> Result<usize>;
    /// Reads one input report, waiting at most `timeout_ms` milliseconds.
    /// `Ok(0)` means nothing arrived in time.
    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize>;
}

impl Transport for HidDevice {
    fn write(&mut self, report: &[u8]) -> Result<usize> {
        Ok(HidDevice::write(self, report)?)
    }

    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<usize> {
        Ok(HidDevice::read_timeout(self, buf, timeout_ms)?)
    }
}

/// A calculator found during enumeration.
#[derive(Clone, Debug)]
pub struct Detected {
    pub vendor_id: u16,
    pub product_id: u16,
    pub model: &'static str,
    pub path: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub release: u16,
    pub interface: i32,
}

impl Detected {
    /// HID report payload size: 64 bytes on the G1, 1024 on the G2.
    pub fn report_size(&self) -> usize {
        if self.product_id == 0x2441 { 1024 } else { 64 }
    }
}

/// Lists connected HP Primes (read-only enumeration).
pub fn detect() -> Result<Vec<Detected>> {
    let api = HidApi::new()?;
    Ok(api
        .device_list()
        .filter(|d| d.vendor_id() == VID_HP)
        .filter_map(|d| {
            let model = KNOWN_PIDS.iter().find(|(p, _)| *p == d.product_id())?.1;
            Some(Detected {
                vendor_id: d.vendor_id(),
                product_id: d.product_id(),
                model,
                path: d.path().to_string_lossy().into_owned(),
                manufacturer: d.manufacturer_string().map(str::to_string),
                product: d.product_string().map(str::to_string),
                serial: d
                    .serial_number()
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                release: d.release_number(),
                interface: d.interface_number(),
            })
        })
        .collect())
}
