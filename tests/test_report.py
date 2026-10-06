import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))

from qac import report  # noqa: E402

POLICY = {"ok": True, "is_submission": True, "proposes_architecture": False, "classified": "onehot-split", "errors": []}
MANIFEST = {"challenge": "femoco", "track": "li", "id": "my-circuit", "architecture": "onehot-split", "claimed": {}}
ROW = {"toffoli": "17855.560", "qubits": "474", "score": "3.877e8", "samples": "524288", "engine": "reference-4096+sliced-524288",
       "ops_sha256": "a" * 64, "seed": "b" * 64, "verifier_sha256": "c" * 64}


class ReportTest(unittest.TestCase):
    def build(self, files, stages=None, approved=False):
        with tempfile.TemporaryDirectory() as tmp:
            for name, value in files.items():
                (pathlib.Path(tmp) / name).write_text(json.dumps(value))
            return report.build(pathlib.Path(tmp), stages or {"build": "success", "evaluate": "success"}, approved, "https://example.invalid/run")

    def test_policy_failure(self):
        result = self.build({"policy.json": {**POLICY, "ok": False, "errors": ["x: outside the editable surface"]}})
        self.assertEqual((result["outcome"], result["merge"]), ("invalid", False))
        self.assertIn("outside the editable surface", result["body"])

    def test_build_and_evaluate_failures(self):
        files = {"policy.json": POLICY, "manifest.json": MANIFEST}
        self.assertEqual(self.build(files, {"build": "failure", "evaluate": "skipped"})["outcome"], "invalid")
        self.assertEqual(self.build(files, {"build": "success", "evaluate": "failure"})["outcome"], "invalid")
        self.assertEqual(self.build({"policy.json": POLICY})["outcome"], "invalid")

    def test_valid_outcomes(self):
        files = {"policy.json": POLICY, "manifest.json": MANIFEST, "row.json": ROW}
        accepted = {"accepted": True, "standing": ["front"], "reason": ""}
        result = self.build({**files, "decision.json": accepted})
        self.assertEqual((result["outcome"], result["merge"], result["label"]), ("validated", True, "submission-validated"))
        self.assertTrue(result["body"].startswith(report.MARKER))
        rejected = self.build({**files, "decision.json": {"accepted": False, "standing": [], "reason": "valid, but no"}})
        self.assertEqual((rejected["outcome"], rejected["merge"]), ("rejected", False))
        proposing = {**files, "policy.json": {**POLICY, "proposes_architecture": True}, "decision.json": accepted}
        self.assertEqual(self.build(proposing)["outcome"], "review")
        self.assertFalse(self.build(proposing)["merge"])
        self.assertTrue(self.build(proposing, approved=True)["merge"])
        # A new builder, or the first circuit of an architecture, also waits for a maintainer.
        unknown = {**files, "policy.json": {**POLICY, "classified": None}, "decision.json": accepted}
        self.assertEqual(self.build(unknown)["outcome"], "review")
        first = {**files, "decision.json": {"accepted": True, "standing": ["new-architecture"], "reason": ""}}
        self.assertEqual(self.build(first)["outcome"], "review")
        self.assertTrue(self.build(first, approved=True)["merge"])

    def test_untrusted_text_is_neutralised(self):
        result = self.build({"policy.json": {**POLICY, "ok": False, "errors": ["x [click](http://evil) @everyone <b>"]}})
        self.assertNotIn("@everyone", result["body"])
        self.assertNotIn("[click]", result["body"])


if __name__ == "__main__":
    unittest.main()
