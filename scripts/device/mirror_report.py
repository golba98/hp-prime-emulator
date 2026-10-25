#!/usr/bin/env python3
"""Summarize --diagnostics JSONL using only Python's standard library."""

import argparse
import json
import math
import sys
from collections.abc import Iterable, Sequence
from pathlib import Path
from statistics import median
from typing import Any

DiagnosticRow = dict[str, Any]


def load(path: Path) -> list[DiagnosticRow]:
    """Missing measurements are allowed; malformed measurements are errors."""
    if not path.exists():
        return []
    rows = []
    for line_number, line in enumerate(path.read_text().splitlines(), 1):
        try:
            row = json.loads(line)
        except ValueError as error:
            raise ValueError(f"{path}:{line_number}: {error}") from error
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{line_number}: expected a JSON object")
        rows.append(row)
    return rows


class TimingStats(tuple):
    """Keep the historical three-value tuple interface while exposing P99.

    P99 is available only for sessions containing at least 100 observations.
    """

    def __new__(
        cls, med: float, p95: float, p99: float | None, maximum: float
    ):
        return super().__new__(cls, (med, p95, maximum))

    def __init__(
        self, med: float, p95: float, p99: float | None, maximum: float
    ) -> None:
        self.median = med
        self.p95 = p95
        self.p99 = p99
        self.max = maximum

    def __str__(self) -> str:
        if self.p99 is not None:
            return (
                f"({self.median:.4f}, {self.p95:.4f}, "
                f"{self.p99:.4f}, {self.max:.4f})"
            )
        return f"({self.median:.4f}, {self.p95:.4f}, {self.max:.4f})"

    def __repr__(self) -> str:
        return str(self)


def stats(values: Iterable[float]) -> TimingStats:
    values = sorted(values)
    if not values:
        raise ValueError("cannot summarize an empty measurement series")
    med = median(values)
    p95 = values[math.ceil(0.95 * len(values)) - 1]
    p99 = (
        values[math.ceil(0.99 * len(values)) - 1]
        if len(values) >= 100
        else None
    )
    return TimingStats(med, p95, p99, max(values))


def rate(rows: Sequence[DiagnosticRow]) -> float:
    if len(rows) < 2 or rows[-1]["at_ms"] <= rows[0]["at_ms"]:
        return 0.0
    return (len(rows) - 1) * 1000 / (rows[-1]["at_ms"] - rows[0]["at_ms"])


def report_incremental(
    rows: list[DiagnosticRow], updates: list[DiagnosticRow]
) -> float:
    """Render counters without treating unchanged replies as frames."""
    if not updates:
        return 0.0
    incremental = [
        row for row in updates if not row["incremental_update"]["baseline"]
    ]
    baselines = [
        row for row in updates if row["incremental_update"]["baseline"]
    ]
    counted = [
        row for row in rows if row.get("requests_attempted") is not None
    ]
    span = rows[-1]["at_ms"] - rows[0]["at_ms"]
    attempt_rate = 0.0
    if len(counted) > 1 and span > 0:
        attempts_in_interval = (
            counted[-1]["requests_attempted"]
            - counted[0]["requests_attempted"]
        )
        attempt_rate = attempts_in_interval * 1000 / span
    print(
        f"Screenshot write attempts/s over recorded interval: "
        f"{attempt_rate:.3f}; "
        f"valid incremental responses/s: {rate(incremental):.3f}; "
        f"resynchronization baselines: {max(0, len(baselines) - 1)}."
    )
    attempts = counted[-1]["requests_attempted"] if counted else None
    errors = sum(row["kind"] == "error" for row in rows)
    if attempts:
        print(
            f"Errors per screenshot write attempt: {errors / attempts:.4f} "
            f"(identity/preflight errors may also be present)."
        )
    rectangles = sorted(
        {
            (
                row["incremental_update"]["width"],
                row["incremental_update"]["height"],
            )
            for row in incremental
        }
    )
    print(f"Incremental rectangles: {rectangles}.")
    if incremental:
        composition_times = [
            row["incremental_update"]["composition_ms"] for row in incremental
        ]
        print(
            f"Incremental composition ms median/P95/max: "
            f"{stats(composition_times)}."
        )
    if baselines:
        print(
            f"Baseline total ms median/P95/max: "
            f"{stats([row['total_ms'] for row in baselines])}."
        )
    return attempt_rate


def report_rates(
    captures: list[DiagnosticRow],
    published: list[DiagnosticRow],
    attempt_rate: float,
) -> None:
    print(
        f"Rates breakdown: Request rate: {attempt_rate:.3f} req/s; "
        f"Response rate: {rate(captures):.3f} responses/s; "
        f"Changed-frame rate: {rate(published):.3f} changed/s."
    )
    span_seconds = (
        (captures[-1]["at_ms"] - captures[0]["at_ms"]) / 1000.0
        if len(captures) > 1
        else 0.0
    )
    if span_seconds > 0:
        total_report_bytes = sum(
            row.get("report_bytes", 0) for row in captures
        )
        total_png_bytes = sum(row.get("png_bytes", 0) for row in captures)
        print(
            f"Throughput: {total_report_bytes / span_seconds:.1f} B/s (report "
            f"bytes), {total_png_bytes / span_seconds:.1f} B/s (PNG payload)."
        )


def report_stages(
    rows: list[DiagnosticRow], captures: list[DiagnosticRow]
) -> None:
    has_p99 = len(captures) >= 100
    if has_p99:
        print(
            "\nStage | Median ms | P95 ms | P99 ms | Max ms | Share of mean "
            "capture time | Errors\n---|---:|---:|---:|---:|---:|---:"
        )
    else:
        print(
            "\nStage | Median ms | P95 ms | Max ms | Share of mean capture "
            "time | Errors\n---|---:|---:|---:|---:|---:"
        )
    stages = [
        "setup",
        "write",
        "first_response",
        "transfer",
        "assembly",
        "extraction",
        "decompression",
        "conversion",
        "publication",
        "total",
    ]
    total = sum(row["total_ms"] for row in captures)
    if total <= 0:
        raise ValueError("total capture time must be positive")
    for stage in stages:
        values = [row[f"{stage}_ms"] for row in captures]
        timing = stats(values)
        errors = sum(
            row["kind"] == "error"
            and (stage == "total" or row.get("stage") == stage)
            for row in rows
        )
        columns = f"{timing.median:.4f} | {timing.p95:.4f} | "
        if has_p99:
            columns += f"{timing.p99:.4f} | "
        columns += (
            f"{timing.max:.4f} | {sum(values) * 100 / total:.3f}% | {errors}"
        )
        print(f"{stage} | {columns}")


def report_presentations(directory: Path) -> None:
    presentations = load(directory / "presentations.jsonl")
    if not presentations:
        print("No GUI presentation measurements; displayed FPS is unmeasured.")
        return
    unique = [row for row in presentations if row["unique"]]
    print(
        f"Unique submitted frames: {len(unique)}; displayed submission FPS: "
        f"{rate(unique):.3f}; "
        f"Display FPS: {rate(presentations):.3f}; Unique visual FPS: "
        f"{rate(unique):.3f}."
    )
    for stage in ["scaling_ms", "present_ms", "request_age_ms"]:
        values = [row[stage] for row in presentations]
        print(f"{stage} median/P95/max: {stats(values)}")
    surfaces = sorted(
        {(row["width"], row["height"], row["dpi"]) for row in presentations}
    )
    print(f"Physical surfaces/DPI: {surfaces}")
    print(
        "Presentation submission is not compositor scanout or measured "
        "LCD-to-eye latency."
    )


def report(directory: Path) -> None:
    rows = load(directory / "captures.jsonl")
    captures = [row for row in rows if row["kind"] == "capture"]
    print(f"\n## {directory}\n")
    if not captures:
        print("No successful captures.")
        return
    published = [row for row in captures if row["changed"]]
    print(
        f"Captures: {len(captures)}; errors: "
        f"{sum(row['kind'] == 'error' for row in rows)}; "
        f"changed publications: {len(published)}; capture FPS: "
        f"{rate(captures):.3f}."
    )
    updates = [row for row in captures if row.get("incremental_update")]
    attempt_rate = report_incremental(rows, updates)
    report_rates(captures, published, attempt_rate)
    report_stages(rows, captures)
    sizes = [row["png_bytes"] for row in captures]
    print(
        f"\nPNG bytes median/P95/max: {stats(sizes)}; "
        f"source sizes: "
        f"{sorted({(row['width'], row['height']) for row in captures})}."
    )
    print(
        f"Published FPS over changed publications: {rate(published):.3f} "
        "(unchanged captures intentionally do not publish)."
    )
    report_presentations(directory)


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directories", type=Path, nargs="+")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    for directory in args.directories:
        try:
            report(directory)
        except (OSError, ValueError, KeyError, TypeError) as error:
            print(f"error: {directory}: {error}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
