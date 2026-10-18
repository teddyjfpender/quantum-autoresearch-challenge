"""Exact small-system and integer-distribution oracles for residual correction."""
from fractions import Fraction
import json
import pathlib
import sys
import unittest

import numpy as np
from flint import arb, arb_mat, ctx, fmpq, fmpq_mat

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'challenges/femoco/tools/chemistry'))
import bounds
import corrected
import inputs
from test_chemistry import fock, exact


def pauli_matrix(n, encoded):
    x, z, phase = encoded
    if phase % 2:
        raise ValueError('oracle expects real Pauli matrices')
    size = 1 << (2*n)
    a = fmpq_mat(size, size)
    for state in range(size):
        a[state ^ x, state] = (-1)**(phase//2+(state & z).bit_count())
    return a


class CorrectedTest(unittest.TestCase):
    def setUp(self):
        self.prec = ctx.prec
        ctx.prec = 128

    def tearDown(self):
        ctx.prec = self.prec

    def test_pauli_dictionary_and_zero_signal_anticommutators_exactly(self):
        # All pairs, all Pauli branches, and all Fock-space matrix elements.
        n = 2
        ident, _, e = fock(n)
        packed = []
        for p, q in inputs.pairs(n):
            a = e[p][p]-ident if p == q else (e[p][q]+e[q][p])/2
            words = corrected.pair_paulis(n, p, q)
            matrices = [pauli_matrix(n, word) for word in words]
            self.assertEqual(sum(matrices, 0*ident)/4, a)
            for matrix in matrices:
                self.assertEqual(matrix.transpose(), matrix)
                self.assertEqual(matrix*matrix, ident)
            packed.append((a, words, matrices))
        anti_seen = 0
        for a, aw, am in packed:
            for b, bw, bm in packed:
                block = 0*ident
                for (xa, za, _), pa in zip(aw, am):
                    for (xb, zb, _), pb in zip(bw, bm):
                        anti = ((xa & zb).bit_count()+(xb & za).bit_count()) % 2
                        if anti:
                            anti_seen += 1
                            self.assertEqual(pa*pb+pb*pa, 0*ident)
                            # SELECT is X on a signal qubit: its zero block is 0.
                        else:
                            self.assertEqual(pa*pb, pb*pa)
                            block += pa*pb
                self.assertEqual(block/16, (a*b+b*a)/2)
        self.assertGreater(anti_seen, 0)

    def test_sparse_correction_bound_against_independent_exact_matrix(self):
        n = 2
        d = arb_mat([['3/16', '1/32768', '-1/131072'],
                     ['1/32768', '-1/8', '1/65536'],
                     ['-1/131072', '1/65536', '7/32']])
        full, compressed, report = corrected.compress(d, n, Fraction(1, 1000), Fraction(1, 10000))
        self.assertLess(np.count_nonzero(compressed), np.count_nonzero(full))
        ident, _, e = fock(n)
        a = [e[p][p]-ident if p == q else (e[p][q]+e[q][p])/2 for p, q in inputs.pairs(n)]
        original = 0*ident
        actual = 0*ident
        values = iter(corrected.weighted_triangle(d, n))
        j = 0
        for p in range(len(a)):
            for q in range(p+1):
                op = (a[p]*a[q]+a[q]*a[p])/2
                original += exact(next(values))*op
                actual += fmpq(int(compressed[j]), 1 << report['coefficient_fraction_bits'])*op
                j += 1
        difference = original-actual
        # Gershgorin infinity norm upper bound for this exact symmetric example.
        actual_bound = max(sum(abs(difference[i, j]) for j in range(ident.nrows())) for i in range(ident.nrows()))
        certified = Fraction(report['compressed_correction_error_upper_Ha'])
        self.assertLessEqual(Fraction(str(actual_bound)), certified)
        self.assertLessEqual(Fraction(report['dropped_error_upper_Ha']), Fraction(1, 1000))
        with self.assertRaises(ValueError):
            corrected.compress(d, n, -1)
        with self.assertRaises(ValueError):
            corrected.compress(arb_mat(2, 2), n)

    def test_alias_distribution_by_exhaustive_uniform_draws(self):
        integer = np.array([1, -3, 0, 7, -2], dtype=np.int64)
        (positions, keep, alternate), r = corrected.alias(integer, 4, Fraction(1, 8))
        capacity = 1 << r['keep_bits']
        histogram = np.zeros(r['padded_buckets'], dtype=np.int64)
        for bucket in range(len(keep)):
            for draw in range(capacity):
                histogram[bucket if draw < keep[bucket] else alternate[bucket]] += 1
        draws = capacity*len(keep)
        norm = Fraction(r['normalization_Ha'])
        error = sum(abs(norm*int(histogram[j])/draws-Fraction(abs(int(integer[pos])), 16)) for j, pos in enumerate(positions))
        self.assertEqual(error, Fraction(r['alias_error_upper_Ha']))
        self.assertLessEqual(error, Fraction(1, 8))
        self.assertEqual(sum(histogram[len(positions):]), 0)
        for bad in (np.zeros(4, dtype=np.int64),):
            with self.assertRaises(ValueError):
                corrected.alias(bad, 4)
        with self.assertRaises(ValueError):
            corrected.alias(integer, 4, 0)

    def test_non_dyadic_residual_and_unresolved_interval_fail_closed(self):
        d = arb_mat([['1/3']])
        full, compressed, r = corrected.compress(d, 1, 0, Fraction(1, 20000))
        want = abs(Fraction(1, 6)-Fraction(int(full[0]), 1 << r['coefficient_fraction_bits']))
        self.assertGreaterEqual(Fraction(r['baseline_quantization_error_upper_Ha']), want)
        self.assertEqual(list(full), list(compressed))
        self.assertLess(Fraction(r['baseline_quantization_error_upper_Ha'])-want, Fraction(1, 10**30))
        with self.assertRaises(ValueError):
            corrected.compress(arb_mat([[arb('0.1 +/- 0.1')]]), 1)

    def test_reports_keep_certificates_separate_from_cost_models(self):
        for name in ('reiher', 'li'):
            r = json.loads((ROOT/f'challenges/femoco/rigorous/corrected-{name}.json').read_text())
            for file, digest in r['checker_sha256'].items():
                self.assertEqual(inputs.sha256(inputs.HERE/file), digest)
            base = json.loads((ROOT/f'challenges/femoco/rigorous/corrected-base-{name}.json').read_text())
            self.assertEqual(base['payload_sha256'], r['base_payload_sha256'])
            self.assertEqual(inputs.sha256(ROOT/'challenges/femoco/src/bin/cost_corrected_base.rs'), base['source_sha256'])
            self.assertEqual(len(base['circuits']), 4)
            for circuit in base['circuits']:
                self.assertLessEqual(Fraction(circuit['exact_coefficient_1norm_Ha']), Fraction(1, 10000))
                self.assertEqual(Fraction(circuit['lambda_Ha']), Fraction(r['base_normalization_Ha']))
            for scenario in r['scenarios'].values():
                budget = scenario['conditional_encoded_budget']
                self.assertTrue(budget['budget_fits_1_6_mHa'])
                self.assertFalse(budget['chemical_accuracy_certified'])
                total = sum(Fraction(budget[key]) for key in ('rotation_preprocessing_Ha', 'correction_Ha',
                    'correction_alias_Ha', 'base_nested_coefficients_reserved_Ha',
                    'outer_combination_reserved_Ha', 'phase_estimation_reserved_Ha'))
                self.assertEqual(total, Fraction(budget['total_with_QPE_Ha']))
                self.assertEqual(corrected.resources(r['target']['spatial_orbitals'], scenario['encoding']), scenario['resource_model'])


if __name__ == '__main__':
    unittest.main()
