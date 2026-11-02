# M17 modifier-state checkpoint controls

These are isolated local-firmware controls, with no physical calculator access.
The production helper was used unchanged, with copied user-supplied initial
storage. Every successful cleanup used the firmware's normal OFF and verified
its save status, process closure and absence of the unclean recovery marker.

A genuine checkpoint bug was reproduced independently of the original debug
post-CAS timeout: the internal blind Shift→ON sequence toggles an already latched
Shift **off**, so ON does not turn the calculator off.

| Initial observed state | Actual checkpoint result | State after an ON wake |
| --- | --- | --- |
| Neutral | Saved in 0.914s | Not sampled in this control |
| Shift latched | Failed at 30.003s; Shift visibly cleared | No sleep occurred |
| Alpha one-shot, upper | Saved in 1.002s | Alpha off |
| Alpha locked, upper | Saved in 0.958s | Alpha lock retained |

The failed Shift control advanced the main wait counter and the last delivered
keydown generation. Its post-failure inspection showed the main UI idle, input
queues empty and Shift absent in the actual framebuffer. Normal PowerDown from
that now-neutral state then saved and closed successfully. This distinguishes
the reproduced modifier-state failure from an unfulfilled idle admission window.
It does **not** prove the earlier post-CAS failure had the same cause; that run
lacked a retained modifier-state capture.

Additional physical-key controls establish that Alpha does not hide Shift:
Alpha upper + Shift displays the lower Alpha glyph **and** the cyan Shift glyph.
A second Shift restores the original upper state. This holds for one-shot and
locked Alpha. After undoing the probe, checkpoints saved in 0.955s and 0.998s;
waking cleared one-shot Alpha and retained Alpha lock respectively. The power
planner can inspect Shift without clearing Alpha lock or guessing its state.

All controls used firmware ELF SHA-256
`ad9779ff72db39fe4e9977ce7c4324f39aad1bb00c849c0aab067dc5bfc2fb39`
and helper SHA-256
`549b62f9f2ef596291e625e14926b33c96a1008541d963b646d0343ecd7e3641`.
This report describes evidence from the helper before the mode-aware OFF fix;
it does not claim that fix is implemented here.

The reusable runner preserves profiles, ordered commands/events, inspection
reports, exact protocol frames and lossless screenshots in a new output
folder. It requires fresh main-UI idle **after the actual delivered keydown**;
an unrelated timer advancing the wait counter cannot acknowledge a pending key.
Forced frame requests are retried when the snapshot gate is busy. Cleanup runs
and is verified even after a control assertion fails.

```sh
python3 scripts/emulator/checkpoint_controls.py \
  --helper /path/to/prime-emulator \
  --modes neutral,shift,alpha,alpha_lock --probe-shift --wake-after \
  --output /path/to/new-evidence
```

The default expects every checkpoint to save. To reproduce the known unfixed
Shift behavior explicitly, add `--expected-latched failed`. This option changes
the declared test expectation; it does not alter calculator input or mark a
failed checkpoint as a successful save.

Ten offline harness tests pass, including genuine retained status-region pixels
from nine observations, controlled mismatch/cleanup paths, fresh-key
acknowledgement, forced-capture retries and initial-profile symlink refusal:

```sh
python3 scripts/emulator/test_checkpoint_controls.py
```

The compact raw-pixel fixtures are classified as local firmware, never physical
verification. Full original profiles and event records remain in the owner's
home cache under `prime-m17-native-tmp/shift-checkpoint-controls-*`,
`alpha-checkpoint-controls-*`, `alpha-shift-glyph-controls-*` and
`alpha-shift-acknowledged-control-*`. One preliminary capture collector required
a new frame generation after only a single request and killed its disposable
failed-Shift process when that request missed a busy gate; its evidence remains
preserved. The repeated Shift control independently verified normal-OFF cleanup.
One preliminary Alpha probe sampled before its keydown was acknowledged and
correctly refused to continue; the corrected acknowledged control is reported
above. Neither preliminary attempt is counted as a successful control.

## Integrated correction

The mode-aware internal OFF sequencer observes firmware Shift before acting.
It sends ON directly when Shift is already active, or acknowledges one Shift
before ON otherwise. Alpha is preserved; no retry or hidden mode write is used.
The final coordinator helper passed all four controls with actual ON wake and
normal-OFF cleanup: neutral 1.049 s, Shift 0.889 s, Alpha 1.111 s and Alpha lock
1.056 s. One-shot Alpha cleared on wake; Alpha lock remained set. Raw final
results are under artifacts/m17/power-controls-root.

The separate post-CAS debug timeout was captured before any OFF input: its
strict idle share was 72.73%, below the unchanged 80% requirement. This is not
evidence of the modifier bug. Phase diagnostics and failure preservation now
distinguish those conditions; see the final M17 report for backend build
measurements and the continuing safe-refusal boundary.
