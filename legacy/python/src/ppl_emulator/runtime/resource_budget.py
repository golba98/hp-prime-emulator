from __future__ import annotations

import sys
import time
from dataclasses import dataclass

_ACTIVE_BUDGET = None


def set_active_budget(budget) -> None:
    global _ACTIVE_BUDGET
    _ACTIVE_BUDGET = budget


def get_active_budget():
    return _ACTIVE_BUDGET


def _format_bytes(num_bytes: int) -> str:
    value = float(num_bytes)
    for suffix in ("B", "KB", "MB", "GB"):
        if value < 1024.0 or suffix == "GB":
            if suffix == "B":
                return f"{int(value)} {suffix}"
            return f"{value:.1f} {suffix}"
        value /= 1024.0
    return f"{num_bytes} B"


@dataclass(slots=True)
class ResourceSnapshot:
    total_bytes: int = 0
    output_chars: int = 0
    call_depth: int = 0
    block_depth: int = 0
    line_events: int = 0


class ResourceLimitExceeded(RuntimeError):
    """Raised when a program exceeds a soft runtime budget."""

    def __init__(self, kind: str, message: str, *, observed: int | None = None, limit: int | None = None):
        super().__init__(message)
        self.kind = kind
        self.observed = observed
        self.limit = limit


class ResourceBudget:
    """Soft runtime guardrails for emulator execution.

    The budget is intentionally approximate. It exists to stop pathological
    programs that would otherwise hang the host process or exhaust memory.
    """

    def __init__(
        self,
        *,
        max_total_bytes: int = 32 * 1024 * 1024,
        max_single_object_bytes: int = 8 * 1024 * 1024,
        max_output_chars: int = 256 * 1024,
        max_call_depth: int = 128,
        max_block_depth: int = 128,
        max_line_events: int = 1_000_000,
        max_elapsed_seconds: float | None = 8.0,
    ):
        self.max_total_bytes = max_total_bytes
        self.max_single_object_bytes = max_single_object_bytes
        self.max_output_chars = max_output_chars
        self.max_call_depth = max_call_depth
        self.max_block_depth = max_block_depth
        self.max_line_events = max_line_events
        self.max_elapsed_seconds = max_elapsed_seconds

        self.active = False
        self._runtime = None
        self._start_monotonic = 0.0
        self._snapshot = ResourceSnapshot()
        self._seen_error = False
        self._previous_trace = None
        self._trace_filename = "<ppl_transpiled>"

    def activate(self, runtime, *, trace_filename: str = "<ppl_transpiled>"):
        self._runtime = runtime
        self._trace_filename = trace_filename
        self._start_monotonic = time.monotonic()
        self.active = True
        self._previous_trace = sys.gettrace()
        set_active_budget(self)
        sys.settrace(self._trace)
        self.recalculate(runtime)

    def deactivate(self):
        self.active = False
        if get_active_budget() is self:
            set_active_budget(None)
        if sys.gettrace() is self._trace:
            sys.settrace(self._previous_trace)
        self._previous_trace = None
        self._runtime = None

    def _raise(self, kind: str, message: str, *, observed: int | None = None, limit: int | None = None):
        self._seen_error = True
        raise ResourceLimitExceeded(kind, message, observed=observed, limit=limit)

    def _trace(self, frame, event, arg):
        if not self.active:
            return self._trace

        if frame.f_code.co_filename != self._trace_filename:
            return self._trace

        if event == "call":
            self._snapshot.call_depth += 1
            if self._snapshot.call_depth > self.max_call_depth:
                self._raise(
                    "depth",
                    (
                        f"Resource limit exceeded (call depth): "
                        f"{self._snapshot.call_depth} active calls > limit {self.max_call_depth}."
                    ),
                    observed=self._snapshot.call_depth,
                    limit=self.max_call_depth,
                )
        elif event == "return":
            self._snapshot.call_depth = max(0, self._snapshot.call_depth - 1)
        elif event == "line":
            self._snapshot.line_events += 1
            if self._snapshot.line_events > self.max_line_events:
                self._raise(
                    "steps",
                    (
                        f"Resource limit exceeded (execution steps): "
                        f"{self._snapshot.line_events} line events > limit {self.max_line_events}."
                    ),
                    observed=self._snapshot.line_events,
                    limit=self.max_line_events,
                )
            elapsed = time.monotonic() - self._start_monotonic
            if self.max_elapsed_seconds is not None and elapsed > self.max_elapsed_seconds:
                self._raise(
                    "time",
                    (
                        f"Resource limit exceeded (time): "
                        f"{elapsed:.2f}s > limit {self.max_elapsed_seconds:.2f}s."
                    ),
                    observed=int(elapsed * 1000),
                    limit=int(self.max_elapsed_seconds * 1000),
                )
        return self._trace

    def push_block(self):
        self._snapshot.block_depth += 1
        if self._snapshot.block_depth > self.max_block_depth:
            self._raise(
                "depth",
                (
                    f"Resource limit exceeded (block depth): "
                    f"{self._snapshot.block_depth} nested blocks > limit {self.max_block_depth}."
                ),
                observed=self._snapshot.block_depth,
                limit=self.max_block_depth,
            )

    def pop_block(self):
        self._snapshot.block_depth = max(0, self._snapshot.block_depth - 1)

    def _estimate(self, value, seen: set[int] | None = None) -> int:
        if seen is None:
            seen = set()

        obj_id = id(value)
        if obj_id in seen:
            return 0
        seen.add(obj_id)

        if value is None:
            return 0

        if isinstance(value, (bool, int, float, complex)):
            return sys.getsizeof(value)

        if isinstance(value, str):
            return sys.getsizeof(value) + len(value)

        if hasattr(value, "img") and hasattr(value, "width") and hasattr(value, "height"):
            width = int(getattr(value, "width", 0) or 0)
            height = int(getattr(value, "height", 0) or 0)
            return max(0, width * height * 3)

        if hasattr(value, "value") and value.__class__.__name__ == "PPLVar":
            return self._estimate(value.value, seen)

        if value.__class__.__name__ == "PPLString":
            text = str(value)
            return sys.getsizeof(value) + len(text)

        if value.__class__.__name__ == "PPLMatrix":
            total = sys.getsizeof(value)
            try:
                for row in value:
                    total += self._estimate(row, seen)
            except Exception:
                pass
            return total

        if isinstance(value, dict):
            total = sys.getsizeof(value)
            for key, item in value.items():
                total += self._estimate(key, seen)
                total += self._estimate(item, seen)
            return total

        if isinstance(value, (list, tuple, set, frozenset)):
            total = sys.getsizeof(value)
            for item in value:
                total += self._estimate(item, seen)
            return total

        try:
            return sys.getsizeof(value)
        except Exception:
            return 0

    def _runtime_roots(self, runtime):
        roots = []
        if getattr(runtime, "scopes", None) is not None:
            for scope in runtime.scopes.stack:
                roots.extend(scope.values())
        roots.extend(getattr(runtime, "_terminal_lines", []))
        roots.extend(getattr(runtime, "_program_store", {}).values())
        roots.extend(getattr(runtime, "_notes_store", {}).values())
        roots.extend(getattr(runtime, "_afile_store", {}).values())
        roots.extend(getattr(runtime, "grobs", []))
        return roots

    def recalculate(self, runtime=None):
        if runtime is None:
            runtime = self._runtime
        if not self.active or runtime is None:
            return

        seen: set[int] = set()
        total = 0
        for root in self._runtime_roots(runtime):
            total += self._estimate(root, seen)

        self._snapshot.total_bytes = total
        self._snapshot.output_chars = sum(len(str(line)) for line in getattr(runtime, "_terminal_lines", []))
        if self._snapshot.output_chars > self.max_output_chars:
            self._raise(
                "output",
                (
                    f"Resource limit exceeded (output): "
                    f"{self._snapshot.output_chars} output characters > limit {self.max_output_chars}."
                ),
                observed=self._snapshot.output_chars,
                limit=self.max_output_chars,
            )
        if total > self.max_total_bytes:
            self._raise(
                "memory",
                (
                    f"Resource limit exceeded (memory): "
                    f"estimated { _format_bytes(total) } > limit { _format_bytes(self.max_total_bytes) }."
                ),
                observed=total,
                limit=self.max_total_bytes,
            )

    def account_output(self, text: str, runtime=None):
        if not self.active:
            return
        self.recalculate(runtime)

    def account_value(self, value, *, runtime=None, label: str = "value"):
        if not self.active:
            return
        size = self._estimate(value)
        if size > self.max_single_object_bytes:
            self._raise(
                "memory",
                (
                    f"Resource limit exceeded (memory): {label} is { _format_bytes(size) } "
                    f"> single-object limit { _format_bytes(self.max_single_object_bytes) }."
                ),
                observed=size,
                limit=self.max_single_object_bytes,
            )
        self.recalculate(runtime)
