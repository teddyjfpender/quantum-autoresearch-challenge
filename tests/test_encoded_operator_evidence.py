"""The committed coefficient audit must remain bound to its inputs and implementation."""
import csv
import hashlib
import json
import pathlib
import unittest
from fractions import Fraction as Q

FEMOCO = pathlib.Path(__file__).resolve().parents[1]/'challenges/femoco'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class CoefficientEvidenceTest(unittest.TestCase):
    def test_coefficient_evidence_is_current_and_keeps_energy_claims_false(self):
        report = json.loads((FEMOCO/'rigorous/operator-coefficients.json').read_text())
        self.assertIs(report['authenticated_ledger_result'], False)
        self.assertIs(report['certifies_energy_error'], False)
        self.assertIs(report['chemical_accuracy_certified'], False)
        for name, digest in report['coefficient_source_sha256'].items():
            self.assertEqual(digest, sha(FEMOCO/'tools/verification'/name))
        self.assertEqual(set(report['coefficient_source_sha256']), {'artifacts.py', 'exact.py'})
        with (FEMOCO/'results.tsv').open() as file:
            ledger = {r['submission']: r for r in csv.DictReader(file, delimiter='\t')}
        self.assertEqual({r['submission'] for r in report['results']},
                         {'reiher/outer-hi-reconstruct-r1', 'li/composed-alias-m8'})
        for row in report['results']:
            with self.subTest(submission=row['submission']):
                recorded = ledger[row['submission']]
                self.assertEqual(row['spec'], recorded['spec'])
                for key in ('ops', 'lanemap', 'family'):
                    self.assertEqual(row['circuit_digests'][key], recorded[key+'_sha256'])
                spec_dir = FEMOCO/'specs'/row['spec']
                spec = json.loads((spec_dir/'spec.json').read_text())
                self.assertEqual(row['payload_sha256'], sha(spec_dir/'sa.bin'))
                self.assertEqual(row['source_manifest_sha256'], sha(FEMOCO/'submissions'/row['submission']/'submission.json'))
                coefficients = row['coefficients']
                self.assertEqual(coefficients['status'], 'proved')
                self.assertEqual(coefficients['ordered_off_diagonal_terms'], spec['terms'])
                self.assertEqual(Q(coefficients['lambda_payload']['exact']), Q(spec['lambda']['exact']))
                self.assertEqual(Q(coefficients['payload_identity']['exact']), Q(spec['identity']['exact']))
                self.assertEqual(Q(coefficients['E_SOS']['exact']), Q(spec['E_SOS']['exact']))
                l1 = Q(coefficients['off_diagonal_coefficient_l1_Ha']['exact'])
                shift = Q(coefficients['identity_shift_Ha']['exact'])
                self.assertEqual(Q(coefficients['encoded_to_payload_norm_upper_Ha']['exact']), l1+abs(shift))
                self.assertEqual(coefficients['rigorous_0p1_mHa_coefficient_rule_met'], l1 <= Q(1, 10000))
                self.assertIs(coefficients['certifies_energy_error'], False)
                screen = row['sampled_screen']
                self.assertEqual((screen['engine'], screen['samples'], screen['passed']), ('sliced', 4096, True))
                self.assertIs(screen['streamed_export_byte_identity_checked'], True)
                chemistry = row['chemistry_evidence']
                path = FEMOCO/'rigorous'/chemistry['report']
                self.assertEqual(chemistry['sha256'], sha(path))
                previous = json.loads(path.read_text())
                self.assertIs(chemistry['original_integrals_recomputed_in_this_audit'], False)
                self.assertIs(chemistry['ground_energy_accuracy_decided'], False)
                self.assertEqual(chemistry['factorization_operator_norm_upper_Ha'], previous['factorization']['operator_norm_upper_Ha'])
                self.assertEqual(chemistry['rotation_and_preprocessing_upper_Ha'], previous['rotation_and_preprocessing']['upper_Ha'])


if __name__ == '__main__':
    unittest.main()
