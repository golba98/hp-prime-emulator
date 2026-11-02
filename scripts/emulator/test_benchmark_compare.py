import copy
import hashlib
import tempfile
import unittest
from pathlib import Path
from benchmark_compare import summarize
from benchmark_run import profile_sha256, workloads


def trial():
    cases, tiny = workloads()
    cases += [("tiny_after_restart", "run", tiny, 42), ("repeat_after_restart", "run", tiny, 42)]
    return {"firmware_sha256": "firmware", "os_rom_sha256": "rom", "helper_sha256": "helper",
            "initial_profile_sha256": "profile", "script_sha256": "script", "platform": "host",
            "boot_seconds": [1, 2], "results": [
                {"case": name, "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
                 "source_bytes": len(source.encode()), "correct": True, "elapsed_seconds": i + 1}
                for i, (name, _, source, _) in enumerate(cases)]}


class ComparisonTests(unittest.TestCase):
    def test_verified_pairs_keep_samples_and_exact_medians(self):
        before, after = trial(), trial()
        after["helper_sha256"] = "new-helper"
        after["results"][1]["elapsed_seconds"] = 1
        report = summarize([before] * 5, [after] * 5)
        self.assertEqual(report["operations"], 130)
        self.assertEqual(report["rows"][1]["change_percent"], -50)

    def test_partial_changed_failed_and_nonfinite_trials_fail(self):
        original = trial()
        changes = [lambda t: t["results"].pop(),
                   lambda t: t.update(initial_profile_sha256="changed"),
                   lambda t: t.update(os_rom_sha256="changed"),
                   lambda t: t["results"][0].update(source_sha256="wrong"),
                   lambda t: t["results"][0].update(correct=False),
                   lambda t: t["results"][0].update(elapsed_seconds=float("nan")),
                   lambda t: t["results"][0].update(elapsed_seconds=True),
                   lambda t: t.update(boot_seconds=[1]),
                   lambda t: t.update(boot_seconds=[1, float("inf")])]
        for change in changes:
            changed = copy.deepcopy(original)
            change(changed)
            with self.subTest(change=change), self.assertRaises(ValueError):
                summarize([original], [changed])
        with self.assertRaises(ValueError):
            summarize([], [])
        with self.assertRaises(ValueError):
            summarize([original], [original, original])

    def test_helper_change_inside_group_fails(self):
        changed = trial()
        changed["helper_sha256"] = "different"
        with self.assertRaises(ValueError):
            summarize([trial(), changed], [trial(), trial()])

    def test_profile_identity_uses_names_and_bytes_not_timestamps(self):
        with tempfile.TemporaryDirectory() as path:
            root = Path(path)
            file = root / "A"
            file.write_bytes(b"calculator")
            initial = profile_sha256(root)
            file.touch()
            self.assertEqual(profile_sha256(root), initial)
            (root / "empty").mkdir()
            self.assertNotEqual(profile_sha256(root), initial)
            (root / "empty").rmdir()
            self.assertEqual(profile_sha256(root), initial)
            file.rename(root / "B")
            self.assertNotEqual(profile_sha256(root), initial)
            (root / "B").write_bytes(b"changed")
            self.assertNotEqual(profile_sha256(root), initial)
            (root / "link").symlink_to(root / "B")
            with self.assertRaises(ValueError):
                profile_sha256(root)


if __name__ == "__main__":
    unittest.main()
