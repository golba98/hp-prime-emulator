//! Managed request state; the restored compiler plan binds the exact channel
//! reconstructed here instead of inventing a new unrelated output sink.
use super::*;
use serde::{Deserialize, Serialize};
pub(super) mod output {
    use super::*;
    #[derive(Serialize, Deserialize)]
    #[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
    enum Image {
        Absent,
        Channel { outcome: Option<direct::Outcome> },
    }
    pub fn serialize<S: serde::Serializer>(
        value: &Option<DirectOutput>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let image = match value {
            None => Image::Absent,
            Some(out) => Image::Channel {
                outcome: out
                    .lock()
                    .map_err(|_| serde::ser::Error::custom("Compiler output channel is poisoned"))?
                    .clone(),
            },
        };
        image.serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DirectOutput>, D::Error> {
        Ok(match Image::deserialize(deserializer)? {
            Image::Absent => None,
            Image::Channel { outcome } => Some(Arc::new(Mutex::new(outcome))),
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::session) struct Image {
    version: u32,
    identity: String,
    state: Vec<u8>,
}
impl Jobs {
    pub(in crate::session) fn inspect_capture(
        &self,
        mem: &GuestMemory,
        shared: &SharedState,
        trace: &TraceHub,
        identity: &str,
        gate: &crate::memory::snapshot::Snapshot<'_>,
    ) -> Result<(), String> {
        let calls = shared
            .guest_calls
            .image_while_guarded(mem, identity, gate)?;
        let at = Instant::now();
        let image = self.image(mem, identity, at)?;
        let restored = image.restore(mem, identity, at)?;
        restored.output_binding(calls.binding())?;
        let observers = trace.image();
        let mut cfg = trace.config().clone();
        cfg.jsonl_path = None;
        let restored_trace = observers.restore(mem, cfg)?;
        restored.validate_trace(&restored_trace)?;
        Ok(())
    }
    pub(in crate::session) fn validate_trace(&self, trace: &TraceHub) -> Result<(), String> {
        let files = trace.fs_log_since(usize::MAX).1;
        let checks = trace
            .compiler
            .as_ref()
            .map(|o| o.snapshot().checks.len())
            .unwrap_or(0);
        if self.seen_fs > files
            || self.user_seen > files
            || self
                .active
                .as_ref()
                .is_some_and(|job| job.catalog_start > files)
            || self.prepared.as_ref().is_some_and(|program| {
                program.fs_cursor > files || program.compiler_cursor > checks
            })
        {
            return Err("Managed job catalog/compiler cursors exceed restored observations".into());
        }
        Ok(())
    }
    pub(in crate::session) fn image(
        &self,
        mem: &GuestMemory,
        identity: &str,
        at: Instant,
    ) -> Result<Image, String> {
        self.validate_image(mem)?;
        let state =
            crate::image_time::at(at, || serde_json::to_vec(self)).map_err(|e| e.to_string())?;
        if state.len() > 16 * 1024 * 1024 {
            return Err("Managed job image exceeds byte budget".into());
        }
        Ok(Image {
            version: 1,
            identity: identity.into(),
            state,
        })
    }
    pub(in crate::session) fn validate_image(&self, mem: &GuestMemory) -> Result<(), String> {
        if self.catalog.len() > 65536
            || self.observed.len() > 65536
            || self.user_burst.len() > 65536
            || self.break_presses.0 > BREAK_PRESSES
        {
            return Err("Managed job image exceeds catalog/interruption bounds".into());
        }
        for address in [
            self.loader_context,
            self.named_loader_context,
            self.importer_context,
            self.direct_ctx,
        ]
        .into_iter()
        .flatten()
        {
            mem.validate_range(address, 4).map_err(|e| e.to_string())?;
        }
        if let Some(job) = &self.active {
            if job.source.encode_utf16().count() > crate::compiler::MAX_SOURCE_UNITS
                || job.arguments.len() > 1024 * 1024
                || job.timeline.len() > 65536
            {
                return Err("Managed request exceeds source/argument/timeline bounds".into());
            }
            if job.direct_text != 0 {
                let units = job.source.encode_utf16().count();
                if mem
                    .heap_size_of(job.direct_text)
                    .is_none_or(|n| n < (units + 1) * 2)
                    || crate::compiler::read_utf16(mem, job.direct_text, units + 1)? != job.source
                {
                    return Err(
                        "Managed compiler text differs from the owning source revision".into(),
                    );
                }
            }
            if job.stage == Stage::Direct
                && (job.direct_out.is_none()
                    || job.direct_text == 0
                    || job.direct_started.is_none())
            {
                return Err("Direct compiler request lacks its source/output/timing state".into());
            }
            if let Some(script) = &job.script {
                script.validate_image()?;
            }
        }
        Ok(())
    }
    pub(in crate::session) fn output_binding(
        &self,
        binding: Option<&crate::direct::image::Binding>,
    ) -> Result<Option<crate::guest_call::image::OutputBinding<'_>>, String> {
        let Some(binding) = binding else {
            return Ok(None);
        };
        let job = self
            .active
            .as_ref()
            .ok_or("Compiler plan has no owning job")?;
        if job.stage != Stage::Direct
            || crate::direct::image::Binding::new(job.request, &job.source)? != *binding
        {
            return Err("Compiler plan belongs to a different request/source revision".into());
        }
        let output = job
            .direct_out
            .as_ref()
            .ok_or("Compiler job has no bound output")?
            .clone();
        if output
            .lock()
            .map_err(|_| "Compiler output channel is poisoned")?
            .is_some()
        {
            return Err("Active compiler plan already has a completed outcome".into());
        }
        Ok(Some(crate::guest_call::image::OutputBinding {
            request: job.request,
            source: &job.source,
            output,
        }))
    }
}
impl Image {
    pub(in crate::session) fn restore(
        &self,
        mem: &GuestMemory,
        identity: &str,
        at: Instant,
    ) -> Result<Jobs, String> {
        if self.version != 1
            || identity.is_empty()
            || self.identity != identity
            || self.state.len() > 16 * 1024 * 1024
        {
            return Err("Managed job image version/identity/budget mismatch".into());
        }
        let state: Jobs = crate::image_time::at(at, || serde_json::from_slice(&self.state))
            .map_err(|e| e.to_string())?;
        state.validate_image(mem)?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory() -> GuestMemory {
        let mut mem = GuestMemory::new();
        mem.map("job-source", 0x20000000, 65536).unwrap();
        mem.set_heap(0x20000000, 65536);
        mem
    }
    #[test]
    fn active_compiler_job_rebases_time_and_rebinds_its_actual_result_channel() {
        let mem = memory();
        let now = Instant::now();
        let source = "EXPORT T() BEGIN RETURN 37; END;";
        let mut jobs = Jobs::new();
        let mut job = Jobs::named_job(71, "ZZT_TEST".into(), source.into());
        job.stage = Stage::Direct;
        job.direct = true;
        job.started = now - Duration::from_secs(2);
        job.direct_started = Some(now - Duration::from_millis(50));
        job.direct_out = Some(Arc::new(Mutex::new(None)));
        let units = source.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        job.direct_text = mem.heap_alloc(units.len() * 2).unwrap();
        mem.write_bytes(
            job.direct_text,
            &units
                .iter()
                .flat_map(|u| u.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        jobs.active = Some(job);
        jobs.interrupted = Some(now - Duration::from_millis(25));
        let image = jobs.image(&mem, "firmware", now).unwrap();
        let encoded = serde_json::to_vec(&image).unwrap();
        let image: Image = serde_json::from_slice(&encoded).unwrap();
        let later = now + Duration::from_secs(90);
        let restored = image.restore(&mem, "firmware", later).unwrap();
        let job = restored.active.as_ref().unwrap();
        assert_eq!(later.duration_since(job.started), Duration::from_secs(2));
        assert_eq!(
            later.duration_since(job.direct_started.unwrap()),
            Duration::from_millis(50)
        );
        assert_eq!(
            later.duration_since(restored.interrupted.unwrap()),
            Duration::from_millis(25)
        );
        assert!(image.restore(&mem, "other-firmware", later).is_err());
        let binding = crate::direct::image::Binding::new(71, source).unwrap();
        let channel = restored.output_binding(Some(&binding)).unwrap().unwrap();
        assert!(Arc::ptr_eq(
            &channel.output,
            job.direct_out.as_ref().unwrap()
        ));
        let wrong = crate::direct::image::Binding::new(72, source).unwrap();
        assert!(restored.output_binding(Some(&wrong)).is_err());
        let wrong = crate::direct::image::Binding::new(71, "other source").unwrap();
        assert!(restored.output_binding(Some(&wrong)).is_err());
        *channel.output.lock().unwrap() = Some(direct::Outcome {
            message: Some("compiler outcome delivered".into()),
            ..Default::default()
        });
        assert_eq!(
            job.direct_out
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .message
                .as_deref(),
            Some("compiler outcome delivered")
        );
        assert!(
            restored.output_binding(Some(&binding)).is_err(),
            "Cannot bind a still-active plan to an already-completed channel"
        );
    }
    #[test]
    fn completed_but_unconsumed_compiler_outcome_survives_serialization() {
        let mem = memory();
        let now = Instant::now();
        let mut jobs = Jobs::new();
        let mut job = Jobs::named_job(72, "ZZT_TEST".into(), String::new());
        job.direct_out = Some(Arc::new(Mutex::new(Some(direct::Outcome {
            return_value: Some(123),
            ..Default::default()
        }))));
        jobs.active = Some(job);
        let restored = jobs
            .image(&mem, "firmware", now)
            .unwrap()
            .restore(&mem, "firmware", now)
            .unwrap();
        assert_eq!(
            restored
                .active
                .unwrap()
                .direct_out
                .unwrap()
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .return_value,
            Some(123)
        );
        jobs.active.as_mut().unwrap().direct_out = Some(Arc::new(Mutex::new(None)));
        let restored = jobs
            .image(&mem, "firmware", now)
            .unwrap()
            .restore(&mem, "firmware", now)
            .unwrap();
        assert!(
            restored
                .active
                .unwrap()
                .direct_out
                .unwrap()
                .lock()
                .unwrap()
                .is_none()
        );
        jobs.active.as_mut().unwrap().direct_out = None;
        let restored = jobs
            .image(&mem, "firmware", now)
            .unwrap()
            .restore(&mem, "firmware", now)
            .unwrap();
        assert!(restored.active.unwrap().direct_out.is_none());
    }
}
