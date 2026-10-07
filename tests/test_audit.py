"""Re-validation of a recorded row against a fresh evaluation (`challenge.py compare-row`)."""
import argparse
import contextlib
import io
import json
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))
from qac import cli, ledger, verifier  # noqa: E402
from qac.common import Challenge  # noqa: E402


class CompareRowTest(unittest.TestCase):
    def setUp(self):
        self.row = ledger.read(Challenge("femoco").ledger)[0]

    def compare(self, same_verifier=False, **changes):
        metrics = {
            "toffoli": float(self.row["toffoli"]), "qubits": int(self.row["qubits"]),
            "samples": int(self.row["samples"]),
            "digests": {"ops": self.row["ops_sha256"], "lanemap": self.row["lanemap_sha256"]},
        }
        metrics.update(changes)
        with tempfile.TemporaryDirectory() as tmp:
            score = pathlib.Path(tmp) / "score.json"
            score.write_text(json.dumps({"metrics": metrics}))
            args = argparse.Namespace(challenge="femoco", submission=self.row["submission"],
                                      score=str(score), seed=self.row["seed"], same_verifier=same_verifier)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                code = cli.cmd_compare_row(args)
        return code, out.getvalue()

    def test_a_row_reproduces_under_the_evaluator_that_recorded_it(self):
        with patch.object(verifier, "digest", return_value=self.row["verifier_sha256"]):
            code, out = self.compare(same_verifier=True)
        self.assertEqual(code, 0)
        self.assertNotIn("note", out)

    def test_a_later_evaluator_is_reported_and_is_not_a_failure(self):
        with patch.object(verifier, "digest", return_value="f" * 64):
            code, out = self.compare()
            strict, _ = self.compare(same_verifier=True)
        self.assertEqual(code, 0)
        self.assertIn("note verifier_sha256", out)
        self.assertEqual(strict, 1)

    def test_a_different_count_fails_whatever_the_evaluator(self):
        with patch.object(verifier, "digest", return_value="f" * 64):
            code, out = self.compare(qubits=int(self.row["qubits"]) + 1)
        self.assertEqual(code, 1)
        self.assertIn("FAIL qubits", out)


if __name__ == "__main__":
    unittest.main()
