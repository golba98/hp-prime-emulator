import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts.compat import closure_campaign as campaign


class ClosureCampaignTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.baseline = Path(self.temporary.name) / "synthetic-selection-input.json"
        # These are authored selection inputs, not calculator observations.
        self.records = [
            {"suite": suite, "id": case, "engine": "native", "verdict": "not_run"}
            for _, suite, cases in campaign.BATCHES for case in cases
        ]
        self.write_baseline()

    def write_baseline(self):
        self.baseline.write_text(json.dumps({"records": self.records}))

    def test_only_distinct_unrun_nonisolated_baseline_cases_are_batched(self):
        batches = campaign.selection(self.baseline)
        self.assertEqual([len(b["cases"]) for b in batches], [8, 8, 7, 12, 6])
        keys = [tuple(c["baseline_case"]) for b in batches for c in b["cases"]]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertEqual(len(keys), 41)
        self.assertTrue(all(len(c["source_sha256"]) == 64 for b in batches for c in b["cases"]))

    def test_firmware_or_completed_native_records_do_not_authorize_selection(self):
        for replacement in [{"engine": "firmware"}, {"verdict": "passed"}]:
            with self.subTest(replacement=replacement):
                original = self.records[0].copy()
                self.records[0].update(replacement)
                self.write_baseline()
                with self.assertRaisesRegex(ValueError, "Not a distinct pending baseline case"):
                    campaign.selection(self.baseline)
                self.records[0] = original

    def test_duplicate_case_is_refused(self):
        duplicate = [*campaign.BATCHES, campaign.BATCHES[0]]
        with patch.object(campaign, "BATCHES", duplicate):
            with self.assertRaisesRegex(ValueError, "Not a distinct pending baseline case"):
                campaign.selection(self.baseline)

    def test_isolated_or_mutating_cases_cannot_be_batched(self):
        root = Path(self.temporary.name)
        cases = root / "tests/compat/cases"
        cases.mkdir(parents=True)
        name, suite, ids = campaign.BATCHES[0]
        for flag in ["isolate", "mutates"]:
            with self.subTest(flag=flag):
                (cases / (suite + ".toml")).write_text(
                    f'[[case]]\nid = "{ids[0]}"\nexpr = "1"\n{flag} = true\n'
                )
                with patch.object(campaign, "ROOT", root), patch.object(
                    campaign, "BATCHES", [(name, suite, ids[:1])]
                ):
                    with self.assertRaisesRegex(ValueError, "Isolated/mutating"):
                        campaign.selection(self.baseline)


if __name__ == "__main__":
    unittest.main()
