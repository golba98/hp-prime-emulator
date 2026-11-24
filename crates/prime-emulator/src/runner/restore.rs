//! Restore engines on their owning host threads before allowing any execution.
use super::*;
use crate::services::sync::image::RestoredSync;
use std::{
    collections::BTreeMap,
    sync::{Condvar, Mutex},
};
#[derive(Default)]
enum Start {
    #[default]
    Waiting,
    Run,
    Cancel,
}
#[derive(Default)]
struct Barrier {
    state: Mutex<Start>,
    wake: Condvar,
}
impl Barrier {
    fn wait(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        while matches!(*state, Start::Waiting) {
            state = self.wake.wait(state).unwrap();
        }
        matches!(*state, Start::Run)
    }
    fn set(&self, next: Start) {
        *self.state.lock().unwrap() = next;
        self.wake.notify_all();
    }
}
struct StartGuard(Option<Arc<Barrier>>);
impl Drop for StartGuard {
    fn drop(&mut self) {
        if let Some(barrier) = &self.0 {
            barrier.set(Start::Cancel);
        }
    }
}
struct RestoreBatch {
    images: BTreeMap<u32, crate::engine::ExecutionImage>,
    resources: Arc<RestoredSync>,
    barrier: Arc<Barrier>,
}
impl GuestWorkers {
    fn restore_owners(
        &self,
        mem: &Arc<GuestMemory>,
        registry: &Arc<Registry>,
        trace: &Arc<TraceHub>,
        batch: RestoreBatch,
        cfg: &RunConfig,
    ) -> Result<(), String> {
        let RestoreBatch {
            images,
            resources,
            barrier,
        } = batch;
        let mut ready = Vec::new();
        for (tid, image) in images {
            if tid == 0 {
                return Err("Main owner cannot be restored as a worker".into());
            }
            let address = *self
                .shared
                .thread_stacks
                .lock()
                .unwrap()
                .get(&tid)
                .ok_or("Restored worker has no saved stack")?;
            let mem = mem.clone();
            let registry = registry.clone();
            let trace = trace.clone();
            let shared = self.shared.clone();
            let resources = resources.clone();
            let barrier = barrier.clone();
            let policy = cfg.on_unsupported;
            let slice = cfg.slice_instructions.min(10_000);
            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::Builder::new()
                .name(format!("prime-restored-{tid}"))
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut cpu = match CpuThread::new(
                            mem.clone(),
                            registry,
                            shared.clone(),
                            trace.clone(),
                            policy,
                            tid,
                            vec![],
                        )
                        .map_err(|e| e.to_string())
                        .and_then(|mut cpu| {
                            cpu.restore_execution_image(image, &resources)?;
                            Ok(cpu)
                        }) {
                            Ok(cpu) => {
                                let _ = tx.send(Ok(()));
                                cpu
                            }
                            Err(e) => {
                                let _ = tx.send(Err(e));
                                return;
                            }
                        };
                        drop(resources);
                        if !barrier.wait() {
                            return;
                        }
                        let _stack = GuestStack {
                            mem,
                            address,
                            shared: Arc::downgrade(&shared),
                            tid,
                        };
                        if let Err(failure) =
                            drive_guest_worker(&mut cpu, &shared, &trace, tid, slice)
                        {
                            shared.note_thread_failure(failure);
                        }
                    }));
                    if result.is_err() {
                        shared.note_thread_failure(crate::services::ThreadFailure {
                            tid,
                            reason: StopReason::Fault,
                            fault: None,
                            detail: "restored host worker panicked".into(),
                        });
                    }
                })
                .map_err(|e| e.to_string())?;
            self.shared
                .thread_handles
                .lock()
                .unwrap()
                .insert(tid, handle);
            ready.push(rx);
        }
        for ready in ready {
            ready
                .recv_timeout(Duration::from_secs(30))
                .map_err(|e| format!("Restored CPU startup: {e}"))??;
        }
        Ok(())
    }
}
pub(super) fn prepare(
    cfg: &RunConfig,
    session: &Arc<crate::session::LiveSession>,
) -> Result<World, Box<dyn std::error::Error>> {
    let elf = ElfImage::load(&cfg.firmware_path)?;
    let identity = session.resource_identity()?;
    let loaded = crate::session::live_image::Loaded::open(&cfg.data_dir, &identity)?;
    let mut kernel = loaded.state.kernel.restore(
        loaded.memory.clone(),
        loaded.storage.path().into(),
        &identity,
    )?;
    kernel.shared.vfs.relocate_root(cfg.data_dir.clone())?;
    kernel.shared.data_dir = cfg.data_dir.clone();
    let shared = Arc::new(kernel.shared);
    let trace = Arc::new(
        loaded
            .state
            .observers
            .restore(&loaded.memory, cfg.trace.clone())?,
    );
    let mut registry = Registry::new();
    handlers::install(&mut registry);
    let registry = Arc::new(registry);
    let mut images = kernel.owners;
    let main = images.remove(&0).ok_or("Live machine has no main owner")?;
    let sp = main.cpu.gprs[13];
    let resources = Arc::new(kernel.synchronization);
    let mut cpu = CpuThread::new(
        loaded.memory.clone(),
        registry.clone(),
        shared.clone(),
        trace.clone(),
        cfg.on_unsupported,
        0,
        cfg.watches.clone(),
    )?;
    cpu.restore_execution_image(main, &resources)?;
    let workers = GuestWorkers::install(
        &loaded.memory,
        &registry,
        &shared,
        &trace,
        cfg.on_unsupported,
        cfg.slice_instructions.min(10_000),
    );
    let barrier = Arc::new(Barrier::default());
    // On every error path, cancel parked constructors BEFORE workers join.
    let mut start = StartGuard(Some(barrier.clone()));
    if let Err(error) = workers.restore_owners(
        &loaded.memory,
        &registry,
        &trace,
        RestoreBatch {
            images,
            resources,
            barrier: barrier.clone(),
        },
        cfg,
    ) {
        drop(start);
        workers.shutdown();
        return Err(error.into());
    }
    let outcome = (|| -> Result<ppl_engine::session::ResumeAttachment, String> {
        session.admit_live(&loaded.state, &loaded.memory, &shared, &trace)?;
        let promotion =
            crate::session::promotion::Promotion::prepare(&cfg.data_dir, loaded.storage)?;
        promotion.install()?;
        // Rebase producer deadlines only after archive I/O and CPU creation.
        let attachment = session.install_live(
            &loaded.state,
            &loaded.memory,
            &shared,
            &trace,
            loaded.previous,
        )?;
        promotion.finish()?;
        Ok(attachment)
    })();
    let attachment = match outcome {
        Ok(attachment) => attachment,
        Err(error) => {
            drop(start);
            workers.shutdown();
            return Err(error.into());
        }
    };
    session.announce_resume(attachment);
    session.capture_forced(&loaded.memory, &shared);
    drop(kernel.startup);
    start.0.take().unwrap().set(Start::Run);
    for (tid, record) in kernel.pending_starts {
        shared.spawn_thread(tid, record.entry, record.arg, record.stack_size);
    }
    let segments = elf
        .segments
        .iter()
        .map(|s| SegmentInfo {
            vaddr: s.vaddr,
            file_size: s.file_size,
            mem_size: s.mem_size,
            flags: [
                s.readable.then_some('r'),
                s.writable.then_some('w'),
                s.executable.then_some('x'),
            ]
            .into_iter()
            .flatten()
            .collect(),
        })
        .collect();
    Ok(World {
        registry,
        mem: loaded.memory,
        shared,
        trace,
        sp,
        workers,
        cpu,
        segments,
    })
}
