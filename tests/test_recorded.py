"""Recorded circuits without a pin are rebuilt from their manifests and compared with the ledger."""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))

from qac import ledger, recorded  # noqa: E402
from qac.common import Challenge, ContractError  # noqa: E402


class RecordedTest(unittest.TestCase):
    def setUp(self):
        self.challenge = Challenge("femoco")
        self.rows = ledger.read(self.challenge.ledger)
        self.targets = recorded.unpinned(self.challenge)
        self.by_id = {(c["track"], c["id"]): r for r, c in self.targets}

    def faithful(self, checked):
        row = self.by_id[(checked["track"], checked["id"])]
        return (row["ops_sha256"], row["lanemap_sha256"], row["family_sha256"])

    def test_every_judged_submission_is_rebuilt(self):
        names = {row["submission"] for row, _ in self.targets}
        for row in self.rows:
            if row["kind"] == "submission":
                self.assertIn(row["submission"], names)
        self.assertTrue(all(not checked.get("pin") for _, checked in self.targets))
        pinned = [r["submission"] for r in self.rows if r["submission"] not in names]
        self.assertEqual(len(pinned) + len(self.targets), len(self.rows))
        if self.targets:
            first = self.targets[0][0]["submission"]
            self.assertEqual([r["submission"] for r, _ in recorded.unpinned(self.challenge, only=[first])], [first])

    def test_compare_names_the_file_that_moved(self):
        row = {"submission": "li/x", "ops_sha256": "a" * 64, "lanemap_sha256": "b" * 64, "family_sha256": "c" * 64}
        self.assertEqual(recorded.compare(row, ("a" * 64, "b" * 64, "c" * 64)), [])
        errors = recorded.compare(row, ("a" * 64, "0" * 64, "1" * 64))
        self.assertEqual(len(errors), 2)
        self.assertIn("lanemap.bin", errors[0])
        self.assertIn("family.out.json", errors[1])

    def test_rebuild_reports_mismatches_and_failed_builds(self):
        if not self.targets:
            self.skipTest("no unpinned recorded circuit")
        quiet = lambda _message: None  # noqa: E731
        built = []

        def faithful(checked):
            built.append(checked["id"])
            return self.faithful(checked)

        self.assertEqual(recorded.rebuild(self.challenge, faithful, log=quiet), [])
        self.assertEqual(len(built), len(self.targets))
        first = self.targets[0][0]["submission"]

        def moved(checked):
            ops, _lanemap, family = self.faithful(checked)
            return (ops, "0" * 64, family)

        errors = recorded.rebuild(self.challenge, moved, only=[first], log=quiet)
        self.assertEqual(len(errors), 1)
        self.assertIn(first, errors[0])
        self.assertIn("lanemap.bin", errors[0])

        def broken(_checked):
            raise ContractError("the build stage exited with status 2")

        errors = recorded.rebuild(self.challenge, broken, only=[first], log=quiet)
        self.assertEqual(errors, [f"{first}: the build stage exited with status 2"])


if __name__ == "__main__":
    unittest.main()
