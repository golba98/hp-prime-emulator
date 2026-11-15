//! Coherent whole-machine capture, run by the sole session/device producer.
//! Publication handles only immutable captured data after all CPUs resume.
use super::*;
use crate::{
    guest_call::image::GuestCallsImage, memory::image::MemoryImage, services::image::KernelImage,
    storage::image::StorageImage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    version: u32,
    identity: String,
    pub kernel: KernelImage,
    pub calls: GuestCallsImage,
    pub observers: crate::trace::image::Image,
    producer: Vec<u8>,
}
pub(super) struct Captured {
    store: archive::Store,
    stage: tempfile::TempDir,
    memory: MemoryImage,
    state: State,
}
pub(crate) struct Loaded {
    pub memory: Arc<GuestMemory>,
    pub state: State,
    pub storage: tempfile::TempDir,
    pub previous: bool,
}
impl Captured {
    pub(super) fn capture(
        session: &LiveSession,
        device: &Device,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
    ) -> Result<Self> {
        let identity = device
            .recovery
            .as_ref()
            .ok_or("No resource identity/recovery owner")?
            .resource_identity();
        let store = archive::Store::new(&shared.data_dir, identity)?;
        let stage = store.stage()?;
        // Only this device producer calls capture. Its timers/input/jobs cannot
        // advance until capture returns. Other memory writers acknowledge here.
        let pause = shared.safe_points.pause(Duration::from_secs(60))?;
        let gate = mem
            .snapshot_gate
            .try_snapshot()
            .ok_or("Live capture has active memory writers")?;
        let at = Instant::now();
        let producer =
            image::Image::capture(session, device, mem, shared, trace, identity, at)?.to_bytes()?;
        let calls = shared
            .guest_calls
            .image_while_guarded(mem, identity, &gate)?;
        device
            .typing
            .validate_insertions(&shared.text_calls, calls.text_binding())?;
        calls.admit_in_world(
            mem,
            identity,
            device.jobs.output_binding(calls.binding())?,
            &shared.text_calls,
        )?;
        let observers = trace.image();
        let kernel = KernelImage::capture_while_guarded(shared, mem, &pause, &gate, identity)?;
        if !kernel.owner_ids().any(|id| id == 0) {
            return Err("Live capture has no main CPU owner".into());
        }
        let memory = MemoryImage::capture_while_guarded(mem, &gate, identity)?;
        StorageImage::capture_with_progress(
            &shared.data_dir,
            &stage.path().join("storage.bin"),
            identity,
            |_| {
                if pause.is_current() {
                    Ok(())
                } else {
                    Err(
                        "Live capture pause expired during storage copy; old archive retained"
                            .into(),
                    )
                }
            },
        )?;
        if !pause.is_current() {
            return Err("Live capture pause expired; old archive retained".into());
        }
        drop(gate);
        drop(pause);
        Ok(Self {
            store,
            stage,
            memory,
            state: State {
                version: 1,
                identity: identity.into(),
                kernel,
                calls,
                observers,
                producer,
            },
        })
    }
    pub(super) fn publish(self) -> Result<PathBuf> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.stage.path().join("memory.gz"))
            .map_err(|e| e.to_string())?;
        let mut compressor = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        self.memory.write_to(&mut compressor)?;
        compressor
            .finish()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(&self.state).map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("Live session state exceeds byte budget".into());
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.stage.path().join("state.json"))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        self.store.publish(self.stage)
    }
}
impl Loaded {
    pub(crate) fn open(profile: &Path, identity: &str) -> Result<Self> {
        let loaded = archive::Store::new(profile, identity)?.load()?;
        let mut bytes = Vec::new();
        fs::File::open(loaded.path.join("state.json"))
            .map_err(|e| e.to_string())?
            .take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 128 * 1024 * 1024 {
            return Err("Live session state exceeds byte budget".into());
        }
        let state: State = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if state.version != 1 || state.identity != identity {
            return Err("Live machine version/resource mismatch".into());
        }
        let compressed =
            fs::File::open(loaded.path.join("memory.gz")).map_err(|e| e.to_string())?;
        let memory = Arc::new(
            MemoryImage::read_from(flate2::read::MultiGzDecoder::new(compressed), identity)?
                .restore(identity)?,
        );
        let storage =
            StorageImage::open(&loaded.path.join("storage.bin"), identity)?.restore(profile)?;
        Ok(Self {
            memory,
            state,
            storage,
            previous: loaded.previous,
        })
    }
}
impl State {
    pub(super) fn producer(
        &self,
        session: &LiveSession,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
        at: Instant,
    ) -> Result<image::Restored> {
        image::Image::from_bytes(&self.producer)?.restore(
            mem,
            shared,
            trace,
            &self.identity,
            at,
            session.device.lock().unwrap().recovery.clone(),
        )
    }
}
