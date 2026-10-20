#!/usr/bin/env python3
"""Rebuild ledger-selected circuits and generate local binary-bound operator evidence.

Does not submit, authenticate or edit any ledger row. Uses existing build manifests;
the evaluator is built without contestant code before building any circuit builder.
"""
from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
from decimal import Decimal
from fractions import Fraction

ROOT = pathlib.Path(__file__).resolve().parents[2]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def selection(rows, submissions=()):
    valid = [r for r in rows if r['status'] == 'OK' and r['track'] in ('reiher', 'li')]
    if submissions:
        selected = [r for s in submissions for r in valid if r['submission'] == s]
        if len(selected) != len(submissions) or len(set(submissions)) != len(submissions):
            raise ValueError('select distinct, recorded estimated-track submissions')
        return selected
    return [min((r for r in valid if r['track'] == track),
                key=lambda r: Decimal(r['toffoli'])*Decimal(r['qubits'])) for track in ('reiher', 'li')]


def record_circuit(directory, row):
    for key, name in (('ops_sha256', 'ops.bin'), ('lanemap_sha256', 'lanemap.bin'),
                      ('family_sha256', 'family.out.json')):
        if sha(directory/name) != row[key]:
            raise ValueError(f'{row["submission"]}: rebuilt {name} differs from ledger')


def check_report(report, row, metrics):
    if report.get('encoded_operator_equivalence_certified') is not True:
        raise ValueError('whole-domain equivalence was not established')
    if report.get('certifies_energy_error') is not False or report.get('chemical_accuracy_certified') is not False:
        raise ValueError('operator audit must not certify chemistry or energy accuracy')
    parts = report['partitions']
    if ([p['partition'] for p in parts] != list(range(1 << report['partition_bits']))
            or any(p['status'] != 'proved' for p in parts)
            or report['binary_binding']['status'] != 'proved'):
        raise ValueError('incomplete partition or binary-binding evidence')
    for name, digest in report['checker_sha256'].items():
        if name not in ('encoded_operator.py', 'artifacts.py', 'exact.py', 'outcomes.py') or sha(ROOT/'tools/verification'/name) != digest:
            raise ValueError('operator evidence is from a different checker')
    if set(report['checker_sha256']) != {'encoded_operator.py', 'artifacts.py', 'exact.py', 'outcomes.py'}:
        raise ValueError('missing checker dependency hash')
    for key in ('ops', 'lanemap', 'family'):
        if report['digests'][key] != row[key+'_sha256']:
            raise ValueError('operator report is for a different circuit')
    if Fraction(report['coefficients']['off_diagonal_coefficient_l1_Ha']['exact']) != Fraction(metrics['rounding_error_exact']):
        raise ValueError('independent coefficient error differs from evaluator')


def run(args):
    out = args.out_root.resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith('FEMOCO_')}
    env.setdefault('RAYON_NUM_THREADS', str(min(args.jobs, 4)))
    def command(argv, cwd=ROOT, name=None, local_env=env):
        if name is None:
            subprocess.run(argv, cwd=cwd, env=local_env, check=True)
        else:
            with name.open('w') as log:
                subprocess.run(argv, cwd=cwd, env=local_env, stdout=log,
                               stderr=subprocess.STDOUT, check=True)
    def git(*argv):
        return subprocess.check_output(['git', *argv], cwd=ROOT, text=True).strip()
    rows = selection(list(csv.DictReader((ROOT/'results.tsv').open(), delimiter='\t')), args.submission)
    summary = dict(schema='femoco-operator-audit-run-v1', authenticated_ledger_result=False,
                   source_commit=git('rev-parse', 'HEAD'), source_clean=not git('status', '--porcelain'),
                   runner_sha256=sha(pathlib.Path(__file__)), ledger_sha256=sha(ROOT/'results.tsv'),
                   python=sys.version.split()[0], samples=args.samples, results=[])
    (out/'audit-results.json').write_text(json.dumps(summary, indent=2)+'\n')
    command([args.cargo, 'build', '--release', '--locked', '--no-default-features', '--bin', 'eval_circuit'])
    evaluator = out/'eval_circuit'
    shutil.copy2(ROOT/'target/release/eval_circuit', evaluator)
    summary['evaluator_sha256'] = sha(evaluator)
    for row in rows:
        manifest = ROOT/'submissions'/row['submission']/'submission.json'
        document = json.loads(manifest.read_text())
        build_env = dict(env, FEMOCO_WALK_SPEC=row['spec'], **document['build'])
        directory = out/row['submission']
        directory.mkdir(parents=True, exist_ok=True)
        result = dict(submission=row['submission'], ledger_row=row,
                      source_manifest_sha256=sha(manifest), status='incomplete')
        summary['results'].append(result)
        try:
            command([args.cargo, 'build', '--release', '--locked', '--bin', 'build_circuit',
                     '--target-dir', str(out/'builder-target')], local_env=build_env,
                    name=directory/'build.log')
            command([str(out/'builder-target/release/build_circuit')], cwd=directory,
                    name=directory/'emit.log', local_env=build_env)
            record_circuit(directory, row)
            for name in ('specs', 'taxonomy'):
                if not (directory/name).exists():
                    (directory/name).symlink_to(ROOT/name, target_is_directory=True)
            command([str(evaluator), '--root', str(directory), '--engine', 'sliced',
                     '--samples', str(args.samples), '--export-symbolic', str(directory/'symbolic-input.json')],
                    name=directory/'sampled-screen.log')
            metrics = json.loads((directory/'score.json').read_text())['metrics']
            command([sys.executable, str(ROOT/'tools/verification/encoded_operator.py'),
                     str(directory/'symbolic-input.json'), '--payload', str(ROOT/'specs'/row['spec']/'sa.bin'),
                     '--circuit', str(directory), '--partition-bits', str(args.partition_bits),
                     '--jobs', str(args.jobs), '--network-form', 'auto',
                     '--out', str(directory/'operator-report.json')], name=directory/'operator.log')
            report = json.loads((directory/'operator-report.json').read_text())
            check_report(report, row, metrics)
            result.update(status='proved', report=f'{row["submission"]}/operator-report.json',
                          sampled_screen_samples=args.samples,
                          input_sha256=report['input_sha256'], coefficients=report['coefficients'])
        except (subprocess.CalledProcessError, ValueError, OSError) as error:
            result.update(status='unproved', reason=str(error))
        (out/'audit-results.json').write_text(json.dumps(summary, indent=2)+'\n')
        print(json.dumps({k: result[k] for k in ('submission', 'status')}), flush=True)
    return 0 if all(r['status'] == 'proved' for r in summary['results']) else 2


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--out-root', type=pathlib.Path, required=True)
    p.add_argument('--submission', action='append', default=[])
    p.add_argument('--cargo', default='cargo')
    p.add_argument('--samples', type=int, default=4096)
    p.add_argument('--partition-bits', type=int, default=6)
    p.add_argument('--jobs', type=int, default=2)
    args = p.parse_args()
    if min(args.samples, args.jobs) <= 0 or args.partition_bits < 0:
        p.error('samples/jobs must be positive and partition bits nonnegative')
    return run(args)


if __name__ == '__main__':
    raise SystemExit(main())
