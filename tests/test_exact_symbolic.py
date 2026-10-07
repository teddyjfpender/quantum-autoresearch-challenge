"""Exact quantum proof: independent gate algebra oracle and adversarial obligations."""
import importlib.util
import itertools
import pathlib
import types
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
try:
    from dd import cudd
except ImportError:
    cudd = None


@unittest.skipIf(cudd is None, 'install tools/verification/requirements.txt')
class ExactTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        path = ROOT / 'challenges/femoco/tools/verification/exact.py'
        spec = importlib.util.spec_from_file_location('exact', path)
        cls.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.m)

    def f(self, names=()):
        return self.m.Boolean(names)

    def test_boolean_arithmetic_and_lookup_exhaustive(self):
        f = self.f(['a0', 'a1', 'a2', 'b0', 'b1', 'b2'])
        a = tuple(f.bdd.var(f'a{i}') for i in range(3))
        b = tuple(f.bdd.var(f'b{i}') for i in range(3))
        add, neg, less = f.add(a, b), f.neg(a), f.less(a, b)
        table = [7, 2, 1, 4, 0, 3, 6, 5]
        lut = f.lookup(table, a, 3)
        for x, y in itertools.product(range(8), repeat=2):
            assignment = {f'{name}{i}': bool(v >> i & 1)
                          for name, v in [('a', x), ('b', y)] for i in range(3)}
            value = lambda word: sum(int(f.bdd.let(assignment, bit) == f.one) << i
                                     for i, bit in enumerate(word))
            self.assertEqual(value(add), (x+y) % 8)
            self.assertEqual(value(neg), -x % 8)
            self.assertEqual(value(lut), table[x])
            self.assertEqual(f.bdd.let(assignment, less) == f.one, x < y)

    def test_gate_normalization_against_exact_fock_matrices(self):
        # Quarter-turn Givens, Pauli X and Z are signed permutation matrices.
        # This oracle has no trigonometry, BDD arithmetic, or rewrite rules.
        f = self.f()

        def matrix_action(events, state):
            amplitude = 1
            for e in events:
                if e[0] == 'Z' and e[2] == f.one:
                    amplitude *= -1 if state >> e[1] & 1 else 1
                elif e[0] == 'X' and e[2] == f.one:
                    state ^= 1 << e[1]
                elif e[0] == 'G':
                    angle = sum(int(bit == f.one) << i for i, bit in enumerate(e[3]))
                    # G(pi/2)|10> = |01>, G(pi/2)|01> = -|10>.
                    for _ in range(angle):
                        if state == 1: state = 2
                        elif state == 2: state, amplitude = 1, -amplitude
            return state, amplitude

        for a, b, z0, z1, x in itertools.product(range(4), range(4), range(2), range(2), range(2)):
            events = [('Z', 0, f.word(z0, 1)[0]), ('Z', 1, f.word(z1, 1)[0]),
                      ('G', 0, 1, f.word(a, 2)), ('X', 0, f.word(x, 1)[0]),
                      ('G', 0, 1, f.word(b, 2)), ('Z', 0, f.one)]
            word, pending, phase = self.m.normalize(f, events, 2, 2, f.one)
            normalized = word + [('Z', q, v) for q, v in enumerate(pending)]
            for state in range(4):
                dest, amplitude = matrix_action(normalized, state)
                amplitude *= -1 if phase == f.one else 1
                self.assertEqual((dest, amplitude), matrix_action(events, state))

    def test_symbolic_inverse_chain_cancels_on_all_angles(self):
        f = self.f([f'a{i}' for i in range(6)] + ['z', 'enable'])
        angles = [tuple(f.bdd.var(f'a{3*j+i}') for i in range(3)) for j in range(2)]
        events = [('Z', 0, f.bdd.var('z'))]
        events += [('G', j, j+1, a) for j, a in enumerate(angles)]
        events += [('G', j, j+1, f.neg(angles[j])) for j in (1, 0)]
        events += [('Z', 0, f.bdd.var('z'))]
        word, z, phase = self.m.normalize(f, events, 3, 3, f.bdd.var('enable'))
        self.assertEqual(word, [])
        self.assertTrue(all(v == f.zero for v in z))
        self.assertEqual(phase, f.zero)

    def test_nonclifford_angles_in_exact_cyclotomic_ring(self):
        # Q[zeta]/(zeta^4+1), zeta = exp(i*pi/4). Exact fractions, no floats.
        from fractions import Fraction
        f = self.f()
        zero = (Fraction(0),)*4
        one = (Fraction(1), *zero[1:])
        add = lambda a, b: tuple(x+y for x, y in zip(a, b))
        scale = lambda a, k: tuple(x*k for x in a)

        def mul(a, b):
            out = list(zero)
            for i, x in enumerate(a):
                for j, y in enumerate(b):
                    out[(i+j) % 4] += x*y*(-1 if i+j >= 4 else 1)
            return tuple(out)

        def root(k):
            k %= 8
            out = list(zero)
            out[k % 4] = Fraction(-1 if k >= 4 else 1)
            return tuple(out)

        def action(events, state):
            v = [one if i == state else zero for i in range(4)]
            for e in events:
                if e[0] == 'Z' and e[2] == f.one:
                    v = [scale(x, -1 if i >> e[1] & 1 else 1) for i, x in enumerate(v)]
                elif e[0] == 'X' and e[2] == f.one:
                    v = [v[i ^ (1 << e[1])] for i in range(4)]
                elif e[0] == 'G':
                    a = sum(int(bit == f.one) << i for i, bit in enumerate(e[3]))
                    co = scale(add(root(a), root(-a)), Fraction(1, 2))
                    si = scale(mul(root(2), add(root(a), scale(root(-a), -1))), Fraction(-1, 2))
                    v[1], v[2] = add(mul(co, v[1]), scale(mul(si, v[2]), -1)), add(mul(si, v[1]), mul(co, v[2]))
            return v

        for a, b, z, x in itertools.product(range(8), range(8), range(2), range(2)):
            events = [('Z', 0, f.word(z, 1)[0]), ('G', 0, 1, f.word(a, 3)),
                      ('X', 0, f.word(x, 1)[0]), ('G', 0, 1, f.word(b, 3)), ('Z', 1, f.one)]
            word, pending, phase = self.m.normalize(f, events, 2, 3, f.one)
            word += [('Z', q, v) for q, v in enumerate(pending)]
            for state in range(4):
                got = [scale(v, -1 if phase == f.one else 1) for v in action(word, state)]
                self.assertEqual(got, action(events, state))

    def fixture(self):
        # One spatial chain; outer buckets 0 and 1 are one-body, 2 is square,
        # 3 aliases to 0. Inner bit selects Majorana 0/1 or square/identity.
        doc = {'schema': 'femoco-symbolic-input-v1', 'spec': 'test', 'digests': {},
               'layout': {'system': 4, 'uniform': 5, 'num_qubits': 12, 'num_bits': 1},
               'inner': {'lo': 3, 'width': 2}, 'b': 1, 'beta': 3,
               'rotation_widths': None, 'registers': [], 'ops': ['Reflect'],
               'outer': {'k': 2, 'mu': 0, 'keep': [1, 1, 1, 0], 'alt': [0, 0, 0, 0]},
               'inner_tables': [{'k': 1, 'mu': 0, 'keep': [1, 1], 'alt': [0, 0]}]*3}
        spec = dict(n=2, r=1, b=1, c=1, beta=3, e=[1, -1], ea=[1, 5],
                    wb=[-1], weights=[2], angles=[3])
        f, c, u, after, _ = self.m.variables(doc, 0, 0)
        # Independent direct Boolean decode of the tiny tables above.
        square = u[1] & ~u[0]
        one = u[0] & ~u[1]
        ob = ~square
        phase = f.zero
        events = []
        for pass_id, word in enumerate((u[3:], after)):
            term, inner_spin = word
            spin = f.bdd.ite(ob, u[2], inner_spin)
            angle = f.mux(square, f.word(3, 3), f.mux(one, f.word(5, 3), f.word(1, 3)))
            z = c & ((ob & term) | (square & ~term))
            x = c & ob
            events.append([('F', True, spin), ('G', 0, 2, f.neg(angle)),
                           ('Z', 0, z), ('X', 0, x), ('G', 0, 2, angle), ('F', False, spin)])
            # Both square branches have negative sign; one-body signs alternate
            # by eigenvalue and by the adjoint on the second pass.
            one_sign = term & (one if pass_id else ~one)
            phase = f.xor(phase, c & (square | (ob & one_sign)))
        actual = types.SimpleNamespace(events=events, phase=(f.zero, f.zero, phase))
        return doc, spec, f, c, u, after, actual

    def test_exact_quantum_reference_and_mutants(self):
        for mutation in ('none', 'angle', 'phase', 'spin', 'pivot', 'inactive', 'extra_gate',
                         'coefficient_sign', 'one_body_sign'):
            with self.subTest(mutation=mutation):
                doc, spec, f, c, u, after, actual = self.fixture()
                if mutation == 'angle': actual.events[0][4] = ('G', 0, 2, f.word(2, 3))
                if mutation == 'phase': actual.phase = (f.zero, f.zero, ~actual.phase[2])
                if mutation == 'spin': actual.events[0][0] = ('F', True, f.zero)
                if mutation == 'pivot': actual.events[0][3] = ('X', 0, f.zero)
                if mutation == 'inactive': actual.events[0][3] = ('X', 0, f.one)
                if mutation == 'extra_gate': actual.events[0].insert(4, ('Z', 3, f.one))
                if mutation == 'coefficient_sign': spec['weights'] = [-2]
                if mutation == 'one_body_sign': spec['e'] = [-1, -1]
                if mutation == 'none':
                    self.assertEqual(self.m.quantum(doc, spec, f, c, u, after, actual), ['payload']*2)
                else:
                    with self.assertRaises(self.m.Unproved):
                        self.m.quantum(doc, spec, f, c, u, after, actual)

    def test_vector_sign_change_accounts_for_one_body_phase(self):
        doc, spec, f, c, u, after, actual = self.fixture()
        # N=2: changing the network angle by pi sends u to -u. Do this only
        # for the leaf with a high last-angle bit, on both copies.
        one = u[0] & ~u[1]
        for events in actual.events:
            angle = events[4][3]
            normalized = f.mux(one, f.add(angle, f.word(4, 3)), angle)
            events[1] = ('G', 0, 2, f.neg(normalized))
            events[4] = ('G', 0, 2, normalized)
        # The pi/Z gauge can already absorb this N=2 sign change using the
        # original payload word; either exact reference form is sufficient.
        self.assertEqual(len(self.m.quantum(doc, spec, f, c, u, after, actual)), 2)
        # Changing only one copy leaves a one-body minus sign: must fail.
        actual.events[1][1] = ('G', 0, 2, f.neg(f.mux(one, f.word(5, 3), actual.events[1][4][3])))
        actual.events[1][4] = ('G', 0, 2, f.mux(one, f.word(5, 3), actual.events[1][4][3]))
        with self.assertRaises(self.m.Unproved):
            self.m.quantum(doc, spec, f, c, u, after, actual)

    def test_controller_covers_all_hmr_outcomes(self):
        for correction in (True, False):
            doc, spec, _, _, _, _, _ = self.fixture()
            nn = self.m.NONE
            doc['ops'] = [{'Ccx': {'a': 0, 'b': 5, 't': 10, 'cond': nn}},
                          {'Hmr': {'t': 10, 'bit': 0, 'cond': nn}}]
            if correction:
                doc['ops'].append({'Phase': {'q': [0, 5, nn], 'k': 4, 'cond': 0}})
            doc['ops'].append('Reflect')
            f, c, u, after, count = self.m.variables(doc, 0, 0)
            actual = self.m.Controller(doc, f, c, u, after, count)
            actual.run()
            self.assertEqual(actual.phase == f.word(0, 3), correction)
            if not correction: self.assertIn('m0', actual.phase[2].support)

    def test_conditional_measurement_and_eighth_root_phase(self):
        doc, _, _, _, _, _, _ = self.fixture()
        nn = self.m.NONE
        doc['layout']['num_bits'] = 2
        doc['ops'] = [
            {'Hmr': {'t': 10, 'bit': 0, 'cond': nn}},
            {'Push': {'bit': 0}},
            {'Cx': {'c': 0, 't': 10, 'cond': nn}},
            {'Hmr': {'t': 10, 'bit': 1, 'cond': nn}},
            {'Phase': {'q': [0, nn, nn], 'k': 4, 'cond': 1}},
            {'Phase': {'q': [0, nn, nn], 'k': 2, 'cond': nn}},
            {'Phase': {'q': [0, nn, nn], 'k': 6, 'cond': nn}},
            'Pop', 'Reflect']
        f, c, u, after, count = self.m.variables(doc, 0, 0)
        actual = self.m.Controller(doc, f, c, u, after, count)
        actual.run()
        self.assertEqual(actual.phase, f.word(0, 3))

    def test_dirty_reset_and_changed_reflection_reject(self):
        for kind, wire in [('Reset', 10), ('Reflect', 8)]:
            doc, _, _, _, _, _, _ = self.fixture()
            doc['ops'] = [{'X': {'t': wire, 'cond': self.m.NONE}}]
            if kind == 'Reset': doc['ops'].append({'Reset': {'t': wire}})
            doc['ops'].append('Reflect')
            f, c, u, after, count = self.m.variables(doc, 0, 0)
            with self.assertRaises(self.m.Unproved):
                self.m.Controller(doc, f, c, u, after, count).run()

    def test_partitions_are_disjoint_and_cover_all_bucket_values(self):
        doc, _, _, _, _, _, _ = self.fixture()
        seen = []
        for shard in range(4):
            f, _, u, _, _ = self.m.variables(doc, 2, shard)
            seen.append(sum(int(u[i] == f.one) << i for i in range(2)))
        self.assertEqual(seen, list(range(4)))
        with self.assertRaises(ValueError): self.m.variables(doc, 3, 0)

    def test_alias_decoder_all_sigma_values_and_boundaries(self):
        doc, _, _, _, _, _, _ = self.fixture()
        doc['layout']['uniform'] = 7
        doc['inner'] = {'lo': 4, 'width': 3}
        doc['outer'] = {'k': 2, 'mu': 1, 'keep': [2, 0, 1, 0], 'alt': [2, 0, 2, 1]}
        doc['inner_tables'] = [
            {'k': 1, 'mu': 1, 'keep': [2, 0], 'alt': [1, 0]},
            {'k': 1, 'mu': 1, 'keep': [0, 1], 'alt': [1, 0]},
            {'k': 1, 'mu': 1, 'keep': [1, 1], 'alt': [1, 0]}]
        f, _, u, after, _ = self.m.variables(doc, 0, 0)
        item, _, terms = self.m.decode(doc, f, u, after)

        def concrete(t, value):
            bucket = value & ((1 << t['k'])-1)
            draw = (value >> t['k']) & ((1 << t['mu'])-1)
            return bucket if draw < t['keep'][bucket] else t['alt'][bucket]

        for s, a in itertools.product(range(128), range(8)):
            env = {f'{name}{i}': bool(v >> i & 1) for name, v, width in [('u', s, 7), ('a', a, 3)] for i in range(width)}
            value = lambda word: sum(int(f.bdd.let(env, bit) == f.one) << i for i, bit in enumerate(word))
            o = concrete(doc['outer'], s)
            self.assertEqual(value(item), o)
            self.assertEqual(value(terms[0][0]), concrete(doc['inner_tables'][o], s >> 4))
            self.assertEqual(value(terms[1][0]), concrete(doc['inner_tables'][o], a))

    def test_payload_digest_mismatch_fails_closed(self):
        path = ROOT / 'challenges/femoco/specs/reiher-sa-v1/sa.bin'
        with self.assertRaisesRegex(ValueError, 'digest'):
            self.m.payload(path, {'digests': {'spec': '0'*64}})

    def test_measurement_dependent_phase_never_certifies(self):
        doc, spec, f, c, u, after, actual = self.fixture()
        f.bdd.declare('measurement')
        actual.phase = (f.zero, f.zero, f.xor(actual.phase[2], f.bdd.var('measurement')))
        with self.assertRaisesRegex(self.m.Unproved, 'scalar phase'):
            self.m.quantum(doc, spec, f, c, u, after, actual)

    def test_unsupported_system_gate_fails_closed(self):
        doc, _, _, _, _, _, _ = self.fixture()
        doc['ops'] = [{'SysS': {'q': 0, 'k': 1, 'cond': self.m.NONE}}, 'Reflect']
        f, c, u, after, count = self.m.variables(doc, 0, 0)
        with self.assertRaisesRegex(ValueError, 'unsupported'):
            self.m.Controller(doc, f, c, u, after, count).run()


if __name__ == '__main__':
    unittest.main()
