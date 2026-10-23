#!/usr/bin/env python3
"""Assemble complete, digest-matched partition evidence; never fill gaps by assumption."""
import argparse
import copy
import hashlib
import json
import pathlib

from artifacts import require

HERE = pathlib.Path(__file__).resolve().parent
COMMON = ('schema', 'spec', 'digests', 'input_sha256', 'checker_sha256', 'backend',
          'boolean_engine', 'claim', 'input_domain', 'binary_binding', 'coefficients',
          'partition_bits', 'partition_domain', 'unresolved_obligations')


def assemble(reports):
    require(bool(reports), 'no partition evidence')
    result = copy.deepcopy(reports[0])
    require(all(k in result for k in COMMON), 'missing report metadata')
    require(result['schema'] == 'femoco-encoded-operator-report-v1', 'unsupported report schema')
    require(type(result['partition_bits']) is int and 0 <= result['partition_bits'] <= 26,
            'invalid partition domain')
    for name, digest in result['checker_sha256'].items():
        require(name in ('encoded_operator.py', 'artifacts.py', 'exact.py', 'outcomes.py'), 'unknown checker dependency')
        require(hashlib.sha256((HERE/name).read_bytes()).hexdigest() == digest, 'stale checker dependency')
    require(set(result['checker_sha256']) == {'encoded_operator.py', 'artifacts.py', 'exact.py', 'outcomes.py'},
            'missing checker dependency')
    require(result['binary_binding']['status'] == 'proved', 'unproved binary binding')
    require(result['coefficients']['status'] == 'proved', 'unproved coefficient evidence')
    partitions = {}
    for report in reports:
        require(all(k in report and report[k] == result[k] for k in COMMON), 'partition evidence metadata differs')
        require(report.get('certifies_energy_error') is False and report.get('chemical_accuracy_certified') is False,
                'operator evidence cannot certify energy or chemistry accuracy')
        require(report.get('encoded_operator_equivalence_certified') is False, 'requires unassembled partition reports')
        require([p['partition'] for p in report['partitions']] == report['requested_partitions'],
                'incomplete requested batch')
        for part in report['partitions']:
            index = part['partition']
            require(type(index) is int and index not in partitions, 'duplicate or invalid partition')
            require(part['status'] == 'proved', 'unproved partition')
            require(part['network_reference'] in ('payload', 'sign-normalized'), 'unknown reference form')
            partitions[index] = part
    total = 1 << result['partition_bits']
    require(sorted(partitions) == list(range(total)), 'partition coverage is incomplete')
    result['partitions'] = [partitions[i] for i in range(total)]
    result['requested_partitions'] = list(range(total))
    result['encoded_operator_equivalence_certified'] = True
    result['wall_seconds'] = None  # parallel batches do not have an additive elapsed time
    result['assembly'] = dict(checker_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
                              input_reports=len(reports),
                              reported_batch_seconds_sum=sum(r.get('wall_seconds') or 0 for r in reports))
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    args = p.parse_args()
    grouped = {}
    for path in sorted(args.root.rglob('operator-report.json')):
        report = json.loads(path.read_bytes())
        grouped.setdefault(report['spec'], []).append(report)
    require(set(grouped) == {'reiher-sa-est-v1', 'li-sa-est-v1'}, 'requires both headline instances')
    args.out.mkdir(parents=True, exist_ok=True)
    for name in ('reiher', 'li'):
        (args.out/f'operator-{name}.json').unlink(missing_ok=True)
    results = {name: assemble(grouped[f'{name}-sa-est-v1']) for name in ('reiher', 'li')}
    for name, report in results.items():
        (args.out/f'operator-{name}.json').write_text(json.dumps(report, indent=2)+'\n')
        print(name, 'whole-domain encoded-operator equivalence certified', flush=True)


if __name__ == '__main__':
    main()
