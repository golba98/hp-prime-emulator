//! Explicit firmware-OFF persistence transaction. No live-machine snapshots.
//!
//! The session owns command admission and key delivery. This controller never
//! stops a running program or navigates an editor silently: a busy request is
//! deferred before any OFF keys are sent.
use ppl_engine::checkpoint::{CheckpointReceipt, CheckpointResult};
use std::time::{Duration, Instant};

pub const LIMIT: Duration = Duration::from_secs(30);

/// A single firmware-native OFF attempt. Mode is observed from firmware, not
/// remembered by the host. The owner admits advance only at a coherent, truly
/// blocked main-UI boundary with all preceding input consumed.
pub struct OffSequence {
    phase: OffPhase,
    native_unknown_fallback: bool,
}
enum OffPhase {
    Observe,
    AwaitShift(Option<u64>),
    AwaitSleep,
    Unsupported,
}
pub enum OffAction {
    Shift,
    On,
    Unsupported,
}
impl OffSequence {
    pub fn new(native_unknown_fallback: bool) -> Self {
        Self {
            phase: OffPhase::Observe,
            native_unknown_fallback,
        }
    }
    pub fn phase(&self) -> &'static str {
        match self.phase {
            OffPhase::Observe => "observe_shift",
            OffPhase::AwaitShift(None) => "shift_queued",
            OffPhase::AwaitShift(Some(_)) => "await_shift_ack",
            OffPhase::AwaitSleep => "await_firmware_sleep",
            OffPhase::Unsupported => "unsupported_status",
        }
    }
    pub fn shift_delivered(&mut self, main_wait: u64) {
        if matches!(self.phase, OffPhase::AwaitShift(None)) {
            self.phase = OffPhase::AwaitShift(Some(main_wait));
        }
    }
    pub fn advance(&mut self, shift: Option<bool>, main_wait: u64) -> Option<OffAction> {
        match self.phase {
            OffPhase::Observe => match shift {
                Some(true) => {
                    self.phase = OffPhase::AwaitSleep;
                    Some(OffAction::On)
                }
                Some(false) => {
                    self.phase = OffPhase::AwaitShift(None);
                    Some(OffAction::Shift)
                }
                None if self.native_unknown_fallback => {
                    self.phase = OffPhase::AwaitShift(None);
                    Some(OffAction::Shift)
                }
                None => {
                    self.phase = OffPhase::Unsupported;
                    Some(OffAction::Unsupported)
                }
            },
            OffPhase::AwaitShift(Some(after)) if main_wait > after => match shift {
                Some(true) => {
                    self.phase = OffPhase::AwaitSleep;
                    Some(OffAction::On)
                }
                None if self.native_unknown_fallback => {
                    self.phase = OffPhase::AwaitSleep;
                    Some(OffAction::On)
                }
                // Never press Shift again to manufacture an acknowledgement.
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Admission {
    pub ready: bool,
    pub sleeping: bool,
    pub recovery_available: bool,
    pub program_active: bool,
    pub guest_call_active: bool,
    pub script_active: bool,
    pub input_pending: bool,
    pub keys_held: bool,
    pub shutdown_active: bool,
}
impl Admission {
    fn refusal(&self) -> Option<&'static str> {
        if !self.recovery_available {
            Some("persistent recovery storage is unavailable")
        } else if self.shutdown_active {
            Some("calculator shutdown is in progress")
        } else if self.program_active || self.guest_call_active {
            Some("finish or stop the active firmware operation before checkpointing")
        } else if self.script_active {
            Some("an input script is active")
        } else if self.keys_held || self.input_pending {
            Some("release calculator keys and finish queued input before checkpointing")
        } else if !self.ready && !self.sleeping {
            Some("the firmware has not reached its idle input boundary")
        } else {
            None
        }
    }
}

pub struct Checkpoint {
    pub id: u64,
    wake: bool,
    started: Instant,
    off_sent: bool,
    finished: bool,
}
pub enum Action {
    /// Deliver the calculator's SHIFT, ON sequence through normal key pacing.
    PowerOff,
    /// Publish exactly once. `wake` requests ON after the save has committed;
    /// callers must still await the ordinary Ready lifecycle before more work.
    Finished {
        result: CheckpointResult,
        wake: bool,
    },
}
impl Checkpoint {
    pub fn off_sent(&self) -> bool {
        self.off_sent
    }
    pub fn begin(id: u64, wake: bool, admission: &Admission) -> Result<Self, CheckpointResult> {
        if let Some(reason) = admission.refusal() {
            return Err(CheckpointResult::Deferred {
                reason: reason.into(),
            });
        }
        Ok(Self {
            id,
            wake,
            started: Instant::now(),
            off_sent: admission.sleeping,
            finished: false,
        })
    }

    /// `saved` must belong to this currently sleeping generation, not an older
    /// successful OFF. The session matches its `saved_sleep` counter first.
    pub fn poll(
        &mut self,
        sleeping: bool,
        idle: bool,
        saved: Option<&Result<CheckpointReceipt, String>>,
    ) -> Option<Action> {
        self.poll_at(Instant::now(), sleeping, idle, saved)
    }
    fn poll_at(
        &mut self,
        now: Instant,
        sleeping: bool,
        idle: bool,
        saved: Option<&Result<CheckpointReceipt, String>>,
    ) -> Option<Action> {
        if self.finished {
            return None;
        }
        let result = if sleeping {
            saved.map(|saved| match saved {
                Ok(receipt) => CheckpointResult::Saved {
                    receipt: receipt.clone(),
                },
                Err(message) => CheckpointResult::Failed {
                    message: message.clone(),
                },
            })
        } else {
            None
        };
        if let Some(result) = result {
            self.finished = true;
            let wake = self.wake && matches!(result, CheckpointResult::Saved { .. });
            return Some(Action::Finished { result, wake });
        }
        if now.saturating_duration_since(self.started) >= LIMIT {
            self.finished = true;
            return Some(Action::Finished {
                result: CheckpointResult::Failed { message: "firmware OFF did not produce a durable checkpoint before the deadline; previous generations retained".into() },
                wake: false,
            });
        }
        if !self.off_sent && !sleeping && idle {
            self.off_sent = true;
            return Some(Action::PowerOff);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ppl_engine::checkpoint::{CheckpointKind, CheckpointSafePoint, RECEIPT_VERSION};

    fn admission() -> Admission {
        Admission {
            ready: true,
            recovery_available: true,
            ..Default::default()
        }
    }
    fn receipt() -> CheckpointReceipt {
        CheckpointReceipt {
            version: RECEIPT_VERSION,
            kind: CheckpointKind::FirmwarePersistence,
            safe_point: CheckpointSafePoint::FirmwareOff,
            generation: "generation-test".into(),
            firmware: "fw".into(),
            manifest_sha256: "digest".into(),
            bytes: 10,
            files: 1,
            restores_cpu: false,
            restores_framebuffer: false,
        }
    }
    #[test]
    fn off_sequence_respects_latched_shift_and_waits_for_actual_press_acknowledgement() {
        let mut active = OffSequence::new(false);
        assert!(matches!(active.advance(Some(true), 7), Some(OffAction::On)));
        assert!(active.advance(Some(true), 8).is_none());
        let mut neutral = OffSequence::new(false);
        assert!(matches!(
            neutral.advance(Some(false), 7),
            Some(OffAction::Shift)
        ));
        assert!(
            neutral.advance(Some(true), 8).is_none(),
            "an undelivered Shift has no acknowledgement"
        );
        neutral.shift_delivered(9);
        assert!(neutral.advance(Some(true), 9).is_none());
        assert!(
            neutral.advance(Some(false), 10).is_none(),
            "an acknowledged but inactive Shift must never produce ON or a retry"
        );
        assert!(matches!(
            neutral.advance(Some(true), 10),
            Some(OffAction::On)
        ));
        assert!(neutral.advance(Some(true), 11).is_none());
    }
    #[test]
    fn unknown_checkpoint_context_is_refused_before_any_key() {
        assert!(matches!(
            OffSequence::new(false).advance(None, 0),
            Some(OffAction::Unsupported)
        ));
        let mut shutdown = OffSequence::new(true);
        assert!(matches!(shutdown.advance(None, 0), Some(OffAction::Shift)));
        shutdown.shift_delivered(0);
        assert!(shutdown.advance(None, 0).is_none());
        assert!(matches!(shutdown.advance(None, 1), Some(OffAction::On)));
    }
    #[test]
    fn busy_input_and_guest_operations_are_deferred_before_off() {
        let mut a = admission();
        a.program_active = true;
        assert!(matches!(
            Checkpoint::begin(1, true, &a),
            Err(CheckpointResult::Deferred { .. })
        ));
        a.program_active = false;
        a.guest_call_active = true;
        assert!(Checkpoint::begin(1, true, &a).is_err());
        a.guest_call_active = false;
        a.keys_held = true;
        assert!(Checkpoint::begin(1, true, &a).is_err());
        a.keys_held = false;
        a.input_pending = true;
        assert!(Checkpoint::begin(1, true, &a).is_err());
        a.input_pending = false;
        a.ready = false;
        assert!(Checkpoint::begin(1, true, &a).is_err());
    }
    #[test]
    fn completion_requires_sleep_and_durable_result_and_is_emitted_once() {
        let mut c =
            Checkpoint::begin(2, true, &admission()).unwrap_or_else(|_| panic!("admission"));
        let saved = Ok(receipt());
        assert!(!c.off_sent());
        assert!(c.poll(false, false, None).is_none());
        assert!(matches!(c.poll(false, true, None), Some(Action::PowerOff)));
        assert!(c.off_sent());
        assert!(c.poll(false, true, Some(&saved)).is_none());
        assert!(c.poll(true, false, None).is_none());
        assert!(matches!(
            c.poll(true, false, Some(&saved)),
            Some(Action::Finished {
                result: CheckpointResult::Saved { .. },
                wake: true
            })
        ));
        assert!(c.poll(true, false, Some(&saved)).is_none());
    }
    #[test]
    fn save_failure_does_not_wake_or_claim_saved_state() {
        let mut c =
            Checkpoint::begin(3, true, &admission()).unwrap_or_else(|_| panic!("admission"));
        let failed = Err("injected disk failure".into());
        assert!(matches!(
            c.poll(true, false, Some(&failed)),
            Some(Action::Finished {
                result: CheckpointResult::Failed { .. },
                wake: false
            })
        ));
    }
    #[test]
    fn sleeping_checkpoint_avoids_duplicate_off_and_timeout_is_explicit() {
        let a = Admission {
            sleeping: true,
            ready: false,
            ..admission()
        };
        let mut c = Checkpoint::begin(4, false, &a).unwrap_or_else(|_| panic!("admission"));
        assert!(c.poll(true, false, None).is_none());
        assert!(matches!(
            c.poll_at(c.started + LIMIT, true, false, None),
            Some(Action::Finished {
                result: CheckpointResult::Failed { .. },
                wake: false
            })
        ));
    }
    #[test]
    fn an_unverified_idle_boundary_never_sends_off_before_timing_out() {
        let mut c =
            Checkpoint::begin(5, true, &admission()).unwrap_or_else(|_| panic!("admission"));
        assert!(c.poll(false, false, None).is_none());
        assert!(!c.off_sent());
        assert!(matches!(
            c.poll_at(c.started + LIMIT, false, false, None),
            Some(Action::Finished {
                result: CheckpointResult::Failed { .. },
                wake: false,
            })
        ));
        // A timeout cannot later emit OFF or a success, even if a stale prior
        // generation is passed by a caller after the transaction has ended.
        assert!(c.poll(true, true, Some(&Ok(receipt()))).is_none());
    }
}
