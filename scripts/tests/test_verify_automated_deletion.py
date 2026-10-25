"""Synthetic CLI responses in temporary directories; never a real CLI."""

import contextlib
import hashlib
import io
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts.device import verify_automated_deletion as acceptance


class DeletionAcceptanceTests(unittest.TestCase):
    def exercise(self, failure_label=None):
        with tempfile.TemporaryDirectory(
            prefix="synthetic acceptance "
        ) as temporary:
            directory = Path(temporary)
            calls = []

            def fake_run(arguments, **options):
                command = arguments[2:]
                calls.append((command, options))
                value = ""
                if failure_label == "send-note" and command[:2] == [
                    "send",
                    "ZZT_ABCDEFN",
                ]:
                    return subprocess.CompletedProcess(
                        arguments, 9, "", "synthetic refusal"
                    )
                if failure_label == "timeout" and command[:2] == [
                    "send",
                    "ZZT_ABCDEFN",
                ]:
                    raise subprocess.TimeoutExpired(arguments, 600)
                if command[0] == "get":
                    Path(command[-1]).write_bytes(
                        b"changed sentinel"
                        if failure_label == "sentinel-changed"
                        and "after" in command[-1]
                        else b"synthetic sentinel"
                    )
                elif command[:2] == ["delete", "verify"]:
                    value = json.dumps({"deletion_confirmed": True})
                elif command[0] == "delete":
                    kind, name = command[1:3]
                    backup = directory / f"{kind}-{name}.bin"
                    backup.write_bytes(b"synthetic backup")
                    value = json.dumps(
                        dict(
                            deletion_confirmed=True,
                            mechanism="usb_delete",
                            helper=None,
                            execution_confirmed=False,
                            error=None,
                            backup_directory=str(directory),
                            results=[
                                dict(
                                    backup_sha256=hashlib.sha256(
                                        backup.read_bytes()
                                    ).hexdigest()
                                )
                            ],
                        )
                    )
                if command[0] == "delete" and command[1] != "verify":
                    if failure_label == "malformed-deletion":
                        value = "{}"
                    elif failure_label == "bad-backup":
                        parsed = json.loads(value)
                        parsed["results"][0]["backup_sha256"] = "0" * 64
                        value = json.dumps(parsed)
                return subprocess.CompletedProcess(arguments, 0, value, "")

            arguments = [
                "verify_automated_deletion.py",
                "--hardware",
                "--ppl",
                "mock-ppl",
                "--output-dir",
                str(directory),
            ]
            with (
                patch.object(
                    acceptance.subprocess, "run", side_effect=fake_run
                ),
                patch.object(
                    acceptance.secrets, "token_hex", return_value="abcdef"
                ),
                patch("sys.argv", arguments),
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                result = acceptance.main()
            summary = json.loads(
                (directory / "ABCDEF/summary.json").read_text()
            )
            # Destroy synthetic artifacts with the temporary directory.
            return result, summary, calls

    def test_exact_owned_object_sequence_and_independent_verification(self):
        result, summary, calls = self.exercise()
        self.assertEqual(result, 0)
        self.assertEqual(
            [command[:3] for command, _ in calls],
            [
                ["send", "ZZT_ABCDEFP", calls[0][0][2]],
                ["send", "ZZT_ABCDEFN", calls[1][0][2]],
                ["send", "ZZT_ABCDEFS", calls[2][0][2]],
                ["get", "note", "ZZT_ABCDEFS"],
                ["delete", "program", "ZZT_ABCDEFP"],
                ["delete", "verify", "program"],
                ["delete", "note", "ZZT_ABCDEFN"],
                ["delete", "verify", "note"],
                ["get", "note", "ZZT_ABCDEFS"],
                ["delete", "note", "ZZT_ABCDEFS"],
                ["delete", "verify", "note"],
            ],
        )
        self.assertTrue(summary["sentinel_unchanged"])
        self.assertFalse(summary["execution_claimed"])
        for index, (_, options) in enumerate(calls):
            self.assertEqual(options["timeout"], 600)
            self.assertFalse(options["check"])
            self.assertEqual(
                options["input"],
                ["ZZT_ABCDEFP\n", "ZZT_ABCDEFN\n", "ZZT_ABCDEFS\n"][index]
                if index < 3
                else "",
            )

    def test_failed_step_stops_without_retry_or_cleanup(self):
        result, summary, calls = self.exercise("send-note")
        self.assertEqual(result, 1)
        self.assertEqual(len(calls), 2)
        self.assertEqual(summary["status"], "UNRESOLVED")
        self.assertEqual(summary["steps"][-1]["exit_code"], 9)
        self.assertIn("stopped without retrying", summary["error"])

    def test_invalid_evidence_stops_without_cleanup_or_false_success(self):
        for failure, expected_calls in (
            ("malformed-deletion", 5),
            ("bad-backup", 5),
            ("sentinel-changed", 9),
            ("timeout", 2),
        ):
            with self.subTest(failure=failure):
                result, summary, calls = self.exercise(failure)
                self.assertEqual(result, 1)
                self.assertEqual(len(calls), expected_calls)
                self.assertEqual(summary["status"], "UNRESOLVED")
                self.assertIn("error", summary)

    def test_hardware_flag_is_required_before_any_subprocess(self):
        with (
            patch.object(acceptance.subprocess, "run") as run,
            patch("sys.argv", ["verify"]),
            contextlib.redirect_stderr(io.StringIO()),
        ):
            with self.assertRaises(SystemExit) as error:
                acceptance.main()
        self.assertEqual(error.exception.code, 2)
        run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
