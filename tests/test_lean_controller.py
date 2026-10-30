"""Lean sidecar: proof-boundary failures and complete binary controller proofs."""
import importlib.util
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "challenges/femoco/lean"
SPEC = importlib.util.spec_from_file_location("lean_controller_verify", PACKAGE / "scripts/verify.py")
VERIFY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFY)
NONE = 2**64-1


def op(kind, *, c2=NONE, c1=NONE, target=NONE, bit=NONE, condition=NONE, reg=NONE, pad=0):
    return struct.pack("<II6Q", kind, pad, c2, c1, target, bit, condition, reg)


def stream(ops):
    return b"FEMOOPS1" + struct.pack("<Q", len(ops)) + b"".join(ops)


def and_erase():
    return [op(13, c2=0, c1=1, target=2), op(12, target=2, bit=0),
            op(9, c1=0, target=1, condition=0), op(11, target=2)]


class LeanBoundaryTest(unittest.TestCase):
    def test_audit_requires_every_expected_declaration(self):
        with self.assertRaises(ValueError):
            VERIFY.audit("", ("proof",))
        with self.assertRaises(ValueError):
            VERIFY.audit("'other' depends on axioms: [propext]", ("proof",))

    def test_audit_rejects_native_computation_and_sorry_axioms(self):
        for ax in ("sorryAx", "Lean.ofReduceBool", "attacker_axiom"):
            with self.subTest(axiom=ax), self.assertRaises(ValueError):
                VERIFY.audit(f"'proof' depends on axioms: [{ax}]", ("proof",))
        VERIFY.audit("'proof' depends on axioms: [propext, Quot.sound]", ("proof",))
        VERIFY.audit("'proof' does not depend on any axioms", ("proof",))

    def test_audit_rejects_duplicate_output(self):
        with self.assertRaises(ValueError):
            VERIFY.audit("'proof' depends on axioms: []\n'proof' depends on axioms: []", ("proof",))

    def test_generator_accepts_only_numeric_literals(self):
        for wires, inputs in [(0, []), (4097, []), (3, [3]), (3, [0, 0]), (3, ["0; axiom x : False"]), (True, [])]:
            with self.subTest(wires=wires, inputs=inputs), self.assertRaises(ValueError):
                VERIFY.generated_source(b"", wires, inputs)
        source = VERIFY.generated_source(bytes(range(256)), 3, [0, 1])
        self.assertIn("Binary.decode 3 bytes", source)
        self.assertIn("decoded := by decide", source)
        self.assertIn("accepted := by decide", source)


# The dedicated workflow requires Lean; ordinary tooling runs may omit it.
@unittest.skipUnless(shutil.which("lake") or os.environ.get("FEMOCO_LEAN_REQUIRED"),
                     "install the pinned Lean toolchain for proof integration tests")
class LeanIntegrationTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if shutil.which("lake") is None:
            raise AssertionError("Lean is required in this job")
        p = subprocess.run(["lake", "--wfail", "build"], cwd=PACKAGE, text=True, capture_output=True, timeout=120)
        if p.returncode:
            raise AssertionError(p.stdout + p.stderr)

    def verify_data(self, data, inputs=(0, 1), wires=3):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "program.femoops"
            path.write_bytes(data)
            return VERIFY.verify(path, wires, list(inputs))

    def test_kernel_replay_and_axiom_audit(self):
        VERIFY.selfcheck()

    def test_complete_and_erase_binary(self):
        report = self.verify_data(stream(and_erase()))
        self.assertTrue(report["controller_identity_certified"])
        self.assertTrue(report["kernel_replayed"])
        self.assertEqual(report["toffoli_upper_bound"], 1)
        self.assertEqual(report["measurements_upper_bound"], 1)
        self.assertFalse(report["full_quantum_equivalence_certified"])
        self.assertFalse(report["chemical_accuracy_certified"])

    def test_negated_and_and_reused_classical_bit(self):
        x = op(6, target=1)
        compute, measure, correct, reset = and_erase()
        self.verify_data(stream([x, compute, x, measure, x, correct, x, reset]))
        report = self.verify_data(stream(and_erase() + and_erase()))
        self.assertEqual(report["measurements_upper_bound"], 2)

    def test_semantic_mutants_are_rejected(self):
        compute, measure, correct, reset = and_erase()
        for ops in ([compute, measure, reset], [compute, reset],
                    [compute, measure, op(7, target=0, condition=0), reset]):
            with self.subTest(ops=ops), self.assertRaises(ValueError):
                self.verify_data(stream(ops))

    def test_binary_and_scope_mutants_are_rejected(self):
        valid = stream(and_erase())
        mutants = [valid[:-1], valid+b"\0", b"BADMAGIC"+valid[8:],
                   stream([op(6, target=0, pad=1)]),
                   stream([op(13, c2=0, c1=0, target=2)]),
                   stream([op(6, target=3)]), stream([op(20, target=0)]),
                   stream([op(6, target=0, bit=0)]),
                   stream([op(6, target=0, condition=0)]),
                   stream([op(12, target=2, bit=0, condition=0)]),
                   stream([op(11, target=2, condition=0)])]
        for index, data in enumerate(mutants):
            with self.subTest(mutant=index), self.assertRaises(ValueError):
                self.verify_data(data)

    def test_failed_run_removes_old_success_report(self):
        with tempfile.TemporaryDirectory() as temp:
            src, report = Path(temp)/"bad.femoops", Path(temp)/"report.json"
            src.write_bytes(b"invalid")
            report.write_text('{"controller_identity_certified": true}')
            p = subprocess.run(["python3", str(PACKAGE/"scripts/verify.py"), str(src),
                                "--wires", "3", "--inputs", "0,1", "--out", str(report)],
                               text=True, capture_output=True, timeout=120)
            self.assertNotEqual(p.returncode, 0)
            self.assertFalse(report.exists())
            self.assertIn("verification refused", p.stderr)


if __name__ == "__main__":
    unittest.main()
