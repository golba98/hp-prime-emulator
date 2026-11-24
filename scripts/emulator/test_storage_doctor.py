import errno
import hashlib
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import storage_cache as cache
import storage_doctor as doctor


class StorageTests(unittest.TestCase):
    def test_gui_scenario_outputs_are_new_only_and_confined(self):
        module_path = (
            Path(__file__).resolve().parents[2]
            / "tests/ide_xvfb_smoke/isolate_scenario.py"
        )
        spec = importlib.util.spec_from_file_location("isolate", module_path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "old.json"
            data = {
                "dump": "/tmp/ppl-ide-e2e/m12/case/dump.json",
                "steps": [{"path": "/tmp/ppl-ide-e2e/m12/case/a.png"}],
                "source": "EXPORT A() BEGIN RETURN 1; END;",
            }
            source.write_text(json.dumps(data))
            target = root / "new.json"
            result = module.relocate(source, target)
            self.assertEqual(result["dump"], str(root / "dump.json"))
            self.assertEqual(result["source"], data["source"])
            with self.assertRaises(FileExistsError):
                module.relocate(source, target)
            data["steps"] = [{"path": "/tmp/ppl-ide-e2e/m12/case/../a"}]
            source.write_text(json.dumps(data))
            with self.assertRaises(ValueError):
                module.relocate(source, root / "escaped.json")
            self.assertFalse((root / "escaped.json").exists())

    def test_overlapping_roots_and_hardlinks_are_counted_once(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "child").mkdir()
            f = root / "file"
            f.write_bytes(b"x" * 8192)
            os.link(f, root / "child/alias")
            (root / "outside").symlink_to("/dev/zero")
            before = f.read_bytes()
            d = doctor.scan([root, root / "child"])
            self.assertEqual(d["logical_bytes"], 8192)
            self.assertEqual(d["hardlink_aliases_avoided"], 1)
            self.assertFalse(d["deletion_authorized"])
            self.assertIsNone(d["physical_exclusive_bytes"])
            self.assertEqual(f.read_bytes(), before)

    def test_sparse_and_missing_inputs_are_explicit(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "sparse"
            with p.open("wb") as f:
                f.truncate(2**20)
            d = doctor.scan([p, Path(tmp) / "missing"])
            self.assertEqual(d["logical_bytes"], 2**20)
            self.assertTrue(d["errors"])
            self.assertTrue(d["largest_20_files"][0]["sparse_candidate"])

    def test_nand_layout_preserves_spare_and_erased_bytes(self):
        chunks = cache.chunks(b"A" * 2050, b"STAGE")
        prefix = next(chunks)
        self.assertEqual(prefix[:2048], b"A" * 2048)
        self.assertEqual(prefix[2048:2112], b"\xff" * 64)
        self.assertEqual(prefix[2112:2114], b"AA")
        self.assertEqual(prefix[8448:8453], b"STAGE")
        self.assertEqual(sum(map(len, chunks)) + len(prefix), 276824064)
        with self.assertRaises(ValueError):
            next(cache.chunks(b"A" * 8193, b""))

    def test_shared_blob_has_private_regular_aliases_and_refuses_corruption(
        self,
    ):
        # Test cache publication independently of the full-size geometry.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            a = root / "a"
            b = root / "b"
            a.mkdir()
            b.mkdir()
            with (
                patch.object(cache, "chunks", lambda *_: iter([b"fixture"])),
                patch.object(cache, "IMAGE_SIZE", 7),
            ):
                h = cache.nand_base(a, cache=root / "cache")
                self.assertEqual(cache.nand_base(b, cache=root / "cache"), h)
                self.assertEqual(
                    (a / "base.raw").stat().st_ino,
                    (b / "base.raw").stat().st_ino,
                )
                self.assertFalse((a / "base.raw").is_symlink())
                with self.assertRaises(FileExistsError):
                    cache.nand_base(a, cache=root / "cache")
                blob = root / "cache" / (h + ".raw")
                blob.chmod(0o600)
                blob.write_bytes(b"corrupt")
                c = root / "c"
                c.mkdir()
                with self.assertRaises(ValueError):
                    cache.nand_base(c, cache=root / "cache")
                self.assertEqual(blob.read_bytes(), b"corrupt")

    def test_unknown_data_is_not_a_cleanup_candidate(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "save").write_bytes(b"irreplaceable")
            d = doctor.scan([root])
            self.assertEqual(d["roots"][0]["category"], "D")
            self.assertIn("approval required", doctor.markdown(d))
            research = root / ".cache/hp-prime-emulator"
            research.mkdir(parents=True)
            self.assertEqual(doctor.classify(research)[0], "D")

    def test_cross_device_base_is_private_read_only_and_new_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            source.mkdir()
            out = root / "out"
            out.mkdir()
            with (
                patch.object(cache, "chunks", lambda *_: iter([b"fixture"])),
                patch.object(cache, "IMAGE_SIZE", 7),
            ):
                identity = cache.nand_base(source, cache=root / "cache")
                with patch.object(
                    cache.os, "link", side_effect=OSError(errno.EXDEV, "test")
                ):
                    cache.nand_base(out, cache=root / "cache")
            blob = root / "cache" / (identity + ".raw")
            self.assertEqual((out / "base.raw").read_bytes(), b"fixture")
            self.assertNotEqual(
                blob.stat().st_ino, (out / "base.raw").stat().st_ino
            )
            self.assertEqual((out / "base.raw").stat().st_mode & 0o222, 0)

    def test_cli_reports_growth_and_budget_without_cleaning(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            data = root / "data"
            data.mkdir()
            (data / "save").write_bytes(b"preserve")
            baseline = root / "baseline.json"
            previous = doctor.scan([data])
            previous["roots"].append(
                {"path": "/absent-old-root", "allocated_bytes": 8192}
            )
            baseline.write_text(json.dumps(previous))
            (data / "new").write_bytes(b"x" * 8192)
            command = [
                "python3",
                doctor.__file__,
                "--root",
                str(data),
                "--compare",
                str(baseline),
                "--budget-gb",
                "1",
            ]
            result = subprocess.run(
                command, capture_output=True, text=True, check=True
            )
            report = json.loads(result.stdout)
            growth = {
                r["path"]: r["delta_accounted_bytes"] for r in report["growth"]
            }
            self.assertGreater(growth[str(data)], 0)
            self.assertEqual(growth["/absent-old-root"], -8192)
            self.assertFalse(report["budget"]["exceeded"])
            self.assertEqual((data / "save").read_bytes(), b"preserve")

    def test_wrapper_honors_explicit_target(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, CARGO_TARGET_DIR=str(Path(tmp) / "chosen"))
            wrapper = Path(__file__).resolve().parent / "run.sh"
            result = subprocess.run(
                [
                    "bash",
                    str(wrapper),
                    "metadata",
                    "--format-version=1",
                    "--no-deps",
                    "--offline",
                ],
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertEqual(
                json.loads(result.stdout)["target_directory"],
                str(Path(tmp) / "chosen"),
            )

    def test_concurrent_base_publication_keeps_one_immutable_blob(self):
        from concurrent.futures import ThreadPoolExecutor

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outputs = [root / str(i) for i in range(4)]
            for p in outputs:
                p.mkdir()
            with (
                patch.object(cache, "chunks", lambda *_: iter([b"fixture"])),
                patch.object(cache, "IMAGE_SIZE", 7),
            ):
                with ThreadPoolExecutor(max_workers=4) as pool:
                    hashes = list(
                        pool.map(
                            lambda out: cache.nand_base(
                                out, cache=root / "cache"
                            ),
                            outputs,
                        )
                    )
            self.assertEqual(len(set(hashes)), 1)
            self.assertEqual(len(list((root / "cache").iterdir())), 1)
            self.assertEqual(
                len({(p / "base.raw").stat().st_ino for p in outputs}), 1
            )

    def test_bundle_rejects_existing_output_and_retains_license(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "source"
            out = root / "bundle"
            binary = source / "build/arm-softmmu/qemu-system-arm"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"executable")
            (source / "COPYING").write_text("GPL")
            meta = {"binary_sha256": hashlib.sha256(b"executable").hexdigest()}
            with patch("full_system_run.checked_manifest", return_value=meta):
                cache.runtime_bundle(source, out)
                with self.assertRaises(FileExistsError):
                    cache.runtime_bundle(source, out)
            self.assertEqual((out / "COPYING").read_text(), "GPL")
            self.assertFalse(
                json.loads((out / "build.json").read_text())["storage_bundle"][
                    "compiled"
                ]
            )
