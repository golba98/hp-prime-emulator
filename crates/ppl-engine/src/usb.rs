//! Local virtual USB transport controls. These never enumerate physical USB.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    #[default]
    Disconnected,
    Computer,
    Calculator,
}
/// Descriptor fields returned by the OS host driver after enumerating a peer.
/// These describe an explicitly configured virtual peer, never host hardware.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub vendor: u16,
    pub product: u16,
    pub class: u8,
    pub receive_bytes: u16,
    pub transmit_bytes: u16,
    pub extension: bool,
}
impl Peer {
    pub fn validate(&self) -> Result<(), String> {
        if self.receive_bytes == 0
            || self.receive_bytes > 640
            || self.transmit_bytes == 0
            || self.transmit_bytes > 640
        {
            return Err("Virtual USB peer endpoint sizes must be within 1..=640".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    ConfigurePeer { peer: Option<Peer> },
    Connect { link: Link },
    Write { host: bool, bytes: Vec<u8> },
    Read { host: bool },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub link: Link,
    pub peripheral_enabled: bool,
    pub host_enabled: bool,
    pub peripheral_callback: bool,
    pub host_callback: bool,
    pub receive_packets: usize,
    pub transmit_packets: usize,
}
