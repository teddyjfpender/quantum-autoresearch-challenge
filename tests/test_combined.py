"""Combined-walk checks: exact small-instance execution, one mutant per check of the
program checker, independently recomputed coefficient errors and costs, and reports."""
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
        # The execution reads each controlled stage from the body: a body that drops or
        # re-conditions one no longer reproduces the Hamiltonian.
        block = lambda program: fmpq(norm.numerator, norm.denominator)*c.execute_projected(program, zs) \
            + fmpq(offset.numerator, offset.denominator)*identity
        # (Reflecting on one-body rows as well is not among them: their second pass is the
        # identity, so the reflection acts trivially there. The layout check rejects it anyway.)
        for change in (lambda b: b.pop(4),                                   # no inner reflection
                       lambda b: b.pop(6),                                   # no row sign
                       lambda b: b[5][1].__setitem__(6, ['select_z', 'control']),  # second SELECT on every row
                       lambda b: b[3][1].__setitem__(6, ['select_z', 'control_and_square'])):
            q = copy.deepcopy(p)
            change(q['body'])
            self.assertNotEqual(block(q), target)
        # A sign bit flipped in an emitted lookup word is executed, not assumed.
        q = copy.deepcopy(p)
        q['outer_words'] = [w ^ (1 << (o['mu']+1+o['k']+1)) for w in q['outer_words']]
        self.assertNotEqual(block(q), target)

    def uneven(self):
        """Weights that no dyadic table represents exactly, so both encoding errors are non-zero."""
        nets = np.array([[0], [1], [2]], dtype=np.uint32)
        rows = [c.Row(False, 1, [F(1, 3)], [0]), c.Row(False, 1, [F(1, 7)], [1]),
                c.Row(True, -1, [F(1, 3), F(-1, 5), F(1, 11)], [0, 1, -1]),
                c.Row(True, 1, [F(2, 7), F(1, 13)], [2, -1])]
        p, o, i = c.compile_program(rows, nets, 3, 3)
        p['outer_table'], p['inner_tables'] = o, i
        return rows, nets, p, o, i

    def test_coefficient_errors_equal_an_independent_count_over_every_draw(self):
        rows, nets, p, o, i = self.uneven()
        proof = c.verify(p, rows, nets, o, i, reference_beta=3)
        ko, mu = o['k'], o['mu']
        norm = sum((r.mass() for r in rows), F(0))
        # Decode every (bucket, draw) of the emitted words; no table field is read.
        hits = [0]*len(rows)
        for bucket in range(1 << ko):
            for draw in range(1 << mu):
                hits[c.decode_word(p['outer_words'][bucket], draw, mu, ko+2) & ((1 << ko)-1)] += 1
        outer = sum((abs(norm*F(h, 1 << (ko+mu))-r.mass()) for h, r in zip(hits, rows)), F(0))
        self.assertGreater(outer, 0)
        self.assertEqual(F(proof['outer_error_upper_Ha']), outer)
        inner = F(0)
        width = max(1, len(nets).bit_length())+2
        for j, (r, table) in enumerate(zip(rows, i)):
            if not r.square:
                continue
            seen = {}
            for bucket in range(1 << table['k']):
                for draw in range(1 << table['mu']):
                    record = c.decode_word(p['inner_words'][j][bucket], draw, table['mu'], width)
                    seen[record >> 2] = seen.get(record >> 2, 0)+1
            draws = 1 << (table['k']+table['mu'])
            delta = sum((abs(F(seen.get(net+1, 0), draws)-abs(w)/r.size())
                         for net, w in zip(r.networks, r.weights)), F(0))
            inner += c.dyadic_upper(4*norm*F(hits[j], 1 << (ko+mu))*delta)
        self.assertGreater(inner, 0)
        self.assertEqual(F(proof['inner_error_upper_Ha']), inner)
        self.assertEqual(F(proof['coefficient_error_upper_Ha']), outer+inner)
        self.assertEqual(F(proof['chebyshev_offset_Ha']), -rows[2].mass()+rows[3].mass())
        self.assertEqual(proof['rows_in_scope'], {'one_body': 2, 'signed_squares': 2})
        self.assertIs(proof['independent_verification'], False)

    def test_each_program_check_is_the_one_that_rejects(self):
        """One mutant per check, with the message of the check that must catch it."""
        rows, nets, p, o, i = self.uneven()
        ko, mu = o['k'], o['mu']
        width = max(1, len(nets).bit_length())+2
        def outer_bit(program, bit):
            program['outer_words'][2] ^= 1 << (mu+1+bit)
        cases = [
            ('wrong emitted outer row/sign', lambda q, O, I: outer_bit(q, ko+1)),
            ('wrong emitted outer row/sign', lambda q, O, I: outer_bit(q, ko)),
            ('wrong emitted outer row/sign', lambda q, O, I: outer_bit(q, 0)),
            ('outer distribution mismatch', lambda q, O, I: O['counts'].__setitem__(0, O['counts'][0]+1)),
            ('inner distribution/record mismatch', lambda q, O, I: I[2]['counts'].__setitem__(0, I[2]['counts'][0]+1)),
            ('wrong emitted orbital/sign/identity', lambda q, O, I: q['inner_words'][2].__setitem__(0, q['inner_words'][2][0] ^ (1 << (I[2]['mu']+2)))),
            ('wrong emitted orbital/sign/identity', lambda q, O, I: q['inner_words'][2].__setitem__(0, q['inner_words'][2][0] ^ (1 << (I[2]['mu']+3)))),
            ('wrong emitted orbital/sign/identity', lambda q, O, I: q['inner_words'][2].__setitem__(0, q['inner_words'][2][0] ^ (1 << (I[2]['mu']+1)))),
            ('bad outer keep/word width', lambda q, O, I: q['outer_words'].__setitem__(0, q['outer_words'][0] | 1 << (mu+1+2*(ko+2)))),
            ('bad inner keep/width', lambda q, O, I: q['inner_words'][2].__setitem__(0, q['inner_words'][2][0] | 1 << (I[2]['mu']+1+2*width))),
            ('incorrect outer combination or walk reflection', lambda q, O, I: q['body'].__setitem__(9, ['erase', 'inner_alias'])),
            ('incorrect outer combination or walk reflection', lambda q, O, I: q['body'].__setitem__(8, ['compare_inverse', 'outer_draw', 'alt'])),
            ('incorrect outer combination or walk reflection', lambda q, O, I: q['body'].__setitem__(10, ['reflect', 'all_uniform', 'control_and_square'])),
            ('incorrect outer combination or walk reflection', lambda q, O, I: q['body'].__setitem__(7, ['fredkin', 'outer_record'])),
            ('incorrect signed-square reflection', lambda q, O, I: q['body'].__setitem__(4, ['reflect', 'inner_uniform', 'control'])),
            ('incorrect first/second pass', lambda q, O, I: q['body'][5][1].__setitem__(6, ['select_z', 'control'])),
            ('rotation outside declared width', None),
            ('network digest', None),
        ]
        for message, mutate in cases:
            with self.subTest(check=message):
                q, O, I, n = copy.deepcopy(p), copy.deepcopy(o), copy.deepcopy(i), nets.copy()
                if mutate is None and message.startswith('rotation'):
                    n[0, 0] = 8
                    q['network_sha256'] = hashlib.sha256(n.astype('<u4').tobytes()).hexdigest()
                elif mutate is None:
                    n[0, 0] = 2
                else:
                    mutate(q, O, I)
                with self.assertRaisesRegex(ValueError, message):
                    c.verify(q, rows, n, O, I, reference_beta=3)

    def test_assemble_reads_signs_networks_and_precision_from_the_inputs(self):
        base = {'n': 2, 'r': 1, 'b': 1, 'c': 2, 'beta': 3, 'const': 0.5,
                'e': np.array([0.25, 0.0]), 'ea': np.array([[1], [2]], dtype=np.uint32),
                'wb': np.array([0.5, -0.25]), 'w': np.array([[0.25], [0.5]]),
                'angles': np.array([[3]], dtype=np.uint32)}
        correction = {'signs': np.array([1, -1], dtype=np.int8),
                      'eigenvalues': np.array([[0.5, -0.25], [0.125, 0.25]]),
                      'angles': np.array([[[5], [6]], [[7], [9]]], dtype=np.uint32)}
        rows, nets, beta = c.assemble(base, correction)
        self.assertEqual(beta, 26)
        # Base networks are padded to the correction's precision; correction words are kept.
        self.assertEqual(nets[:3].ravel().tolist(), [1 << 23, 2 << 23, 3 << 23])
        self.assertEqual(nets[3:].ravel().tolist(), [5, 6, 7, 9])
        self.assertEqual([(r.square, r.sign, r.networks) for r in rows], [
            (False, 1, [0]),                       # the zero one-body eigenvalue is dropped
            (True, 1, [2, -1]), (True, 1, [2, -1]),  # both copies share the square's network
            (True, -1, [3, 4]), (True, 1, [5, 6])])  # the correction subtracts the residual
        self.assertEqual(rows[1].weights, [F(1, 4), F(1, 2)])
        self.assertEqual(rows[3].weights, [F(1, 2), F(-1, 4)])
        # The offset the certificate implies is computed without the rows' signs.
        size = [F(3, 4), F(3, 8)]
        cert = {'correction_rotations': {'correction_chebyshev_offset_Ha': str(-size[0]**2/4+size[1]**2/4)}}
        offset = sum((r.sign*r.mass() for r in rows if r.square), F(0))
        self.assertEqual(c.expected_offset(base, cert), offset)
        rows[3].sign = 1
        self.assertNotEqual(c.expected_offset(base, cert), sum((r.sign*r.mass() for r in rows if r.square), F(0)))

    def test_resource_bounds_equal_a_hand_expansion(self):
        rows, nets, p, o, i = self.uneven()
        n = 4
        r = c.compile_resources(p, rows, nets, n, o, i)
        beta, mu, ko, ki = p['beta'], o['mu'], o['k'], i[0]['k']
        netbits = max(1, len(nets).bit_length())
        # Two passes of G-dagger Z G: four networks of n-1 rotations at 2*beta Toffolis each.
        self.assertEqual(r['givens_count'], 4*(n-1))
        self.assertEqual(r['rotation_toffoli'], 4*(n-1)*2*beta)
        fixed = (4*(n-1)*2*beta + 4*(n+1) + 3*8*(mu+1) + 2*(ko+2) + 4*(netbits+2)
                 + 2*(p['inner_bits']+1) + 2*(p['outer_bits']+p['inner_bits']+1) + 32)
        self.assertEqual(r['non_lookup_toffoli'], fixed)
        tables = [(1 << ko, mu+1+2*(ko+2), 1), (len(rows) << ki, mu+1+2*(netbits+2), 2),
                  (len(nets)+1, (n-1)*beta, 2)]
        self.assertEqual([(t['entries'], t['word_bits'], t['calls_load_and_erase']) for t in r['table_dimensions']], tables)

        def pair(length, width, blocks):
            erase = min(2*(1 << h)+-(-length//(1 << h)) for h in range((length-1).bit_length()+1))
            return -(-length//blocks)+(blocks-1)*width+2*erase
        best = [min(mult*pair(l, w, 1 << a) for a in range(13)) for l, w, mult in tables]
        self.assertEqual(r['min_toffoli']['lookup_toffoli_by_stage'], best)
        self.assertEqual(r['min_toffoli']['controlled_walk_toffoli_upper'], fixed+sum(best))
        for point in r['frontier']:
            self.assertEqual(point['TQ_upper'], point['controlled_walk_toffoli_upper']*point['logical_qubits_upper_including_system'])
        self.assertEqual(r['min_TQ']['TQ_upper'], min(point['TQ_upper'] for point in r['frontier']))

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
            self.assertTrue(proof['checked'])
            self.assertFalse(proof['independent_verification'])
            self.assertFalse(proof['fully_lowered_gate_certificate'])
            self.assertEqual(report['targets_sha256'], inputs.sha256(inputs.ROOT/'targets.json'))
            self.assertEqual(report['program_sha256'], report['artifacts'][f'{name}-combined-program.json']['sha256'])
            self.assertEqual(cert['base_payload_sha256'], report['artifacts'][f'{name}-base.bin']['sha256'])
            self.assertEqual(cert['signed_data_sha256'], report['artifacts'][f'{name}-signed.npz']['sha256'])
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
                    # The comparison is on the challenge's score too, and never favourable.
                    target = next(t for t in json.loads((inputs.ROOT/'targets.json').read_text())['targets'] if t['track'] == name)
                    step = q['step_toffoli_x_qubits']
                    self.assertEqual(step['ours_upper'], resources['min_TQ']['TQ_upper'])
                    self.assertEqual(step['low_published'], target['toffoliPublished']*target['qubitsPublished'])
                    self.assertEqual(q['low_logical_qubits_published'], target['qubitsPublished'])
                    self.assertEqual(q['our_logical_qubits_upper'], resources['min_toffoli']['logical_qubits_upper_including_system'])
                    self.assertGreater(step['ratio_display'], 1)
                    self.assertGreater(q['total_cost_ratio_display'], 1)
                    self.assertGreater(q['total_toffoli_x_qubits_ratio_display'], step['ratio_display'])
                else:
                    self.assertEqual(q['controlled_walk_total_toffoli_upper'], cost*q['controlled_walk_uses'])


if __name__ == '__main__':
    unittest.main()
