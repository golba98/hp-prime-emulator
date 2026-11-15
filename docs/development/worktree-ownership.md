# Checkout ownership

Work only in the assigned checkout and branch. Before writes, inspect shared
`agent-claims/` under `git rev-parse --git-common-dir`. Register owner, task,
branch, checkout, checkpoint and state using atomic claim-directory creation.
Never overwrite or steal an existing claim based on inactivity or missing
processes. Require explicit release or user coordination before another writer.
An ownership record coordinates agents; it cannot enforce exclusive access.

Keep machine-specific ownership history and ongoing task context in ignored
`.private/`, when available. Public clones do not require those notes. Integrate
another agent's work only after a committed checkpoint, validation and explicit
handoff. Temporary worktrees require a concrete isolation need and registration.
Cleanup requires clean status, preserved branch history, explicit release, and
verified preservation of unique ignored/untracked evidence. Recheck ownership
and open files immediately before removal; uncertainty blocks removal.
Commit, push, merge and PR creation still require explicit user authorization.
