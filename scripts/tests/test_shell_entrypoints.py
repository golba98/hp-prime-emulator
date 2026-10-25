"""Exercise shell contracts through fake commands, never Cargo or hardware."""

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class ShellEntrypointTests(unittest.TestCase):
    def test_cargo_arguments_directory_target_and_exit_code(self):
        with tempfile.TemporaryDirectory(prefix="prime scripts ") as temporary:
            directory = Path(temporary)
            cargo = directory / "cargo"
            cargo.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "print(json.dumps({'args': sys.argv[1:], 'cwd': os.getcwd(), "
                "'target': os.environ['CARGO_TARGET_DIR']}))\n"
                "sys.exit(int(os.environ.get('FAKE_CARGO_EXIT', '0')))\n"
            )
            cargo.chmod(0o755)
            environment = dict(
                os.environ, PATH=f"{directory}:{os.environ['PATH']}"
            )
            for arguments in (
                ["check", "--workspace", "--all-targets"],
                ["test", "--workspace", "--all-targets"],
                ["run", "--bin", "ppl", "--", "device", "detect"],
                ["run", "--", "a path with spaces"],
            ):
                result = subprocess.run(
                    [str(ROOT / "scripts/emulator/run.sh"), *arguments],
                    cwd=directory,
                    env=environment,
                    capture_output=True,
                    text=True,
                    timeout=10,
                    check=True,
                )
                value = json.loads(result.stdout)
                self.assertEqual(value["args"], arguments)
                self.assertEqual(Path(value["cwd"]), ROOT)
                self.assertEqual(
                    Path(value["target"]).resolve(),
                    (ROOT.parent / ".cache/prime-emulator-target").resolve(),
                )
            environment["FAKE_CARGO_EXIT"] = "37"
            result = subprocess.run(
                [str(ROOT / "scripts/emulator/run.sh"), "check"],
                env=environment,
                capture_output=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 37)

    def test_factory_extraction_offset_spaces_and_non_overwrite_flag(self):
        with tempfile.TemporaryDirectory(prefix="prime factory ") as temporary:
            directory = Path(temporary)
            image = directory / "factory image.dat"
            image.write_bytes(
                bytes(8192) + b"\xeb\x00\x00BXC.OS.1" + bytes(20)
            )
            output = directory / "factory output"
            tool = directory / "mcopy"
            tool.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, pathlib, sys\n"
                "pathlib.Path(os.environ['MCOPY_LOG']).write_text(\n"
                "    json.dumps(sys.argv[1:]))\n"
                "(pathlib.Path(sys.argv[-1]) / "
                "'programs/misc').mkdir(parents=True)\n"
            )
            tool.chmod(0o755)
            log = directory / "mcopy.json"
            result = subprocess.run(
                [
                    str(ROOT / "scripts/emulator/prepare_factory_a.sh"),
                    str(image),
                    str(output),
                ],
                env=dict(
                    os.environ,
                    PATH=f"{directory}:{os.environ['PATH']}",
                    MCOPY_LOG=str(log),
                ),
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(
                json.loads(log.read_text()),
                [
                    "-s",
                    "-n",
                    "-i",
                    f"{image}@@8192",
                    "::/*",
                    f"{output}/A/",
                ],
            )

    def test_factory_copy_failure_preserves_exit_and_does_not_report_success(
        self,
    ):
        with tempfile.TemporaryDirectory(
            prefix="failed factory "
        ) as temporary:
            directory = Path(temporary)
            image = directory / "image.dat"
            image.write_bytes(b"\0\0\0BXC.OS.1")
            tool = directory / "mcopy"
            tool.write_text("#!/bin/sh\nexit 29\n")
            tool.chmod(0o755)
            result = subprocess.run(
                [
                    str(ROOT / "scripts/emulator/prepare_factory_a.sh"),
                    str(image),
                    str(directory / "out"),
                ],
                env=dict(os.environ, PATH=f"{directory}:{os.environ['PATH']}"),
                capture_output=True,
                text=True,
                timeout=10,
                check=False,
            )
            self.assertEqual(result.returncode, 29)
            self.assertNotIn("factory A: tree in", result.stdout)

    def test_show_tree_forwards_depth_and_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            tool = directory / "tree"
            tool.write_text('#!/bin/sh\nprintf "%s\\n" "$PWD" "$@"\nexit 17\n')
            tool.chmod(0o755)
            result = subprocess.run(
                [str(ROOT / "scripts/maintenance/show-tree.sh"), "3"],
                env=dict(os.environ, PATH=f"{directory}:{os.environ['PATH']}"),
                capture_output=True,
                text=True,
                timeout=10,
            )
            self.assertEqual(result.returncode, 17)
            self.assertEqual(
                result.stdout.splitlines()[:4], [str(ROOT), "-a", "-L", "3"]
            )


if __name__ == "__main__":
    unittest.main()
