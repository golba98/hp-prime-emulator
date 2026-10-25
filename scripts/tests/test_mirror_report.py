"""Synthetic diagnostic accounting; no USB or calculator fixtures."""

import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from scripts.device import mirror_report as report


class IncrementalDiagnosticTests(unittest.TestCase):
    def test_unchanged_responses_do_not_inflate_changed_or_displayed_rates(
        self,
    ):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            rows = []
            for i in range(3):
                row = dict(
                    kind="capture",
                    at_ms=i * 100,
                    changed=i == 0,
                    requests_attempted=i + 1,
                    width=320,
                    height=240,
                    png_bytes=70,
                    incremental_update=dict(
                        baseline=i == 0, width=1, height=1, composition_ms=0.1
                    ),
                )
                for stage in (
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
                ):
                    row[stage + "_ms"] = 1.0
                rows.append(row)
            (directory / "captures.jsonl").write_text(
                "\n".join(json.dumps(row) for row in rows)
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                report.report(directory)
            result = output.getvalue()
            self.assertIn("valid incremental responses/s: 10.000", result)
            self.assertIn("changed publications: 1", result)
            self.assertIn(
                "Published FPS over changed publications: 0.000", result
            )
            self.assertIn("displayed FPS is unmeasured", result)
            self.assertIn("resynchronization baselines: 0", result)

    def test_large_capture_session_computes_p99_and_throughput(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            rows = []
            for i in range(100):
                row = dict(
                    kind="capture",
                    at_ms=i * 20,
                    changed=(i % 10 == 0),
                    requests_attempted=i + 1,
                    width=320,
                    height=240,
                    png_bytes=80,
                    report_bytes=128,
                    incremental_update=dict(
                        baseline=(i == 0),
                        width=1,
                        height=1,
                        composition_ms=0.05,
                    ),
                )
                for stage in (
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
                ):
                    row[stage + "_ms"] = float(i + 1)
                rows.append(row)
            (directory / "captures.jsonl").write_text(
                "\n".join(json.dumps(row) for row in rows)
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                report.report(directory)
            result = output.getvalue()
            self.assertIn("P99 ms", result)
            self.assertIn("Throughput:", result)
            self.assertIn("Rates breakdown:", result)
            self.assertIn("Request rate:", result)
            self.assertIn("Response rate:", result)
            self.assertIn("Changed-frame rate:", result)


def write_session(directory: Path, count: int) -> None:
    """Synthetic rows distinguish responses, changes and display."""
    captures = []
    presentations = []
    for index in range(count):
        row = dict(
            kind="capture",
            at_ms=index * 20,
            changed=index % 10 == 0,
            requests_attempted=index + 1,
            width=320,
            height=240,
            png_bytes=80,
            report_bytes=128,
            incremental_update=dict(
                baseline=index == 0, width=1, height=1, composition_ms=0.05
            ),
        )
        for stage in (
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
        ):
            row[stage + "_ms"] = float(index + 1)
        captures.append(row)
        presentations.append(
            dict(
                at_ms=index * 20,
                unique=index % 10 == 0,
                width=853,
                height=640,
                dpi=4 / 3,
                scaling_ms=0.2,
                present_ms=0.3,
                request_age_ms=1.0,
            )
        )
    (directory / "captures.jsonl").write_text(
        "\n".join(json.dumps(row) for row in captures)
    )
    if count >= 100:
        (directory / "presentations.jsonl").write_text(
            "\n".join(json.dumps(row) for row in presentations)
        )


class ReportContractTests(unittest.TestCase):
    def test_complete_stdout_contract(self):
        for count in (3, 100):
            with (
                self.subTest(count=count),
                tempfile.TemporaryDirectory() as temporary,
            ):
                directory = Path(temporary)
                write_session(directory, count)
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    report.report(directory)
                expected = (
                    Path(__file__).parent / "fixtures" / f"report_{count}.txt"
                ).read_text()
                self.assertEqual(
                    output.getvalue().replace(str(directory), "SESSION"),
                    expected,
                )

    def test_percentiles_and_tuple_compatibility(self):
        small = report.stats(range(1, 100))
        self.assertIsNone(small.p99)
        self.assertEqual(tuple(small), (50, 95, 99))
        large = report.stats(range(1, 101))
        self.assertEqual(
            (large.median, large.p95, large.p99, large.max),
            (50.5, 95, 99, 100),
        )

    def test_missing_capture_file_is_an_empty_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                report.report(Path(temporary))
            self.assertIn("No successful captures.", output.getvalue())


if __name__ == "__main__":
    unittest.main()
