"""Exact combined-walk execution, malformed-program rejection and certificates."""
import copy
from fractions import Fraction as F
import hashlib
import json
import pathlib
import sys
import unittest
import numpy as np
from flint import arb, arb_mat, ctx, fmpq

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'challenges/femoco/tools/chemistry'))
import combined as c
import signed_df
import inputs
from test_chemistry import fock


class CombinedTest(unittest.TestCase):
    def setUp(self):
        self.prec = ctx.prec
        ctx.prec = 128

    def tearDown(self):
        ctx.prec = self.prec

    def example(self):
        nets = np.array([[0], [1]], dtype=np.uint32)
        rows = [c.Row(False, 1, [F(1, 4)], [0]),
                c.Row(True, -1, [F(1, 2), F(-1, 4), F(1, 4)], [0, 1, -1])]
        p, o, i = c.compile_program(rows, nets, 3, 2)
        p['outer_table'], p['inner_tables'] = o, i
        return rows, nets, p, o, i

    def test_full_signed_walk_against_exact_fock_hamiltonian(self):
        rows, nets, p, o, i = self.example()
        proof = c.verify(p, rows, nets, o, i, reference_beta=3)
        self.assertEqual(F(proof['coefficient_error_upper_Ha']), 0)
        identity, a, e = fock(2)
        # Integer angle words 0 and 1 at beta=3. Their projectors have EXACT
        # rational entries even for pi/4, so this tests noncommuting rotations.
        zs = []
        for word in (0, 1):
            c2 = fmpq([1, 0, -1, 0][word % 4])
            s2 = fmpq([0, 1, 0, -1][word % 4])
            spins = []
            for spin in range(2):
                aa, bb = a[spin], a[2+spin]
                number = (1+c2)/2*aa.transpose()*aa+(1-c2)/2*bb.transpose()*bb
                number += s2/2*(aa.transpose()*bb+bb.transpose()*aa)
                spins.append(identity-2*number)
            zs.append(spins)
        self.assertNotEqual(zs[0][0]*zs[1][0], zs[1][0]*zs[0][0])
        et = [-(z[0]+z[1])/2 for z in zs]
        generator = et[0]/2-et[1]/4+identity/4
        target = et[0]/4-generator*generator/2
        actual = c.execute_projected(p, zs)
        norm = F(proof['normalization_Ha'])
        offset = F(proof['chebyshev_offset_Ha'])
        self.assertEqual(fmpq(norm.numerator, norm.denominator)*actual+fmpq(offset.numerator, offset.denominator)*identity, target)
        self.assertEqual(c.execute_projected(p, zs, control=0), identity)

    def test_rejects_sign_reflection_angle_cleanup_and_table_mutants(self):
        rows, nets, p, o, i = self.example()
        mutants = []
        q = copy.deepcopy(p); q['body'][4][2] = 'control'; mutants.append(q)
        q = copy.deepcopy(p); q['body'][3][1][7][1] = -1; mutants.append(q)
        q = copy.deepcopy(p); q['body'][5][1][6][1] = 'control'; mutants.append(q)
        q = copy.deepcopy(p); q['body'][3][1].pop(); mutants.append(q)
        q = copy.deepcopy(p); q['body'][6][0] = 'ignore_sign'; mutants.append(q)
        q = copy.deepcopy(p); q['outer_words'][0] ^= 1; mutants.append(q)
        q = copy.deepcopy(p); q['inner_words'][1][0] ^= 1 << (i[0]['mu']+2); mutants.append(q)
        q = copy.deepcopy(p); q['rows'][1]['sign'] *= -1; mutants.append(q)
        q = copy.deepcopy(p); q['inner_bits'] += 1; mutants.append(q)
        q = copy.deepcopy(p); q['beta'] += 1; mutants.append(q)
        for q in mutants:
            with self.assertRaises(ValueError):
                c.verify(q, rows, nets, o, i, reference_beta=3)
        wrong = nets.copy(); wrong[0, 0] = 2
        with self.assertRaises(ValueError):
            c.verify(p, rows, wrong, o, i, reference_beta=3)
        bad_tables = copy.deepcopy(i); bad_tables[1]['mu'] += 1
        with self.assertRaises(ValueError):
            c.verify(p, rows, nets, o, bad_tables, reference_beta=3)

    def test_alias_all_draws_and_outward_rational_rounding(self):
        weights = [F(1, 3), F(2, 7), F(1, 11)]
        t = c.alias(weights, 2, 4)
        counts = [0]*4
        for bucket in range(4):
            for draw in range(16):
                counts[bucket if draw < t['keep'][bucket] else t['alt'][bucket]] += 1
        self.assertEqual(counts, c.histogram(t))
        self.assertEqual(sum(counts), 64)
        for q in (F(1, 3), F(7, 11), F(-1, 7), F(0)):
            out = c.dyadic_upper(q)
            self.assertGreaterEqual(out, q)
            self.assertLess(out-q, F(1, 1 << 128))

    def test_signed_factor_and_rotation_checks_do_not_trust_eigensolver(self):
        rows = np.array([[.5, .25, -.5], [.25, 0., .25]])
        signs = np.array([1, -1])
        d = arb_mat([[arb(float(rows[0, j]*rows[0, k]-rows[1, j]*rows[1, k])) for k in range(3)] for j in range(3)])
        self.assertEqual(signed_df.certify_factorization(d, rows, signs, 2), 0)
        bad = rows.copy(); bad[0, 0] += .01
        self.assertGreater(signed_df.certify_factorization(d, bad, signs, 2), F(1, 10000))
        with self.assertRaises(ValueError):
            signed_df.certify_factorization(d, rows, np.array([0, 1]), 2)
        ideal = np.array([[.5, 0., -.25]])
        es = np.array([[.5, -.25]])
        angles = np.array([[[0], [1 << 24]]], dtype=np.uint32)
        correct = signed_df.certify_rotations(ideal, [1], es, angles, 2)
        self.assertLess(F(correct['operator_error_upper_Ha']), F(1, 10**30))
        angles[0, 0, 0] += 1 << 23
        wrong = signed_df.certify_rotations(ideal, [1], es, angles, 2)
        self.assertGreater(F(wrong['operator_error_upper_Ha']), F(1, 100))

    def test_report_bindings_and_complete_cost_accounting(self):
        for name in ('reiher', 'li'):
            cert_path = ROOT/f'challenges/femoco/rigorous/signed-{name}.json'
            cert = json.loads(cert_path.read_text())
            report = json.loads((ROOT/f'challenges/femoco/rigorous/combined-{name}.json').read_text())
            self.assertEqual(report['operator_certificate_sha256'], inputs.sha256(cert_path))
            self.assertEqual(report['checker_sha256'], inputs.sha256(inputs.HERE/'combined.py'))
            for file, digest in cert['checker_sha256'].items():
                self.assertEqual(inputs.sha256(inputs.HERE/file), digest)
            proof, budget, resources = report['verification'], report['budget'], report['resources']
            self.assertTrue(proof['verified'])
            self.assertFalse(proof['fully_lowered_gate_certificate'])
            self.assertFalse(budget['chemical_accuracy_certified'])
            self.assertTrue(budget['fits_1_6_mHa'])
            self.assertEqual(F(cert['combined_normalization_Ha']), F(proof['normalization_Ha']))
            self.assertEqual(F(budget['total_upper_Ha']), sum(F(budget[k]) for k in ('operator_Ha', 'encoding_Ha', 'QPE_Ha')))
            cost = resources['min_toffoli']['controlled_walk_toffoli_upper']
            self.assertEqual(cost, resources['non_lookup_toffoli']+sum(resources['min_toffoli']['lookup_toffoli_by_stage']))
            comparison = report['comparison']
            for key in ('paper_convention', 'conditional_99_percent_QPE'):
                q = comparison[key]
                if key == 'paper_convention':
                    self.assertEqual(q['our_total_toffoli_upper'], cost*q['our_queries'])
                    self.assertEqual(F(q['break_even_step_toffoli_at_our_normalization']), F(q['low_total_toffoli_from_printed_parameters'], q['our_queries']))
                else:
                    self.assertEqual(q['controlled_walk_total_toffoli_upper'], cost*q['controlled_walk_uses'])


if __name__ == '__main__':
    unittest.main()
