#!/usr/bin/env python3
"""All-input SA circuit equivalence with independent binary/coefficient checks.

Reuses exact.py's Boolean controller and exact gate-word identities. Adds a second reading
of ops.bin and lanemap.bin, all-length sign-normalized chain references, and a separate exact
account of the error from the rounded encoded Hamiltonian to the stored payload Hamiltonian.
This is a computational certificate on a documented trust base, not a proof-assistant proof
and not a chemical-accuracy or ground-energy certificate. See OPERATOR.md.
"""
from __future__ import annotations

import argparse
import gc
import hashlib
import json
import multiprocessing
import pathlib
import time
from concurrent.futures import ProcessPoolExecutor

import artifacts
import exact
from outcomes import OutcomeBoolean

HERE = pathlib.Path(__file__).resolve().parent
SCHEMA = 'femoco-encoded-operator-report-v1'



def sign_normalized(spec):
    """Reference networks for -u when the last angle has its high bit set.

    For j < n-2, theta_j -> pi-theta_j; for j=n-2, theta_j -> theta_j+pi.
    The hyperspherical-chain formula shows that every component of u changes sign.
    Thus Z(u)=Z(-u); both one-body Majoranas change sign, canceling in M_b^dagger M_a
    because the one-body network depends only on the shared outer item. This is an
    identity for arbitrary chain length and angles, not an approximate vector test.
    """
    half, modulus, length = 1 << (spec['beta']-1), 1 << spec['beta'], spec['n']-1

    def transform(words):
        result = list(words)
        for start in range(0, len(words), length):
            if words[start+length-1] & half:
                for j in range(length):
                    result[start+j] = ((half-words[start+j]) if j < length-1
                                       else (words[start+j]+half)) % modulus
        return tuple(result)

    return dict(spec, ea=transform(spec['ea']), angles=transform(spec['angles']))


_inputs = None


def worker(partition):
    doc, spec, bits, form = _inputs
    started = time.monotonic()
    try:
        f, c, u, after, count = exact.variables(doc, bits, partition)
        f = OutcomeBoolean(f)
        c, u, after = f.lane(c), [f.lane(v) for v in u], [f.lane(v) for v in after]
        actual = exact.Controller(doc, f, c, u, after, count)
        actual.run()
        controller_seconds = time.monotonic()-started
        forms = ('payload', 'sign-normalized') if form == 'auto' else (form,)
        reasons = []
        for reference in forms:
            try:
                exact.quantum(doc, sign_normalized(spec) if reference == 'sign-normalized' else spec,
                              f, c, u, after, actual)
                return dict(partition=partition, status='proved', network_reference=reference,
                            controller_seconds=round(controller_seconds, 3),
                            seconds=round(time.monotonic()-started, 3))
            except exact.Unproved as error:
                reasons.append(f'{reference}: {error}')
        raise exact.Unproved('; '.join(reasons))
    except (exact.Unproved, ValueError, MemoryError) as error:
        return dict(partition=partition, status='unproved', reason=str(error),
                    seconds=round(time.monotonic()-started, 3))


def certify(doc, spec, directory, payload_bytes, input_digest, partition_bits=8,
            partition=None, jobs=1, network_form='auto', progress=None):
    artifacts.require(doc.get('schema') == 'femoco-symbolic-input-v1', 'unsupported export schema')
    artifacts.require('rotation_widths' in doc and doc['rotation_widths'] is None,
                      'requires explicit untapered rotations')
    artifacts.require(network_form in ('auto', 'payload', 'sign-normalized'), 'unknown network form')
    artifacts.require(0 <= partition_bits <= doc['outer']['k'] and jobs >= 1,
                      'invalid exhaustive partition or worker count')
    total = 1 << partition_bits
    artifacts.require(partition is None or 0 <= partition < total, 'invalid partition')
    started = time.monotonic()
    binding, lane_map = artifacts.bind(directory, doc, spec)
    coefficients = artifacts.coefficient_audit(spec, lane_map, payload_bytes)
    report = dict(
        schema=SCHEMA, spec=doc['spec'], digests=doc['digests'], input_sha256=input_digest,
        checker_sha256={name: hashlib.sha256((HERE/name).read_bytes()).hexdigest()
                        for name in ('encoded_operator.py', 'artifacts.py', 'exact.py', 'outcomes.py')},
        backend=f'dd.cudd {exact.dd.__version__}', boolean_engine='lane-BDD/outcome-ANF-v1',
        claim=('For all stated inputs and independent measurement records: emitted binary '
               'circuit registers are restored and its lane action is M_b^dagger M_a for the '
               'binary lane map and pinned integer-angle payload, on the trust base in '
               'tools/verification/OPERATOR.md. Implies the stated rounded block encoding '
               'in the ideal gate model; not a machine-checked proof.'),
        input_domain=dict(controls=2, uniform_bits=doc['layout']['uniform'],
                          second_pass_bits=doc['inner']['width'],
                          independent_measurement_bits=sum(isinstance(o, dict) and 'Hmr' in o
                                                           for o in doc['ops'])),
        binary_binding=binding, coefficients=coefficients, partition_bits=partition_bits,
        partition_domain='most significant outer alias bucket bits', partitions=[],
        encoded_operator_equivalence_certified=False, certifies_energy_error=False,
        chemical_accuracy_certified=False,
        unresolved_obligations=[
            'Proof-assistant verification of interpreter, gate identities and composition theorem.',
            'Physical gate synthesis, phase-gradient preparation and fault-tolerance errors.',
            'Original chemistry target: factorization, preprocessing and rotation error bounds.',
            'Ground-state preparation/selection and complete phase-estimation error budget.'])
    partitions = range(total) if partition is None else [partition]
    global _inputs
    _inputs = doc, spec, partition_bits, network_form
    pool = None
    if jobs == 1:
        results = map(worker, partitions)
    else:
        pool = ProcessPoolExecutor(max_workers=jobs, mp_context=multiprocessing.get_context('fork'))
        results = pool.map(worker, partitions)
    try:
        for result in results:
            report['partitions'].append(result)
            if progress:
                progress(report, result)
    finally:
        if pool:
            pool.shutdown()
    report['encoded_operator_equivalence_certified'] = (
        partition is None and len(report['partitions']) == total
        and all(p['status'] == 'proved' for p in report['partitions']))
    report['wall_seconds'] = round(time.monotonic()-started, 1)
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('input', type=pathlib.Path)
    p.add_argument('--payload', type=pathlib.Path, required=True)
    p.add_argument('--circuit', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--partition-bits', type=int, default=8)
    p.add_argument('--partition', type=int, help='diagnostic subset only; never certifies')
    p.add_argument('--jobs', type=int, default=1)
    p.add_argument('--network-form', choices=('auto', 'payload', 'sign-normalized'), default='auto')
    args = p.parse_args()
    args.out.unlink(missing_ok=True)
    raw = args.input.read_bytes()
    doc = json.loads(raw)
    spec = exact.payload(args.payload, doc)
    doc['ops'] = artifacts.PackedOps(doc['ops'])
    gc.collect()

    def progress(report, part):
        args.out.write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(part), flush=True)

    report = certify(doc, spec, args.circuit, args.payload.read_bytes(),
                     hashlib.sha256(raw).hexdigest(), args.partition_bits, args.partition,
                     args.jobs, args.network_form, progress)
    args.out.write_text(json.dumps(report, indent=2)+'\n')
    # Even a passing diagnostic subset is not a successful whole-domain certificate.
    return 0 if report['encoded_operator_equivalence_certified'] else 2


if __name__ == '__main__':
    raise SystemExit(main())
