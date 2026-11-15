//! Calculator-side producer state. Host transport threads and a recovery-store
//! owner are rebound by the enclosing restorer, never serialized as pointers.
//! An unfinished storage publisher is an explicit capture-admission refusal.
use super::*;
use crate::image_time::Portable;
use serde::{Deserialize, Serialize};
const MAGIC: &[u8; 8] = b"HPSESS01";
const MAX_BYTES: usize = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Image {
    version: u32,
    identity: String,
    device: Box<serde_json::value::RawValue>,
    started: i64,
    generation: u64,
    next_frame: i64,
    last_frame: Vec<u8>,
}
pub(super) struct Restored {
    pub device: Device,
    pub capture: Capture,
    pub started: Instant,
}
fn validate(
    device: &Device,
    mem: &GuestMemory,
    shared: &SharedState,
    trace: &TraceHub,
) -> Result<(), String> {
    if device.save_worker.is_some() {
        return Err("A host storage publication is still in progress; its native receiver cannot be captured".into());
    }
    if device.request_high_water == u64::MAX
        || device.keys.len() > 65536
        || device.held.len() > 65536
        || device.delivered.len() > 65536
        || device
            .scenario_touch
            .is_some_and(|(_, x, y)| x >= 320 || y >= 240)
        || device
            .typing_ack
            .baseline()
            .is_some_and(|n| n > shared.main_wait_entries())
    {
        return Err("Session producer image has invalid key/touch/acknowledgement state".into());
    }
    if device
        .jobs
        .resume_requests()
        .iter()
        .any(|r| r.request > device.request_high_water)
        || device
            .input_cleanup
            .as_ref()
            .is_some_and(|(id, _, _)| *id > device.request_high_water)
        || device
            .checkpoint
            .as_ref()
            .is_some_and(|c| c.id > device.request_high_water)
    {
        return Err("Session producer request floor excludes pending operations".into());
    }
    device.typing.validate_image()?;
    device
        .typing
        .validate_insertions(&shared.text_calls, None)?;
    device.repeat.validate_image()?;
    device.jobs.validate_image(mem)?;
    device.jobs.validate_trace(trace)?;
    if let Some(script) = &device.script {
        script.validate_image()?;
    }
    let mut seen = 0;
    let _ = trace.unsupported_sites_since(&mut seen);
    if device.unsupported_seen > seen {
        return Err("Unsupported-service cursor exceeds restored observations".into());
    }
    Ok(())
}
impl Image {
    pub(super) fn capture(
        session: &LiveSession,
        device: &Device,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
        identity: &str,
        at: Instant,
    ) -> Result<Self, String> {
        validate(device, mem, shared, trace)?;
        let capture = session
            .capture
            .lock()
            .map_err(|_| "Frame producer lock is poisoned")?;
        let body = crate::image_time::at(at, || serde_json::to_string(device))
            .map_err(|e| e.to_string())?;
        if body.len() > MAX_BYTES {
            return Err("Session producer image exceeds byte budget".into());
        }
        let (started, next_frame) = crate::image_time::at(at, || {
            Ok::<_, String>((
                session
                    .started
                    .lock()
                    .map_err(|_| "Session origin lock is poisoned")?
                    .image()?,
                capture.next.image()?,
            ))
        })?;
        Ok(Self {
            version: 1,
            identity: identity.into(),
            device: serde_json::value::RawValue::from_string(body).map_err(|e| e.to_string())?,
            started,
            generation: capture.generation,
            next_frame,
            last_frame: capture.last.clone(),
        })
    }
    pub(super) fn restore(
        &self,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
        identity: &str,
        at: Instant,
        recovery: Option<Arc<crate::recovery::Recovery>>,
    ) -> Result<Restored, String> {
        if self.version != 1
            || identity.is_empty()
            || identity.len() > 256
            || self.identity != identity
            || self.device.get().len() > MAX_BYTES
            || self.last_frame.len() != 320 * 240 * 4
            || self.generation == u64::MAX
            || self.started > 0
        {
            return Err("Session image identity/version/timing/frame/budget mismatch".into());
        }
        let mut device: Device =
            crate::image_time::at(at, || serde_json::from_str(self.device.get()))
                .map_err(|e| e.to_string())?;
        validate(&device, mem, shared, trace)?;
        if recovery
            .as_ref()
            .is_some_and(|owner| owner.resource_identity() != identity)
        {
            return Err("Session recovery owner has a different resource identity".into());
        }
        device.recovery = recovery;
        let (started, next) = crate::image_time::at(at, || {
            Ok::<_, String>((
                Instant::restore(self.started)?,
                Instant::restore(self.next_frame)?,
            ))
        })?;
        Ok(Restored {
            device,
            started,
            capture: Capture {
                last: self.last_frame.clone(),
                scratch: vec![0; 320 * 240 * 4],
                generation: self.generation,
                next,
            },
        })
    }
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let body = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if body.len() > MAX_BYTES {
            return Err("Session image exceeds byte budget".into());
        }
        let mut bytes = Vec::with_capacity(80 + body.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(crate::sha256::hex(&body).as_bytes());
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }
    pub(super) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 80
            || bytes.len() > MAX_BYTES + 80
            || &bytes[..8] != MAGIC
            || u64::from_le_bytes(bytes[8..16].try_into().unwrap()) != (bytes.len() - 80) as u64
            || crate::sha256::hex(&bytes[80..]).as_bytes() != &bytes[16..80]
        {
            return Err("Session image header/length/checksum mismatch".into());
        }
        serde_json::from_slice(&bytes[80..]).map_err(|e| e.to_string())
    }
}
impl Restored {
    pub(super) fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({"captured":true,"lifecycle":self.device.lifecycle,"active_request":self.device.jobs.active.as_ref().map(|job|job.request),
            "typing_pending":self.device.typing.pending_len(),"queued_keys":self.device.keys.len(),"held_keys":self.device.held.len(),
            "frame_generation":self.capture.generation,"elapsed_ms":self.started.elapsed().as_millis()})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(at: Instant) -> (LiveSession, GuestMemory, SharedState, TraceHub) {
        let (_, rx) = std::sync::mpsc::channel();
        let session = LiveSession {
            commands: Mutex::new(rx),
            output: Arc::new((Mutex::new(Output::default()), Condvar::new())),
            device: Mutex::new(super::super::output_delivery_tests::test_device()),
            capture: Mutex::new(Capture {
                last: vec![0; 320 * 240 * 4],
                scratch: vec![0; 320 * 240 * 4],
                generation: 7,
                next: at + Duration::from_millis(16),
            }),
            started: Mutex::new(at),
            shutdown: AtomicBool::new(false),
            writer: Mutex::new(None),
            live_publication: Mutex::new(None),
        };
        let mut mem = GuestMemory::new();
        mem.map("producer", 0x1000, 65536).unwrap();
        mem.set_heap(0x1000, 65536);
        let shared = SharedState::new(Default::default(), "A:/producer-test".into());
        let trace = TraceHub::new(crate::trace::TraceConfig {
            verbosity: 0,
            ..Default::default()
        })
        .unwrap();
        (session, mem, shared, trace)
    }
    #[test]
    fn queued_template_typing_and_repeat_keep_their_exact_resume_position_and_deadline() {
        let at = Instant::now();
        let (session, mem, shared, trace) = fixture(at);
        shared.set_main_waiting(true);
        let mut source = session.device.lock().unwrap();
        source.typing.paste("2^(3+4)^5+6").unwrap();
        let modifiers = Some(super::super::typing::Modifiers {
            lower: None,
            shift: false,
        });
        for _ in 0..3 {
            source.typing.next(modifiers).unwrap();
        }
        source.typing_ack.pressed(shared.main_wait_entries());
        source.keys.extend([(12, false), (12, true)]);
        source.held.insert(12);
        source.delivered.insert(12);
        source.repeat.press(12, at);
        let captured_at = at + Duration::from_millis(449);
        let image = Image::capture(
            &session,
            &source,
            &mem,
            &shared,
            &trace,
            "resources",
            captured_at,
        )
        .unwrap();
        let bytes = image.to_bytes().unwrap();
        let decoded = Image::from_bytes(&bytes).unwrap();
        let resume_at = captured_at + Duration::from_secs(90);
        let mut restored = decoded
            .restore(&mem, &shared, &trace, "resources", resume_at, None)
            .unwrap();
        assert_eq!(restored.device.keys, source.keys);
        assert_eq!(restored.device.held, source.held);
        assert_eq!(restored.device.delivered, source.delivered);
        assert_eq!(restored.capture.generation, 7);
        assert_eq!(restored.capture.last.len(), 320 * 240 * 4);
        assert!(
            !restored
                .device
                .typing_ack
                .ready(true, shared.main_wait_entries())
        );
        shared.set_main_waiting(false);
        shared.set_main_waiting(true);
        assert!(
            restored
                .device
                .typing_ack
                .ready(true, shared.main_wait_entries())
        );
        assert_eq!(restored.device.repeat.poll(resume_at), None);
        assert_eq!(
            restored
                .device
                .repeat
                .poll(resume_at + Duration::from_millis(1)),
            Some((12, false))
        );
        assert_eq!(
            restored
                .device
                .repeat
                .poll(resume_at + Duration::from_millis(2)),
            Some((12, true))
        );
        let next = restored.device.typing.next(modifiers).unwrap();
        assert_eq!(next, vec![(0x33, true), (0x33, false)]);
        assert_eq!(source.typing.next(modifiers).unwrap(), next);
        let mut transitions = 0;
        while !source.typing.is_empty() {
            let expected = source.typing.next(modifiers).unwrap();
            assert_eq!(restored.device.typing.next(modifiers).unwrap(), expected);
            transitions += expected.len();
        }
        assert!(transitions > 10);
        assert!(restored.device.typing.is_empty());
        assert!(
            decoded
                .restore(&mem, &shared, &trace, "other-assets", resume_at, None)
                .is_err()
        );
        let mut corrupt = bytes;
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(Image::from_bytes(&corrupt).is_err());
    }
    #[test]
    fn a_native_storage_receiver_is_refused_instead_of_discarded_from_an_image() {
        let at = Instant::now();
        let (session, mem, shared, trace) = fixture(at);
        let mut device = session.device.lock().unwrap();
        let (_sender, receiver) = std::sync::mpsc::channel();
        device.save_worker = Some(receiver);
        let result = Image::capture(&session, &device, &mem, &shared, &trace, "resources", at);
        assert!(matches!(result,Err(error) if error.contains("storage publication")));
        assert!(
            device.save_worker.is_some(),
            "Failed admission must preserve the live publisher"
        );
    }
}
