"""CLI validation runs offline; help never launches native or hardware work."""

import contextlib
import io
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from scripts.research import ck_trace_evidence as trace

ROOT = Path(__file__).resolve().parents[2]


class CLIContractTests(unittest.TestCase):
    def test_every_python_entrypoint_has_help_from_another_directory(self):
        scripts = [
            "device/mirror_report.py",
            "device/mirror_xvfb_smoke.py",
            "device/verify_automated_deletion.py",
            "maintenance/xvfb_miniwm.py",
            "research/ck_inspect.py",
            "research/ck_screen_replay.py",
            "research/ck_trace_evidence.py",
        ]
        with tempfile.TemporaryDirectory() as temporary:
            for script in scripts:
                with self.subTest(script=script):
                    result = subprocess.run(
                        [
                            sys.executable,
                            str(ROOT / "scripts" / script),
                            "--help",
                        ],
                        cwd=temporary,
                        capture_output=True,
                        text=True,
                        timeout=10,
                        check=False,
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn("usage:", result.stdout)
                    self.assertEqual(result.stderr, "")
            self.assertEqual(list(Path(temporary).iterdir()), [])

    def test_bad_screenshot_log_has_no_json_on_stdout(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "broken.log"
            path.write_text("not a HID log")
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts/research/ck_screen_replay.py"),
                    str(path),
                ],
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 1)
            self.assertEqual(result.stdout, "")
            self.assertIn(str(path), result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_trace_hash_gate_refuses_before_creating_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "must-not-exist"
            with (
                patch.object(
                    trace,
                    "PE",
                    return_value=Mock(data=b"synthetic wrong binary"),
                ),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                with self.assertRaises(SystemExit) as error:
                    trace.main(["synthetic.exe", str(output)])
            self.assertEqual(error.exception.code, 2)
            self.assertFalse(output.exists())
            with self.assertRaisesRegex(ValueError, "input hash differs"):
                trace.write_evidence(
                    Mock(data=b"synthetic wrong binary"), output
                )
            self.assertFalse(output.exists())

    def test_report_invalid_json_identifies_line(self):
        from scripts.device.mirror_report import load

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "captures.jsonl"
            path.write_text(json.dumps({"kind": "capture"}) + "\ninvalid\n")
            with self.assertRaisesRegex(ValueError, "captures.jsonl:2:"):
                load(path)


if __name__ == "__main__":
    unittest.main()
