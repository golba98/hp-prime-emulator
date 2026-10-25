"""Offline import, resource ownership and bounded process cleanup checks."""

import ctypes
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from scripts.device import mirror_xvfb_smoke as smoke
from scripts.lib.x11 import Display, XEvent

ROOT = Path(__file__).resolve().parents[2]


class X11ToolTests(unittest.TestCase):
    def test_all_tools_import_without_cli_process_or_native_side_effects(self):
        source = """
import argparse, ctypes, importlib, subprocess
from unittest.mock import patch
names = [
    'scripts.device.mirror_report', 'scripts.device.mirror_xvfb_smoke',
    'scripts.device.verify_automated_deletion',
    'scripts.maintenance.xvfb_miniwm',
    'scripts.research.ck_inspect', 'scripts.research.ck_screen_replay',
    'scripts.research.ck_trace_evidence', 'scripts.lib.x11',
]
def forbidden(*args, **kwargs):
    raise AssertionError('side effect during import')
with (
    patch.object(argparse.ArgumentParser, 'parse_args', side_effect=forbidden),
    patch.object(ctypes, 'CDLL', side_effect=forbidden),
    patch.object(subprocess, 'Popen', side_effect=forbidden),
):
    for name in names:
        importlib.import_module(name)
"""
        result = subprocess.run(
            [sys.executable, "-c", source],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_window_tree_storage_is_released(self):
        display = Display.__new__(Display)
        display.handle = 1
        display.root = 2
        display.library = Mock()
        children = (ctypes.c_ulong * 2)(11, 12)

        def query(_handle, _root, _returned_root, _parent, output, count):
            ctypes.cast(
                output, ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong))
            )[0] = children
            ctypes.cast(count, ctypes.POINTER(ctypes.c_uint))[0] = 2
            return 1

        display.library.XQueryTree.side_effect = query
        self.assertEqual(display.windows(), [11, 12])
        display.library.XFree.assert_called_once()

    def test_process_cleanup_escalates_and_reaps(self):
        process = Mock()
        process.poll.return_value = None
        process.wait.side_effect = [
            subprocess.TimeoutExpired("synthetic", 1),
            0,
        ]
        smoke.stop_process(process, 1)
        process.terminate.assert_called_once()
        process.kill.assert_called_once()
        self.assertEqual(process.wait.call_count, 2)

    def test_no_available_display_is_an_explicit_error(self):
        with patch.object(Path, "exists", return_value=True):
            with self.assertRaisesRegex(RuntimeError, "no free isolated"):
                smoke.available_display()

    def test_missing_fixture_does_not_create_output_or_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with patch.object(smoke.subprocess, "Popen") as start:
                with self.assertRaisesRegex(ValueError, "does not exist"):
                    smoke.run_smoke(
                        directory / "missing.png",
                        directory / "out",
                        Path(sys.executable),
                    )
            self.assertFalse((directory / "out").exists())
            start.assert_not_called()

    def test_event_storage_matches_xlib_native_long_layout(self):
        self.assertEqual(
            ctypes.sizeof(XEvent), ctypes.sizeof(ctypes.c_long) * 24
        )


if __name__ == "__main__":
    unittest.main()
