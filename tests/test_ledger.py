import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools"))

from qac import ledger  # noqa: E402

KEY = bytes(range(32))
TRACKS = {"reiher": {"name": "reiher", "spec": "reiher-sa-est-v1"}, "li": {"name": "li", "spec": "li-sa-est-v1"}}
ARCHS = {"a": {}, "b": {}}
LAMBDA = 2.0


def row(toffoli, qubits, arch="a", track="reiher", ops="1", when=100):
    return {
        "unix_time": str(when), "track": track, "spec": TRACKS[track]["spec"], "architecture": arch,
        "toffoli": f"{toffoli:.3f}", "qubits": str(qubits), "score": f"{LAMBDA * toffoli * qubits:.6e}",
        "lambda_eff": repr(LAMBDA), "samples": "524288", "engine": "reference-4096+sliced-524288", "seed": "ab" * 32,
        "ops_sha256": ops.rjust(64, "0"), "lanemap_sha256": "cd" * 32, "family_sha256": "ef" * 32,
        "family_name": "f", "verifier_sha256": "12" * 32, "commit": "abc1234", "pr": "", "author": "someone",
        "model": "m", "harness": "h", "submission": "pin:x", "kind": "historical", "standing": "",
        "status": "OK", "note": "n", "mac": ledger.UNSIGNED,
    }


class LedgerTest(unittest.TestCase):
    def test_seed_is_keyed_and_bound_to_the_circuit(self):
        a = ledger.derive_seed(KEY, "aa" * 32, "bb" * 32, "cc" * 32)
        self.assertRegex(a, r"^[0-9a-f]{64}$")
        self.assertNotEqual(a, ledger.derive_seed(KEY, "ab" * 32, "bb" * 32, "cc" * 32))
        self.assertNotEqual(a, ledger.derive_seed(bytes(32), "aa" * 32, "bb" * 32, "cc" * 32))
        with self.assertRaises(Exception):
            ledger.derive_seed(KEY, "short", "bb" * 32, "cc" * 32)

    def test_round_trip_and_chain(self):
        rows = [row(100, 10, ops="1"), row(90, 10, ops="2", when=200), row(80, 12, ops="3", when=300)]
        ledger.sign(KEY, rows)
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / "results.tsv"
            ledger.write(path, rows)
            self.assertEqual(ledger.read(path), rows)
        self.assertEqual(ledger.verify_chain(KEY, rows), [])
        self.assertEqual(ledger.check_rows(rows, TRACKS, ARCHS, signed=True), [])
        for tamper in (
            lambda r: r[1].__setitem__("toffoli", "89.000"),
            lambda r: r.pop(1),
            lambda r: r.reverse(),
            lambda r: r.insert(1, dict(r[0])),
        ):
            copy = [dict(r) for r in rows]
            tamper(copy)
            self.assertTrue(ledger.verify_chain(KEY, copy))
        self.assertTrue(ledger.verify_chain(bytes(32), rows))

    def test_static_checks(self):
        good = [row(100, 10)]
        ledger.sign(KEY, good)
        bad = [
            {**good[0], "score": "1.0e3"},
            {**good[0], "architecture": "zzz"},
            {**good[0], "track": "li"},
            {**good[0], "ops_sha256": "xyz"},
            {**good[0], "status": "FAIL"},
            {**good[0], "standing": "best"},
            {**good[0], "mac": ledger.UNSIGNED},
        ]
        for r in bad:
            self.assertTrue(ledger.check_rows([r], TRACKS, ARCHS, signed=True), r)
        self.assertTrue(ledger.check_rows([good[0], good[0]], TRACKS, ARCHS, signed=True))
        self.assertTrue(ledger.check_rows([row(1, 1, when=5, ops="2"), row(1, 1, when=4, ops="3")], TRACKS, ARCHS, signed=False))

    def test_standing(self):
        rows = [row(1000, 100, "a", ops="1"), row(800, 200, "a", ops="2")]
        bips = 10
        # First circuit of an architecture is always recorded.
        self.assertEqual(ledger.standing(rows, row(5000, 500, "b", ops="9"), bips)[0], ["new-architecture"])
        # Beats the architecture's best score and advances the front.
        self.assertEqual(ledger.standing(rows, row(900, 100, "a", ops="9"), bips)[0], ["architecture-elite", "front"])
        # Advances the front without beating the elite score: fewer qubits than anything recorded.
        self.assertEqual(ledger.standing(rows, row(1500, 90, "a", ops="9"), bips)[0], ["front"])
        # Inside the margin: neither.
        standings, reason = ledger.standing(rows, row(999.5, 100, "a", ops="9"), bips)
        self.assertEqual(standings, [])
        self.assertIn("neither", reason)
        # Dominated.
        self.assertEqual(ledger.standing(rows, row(1100, 150, "a", ops="9"), bips)[0], [])
        # The same circuit again.
        self.assertIn("already", ledger.standing(rows, row(1, 1, "a", ops="1"), bips)[1])
        # Tracks are separate boards.
        self.assertEqual(ledger.standing(rows, row(9999, 999, "a", track="li", ops="9"), bips)[0], ["new-architecture", "front"])


if __name__ == "__main__":
    unittest.main()
