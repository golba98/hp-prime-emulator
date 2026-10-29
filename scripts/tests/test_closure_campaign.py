import unittest
from scripts.compat.closure_campaign import selection


class ClosureCampaignTests(unittest.TestCase):
    def test_only_distinct_unrun_nonisolated_baseline_cases_are_batched(self):
        batches = selection()
        self.assertEqual([len(b["cases"]) for b in batches], [8, 8, 7, 12, 6])
        keys = [tuple(c["baseline_case"]) for b in batches for c in b["cases"]]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertEqual(len(keys), 41)
        self.assertTrue(all(len(c["source_sha256"]) == 64 for b in batches for c in b["cases"]))


if __name__ == "__main__":
    unittest.main()
