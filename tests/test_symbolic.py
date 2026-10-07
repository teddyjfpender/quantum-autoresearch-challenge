"""Exact controller proof obligations, including faults hidden in ONE unsampled input."""
import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
try:
    import z3  # noqa: F401
except ImportError:
    z3 = None


@unittest.skipIf(z3 is None, "install challenges/femoco/tools/verification/requirements.txt")
class SymbolicTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("symbolic", ROOT / "challenges/femoco/tools/verification/symbolic.py")
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def doc(self, ops):
        n = self.module.NONE
        return {"schema": "femoco-symbolic-input-v1", "spec": "test-sa-v1", "digests": {},
                "layout": {"system": 2, "uniform": 2, "num_qubits": 8, "num_bits": 1},
                "inner": {"lo": 1, "width": 1}, "registers": [[3]],
                "ops": [*ops, "Reflect"], "beta": 8, "none": n}

    def gate(self, kind, **operands):
        operands.setdefault("cond", self.module.NONE)
        return {kind: operands}

    def verify(self, ops):
        return self.module.verify(self.doc(ops), timeout_ms=2000)

    def test_correct_measured_and_uncompute(self):
        # Compute c & s, X-measure, then (-1)^(outcome c s) to erase the kickback.
        ops = [self.gate("Ccx", a=0, b=3, t=5), self.gate("Hmr", t=5, bit=0),
               self.gate("Phase", q=[0, 3, self.module.NONE], k=4, cond=0)]
        report = self.verify(ops)
        self.assertTrue(report["controller_obligations_proved"], report)
        self.assertFalse(report["full_quantum_equivalence_certified"])

    def test_missing_measurement_phase_correction(self):
        r = self.verify([self.gate("Ccx", a=0, b=3, t=5), self.gate("Hmr", t=5, bit=0)])
        counterexample = r["obligations"]["measurement_independent_system_trace_and_phase"]
        self.assertEqual(counterexample["status"], "counterexample")
        self.assertEqual(counterexample["measurement_outcomes"], {"default": False, "true_indices": [0]})
        self.assertFalse(r["all_obligations_proved"])

    def test_rare_dirty_reset_is_proved_bad(self):
        r = self.verify([self.gate("Ccx", a=0, b=3, t=5), self.gate("Reset", t=5),
                         self.gate("Ccx", a=0, b=3, t=5)])
        self.assertEqual(r["obligations"]["classical_restoration_and_cleanup"]["status"], "counterexample")

    def test_changed_control_or_inner_register_is_rejected(self):
        for wire in (0, 4):
            r = self.verify([self.gate("X", t=wire)])
            self.assertEqual(r["obligations"]["classical_restoration_and_cleanup"]["status"], "counterexample")

    def test_measurement_dependent_system_gate_is_rejected(self):
        r = self.verify([self.gate("Hmr", t=5, bit=0), self.gate("Sys", q=0, z=True,
                           ctrl=[self.module.NONE]*2, cond=0)])
        self.assertEqual(r["obligations"]["measurement_independent_system_trace_and_phase"]["status"], "counterexample")

    def test_measurement_dependent_givens_angle_is_rejected(self):
        r = self.verify([self.gate("Hmr", t=5, bit=0), self.gate("X", t=3, cond=0),
                         self.gate("Givens", p=0, q=1, reg=0), self.gate("X", t=3, cond=0)])
        self.assertEqual(r["obligations"]["measurement_independent_system_trace_and_phase"]["status"], "counterexample")

    def test_no_claim_of_term_equivalence(self):
        # A constant WRONG system action is measurement-independent. The report must never
        # present the controller proof as a quantum circuit equivalence certificate.
        r = self.verify([self.gate("Sys", q=0, z=True, ctrl=[self.module.NONE]*2)])
        self.assertTrue(r["controller_obligations_proved"])
        self.assertFalse(r["full_quantum_equivalence_certified"])

    def test_unknown_is_never_a_proof(self):
        import unittest.mock
        with unittest.mock.patch.object(self.module.z3.Solver, "check", return_value=self.module.z3.unknown):
            r = self.verify([])
        self.assertFalse(r["controller_obligations_proved"])
        self.assertTrue(all(x["status"] == "inconclusive" for x in r["obligations"].values()))

    def test_unknown_opcode_fails_closed(self):
        with self.assertRaises(ValueError):
            self.verify([{"MadeUp": {}}])

    def test_semantic_reduction_catches_alias_route_dependent_operator(self):
        # Both outer buckets name item 0. A system gate controlled by the RAW bucket bit
        # therefore applies different operators to two inputs naming the same term.
        doc = self.doc([self.gate("Sys", q=0, z=True, ctrl=[3, self.module.NONE])])
        doc["layout"].update(uniform=4, num_qubits=10)
        doc["inner"] = {"lo": 2, "width": 2}
        doc["outer"] = {"k": 1, "mu": 0, "keep": [0, 0], "alt": [0, 0]}
        doc["inner_tables"] = [{"k": 1, "mu": 0, "keep": [0, 0], "alt": [0, 0]}]
        doc["b"] = 1
        report = self.module.verify(doc, timeout_ms=2000)
        self.assertTrue(report["controller_obligations_proved"])
        self.assertEqual(report["semantic_trace_and_phase_invariance"]["status"], "counterexample")
        self.assertFalse(report["all_obligations_proved"])

    def test_semantic_reduction_proves_route_independence(self):
        doc = self.doc([self.gate("Sys", q=0, z=True, ctrl=[0, self.module.NONE])])
        doc["layout"].update(uniform=4, num_qubits=10)
        doc["inner"] = {"lo": 2, "width": 2}
        doc["outer"] = {"k": 1, "mu": 0, "keep": [0, 0], "alt": [0, 0]}
        doc["inner_tables"] = [{"k": 1, "mu": 0, "keep": [0, 0], "alt": [0, 0]}]
        doc["b"] = 1
        report = self.module.verify(doc, timeout_ms=2000)
        self.assertTrue(report["semantic_reduction_proved"])
        # Term equality STILL needs a spec comparison. Reduction alone does not prove it.
        self.assertFalse(report["full_quantum_equivalence_certified"])

    def test_symbolic_alias_decoder_matches_every_small_domain_input(self):
        # Includes padding bucket 3, keep=0, both sides of the comparator, one-body and
        # square generators, and ignored identity-spin values on either inner pass.
        doc = self.doc([])
        doc["layout"].update(uniform=7, num_qubits=10)
        doc["inner"] = {"lo": 4, "width": 3}
        doc["outer"] = {"k": 2, "mu": 1, "keep": [1, 0, 1, 0], "alt": [2, 0, 2, 1]}
        doc["inner_tables"] = [
            {"k": 1, "mu": 1, "keep": [1, 0], "alt": [1, 0]},
            {"k": 1, "mu": 1, "keep": [0, 1], "alt": [1, 0]},
            {"k": 1, "mu": 1, "keep": [1, 1], "alt": [1, 0]},
        ]
        doc["b"] = 1
        s, after = z3.BitVec("decoder_s", 7), z3.BitVec("decoder_after", 3)
        keys = self.module.semantic_keys(doc, s, after)

        def decode(table, value):
            i = value & ((1 << table["k"]) - 1)
            draw = (value >> table["k"]) & ((1 << table["mu"]) - 1)
            return i if draw < table["keep"][i] else table["alt"][i]

        for uniform in range(128):
            outer = decode(doc["outer"], uniform)
            first = decode(doc["inner_tables"][outer], uniform >> 4)
            for second in range(8):
                last = decode(doc["inner_tables"][outer], second)
                expected = (outer, first, last, (uniform >> 3) & 1 if outer < 1 else 0,
                            (uniform >> 6) & 1 if outer >= 1 and first != 1 else 0,
                            (second >> 2) & 1 if outer >= 1 and last != 1 else 0)
                got = tuple(z3.simplify(z3.substitute(k, (s, z3.BitVecVal(uniform, 7)),
                                                     (after, z3.BitVecVal(second, 3)))).as_long()
                            for k in keys)
                self.assertEqual(got, expected, (uniform, second))


if __name__ == "__main__":
    unittest.main()
