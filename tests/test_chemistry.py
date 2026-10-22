"""Independent small Fock-space oracles and failure tests for energy certificates."""
from fractions import Fraction
import hashlib
import io
import json
import pathlib
import pickle
import struct
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
        # The signed-square spectral bound is a second, independent certificate
        # of the same operator norm; the checker reports the smaller of the two.
        spectral = bounds.spectral_fit_norm(d, n)
        self.assertLessEqual(np.max(np.abs(np.linalg.eigvalsh(dense))), float(Fraction(spectral['upper_Ha']))+1e-14)
        self.assertEqual(Fraction(spectral['upper_Ha']),
                         Fraction(spectral['signed_squares_upper_Ha'])+Fraction(spectral['remainder_triangle_upper_Ha']))
        self.assertLess(Fraction(spectral['remainder_triangle_upper_Ha']), Fraction(1, 10**12))
        with self.assertRaises(ValueError):
            bounds.spectral_fit_norm(arb_mat(2, 2), n)

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
            tail, bits = plan['tail_bins'], plan['phase_register_qubits']
            self.assertLessEqual(bounds.qpe_tail_bound(tail, bits), Fraction(plan['failure_probability']))
            self.assertEqual(Fraction(plan['tail_probability_upper']), bounds.qpe_tail_bound(tail, bits))
            error = bounds.upper(2*arb.pi()*norm*tail/(1 << bits))
            self.assertLessEqual(error, Fraction(plan['energy_error_Ha']))
            self.assertFalse(plan['includes_state_preparation_or_gate_synthesis'])
            # Never more phase bits than the classical 1/[2(L-1)] rule needs.
            classical = 1
            while bounds.upper(2*arb.pi()*norm*51/(1 << classical)) > Fraction(1, 1000):
                classical += 1
            self.assertLessEqual(bits, classical)
        for args in ((0,), (1, 0), (1, Fraction(1, 1000), 0)):
            with self.assertRaises(ValueError):
                bounds.qpe_plan(*args)
        with self.assertRaises(ValueError):
            bounds.qpe_tail_bound(1, 4)

    def test_qpe_tail_bound_against_exact_outcome_distribution(self):
        # Exact m-bit QPE statistics of an eigenstate with phase (b+delta)/2^m:
        # the probability of landing more than L bins (circularly) from the true
        # phase must never exceed the certified tail, for every offset delta.
        rng = np.random.default_rng(7)
        for bits in (5, 7, 9):
            size = 1 << bits
            k = np.arange(size)
            for delta in (0.0, 1e-9, 0.25, 0.5, 0.75, 1-1e-9, *rng.random(5)):
                amplitudes = np.exp(2j*np.pi*k[None, :]*(delta-k[:, None])/size).sum(axis=1)/size
                p = np.abs(amplitudes)**2
                self.assertAlmostEqual(p.sum(), 1.0, places=12)
                signed = ((k+size//2) % size)-size//2
                distance = np.abs(signed-delta)
                for tail in range(2, 25):
                    exact = p[distance > tail].sum()
                    self.assertLessEqual(exact, float(bounds.qpe_tail_bound(tail, bits))+1e-12)

    def test_nuclear_norm_certificate_against_exact_rational_spectrum(self):
        # Rational Householder reflection and integer eigenvalues 1, -2, 3.
        v = fmpq_mat([[1], [2], [2]])
        q = fmpq_mat([[int(i == j) for j in range(3)] for i in range(3)]) - fmpq(2, 9)*v*v.transpose()
        self.assertEqual(q*q.transpose(), fmpq_mat([[int(i == j) for j in range(3)] for i in range(3)]))
        m = q*fmpq_mat([[1, 0, 0], [0, -2, 0], [0, 0, 3]])*q.transpose()
        a = arb_mat([[arb(m[i, j]) for j in range(3)] for i in range(3)])
        bound = bounds.nuclear_norm_upper(a)
        self.assertGreaterEqual(bound, 6)
        # Tightness is limited by the float64 proposal's orthogonality defect.
        self.assertLess(bound-6, Fraction(1, 10**12))
        self.assertEqual(bound.denominator & (bound.denominator-1), 0)
        self.assertEqual(bounds.dyadic_upper(Fraction(1, 3), 4), Fraction(6, 16))
        self.assertEqual(bounds.dyadic_upper(Fraction(-1, 3), 4), Fraction(-5, 16))
        entry = sum(abs(Fraction(str(m[i, j]))) for i in range(3) for j in range(3))
        self.assertLess(bound, entry)
        # A widened ball still encloses the exact value, from below as well.
        wide = arb_mat([[a[i, j]+arb('0 +/- 1e-6') for j in range(3)] for i in range(3)])
        self.assertGreaterEqual(bounds.nuclear_norm_upper(wide), 6)
        self.assertEqual(bounds.one_body_norm(arb_mat(2, 2)), 0)
        with self.assertRaises(ValueError):
            bounds.nuclear_norm_upper(arb_mat(2, 3))

    def test_rotation_bound_encloses_exact_fock_operator_difference(self):
        # ||H_spec - H_fit|| on the full two-orbital Fock space, computed from the
        # exact quantized network vectors, never exceeds the certified bound.
        n = 2
        spec = dict(n=n, r=1, b=1, c=1, beta=3, e=[0.5, -0.25],
                    ea=[0, 2], angles=[1], wb=[0.125], w=[0.75], const=arb('1/8'))
        h = arb_mat([['1/2', 0], [0, '-1/4']])
        us = [[arb(1), arb(0)]]
        report = bounds.rotation_bound(spec, us, h, spec['const'])
        ident, _, e = fock(n)
        et = [[e[p][q]-(ident if p == q else 0*ident) for q in range(n)] for p in range(n)]
        def one_body(matrix):
            return sum((matrix[p][q]*et[p][q] for p in range(n) for q in range(n)), 0*ident)
        v = bounds.network([1], 3)
        rows = [[v[p]*v[q] for q in range(n)] for p in range(n)]
        dim = ident.nrows()
        def dense(op):
            return np.array([[float(arb(op[i, j])) for j in range(dim)] for i in range(dim)])
        et_v = sum((float(rows[p][q])*dense(et[p][q]) for p in range(n) for q in range(n)), np.zeros((dim, dim)))
        et_u = dense(et[0][0])
        ideal = 0.125*np.eye(dim)+0.5*dense(et[0][0])-0.25*dense(et[1][1])+0.5*(0.125*np.eye(dim)+0.75*et_u)@(0.125*np.eye(dim)+0.75*et_u)
        stored = 0.125*np.eye(dim)+0.5*dense(et[0][0])-0.25*dense(et[1][1])+0.5*(0.125*np.eye(dim)+0.75*et_v)@(0.125*np.eye(dim)+0.75*et_v)
        exact_norm = np.max(np.abs(np.linalg.eigvalsh(stored-ideal)))
        self.assertGreater(exact_norm, 0.1)
        self.assertLessEqual(exact_norm, float(Fraction(report['upper_Ha']))+1e-12)
        self.assertLessEqual(float(Fraction(report['squares_upper_Ha'])), 0.875*0.75*2*np.sqrt(1-np.cos(np.pi/4)**2)+1e-12)

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
            # Repeated packed pairs must agree to the decimal text; a dense
            # FCIDUMP that lists (pq|rs) and (rs|pq) differently is refused.
            for extra in ('0.1000000001 1 1 1 1\n', '0.2 2 1 0 0\n0.3 1 2 0 0\n'):
                path.write_text('&FCI NORB=2,NELEC=1,MS2=1,\n&END\n0.1 1 1 1 1\n0.2 1 1 0 0\n3.5 0 0 0 0\n'+extra)
                source['integrals_sha256'] = inputs.sha256(path)
                with self.assertRaises(ValueError):
                    inputs.integrals(path, source, 2, 1)
            path.write_text('&FCI NORB=2,NELEC=1,MS2=1,\n&END\n0.1 1 1 2 2\n0.1 2 2 1 1\n0.2 2 1 0 0\n0.2 1 2 0 0\n')
            source['integrals_sha256'] = inputs.sha256(path)
            h, g, _ = inputs.integrals(path, source, 2, 1)
            self.assertTrue(g[0, 2].contains(arb(fmpq(1, 10))) and h[1, 0].contains(arb(fmpq(1, 5))))

    def test_pickle_globals_and_non_range_http_fail_closed(self):
        with self.assertRaises(ValueError):
            inputs.ArrayReader(io.BytesIO(pickle.dumps(eval))).load()
        with patch('urllib.request.urlopen') as opener:
            opener.return_value.__enter__.return_value.status = 200
            with self.assertRaises(ValueError):
                fetch.download('https://example.org/a?download=1', 0, 10)
        # Whole-resource downloads need a pinned size and stop one byte past it.
        with self.assertRaises(ValueError):
            fetch.download('https://example.org/a?download=1')
        with patch('urllib.request.urlopen') as opener:
            response = opener.return_value.__enter__.return_value
            response.status = 200
            response.read = lambda count=None: b'x'*min(count, 100)
            with self.assertRaises(ValueError):
                fetch.download('https://example.org/a?download=1', limit=50)
            self.assertEqual(fetch.download('https://example.org/a?download=1', limit=100), b'x'*100)
            self.assertEqual(fetch.download('https://example.org/a?download=1', limit=200), b'x'*100)
        sources = json.loads((inputs.HERE/'sources.json').read_text())
        self.assertGreater(sources['integrals_archive']['size'], 0)

    def test_existing_base_payload_is_parsed_or_refused(self):
        import corrected
        spec = dict(n=2, r=1, b=1, c=1, eta=1, wb=[0.125], w=[0.75])
        data = bytearray(b'FEMOSAS1')+struct.pack('<7Id', 1, 2, 1, 1, 1, 3, 1, 0.5)
        data += struct.pack('<2d2I1d1d1I', 0.25, -0.5, 1, 2, 0.125, 0.75, 3)
        const, eigs, ea, angles = corrected.read_payload(bytes(data), spec, 3)
        self.assertEqual((const, eigs, ea, angles), (0.5, (0.25, -0.5), (1, 2), (3,)))
        for bad in (data[:-4], data[:8]+struct.pack('<7Id', 1, 2, 1, 1, 1, 4, 1, 0.5)+data[44:],
                    data[:44]+struct.pack('<2d2I1d1d1I', 0.25, -0.5, 1, 8, 0.125, 0.75, 3),
                    data[:44]+struct.pack('<2d2I1d1d1I', 0.25, -0.5, 1, 2, 0.125, 0.5, 3)):
            with self.assertRaises(ValueError):
                corrected.read_payload(bytes(bad), spec, 3)

    def test_shipped_reports_remain_explicit_about_scope_and_obstruction(self):
        for name in ('reiher', 'li'):
            p = ROOT/'challenges/femoco/rigorous'/f'energy-{name}.json'
            if not p.exists():
                self.fail('missing reproducible chemistry report')
            r = json.loads(p.read_text())
            self.assertFalse(r['end_to_end']['chemical_accuracy_certified'])
            self.assertFalse(r['factorization']['ground_energy_accuracy_decided'])
            self.assertGreater(Fraction(r['factorization']['witnesses']['any_scalar_shift_norm_lower_Ha']), Fraction(1, 625))
            # Every module in the tool directory is pinned, so a planted file is
            # detected, and every pinned hash is current.
            self.assertEqual(set(r['checker_sha256']), {p.name for p in inputs.HERE.glob('*.py')})
            for file, digest in r['checker_sha256'].items():
                self.assertEqual(inputs.sha256(inputs.HERE/file), digest)
            self.assertEqual(inputs.sha256(inputs.HERE/'sources.json'), r['inputs']['sources_sha256'])
            self.assertTrue(all(not p['implemented_circuit'] for p in r['rotation_precision_proposals']))


if __name__ == '__main__':
    unittest.main()
