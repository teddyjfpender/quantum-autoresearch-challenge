"""The symbolic checker: an independent gate-algebra oracle, real evaluator exports of small
circuits, and one test per obligation so that weakening any single check fails the suite."""
import hashlib
import importlib.util
import itertools
import json
import os
import pathlib
import tempfile
import types
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
try:
    from dd import cudd
except ImportError:
    cudd = None


FIXTURES = ROOT / 'challenges/femoco/tests/fixtures/exact'


# Skipping these on a developer machine without CUDD is fine. In CI it would mean the checker
# went untested, so there a missing CUDD is a failure.
@unittest.skipIf(cudd is None and not os.environ.get('CI'), 'install tools/verification/requirements.txt')
class ExactTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if cudd is None:
            raise AssertionError('dd was installed without CUDD; the checker tests cannot run')
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

    def test_each_quantum_obligation_is_the_one_that_rejects(self):
        """One mutant per comparison, built so that no other comparison can catch it. The
        expected message names the obligation, so dropping that check fails the test."""
        one_body = lambda u: ~(u[1] & ~u[0])
        cases = {
            # Same low-bit change in the network and its inverse: no pi, so no Pauli moves.
            'angle_low_bit': 'network/Pauli word differs',
            # A Z on an untouched qubit, on active branches only.
            'active_extra_z': 'network/Pauli word differs',
            # The pivot X also fires on active square branches; the phase is made consistent
            # with it, so only the comparison of X masks can tell.
            'x_mask': 'network/Pauli word differs',
            # Gates that act only when the control is off.
            'inactive_only_x': 'inactive system word',
            'inactive_only_z': 'inactive diagonal identity',
            'phase_low_bits': 'exact scalar phase',
            'spin_entry': 'selected spin on entry',
            'spin_exit': 'selected spin on exit',
            'spin_pair': 'inverse spin swaps',
        }
        for mutation, message in cases.items():
            with self.subTest(mutation=mutation):
                doc, spec, f, c, u, after, actual = self.fixture()
                first = actual.events[0]
                if mutation == 'angle_low_bit':
                    angle = first[4][3]
                    changed = (~angle[0],) + angle[1:]
                    first[1], first[4] = ('G', 0, 2, f.neg(changed)), ('G', 0, 2, changed)
                if mutation == 'active_extra_z': first.insert(4, ('Z', 3, c & one_body(u)))
                if mutation == 'x_mask':
                    active = c & ~(~one_body(u) & u[3])
                    before = self.m.normalize(f, first[1:-1], 4, 3, active)[2]
                    first[3] = ('X', 0, active)
                    after_phase = self.m.normalize(f, first[1:-1], 4, 3, active)[2]
                    self.assertNotEqual(active, c & one_body(u))
                    actual.phase = (f.zero, f.zero,
                                    f.xor(actual.phase[2], f.xor(before, after_phase)))
                if mutation == 'inactive_only_x': first.insert(4, ('X', 1, ~c))
                if mutation == 'inactive_only_z': first.insert(4, ('Z', 1, ~c))
                if mutation == 'phase_low_bits': actual.phase = (f.one, f.zero, actual.phase[2])
                spin = first[0][2]
                # Wrong only where the branch is active and the two swaps still agree.
                if mutation == 'spin_entry':
                    first[0], first[-1] = ('F', True, ~spin), ('F', False, ~spin)
                # Entry right; the exit differs from it only where the branch is active.
                if mutation == 'spin_exit': first[-1] = ('F', False, f.xor(spin, c & one_body(u)))
                # The two swaps differ only where the branch is inactive.
                if mutation == 'spin_pair': first[-1] = ('F', False, f.xor(spin, ~c))
                with self.assertRaisesRegex(self.m.Unproved, message):
                    self.m.quantum(doc, spec, f, c, u, after, actual)

    def controller(self, ops, num_bits=2, registers=()):
        doc, _, _, _, _, _, _ = self.fixture()
        doc['layout']['num_bits'] = num_bits
        doc['registers'] = [list(r) for r in registers]
        doc['ops'] = ops
        f, c, u, after, count = self.m.variables(doc, 0, 0)
        return self.m.Controller(doc, f, c, u, after, count), f, c, u, after

    def test_each_controller_obligation_is_the_one_that_rejects(self):
        nn = self.m.NONE
        x = lambda t: {'X': {'t': t, 'cond': nn}}
        cases = [
            ('control restoration', [x(0), 'Reflect']),
            ('uniform restoration', ['Reflect', x(5)]),
            ('final clean ancillas', ['Reflect', x(11)]),
            ('clean reset', [x(10), {'Reset': {'t': 10}}, x(10), 'Reflect']),
            ('inner register at reflection', [x(8), 'Reflect', x(8)]),
            ('unconditional reflection',
             [{'Hmr': {'t': 10, 'bit': 0, 'cond': nn}}, {'Push': {'bit': 0}}, 'Reflect', 'Pop']),
        ]
        for message, ops in cases:
            with self.subTest(obligation=message):
                actual = self.controller(ops)[0]
                with self.assertRaisesRegex((self.m.Unproved, ValueError), message):
                    actual.run()

    def test_lowered_ops_become_the_right_events_and_state(self):
        """The glue from lowered ops to gate events and Boolean state, op by op."""
        nn = self.m.NONE
        # Wires: 0 control, 1-4 system, 5-9 uniform (u0..u4), 10-11 ancillas.
        ops = [
            {'Sys': {'q': 2, 'z': True, 'ctrl': [5, nn], 'cond': nn}},
            {'Sys': {'q': 1, 'z': False, 'ctrl': [5, 6], 'cond': nn}},
            {'SpinSwap': {'c': 7, 'dagger': True, 'cond': nn}},
            {'Givens': {'p': 0, 'q': 2, 'reg': 0, 'cond': nn}},
            'Reflect',
        ]
        actual, f, c, u, after = self.controller(ops, registers=[[5, 6, 7]])
        actual.run()
        self.assertEqual(actual.events[0], [
            ('Z', 2, u[0]), ('X', 1, u[0] & u[1]), ('F', True, u[2]),
            ('G', 0, 2, (u[0], u[1], u[2]))])

        # Swap, conditional on a measured bit; Bit kinds; Neg; Push masks what follows.
        ops = [
            {'Hmr': {'t': 10, 'bit': 0, 'cond': nn}},
            {'Bit': {'bit': 1, 'kind': 2, 'cond': 0}},      # bit1 |= m0
            {'Bit': {'bit': 1, 'kind': 0, 'cond': nn}},     # bit1 ^= 1
            {'Phase': {'q': [0, nn, nn], 'k': 2, 'cond': nn}},
            {'Cx': {'c': 5, 't': 10, 'cond': nn}},
            {'Push': {'bit': 1}},
            {'Swap': {'a': 10, 'b': 11, 'cond': nn}},
            {'Neg': {'cond': nn}},
            'Pop',
        ]
        actual, f, c, u, after = self.controller(ops)
        with self.assertRaises((self.m.Unproved, ValueError)):
            actual.run()  # no Reflect: the run is incomplete, but the state is what we inspect
        m0 = f.bdd.var('m0')
        self.assertEqual(actual.bits[1], ~m0)
        self.assertEqual(actual.q[10], m0 & u[0])
        self.assertEqual(actual.q[11], ~m0 & u[0])
        self.assertEqual(actual.phase, (f.zero, c, ~m0))

    def test_classical_gate_on_a_system_wire_fails_closed(self):
        nn = self.m.NONE
        for op in ({'X': {'t': 2, 'cond': nn}}, {'Cx': {'c': 3, 't': 10, 'cond': nn}},
                   {'Phase': {'q': [0, 4, nn], 'k': 4, 'cond': nn}},
                   {'Sys': {'q': 0, 'z': True, 'ctrl': [1, nn], 'cond': nn}},
                   {'Sys': {'q': 4, 'z': True, 'ctrl': [nn, nn], 'cond': nn}},
                   {'SpinSwap': {'c': 2, 'dagger': True, 'cond': nn}}):
            with self.subTest(op=op):
                with self.assertRaisesRegex(ValueError, 'wire|qubit'):
                    self.controller([op, 'Reflect'])[0].run()

    def export(self, name):
        raw = (FIXTURES / f'{name}.json').read_bytes()
        doc = json.loads(raw)
        return doc, self.m.payload(FIXTURES / f'{name}.sa.bin', doc), hashlib.sha256(raw).hexdigest()

    def test_real_evaluator_exports_are_proved(self):
        """Circuits built by the challenge's builder and lowered by the evaluator. `wide` has
        three orbitals and a two-item square, so every reference-word branch runs."""
        for name in ('tiny', 'wide'):
            with self.subTest(circuit=name):
                doc, spec, digest = self.export(name)
                report = self.m.certify(doc, spec, digest, partition_bits=doc['outer']['k'])
                self.assertTrue(report['certified'], report['partitions'])
                self.assertEqual(len(report['partitions']), 1 << doc['outer']['k'])
                self.assertEqual(report['schema'], self.m.SCHEMA)

    def test_real_export_of_a_spin_keyed_sign_error_is_rejected(self):
        doc, spec, digest = self.export('tiny-spin-mutant')
        report = self.m.certify(doc, spec, digest, partition_bits=1)
        self.assertFalse(report['certified'])
        reasons = [p.get('reason', '') for p in report['partitions'] if p['status'] != 'proved']
        self.assertTrue(reasons and all('scalar phase' in r for r in reasons), reasons)

    def test_mutated_real_exports_are_rejected(self):
        """Edits to a real export that keep it well formed. Each changes the circuit's action,
        so each must leave at least one partition unproved."""
        doc, spec, digest = self.export('wide')
        index = lambda kind, nth=0: [i for i, o in enumerate(doc['ops'])
                                     if isinstance(o, dict) and kind in o][nth]

        def swap_low_register_bits(d):
            reg = d['registers'][d['ops'][index('Givens')]['Givens']['reg']]
            reg[0], reg[1] = reg[1], reg[0]

        def flip_pauli(d): d['ops'][index('Sys')]['Sys']['z'] ^= True
        def drop_pauli_control(d):
            i = next(i for i, o in enumerate(d['ops']) if isinstance(o, dict) and 'Sys' in o
                     and any(v != self.m.NONE for v in o['Sys']['ctrl']))
            d['ops'][i]['Sys']['ctrl'] = [self.m.NONE] * len(d['ops'][i]['Sys']['ctrl'])
        def drop_givens(d): del d['ops'][index('Givens')]
        def drop_spin_swap(d): del d['ops'][index('SpinSwap')]
        def drop_phase(d): del d['ops'][index('Phase')]
        def wrong_table(d): d['outer']['alt'][0] ^= 1

        for mutate in (swap_low_register_bits, flip_pauli, drop_pauli_control, drop_givens,
                       drop_spin_swap, drop_phase, wrong_table):
            with self.subTest(mutation=mutate.__name__):
                changed = json.loads(json.dumps(doc))
                mutate(changed)
                report = self.m.certify(changed, spec, digest, partition_bits=0)
                self.assertFalse(report['certified'], mutate.__name__)

    def test_a_partial_or_failed_run_never_certifies(self):
        doc, spec, digest = self.export('tiny')
        bits = doc['outer']['k']
        one = self.m.certify(doc, spec, digest, partition_bits=bits, partition=0)
        self.assertEqual([p['status'] for p in one['partitions']], ['proved'])
        self.assertFalse(one['certified'])
        with self.assertRaises(ValueError): self.m.certify(doc, spec, digest, partition_bits=bits + 1)
        with self.assertRaises(ValueError): self.m.certify(doc, spec, digest, partition_bits=bits, partition=1 << bits)
        tapered = dict(doc, rotation_widths=[3])
        with self.assertRaises(ValueError): self.m.certify(tapered, spec, digest)
        with self.assertRaises(ValueError): self.m.certify(dict(doc, schema='other'), spec, digest)

    def test_export_is_bound_to_the_circuit_files(self):
        doc, _, _ = self.export('tiny')
        with tempfile.TemporaryDirectory() as tmp:
            directory = pathlib.Path(tmp)
            for key, name in self.m.CIRCUIT_FILES.items():
                (directory / name).write_bytes(key.encode())
                doc['digests'][key] = hashlib.sha256(key.encode()).hexdigest()
            self.m.bind_circuit(doc, directory)
            (directory / 'ops.bin').write_bytes(b'another circuit')
            with self.assertRaisesRegex(ValueError, 'ops.bin'):
                self.m.bind_circuit(doc, directory)

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
