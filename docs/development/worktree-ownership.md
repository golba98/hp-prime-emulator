# Checkout ownership

Canonical project: `3-HP Prime Emulator`, owned by Codex for consolidation on
`feat/g1-compat-integration`. User confirmed Claude released this checkout.

Claude retains exclusive ownership of `hp-prime-runtime-parity` on
`feat/g1-runtime-parity`; its dirty implementation is excluded. No runtime
handoff has been received. Do not change or remove that checkout.

The user confirmed release of `hp-prime-integration` and
`hp-prime-location-parity`. Codex completed and released file compatibility.
These three checkouts may be removed only after validation and evidence
preservation. Keep their branches.

Shared coordination lives under `git rev-parse --git-common-dir` in
`agent-claims/<checkout>/claim.json`. Create claim directories atomically;
record owner, task, branch, checkpoint and active/released state. A claim is
released only by its owner or explicit user coordination, never by timeout.
Before runtime integration, Claude must supply a committed SHA, clean/dirty
status, validation results and explicit handoff. Recheck all claims before
writes. A written policy cannot itself prevent concurrent writes.
