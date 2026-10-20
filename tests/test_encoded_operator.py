"""Independent artifact binding, full-domain rejection, and coefficient regressions."""
import copy
import csv
import hashlib
import itertools
import json
import pathlib
import random
import struct
import sys
import tempfile
import unittest
from fractions import Fraction as Q

ROOT = pathlib.Path(__file__).resolve().parents[1]
VERIFY = ROOT/'challenges/femoco/tools/verification'
FIXTURES = ROOT/'challenges/femoco/tests/fixtures/exact'
sys.path.insert(0, str(VERIFY))
import artifacts
sys.path.insert(0, str(ROOT/'challenges/femoco/tools/repro'))
import audit_encoded

try:
    from dd import cudd
except ImportError:
    cudd = None


def fixture(directory, name):
    for suffix, target in [('ops.bin', 'ops.bin'), ('lanemap.bin', 'lanemap.bin'),
                           ('family.json', 'family.out.json')]:
        (directory/target).write_bytes((FIXTURES/f'{name}.{suffix}').read_bytes())
    raw = (FIXTURES/f'{name}.json').read_bytes()
    return json.loads(raw), raw


class BinaryTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # The independent binary tests can still run without the BDD dependencies.
        cls.spec = dict(n=3, r=1, b=2, c=1)

    def test_all_alias_draws_are_counted_exactly(self):
        for keep in itertools.product(range(4), repeat=4):
            table = dict(k=2, mu=2, keep=list(keep), alt=[3, 0, 1, 2])
            brute = [0]*4
            for bucket, sigma in itertools.product(range(4), repeat=2):
                brute[bucket if sigma < keep[bucket] else table['alt'][bucket]] += 1
            self.assertEqual(artifacts.counts(table, 4), brute)

    def raw_fixture(self):
        nn = artifacts.NONE
        doc = dict(layout=dict(system=4, uniform=5, num_qubits=11, num_bits=1),
                   inner=dict(lo=3, width=2), registers=[[8, 9]])
        ops = [dict(Ccx=dict(a=0, b=5, t=10, cond=nn)),
               dict(Hmr=dict(t=10, bit=0, cond=nn)),
               dict(Phase=dict(q=[0, 5, nn], k=4, cond=0))]
        doc['ops'] = ops+['Reflect']+copy.deepcopy(ops)
        def op(kind, c2=nn, c1=nn, t=nn, bit=nn, cond=nn, reg=nn):
            return artifacts.RECORD.pack(kind, 0, *(artifacts.DISK_NONE if x == nn else x
                                                   for x in (c2, c1, t, bit, cond, reg)))
        block = [op(13, c2=5, c1=0, t=10), op(12, t=10, bit=0), op(9, c1=5, t=0, cond=0)]
        records = [op(1, reg=0), op(2, t=8, reg=0), op(2, t=9, reg=0), op(40, reg=3)]
        records += block+[op(40, reg=4), op(31, reg=0), op(40, reg=3)]
        records += block+[op(40, reg=4)]
        data = b'FEMOOPS1'+struct.pack('<Q', len(records))+b''.join(records)
        return doc, data

    def test_binary_lowering_checks_every_op_and_the_reflection(self):
        doc, data = self.raw_fixture()
        self.assertEqual(artifacts.bind_program(data, doc)['status'], 'proved')
        for mutation in ('phase', 'condition', 'missing', 'register', 'layout', 'reflection',
                         'raw_copy', 'padding', 'length', 'shape'):
            d, raw = copy.deepcopy(doc), bytearray(data)
            with self.subTest(mutation=mutation):
                if mutation == 'phase': d['ops'][2]['Phase']['k'] = 2
                if mutation == 'condition': d['ops'][0]['Ccx']['cond'] = 0
                if mutation == 'missing': d['ops'].pop()
                if mutation == 'register': d['registers'][0].reverse()
                if mutation == 'layout': d['layout']['num_qubits'] += 1
                if mutation == 'reflection':
                    # A forged export with a changed raw reflection register must fail
                    # even though both still lower to the same string "Reflect".
                    for rec, t in [(1, 7), (2, 9)]:
                        struct.pack_into('<Q', raw, 16+56*rec+24, t)
                    d['registers'] = [[7, 9]]
                if mutation == 'raw_copy':
                    # DebugPrint lowers away: only raw-copy equality can reject this.
                    # Replace each block's CCX by DebugPrint, but change one ignored field.
                    for rec in (4, 10):
                        struct.pack_into('<I', raw, 16+56*rec, 17)
                    struct.pack_into('<Q', raw, 16+56*10+24, 9)
                    d['ops'] = [d['ops'][1], d['ops'][2], 'Reflect', d['ops'][4], d['ops'][5]]
                if mutation == 'padding': raw[16+4] = 1
                if mutation == 'length': raw.append(0)
                if mutation == 'shape': struct.pack_into('<Q', raw, 16+8, 0)
                with self.assertRaises(ValueError): artifacts.bind_program(raw, d)

    def test_packed_program_preserves_every_supported_op(self):
        # Round-trip real exports and require identical independent binary binding.
        for name in ('tiny', 'wide', 'wide-normalized', 'tiny-spin-mutant'):
            with self.subTest(name=name):
                doc = json.loads((FIXTURES/f'{name}.json').read_bytes())
                packed = artifacts.PackedOps(doc['ops'])
                self.assertEqual(list(packed), doc['ops'])
                self.assertEqual([packed[i] for i in range(len(packed))], doc['ops'])
                doc['ops'] = packed
                artifacts.bind_program((FIXTURES/f'{name}.ops.bin').read_bytes(), doc)

    def test_lane_map_parser_rejects_malformed_tables_and_trailing_bytes(self):
        # Use the real fixture; the payload parser is intentionally not needed here.
        raw = (FIXTURES/'wide.lanemap.bin').read_bytes()
        m = artifacts.read_map(raw, self.spec)
        self.assertEqual(len(m['inner_tables']), 4)
        for data in (raw[:-1], raw+b'\0', b'broken'+raw[6:]):
            with self.assertRaises(ValueError): artifacts.read_map(data, self.spec)

    def test_reproduction_selects_current_product_leaders_and_rejects_wrong_evidence(self):
        with (ROOT/'challenges/femoco/results.tsv').open() as file:
            rows = list(csv.DictReader(file, delimiter='\t'))
        chosen = audit_encoded.selection(rows)
        self.assertEqual([r['submission'] for r in chosen],
                         ['reiher/outer-hi-reconstruct-r1', 'li/composed-alias-m8'])
        with self.assertRaises(ValueError): audit_encoded.selection(rows, ['missing'])
        with self.assertRaises(ValueError): audit_encoded.selection(rows, [chosen[0]['submission']]*2)
        row = chosen[0]
        report = dict(encoded_operator_equivalence_certified=True, certifies_energy_error=False,
                      chemical_accuracy_certified=False,
                      partitions=[dict(partition=0, status='proved')], partition_bits=0,
                      binary_binding=dict(status='proved'),
                      checker_sha256={name: hashlib.sha256((VERIFY/name).read_bytes()).hexdigest()
                                      for name in ('encoded_operator.py', 'artifacts.py', 'exact.py', 'outcomes.py')},
                      digests={key: row[key+'_sha256'] for key in ('ops', 'lanemap', 'family')},
                      coefficients=dict(off_diagonal_coefficient_l1_Ha=dict(exact='1/8')))
        audit_encoded.check_report(report, row, dict(rounding_error_exact='1/8'))
        for mutation in ('partition', 'energy', 'circuit', 'coefficient', 'missing_partition', 'stale_checker'):
            d = copy.deepcopy(report)
            if mutation == 'partition': d['encoded_operator_equivalence_certified'] = False
            if mutation == 'energy': d['certifies_energy_error'] = True
            if mutation == 'circuit': d['digests']['ops'] = '0'*64
            if mutation == 'coefficient': d['coefficients']['off_diagonal_coefficient_l1_Ha']['exact'] = '1/7'
            if mutation == 'missing_partition': d['partitions'] = []
            if mutation == 'stale_checker': d['checker_sha256']['exact.py'] = '0'*64
            with self.assertRaises(ValueError):
                audit_encoded.check_report(d, row, dict(rounding_error_exact='1/8'))


@unittest.skipIf(cudd is None, 'install tools/verification/requirements.txt')
class OperatorTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import encoded_operator
        cls.m = encoded_operator

    def certify(self, name, **kw):
        with tempfile.TemporaryDirectory() as temp:
            d = pathlib.Path(temp)
            doc, raw = fixture(d, name)
            doc['ops'] = artifacts.PackedOps(doc['ops'])
            payload = FIXTURES/f'{name}.sa.bin'
            spec = self.m.exact.payload(payload, doc)
            return self.m.certify(doc, spec, d, payload.read_bytes(), hashlib.sha256(raw).hexdigest(),
                                  partition_bits=doc['outer']['k'], **kw)

    def test_real_binary_circuits_have_full_domain_equivalence(self):
        for name in ('tiny', 'wide', 'wide-normalized'):
            with self.subTest(name=name):
                report = self.certify(name, **({'network_form': 'sign-normalized'}
                                               if name == 'wide-normalized' else {}))
                self.assertTrue(report['encoded_operator_equivalence_certified'], report['partitions'])
                self.assertIs(report['certifies_energy_error'], False)
                self.assertIs(report['chemical_accuracy_certified'], False)
                self.assertEqual(report['coefficients']['status'], 'proved')
                if name == 'wide-normalized':
                    self.assertTrue(all(p['network_reference'] == 'sign-normalized'
                                        for p in report['partitions']))

    def test_real_spin_keyed_mutant_is_not_equivalent(self):
        report = self.certify('tiny-spin-mutant')
        self.assertFalse(report['encoded_operator_equivalence_certified'])
        self.assertTrue(any('scalar phase' in p.get('reason', '') for p in report['partitions']))

    def test_rare_joint_measurement_branch_is_rejected_without_sampling(self):
        report = self.certify('tiny-rare-outcome-mutant')
        self.assertEqual(report['binary_binding']['status'], 'proved')
        self.assertFalse(report['encoded_operator_equivalence_certified'])
        failures = [p for p in report['partitions'] if p['status'] != 'proved']
        self.assertTrue(failures)
        self.assertTrue(all('scalar phase' in p['reason'] for p in failures))

    def test_passing_diagnostic_partition_never_certifies(self):
        report = self.certify('wide', partition=0)
        self.assertEqual(report['partitions'][0]['status'], 'proved')
        self.assertFalse(report['encoded_operator_equivalence_certified'])

    def test_export_forgery_cannot_hide_behind_matching_file_hashes(self):
        for mutant in ('ops', 'alias', 'registers', 'family_spec'):
            with self.subTest(mutant=mutant), tempfile.TemporaryDirectory() as temp:
                directory = pathlib.Path(temp)
                doc, _ = fixture(directory, 'wide')
                spec = self.m.exact.payload(FIXTURES/'wide.sa.bin', doc)
                if mutant == 'ops':
                    op = next(o['Phase'] for o in doc['ops'] if isinstance(o, dict) and 'Phase' in o)
                    op['k'] ^= 4
                if mutant == 'alias': doc['outer']['keep'][0] ^= 1
                if mutant == 'registers': doc['registers'][0].reverse()
                if mutant == 'family_spec': doc['spec'] = 'different-spec'
                with self.assertRaises(ValueError): artifacts.bind(directory, doc, spec)

    def test_coefficient_magnitudes_and_identity_shift_are_not_ignored(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = pathlib.Path(temp)
            doc, _ = fixture(directory, 'tiny')
            data = (FIXTURES/'tiny.sa.bin').read_bytes()
            spec = self.m.exact.payload(FIXTURES/'tiny.sa.bin', doc)
            _, lane_map = artifacts.bind(directory, doc, spec)
            audit = artifacts.coefficient_audit(spec, lane_map, data)
            self.assertEqual(Q(audit['encoded_to_payload_norm_upper_Ha']['exact']), 0)
            for mutate in ('magnitude', 'normalization', 'counts'):
                s, m = copy.deepcopy(spec), copy.deepcopy(lane_map)
                if mutate == 'magnitude': s['e'] = (4.0, 2.0)
                if mutate == 'normalization': m['lambda_decl'] += Q(1, 8)
                if mutate == 'counts': m['outer']['alt'][0] = 1
                result = artifacts.coefficient_audit(s, m, data)
                self.assertGreater(Q(result['encoded_to_payload_norm_upper_Ha']['exact']), 0)
                self.assertIs(result['certifies_energy_error'], False)

    def test_long_sign_normalization_angle_transform(self):
        # The independent Fock oracle checks the operator identity and partial-flip
        # mutant; here exhaust the angle-reference generation itself.
        for a, b in itertools.product(range(8), repeat=2):
            s = dict(n=3, beta=3, ea=(a, b), angles=(a, b))
            got = self.m.sign_normalized(s)['angles']
            self.assertEqual(got, ((4-a) % 8, (b+4) % 8) if b & 4 else (a, b))

    def test_outcome_polynomials_against_boolean_and_mod_eight_oracles(self):
        raw = self.m.exact.Boolean(['u0', 'u1', 'm0', 'm1', 'm2', 'm3'])
        f = self.m.OutcomeBoolean(raw)
        names = list(raw.bdd.vars)
        variables = [(f.bdd.var(v), raw.bdd.var(v)) for v in names]
        rng = random.Random(7023)
        def value(node, assignment):
            lane = {k: v for k, v in assignment.items() if not k.startswith('m')}
            answer = 0
            for monomial, coefficient in node.terms.items():
                answer ^= int(raw.bdd.let(lane, coefficient) == raw.one)*int(all(assignment[v] for v in monomial))
            return answer
        for _ in range(40):
            phase, original, increments = f.word(0, 3), raw.word(0, 3), []
            for _ in range(25):
                (a, aa), (b, bb), (c, cc) = (rng.choice(variables) for _ in range(3))
                mask, reference = (a & b) | ~c, (aa & bb) | ~cc
                k = rng.choice(range(8))
                increments.append((mask, reference, k))
                phase = f.add(phase, tuple(mask if k >> j & 1 else f.zero for j in range(3)))
                original = raw.add(original, tuple(reference if k >> j & 1 else raw.zero for j in range(3)))
            for values in itertools.product((False, True), repeat=len(names)):
                assignment = dict(zip(names, values))
                for mask, reference, _ in increments:
                    self.assertEqual(value(mask, assignment), int(raw.bdd.let(assignment, reference) == raw.one))
                actual = sum(value(bit, assignment) << j for j, bit in enumerate(phase))
                expected = sum(k*value(mask, assignment) for mask, _, k in increments) % 8
                self.assertEqual(actual, expected)
                self.assertEqual(actual, sum(int(raw.bdd.let(assignment, bit) == raw.one) << j for j, bit in enumerate(original)))
            wrong = f.xor(phase[2], f.one)
            self.assertNotEqual(phase[2], wrong)
        # A returned polynomial witness really satisfies the represented function.
        node = (variables[0][0] & variables[2][0]) | (variables[1][0] & variables[3][0])
        witness = f.bdd.pick(node)
        complete = {v: witness.get(v, False) for v in names}
        self.assertEqual(value(node, complete), 1)
        limited = self.m.OutcomeBoolean(raw, term_limit=1)
        with self.assertRaisesRegex(ValueError, 'budget'):
            limited.bdd.var('m0') | limited.bdd.var('m1')
        with self.assertRaisesRegex(ValueError, 'lane coefficient'):
            f.lane(raw.bdd.var('m0'))


if __name__ == '__main__':
    unittest.main()
