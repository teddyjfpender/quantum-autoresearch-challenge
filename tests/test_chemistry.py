"""Independent small Fock-space oracles and failure tests for energy certificates."""
from fractions import Fraction
import hashlib
import io
import json
import pathlib
import pickle
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'challenges/femoco/tools/chemistry'))

import numpy as np
from flint import arb, arb_mat, ctx, fmpq, fmpq_mat
import bounds
import certify
import fetch
import inputs


def exact(x):
    q = bounds.rational(x)
    return fmpq(q.numerator, q.denominator)


def fock(n):
    dim = 1 << (2*n)
    identity = fmpq_mat([[int(i == j) for j in range(dim)] for i in range(dim)])
    annihilation = []
    for mode in range(2*n):
        a = fmpq_mat(dim, dim)
        for state in range(dim):
            if state >> mode & 1:
                a[state ^ (1 << mode), state] = (-1)**((state & ((1 << mode)-1)).bit_count())
        annihilation.append(a)
    e = [[sum((annihilation[2*p+s].transpose()*annihilation[2*q+s]
                for s in range(2)), fmpq_mat(dim, dim)) for q in range(n)] for p in range(n)]
    return identity, annihilation, e


class ChemistryTest(unittest.TestCase):
    def setUp(self):
        self.precision = ctx.prec
        ctx.prec = 128

    def tearDown(self):
        ctx.prec = self.precision

    def test_decimal_enclosure_and_exact_endpoint_serialization(self):
        for s in ('1/3', '-1/7', '0.000000000000000000000000001'):
            q = Fraction(s)
            x = arb(s)
            data = bounds.enclosure(x)
            self.assertLessEqual(Fraction(data['lower_Ha']), q)
            self.assertGreaterEqual(Fraction(data['upper_Ha']), q)
        with self.assertRaises(ValueError):
            bounds.rational(arb('1/3'))
        with self.assertRaises(ValueError):
            bounds.upper(arb('nan'))

    def test_bliss_and_fit_identity_against_exact_fermion_hamiltonian(self):
        n, eta = 2, 1
        ident, a, e = fock(n)
        et = [[e[p][q] - (ident if p == q else 0*ident) for q in range(n)] for p in range(n)]
        h = arb_mat([[1, '1/4'], ['1/4', '-1/2']])
        g = arb_mat([['1/2', '1/8', '1/4'], ['1/8', '1/4', '-1/8'], ['1/4', '-1/8', '3/4']])
        core = arb('3/8')
        spec = dict(n=n, eta=eta, r=1, b=2, c=1, wb=[0.5], w=[0.75, -0.25])
        params = {'U': np.array([[[1., 0.], [0., 1.]]]),
                  'W': np.array([[[0.75], [-0.25]]]),
                  'Hsym_lower': np.array([0.25, -0.125, 0.5]), 'H1_I': np.array(0.125)}
        rows, _ = bounds.factor_rows(params, spec)
        hs, c, shift = bounds.shifted_one_body(h, g, core, rows, params, spec)
        d = bounds.residual(rows, g, shift, n)
        target = exact(core)*ident
        for p in range(n):
            for q in range(n):
                target += exact(h[p, q])*e[p][q]
                for r in range(n):
                    for s in range(n):
                        for spin in range(2):
                            for tau in range(2):
                                target += (exact(g[inputs.pair(p, q), inputs.pair(r, s)])/2
                                           *a[2*p+spin].transpose()*a[2*r+tau].transpose()
                                           *a[2*s+tau]*a[2*q+spin])
        fit = exact(c)*ident
        for p in range(n):
            for q in range(n):
                fit += exact(hs[p, q])*et[p][q]
        op = fmpq(1, 2)*ident
        for p in range(n):
            for q in range(n):
                op += exact(rows[0, inputs.pair(p, q)])*et[p][q]
        fit += op*op/2
        residual = 0*ident
        for p in range(n):
            for q in range(n):
                for r in range(n):
                    for s in range(n):
                        residual += exact(d[inputs.pair(p, q), inputs.pair(r, s)])/2*et[p][q]*et[r][s]
        sector = [i for i in range(ident.nrows()) if i.bit_count() == eta]
        diff = fit-target-residual
        self.assertTrue(all(diff[i, j] == 0 for i in sector for j in sector))
        self.assertTrue(any(diff[i, i] != 0 for i in range(ident.nrows()) if i not in sector))
        # Determinant formula and conservative norm independently checked against
        # every Fock basis state / exact matrix row sum, not a sampled state.
        for state in range(ident.nrows()):
            alpha = [i for i in range(n) if state >> (2*i) & 1]
            beta = [i for i in range(n) if state >> (2*i+1) & 1]
            self.assertEqual(exact(bounds.fit_expectation(d, alpha, beta, n)), residual[state, state])
        norm_bound = bounds.upper(bounds.fit_norm(d, n))
        dense = np.array([[float(arb(residual[i, j])) for j in range(ident.nrows())] for i in range(ident.nrows())])
        self.assertLessEqual(np.max(np.abs(np.linalg.eigvalsh(dense))), float(norm_bound)+1e-14)

    def test_interval_slater_energy_against_exact_normalized_fock_state(self):
        # Non-orthonormal input columns require the Gram inverse, not C C^T.
        spec = dict(n=2, r=1, b=2, c=1, beta=3, e=[0.5, -0.25],
                    ea=[0, 2], angles=[0, 2], wb=[0.125], w=[0.75, -0.5], const=arb('1/8'))
        ca, cb = arb_mat([[3], [4]]), arb_mat([[2], [0]])
        energy = bounds.determinant_energy(spec, bounds.projector(ca), bounds.projector(cb))
        ident, a, e = fock(2)
        z0, z1 = e[0][0]-ident, e[1][1]-ident
        q = fmpq(1, 8)*ident + fmpq(3, 4)*z0-fmpq(1, 2)*z1
        ham = fmpq(1, 8)*ident+z0/2-z1/4+q*q/2
        vacuum = fmpq_mat(16, 1)
        vacuum[0, 0] = 1
        state = (fmpq(3, 5)*a[0].transpose()+fmpq(4, 5)*a[2].transpose())*a[1].transpose()*vacuum
        want = (state.transpose()*ham*state)[0, 0]
        self.assertTrue(energy.contains(arb(want)))
        with self.assertRaises(ZeroDivisionError):
            bounds.projector(arb_mat([[1, 2], [2, 4]]))

    def test_rounding_bound_rejects_wrong_angles_and_wrong_scalar(self):
        spec = dict(n=2, r=1, b=1, c=1, beta=3, e=[0.5, -0.25],
                    ea=[0, 2], angles=[0], wb=[0.125], w=[0.75], const=arb('1/8'))
        h = arb_mat([['1/2', 0], [0, '-1/4']])
        us = [[arb(1), arb(0)]]
        correct = bounds.rotation_bound(spec, us, h, spec['const'])
        self.assertLess(float(Fraction(correct['upper_Ha'])), 1e-15)
        wrong_angle = bounds.rotation_bound(spec, us, h, spec['const'], angles=[1])
        self.assertGreater(float(Fraction(wrong_angle['upper_Ha'])), 0.5)
        wrong_scalar = bounds.rotation_bound(spec, us, h, spec['const']+1)
        self.assertGreaterEqual(Fraction(wrong_scalar['upper_Ha']), 1)

    def test_witness_obstruction_does_not_claim_ground_energy_failure(self):
        d = arb_mat([[1, 0, 0], [0, 0, 0], [0, 0, 0]])
        report = bounds.determinant_witnesses(d, 2, 1)
        self.assertIsNone(report['ground_energy_error_lower_bound'])
        for state in report['states']:
            self.assertEqual(len(state['alpha'])+len(state['beta']), 1)
        with self.assertRaises(ValueError):
            bounds.fit_expectation(d, [0, 0], [], 2)

    def test_budget_factor_two_boundaries_and_unconditional_claim(self):
        report = certify.budget(0, 0)
        self.assertEqual(report['components_Ha']['coefficient'], '1/5000')
        self.assertTrue(report['energy_budget_met'])
        self.assertFalse(report['chemical_accuracy_certified'])
        self.assertTrue(certify.budget('1/2500', 0)['energy_budget_met'])
        self.assertFalse(certify.budget('401/1000000', 0)['energy_budget_met'])
        with self.assertRaises(ValueError):
            certify.budget(-1, 0)

    def test_qpe_tail_and_lipschitz_budget(self):
        for norm in (1, 60, 180):
            plan = bounds.qpe_plan(norm)
            self.assertLessEqual(Fraction(1, 2*(plan['tail_bins']-1)), Fraction(plan['failure_probability']))
            error = bounds.upper(2*arb.pi()*norm*plan['tail_bins']/(1 << plan['phase_register_qubits']))
            self.assertLessEqual(error, Fraction(plan['energy_error_Ha']))
            self.assertFalse(plan['includes_state_preparation_or_gate_synthesis'])
        for args in ((0,), (1, 0), (1, Fraction(1, 1000), 0)):
            with self.assertRaises(ValueError):
                bounds.qpe_plan(*args)

    def test_pinned_integrals_exact_decimal_core_sector_and_tampering(self):
        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp)/'test.fcidump'
            path.write_text('&FCI NORB=2,NELEC=1,MS2=1,\n&END\n0.1 1 1 1 1\n0.2 1 1 0 0\n3.5 0 0 0 0\n')
            source = {'integrals_sha256': inputs.sha256(path), 'core_policy': 'fcidump'}
            h, g, c = inputs.integrals(path, source, 2, 1)
            self.assertTrue(g[0, 0].contains(arb(fmpq(1, 10))))
            self.assertEqual(c, arb('3.5'))
            source['core_policy'] = 'zero'
            self.assertEqual(inputs.integrals(path, source, 2, 1)[2], 0)
            with self.assertRaises(ValueError):
                inputs.integrals(path, source, 2, 2)
            path.write_text(path.read_text()+'1 2 2 0 0\n')
            with self.assertRaises(ValueError):
                inputs.integrals(path, source, 2, 1)

    def test_pickle_globals_and_non_range_http_fail_closed(self):
        with self.assertRaises(ValueError):
            inputs.ArrayReader(io.BytesIO(pickle.dumps(eval))).load()
        with patch('urllib.request.urlopen') as opener:
            opener.return_value.__enter__.return_value.status = 200
            with self.assertRaises(ValueError):
                fetch.download('https://example.org/a?download=1', 0, 10)

    def test_shipped_reports_remain_explicit_about_scope_and_obstruction(self):
        for name in ('reiher', 'li'):
            p = ROOT/'challenges/femoco/rigorous'/f'energy-{name}.json'
            if not p.exists():
                self.fail('missing reproducible chemistry report')
            r = json.loads(p.read_text())
            self.assertFalse(r['end_to_end']['chemical_accuracy_certified'])
            self.assertFalse(r['factorization']['ground_energy_accuracy_decided'])
            self.assertGreater(Fraction(r['factorization']['witnesses']['any_scalar_shift_norm_lower_Ha']), Fraction(1, 625))
            for file, digest in r['checker_sha256'].items():
                self.assertEqual(inputs.sha256(inputs.HERE/file), digest)
            self.assertEqual(inputs.sha256(inputs.HERE/'sources.json'), r['inputs']['sources_sha256'])
            self.assertTrue(all(not p['implemented_circuit'] for p in r['rotation_precision_proposals']))


if __name__ == '__main__':
    unittest.main()
