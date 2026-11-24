//! Preserve observer context and captures which are deliberately omitted from
//! ordinary transport diagnostics. A live continuation must not lose them.
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    version: u32,
    trace: RuntimeTrace,
    program_context: Option<u32>,
    discover_source: bool,
    frames: Vec<Option<Frame>>,
    writes_before: Vec<u64>,
}
impl Observer {
    pub fn image(&self) -> Image {
        let state = self.0.lock().unwrap();
        Image {
            version: 1,
            trace: state.trace.clone(),
            program_context: state.program_context,
            discover_source: state.discover_source,
            frames: state
                .trace
                .invocations
                .iter()
                .map(|i| i.frame.clone())
                .collect(),
            writes_before: state
                .trace
                .invocations
                .iter()
                .map(|i| i.writes_before)
                .collect(),
        }
    }
}
impl Image {
    pub fn restore(&self, mem: &crate::memory::GuestMemory) -> Result<Observer, String> {
        let count = self.trace.invocations.len();
        if self.version != 1
            || count > MAX_INVOCATIONS
            || self.frames.len() != count
            || self.writes_before.len() != count
        {
            return Err("Runtime observer image version/invocation/capture mismatch".into());
        }
        if let Some(context) = self.program_context.filter(|p| *p != 0) {
            mem.validate_range(context, 4).map_err(|e| e.to_string())?;
        }
        let valid_time =
            |value: f64| value.is_finite() && (0.0..=365.0 * 86400000.0).contains(&value);
        for (index, invocation) in self.trace.invocations.iter().enumerate() {
            if invocation.id != index
                || !valid_time(invocation.issued_ms)
                || invocation.started_ms.is_some_and(|v| !valid_time(v))
                || invocation.ended_ms.is_some_and(|v| !valid_time(v))
                || invocation.calls.len() > MAX_EVENTS
                || invocation.output.len() > MAX_EVENTS
                || invocation.errors.len() > MAX_EVENTS
                || invocation.checkpoints.len() > MAX_EVENTS
                || self.frames[index].as_ref().is_some_and(|f| {
                    f.width != 320 || f.height != 240 || f.pixels.len() != 320 * 240
                })
            {
                return Err(
                    "Runtime observer image sequence/timing/display/event bounds are invalid"
                        .into(),
                );
            }
        }
        let mut trace = self.trace.clone();
        for (index, invocation) in trace.invocations.iter_mut().enumerate() {
            invocation.frame = self.frames[index].clone();
            invocation.writes_before = self.writes_before[index];
        }
        Ok(Observer(Mutex::new(Inner {
            trace,
            program_context: self.program_context,
            discover_source: self.discover_source,
        })))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_observer_keeps_context_frame_and_write_baseline_after_roundtrip() {
        let mut mem = crate::memory::GuestMemory::new();
        mem.map("context", 0x1000, 4096).unwrap();
        let observer = Observer::default();
        observer
            .arm(
                0x1000,
                "source",
                &ExecutionRequest {
                    entry: "PROGRAM".into(),
                    arguments: vec![],
                },
                10.,
            )
            .unwrap();
        let frame = Frame {
            width: 320,
            height: 240,
            pixels: (0..320 * 240).collect(),
        };
        {
            let mut inner = observer.0.lock().unwrap();
            inner.discover_source = true;
            let call = &mut inner.trace.invocations[0];
            call.started_ms = Some(11.);
            call.body_entered = true;
            call.frame = Some(frame.clone());
            call.writes_before = 39;
        }
        let image: Image =
            serde_json::from_slice(&serde_json::to_vec(&observer.image()).unwrap()).unwrap();
        let restored = image.restore(&mem).unwrap();
        assert!(restored.unfinished());
        assert_eq!(restored.progress(), (true, false));
        {
            let inner = restored.0.lock().unwrap();
            assert_eq!(inner.program_context, Some(0x1000));
            assert!(inner.discover_source);
            assert_eq!(inner.trace.invocations[0].frame.as_ref(), Some(&frame));
            assert_eq!(inner.trace.invocations[0].writes_before, 39);
        }
        restored.finish_invocation(ExecutionStatus::Cancelled);
        assert!(!restored.unfinished());
        let mut corrupt = image;
        corrupt.frames[0].as_mut().unwrap().pixels.pop();
        assert!(corrupt.restore(&mem).is_err());
    }
}
