"""Assessment bookkeeping tests. No calculator, emulator or desktop is opened."""
import json
import tempfile
import unittest
from pathlib import Path
from scripts.compat.assessment import STATUS, category, render


class AssessmentTests(unittest.TestCase):
    def test_formatting_and_unknown_results_never_become_matches(self):
        self.assertEqual(STATUS["PARTIAL"], "mismatched")
        self.assertEqual(STATUS["INCONCLUSIVE"], "inconclusive")
        self.assertEqual(STATUS["UNVERIFIED"], "not_run")
        self.assertEqual(STATUS["UNSTABLE"], "unstable")

    def test_cas_and_errors_are_not_hidden_in_general_language_counts(self):
        self.assertEqual(category({"id": "cas.exact_factor"}), "cas_symbolic")
        self.assertEqual(category({"id": "errors.invalid_index"}), "errors_stability")
        self.assertEqual(category({"id": "numerical.matrix_product"}), "numerical")

    def test_disputed_transcript_is_excluded_from_valid_reference_counts(self):
        manifest = {
            "suite_version": "test",
            "suites": [{"suite": "test", "stratum": "curated", "cases": [
                {"id": "quoted", "category": "language_runtime"}
            ]}],
            "real_programs": [],
        }
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            (out / "audited").mkdir()
            data = {
                "audits": [],
                "observations": {"quoted": [{"disputed": True}]},
                "comparison_scope": "TYPE/STRING",
                "report": {"cases": [{"id": "quoted", "status": "INCONCLUSIVE",
                                        "emulator": {"kind": "value"}}]},
            }
            path = out / "audited/assessment-test.json"
            for confirmed_reference, extra in [(False, []), (True, [{"disputed": False}])]:
                data["observations"]["quoted"] = [{"disputed": True}, *extra]
                path.write_text(json.dumps(data))
                render(manifest, out)
                result = json.loads((out / "results.json").read_text())
                records = [r for r in result["records"] if r["id"] == "quoted"]
                self.assertEqual(len(records), 2)
                self.assertTrue(all(r["physical_reference"] == confirmed_reference for r in records))
                counts = [c for c in result["category_counts"]
                          if c["stratum"] == "curated" and c["category"] == "language_runtime"]
                self.assertTrue(all(c["valid_physical_reference"] == int(confirmed_reference)
                                    for c in counts))


if __name__ == "__main__":
    unittest.main()
