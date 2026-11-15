//! Virtual USB driver endpoint. No host USB enumeration or physical-device I/O.
//! Driver ABI is taken from the installed OS ROM's 0x300148CC/0x300151D4.
use std::{collections::VecDeque, sync::Mutex};

const MAX_QUEUED_BYTES: usize = 1024 * 1024;
pub const MAX_PACKET_BYTES: usize = 640;

pub use ppl_engine::usb::Link;
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub enabled: bool,
    pub callback: u32,
    received: VecDeque<Vec<u8>>,
    transmitted: VecDeque<Vec<u8>>,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub link: Link,
    /// Optional peer-specific extension negotiated by the OS descriptor code.
    pub peer_extension: bool,
    pub peer: Option<ppl_engine::usb::Peer>,
    pub host_address: u8,
    pub host_initialized: bool,
    pub peripheral: Endpoint,
    pub host: Endpoint,
}
#[derive(Default)]
pub struct Usb {
    state: Mutex<Image>,
}
#[derive(Clone, Debug)]
pub struct Delivery {
    pub host: bool,
    pub callback: u32,
    pub bytes: Vec<u8>,
}
impl Image {
    pub fn restore(&self, mem: &crate::memory::GuestMemory) -> Result<Usb, String> {
        if let Some(peer) = &self.peer {
            peer.validate()?;
        }
        if self.host_address > 1
            || (self.host_initialized && self.host_address != 1)
            || (self.link == Link::Calculator && self.peer.is_none())
            || (self.link != Link::Calculator && (self.peer_extension || self.host_initialized))
        {
            return Err("USB image has inconsistent link/peer/host state".into());
        }
        for (host, endpoint) in [(false, &self.peripheral), (true, &self.host)] {
            if endpoint.callback != 0 {
                mem.validate_range(endpoint.callback & !1, 4)
                    .map_err(|e| e.to_string())?;
            }
            for (receive, queue) in [(true, &endpoint.received), (false, &endpoint.transmitted)] {
                if queue.len() > MAX_QUEUED_BYTES
                    || queue.iter().any(|bytes| {
                        bytes.is_empty()
                            || bytes.len()
                                > if receive {
                                    MAX_PACKET_BYTES
                                } else {
                                    MAX_QUEUED_BYTES
                                }
                    })
                    || queue.iter().map(Vec::len).sum::<usize>() > MAX_QUEUED_BYTES
                    || (!queue.is_empty() && (!endpoint.enabled || !self.connected(host)))
                {
                    return Err(
                        "USB image has an invalid, oversized or detached packet queue".into(),
                    );
                }
            }
        }
        Ok(Usb {
            state: Mutex::new(self.clone()),
        })
    }
    fn endpoint(&self, host: bool) -> &Endpoint {
        if host { &self.host } else { &self.peripheral }
    }
    fn endpoint_mut(&mut self, host: bool) -> &mut Endpoint {
        if host {
            &mut self.host
        } else {
            &mut self.peripheral
        }
    }
    fn connected(&self, host: bool) -> bool {
        self.link
            == if host {
                Link::Calculator
            } else {
                Link::Computer
            }
    }
}
impl Usb {
    pub fn status(&self) -> ppl_engine::usb::Status {
        let state = self.state.lock().unwrap();
        ppl_engine::usb::Status {
            link: state.link,
            peripheral_enabled: state.peripheral.enabled,
            host_enabled: state.host.enabled,
            peripheral_callback: state.peripheral.callback != 0,
            host_callback: state.host.callback != 0,
            receive_packets: state.peripheral.received.len() + state.host.received.len(),
            transmit_packets: state.peripheral.transmitted.len() + state.host.transmitted.len(),
        }
    }
    pub fn operation(
        &self,
        operation: ppl_engine::usb::Operation,
    ) -> Result<Option<Vec<u8>>, String> {
        use ppl_engine::usb::Operation;
        match operation {
            Operation::ConfigurePeer { peer } => {
                if let Some(peer) = &peer {
                    peer.validate()?;
                }
                let mut state = self.state.lock().unwrap();
                if state.link != Link::Disconnected {
                    return Err(
                        "Detach the virtual cable before changing the peer descriptor".into(),
                    );
                }
                state.peer = peer;
                Ok(None)
            }
            Operation::Connect { link } => {
                self.validate_link(link)?;
                self.connect(link);
                Ok(None)
            }
            Operation::Write { host, bytes } => {
                self.receive(host, bytes)?;
                Ok(None)
            }
            Operation::Read { host } => Ok(self.read(host)),
        }
    }
    pub fn image(&self) -> Image {
        self.state.lock().unwrap().clone()
    }
    pub fn connect(&self, link: Link) {
        let mut state = self.state.lock().unwrap();
        if state.link != link {
            // Bytes on a detached physical wire cannot reappear on a new link.
            let Image {
                peripheral, host, ..
            } = &mut *state;
            for endpoint in [peripheral, host] {
                endpoint.received.clear();
                endpoint.transmitted.clear();
            }
            state.host_initialized = false;
            state.host_address = 0;
            state.peer_extension =
                link == Link::Calculator && state.peer.as_ref().is_some_and(|p| p.extension);
            state.link = link;
        }
    }
    pub fn validate_link(&self, link: Link) -> Result<(), String> {
        if link == Link::Calculator && self.state.lock().unwrap().peer.is_none() {
            return Err(
                "Configure the virtual peer descriptor before attaching a calculator cable".into(),
            );
        }
        Ok(())
    }
    pub fn initialize_host(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.host_initialized =
            state.connected(true) && state.host.enabled && state.peer.is_some();
        state.host_address = u8::from(state.host_initialized);
        state.host_initialized
    }
    pub fn host_descriptor(&self, caller_size: u32) -> Option<[u8; 16]> {
        let state = self.state.lock().unwrap();
        if !state.connected(true) || !state.host.enabled {
            return None;
        }
        let peer = state.peer.as_ref()?;
        let mut out = [0; 16];
        out[..2].copy_from_slice(&peer.vendor.to_le_bytes());
        out[2..4].copy_from_slice(&peer.product.to_le_bytes());
        out[4..8].copy_from_slice(&u32::from(peer.class).to_le_bytes());
        out[8..10].copy_from_slice(&peer.receive_bytes.to_le_bytes());
        out[10..12].copy_from_slice(&peer.transmit_bytes.to_le_bytes());
        // The ROM copies its 16-byte saved argument area. The descriptor
        // fills twelve bytes; the final word is the original caller's r3.
        out[12..16].copy_from_slice(&caller_size.to_le_bytes());
        Some(out)
    }
    pub fn enable(&self, host: bool, enabled: bool) {
        let mut state = self.state.lock().unwrap();
        let endpoint = state.endpoint_mut(host);
        endpoint.enabled = enabled;
        if !enabled {
            endpoint.received.clear();
            endpoint.transmitted.clear();
        }
    }
    pub fn connected(&self, host: bool) -> bool {
        self.state.lock().unwrap().connected(host)
    }
    pub fn peer_extension(&self, host: bool) -> u32 {
        let state = self.state.lock().unwrap();
        u32::from(state.connected(host) && state.peer_extension)
    }
    pub fn mode(&self) -> u32 {
        let state = self.state.lock().unwrap();
        if state.connected(true) {
            2
        } else {
            u32::from(state.connected(false))
        }
    }
    pub fn callback(&self, host: bool, callback: u32) {
        self.state.lock().unwrap().endpoint_mut(host).callback = callback;
    }
    pub fn receive(&self, host: bool, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.is_empty() || bytes.len() > MAX_PACKET_BYTES {
            return Err("virtual USB packet must contain 1..=640 bytes".into());
        }
        let mut state = self.state.lock().unwrap();
        if !state.connected(host) || !state.endpoint(host).enabled {
            return Err("virtual USB endpoint is disconnected or disabled".into());
        }
        let endpoint = state.endpoint_mut(host);
        if endpoint.received.iter().map(Vec::len).sum::<usize>() + bytes.len() > MAX_QUEUED_BYTES {
            return Err("virtual USB receive queue is full".into());
        }
        endpoint.received.push_back(bytes);
        Ok(())
    }
    pub fn transmit(&self, host: bool, bytes: Vec<u8>) -> bool {
        let mut state = self.state.lock().unwrap();
        if !state.connected(host)
            || !state.endpoint(host).enabled
            || bytes.is_empty()
            || bytes.len() > MAX_QUEUED_BYTES
        {
            return false;
        }
        let endpoint = state.endpoint_mut(host);
        if endpoint.transmitted.iter().map(Vec::len).sum::<usize>() + bytes.len() > MAX_QUEUED_BYTES
        {
            return false;
        }
        endpoint.transmitted.push_back(bytes);
        true
    }
    pub fn read(&self, host: bool) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .endpoint_mut(host)
            .transmitted
            .pop_front()
    }
    pub fn take_delivery(&self) -> Option<Delivery> {
        let mut state = self.state.lock().unwrap();
        for host in [false, true] {
            if !state.connected(host) || !state.endpoint(host).enabled {
                continue;
            }
            let endpoint = state.endpoint_mut(host);
            if endpoint.callback != 0 {
                if let Some(bytes) = endpoint.received.pop_front() {
                    return Some(Delivery {
                        host,
                        callback: endpoint.callback,
                        bytes,
                    });
                }
            }
        }
        None
    }
}

/// Private adapter service, outside all vendor SDK ordinals. Its continuation
/// is ordinary serializable service data; no native callback stays parked.
pub const RECEIVE_SVC: u32 = 0xF_FFF1;
pub const RECEIVE_BYTES: usize = 4 + MAX_PACKET_BYTES;

pub fn receive_service(ctx: &mut super::SvcCtx) -> Result<u32, super::HandlerFault> {
    ctx.mem
        .validate_range(ctx.args.r0, RECEIVE_BYTES)
        .map_err(super::HandlerFault)?;
    ctx.deferred = Some(super::pending::PendingService::new(
        ctx.args,
        super::pending::Wait::Usb {
            output: ctx.args.r0,
        },
        ctx.thread_id,
    ));
    Ok(0)
}

/// The OS runs receive callbacks on a driver thread, not in the interrupted
/// UI's ISR context. This adapter uses the same ABI (r0=bytes, r1=length), so
/// callbacks may sleep/wait and retain their actual guest stack and registers.
pub fn install_worker(
    mem: &crate::memory::GuestMemory,
    shared: &super::SharedState,
) -> Result<(), String> {
    let block = mem
        .heap_alloc(48 + RECEIVE_BYTES)
        .map_err(|e| e.to_string())?;
    let words = [
        0xe1a04000u32, // mov r4,r0
        0xe1a00004,    // loop: mov r0,r4
        0xeb000004,    // bl receive adapter
        0xe1a02000,    // mov r2,r0 (callback)
        0xe5941000,    // ldr r1,[r4] (length)
        0xe2840004,    // add r0,r4,#4 (bytes)
        0xe12fff32,    // blx r2 (ARM or Thumb)
        0xeafffff8,    // b loop
        0xe92d0001,    // receive: push {r0}
        0xe92d4000,    // push {lr}
        0xef000000 | RECEIVE_SVC,
        0xeafffffe, // unreachable guard
    ];
    let result = (|| {
        for (index, word) in words.into_iter().enumerate() {
            mem.write_u32(block + index as u32 * 4, word)
                .map_err(|e| e.to_string())?;
        }
        let tid = shared.alloc_thread_id();
        shared
            .create_guest_thread(
                tid,
                super::ThreadRecord {
                    entry: block,
                    arg: block + 48,
                    stack_size: 0x4000,
                },
                false,
            )
            .ok_or("no priority slot available for virtual USB driver")?;
        Ok(())
    })();
    if result.is_err() {
        mem.heap_free(block);
    }
    result
}
