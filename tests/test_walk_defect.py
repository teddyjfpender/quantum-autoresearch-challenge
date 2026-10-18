"""The rare-lane defect calculation behind challenges/femoco/spec/SAMPLING.md."""
import cmath
import math
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "challenges/femoco/tools/analysis"))
import walk_defect as w  # noqa: E402


def dense_measure(u):
    """Spectral measure of the uniform state under W = (2|G><G| - I) diag(u), by repeated
    application: the moments <G|W^t|G> computed directly from the definition."""
    n = len(u)
    state = [1 / math.sqrt(n)] * n
    moments = []
    for _ in range(40):
        moments.append(sum(state) / math.sqrt(n))
        applied = [x * y for x, y in zip(u, state)]
        mean = sum(applied) / n
        state = [2 * mean - x for x in applied]
    return moments


class WalkDefectTest(unittest.TestCase):
    def test_secular_spectrum_reproduces_the_walk_moments(self):
        # 64 lanes, 3 defective with a generic phase. The moments of the start state from the
        # secular roots and weights must equal those from applying the walk itself.
        n, bad, phase = 64, 3, cmath.exp(0.7j)
        plus = 40
        u = [phase] * bad + [1 + 0j] * (plus - bad) + [-1 + 0j] * (n - plus)
        cl = [(bad / n, phase), ((plus - bad) / n, 1 + 0j), ((n - plus) / n, -1 + 0j)]
        measure = w.spectral_measure(cl)
        self.assertAlmostEqual(sum(weight for _, weight in measure), 1, places=9)
        for t, moment in enumerate(dense_measure(u)):
            self.assertAlmostEqual(sum(weight * z ** t for z, weight in measure), moment, places=8)

    def test_hermitian_defects_are_linear_and_tuned_ones_go_as_the_square_root(self):
        for big_lambda, lambda_eff in w.TRACKS.values():
            for defect, exponent in (("sign", 1.0), ("phase-i", 1.0), ("resonant", 0.5), ("pair", 0.5)):
                self.assertAlmostEqual(w.scaling_exponent(big_lambda, lambda_eff, defect), exponent, places=2)

    def test_a_sign_defect_stays_within_the_linear_bound(self):
        for big_lambda, lambda_eff in w.TRACKS.values():
            for f in (1e-5, 1e-7, 1e-9):
                self.assertLessEqual(w.worst_shift(big_lambda, lambda_eff, f, "sign"), 2 * big_lambda * f * 1.001)

    def test_a_tuned_defect_exceeds_chemical_accuracy_at_a_fraction_the_default_sample_misses(self):
        f = 2 * math.log(1 / 0.61) / w.DEFAULT_LANES    # passes 2^19 lanes 61% of the time
        for big_lambda, lambda_eff in w.TRACKS.values():
            self.assertLess(w.worst_shift(big_lambda, lambda_eff, f, "sign"), w.EPSILON_HA)
            for defect in ("resonant", "pair"):
                shift = w.worst_shift(big_lambda, lambda_eff, f, defect)
                self.assertGreater(shift, 10 * w.EPSILON_HA)
                # The splitting is lambda_eff sqrt(2 f) up to a factor below two.
                self.assertLess(shift, 2 * lambda_eff * math.sqrt(2 * f))
                self.assertGreater(shift, 0.5 * lambda_eff * math.sqrt(2 * f))
            # With both branches tuned no outcome is within chemical accuracy.
            self.assertTrue(all(error > w.EPSILON_HA for error, _ in w.displaced_peaks(big_lambda, lambda_eff, f, "pair")))

    def test_document_figures(self):
        text = (ROOT / "challenges/femoco/spec/SAMPLING.md").read_text()
        g = w.nested_wrong_weight(w.DEFAULT_LANES)
        self.assertAlmostEqual(g, 3.2e-4, delta=1e-5)
        for (big_lambda, lambda_eff), bound, lanes in zip(w.TRACKS.values(), ("36.9", "113.7"),
                                                          (("2^23.5", "2^35.8"), ("2^25.2", "2^37.8"))):
            self.assertEqual(f"{2 * big_lambda * g * 1e3:.1f}", bound)
            self.assertIn(bound + " mHa", text)
            need = w.lanes_needed(big_lambda, lambda_eff)
            self.assertEqual((f"2^{math.log2(need['hermitian']):.1f}", f"2^{math.log2(need['non_hermitian']):.1f}"), lanes)
            for figure in lanes:
                self.assertIn(figure, text)
        self.assertEqual([f"{w.rare_branch_hit_probability(le):.2f}" for _, le in w.TRACKS.values()], ["0.67", "0.90"])


if __name__ == "__main__":
    unittest.main()
