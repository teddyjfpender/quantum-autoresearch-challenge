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

HERE = pathlib.Path(__file__).resolve().parent
SCHEMA = 'femoco-encoded-operator-report-v1'


class OutcomePhase:
    """Exact even Z/8 phase: one BDD low bit and GF(2)[outcomes] high bit.

    Phase increments are 0, 2, 4 or 6. Keeping the high bit in algebraic normal form
    avoids exponential BDDs for sums of many independent HMR signs. Coefficients are
    BDDs of lane bits only; outcome monomials are square-free. Nonlinear and rare
    joint outcome dependencies are retained, not assumed absent.
    """
    def __init__(self, f, term_limit=65536):
        self.f, self.low, self.high, self.cache = f, f.zero, {}, {}
        self.term_limit = term_limit

    def parts(self, node, depth=0):
        if depth > 128:
            raise ValueError('outcome phase polynomial depth budget exceeded')
        if node == self.f.zero:
            return {}
        if node in self.cache:
            return self.cache[node]
        outcomes = [v for v in node.support if v.startswith('m')]
        if not outcomes:
            result = {frozenset(): node}
        else:
            variable = min(outcomes, key=lambda v: int(v[1:]))
            zero = self.f.bdd.let({variable: False}, node)
            one = self.f.bdd.let({variable: True}, node)
            # Shannon expansion over GF(2): P = P(0) XOR m*(P(1) XOR P(0)).
            result = dict(self.parts(zero, depth+1))
            result.update({monomial | {variable}: coefficient
                           for monomial, coefficient in self.parts(self.f.xor(one, zero), depth+1).items()})
        if len(result) > self.term_limit:
            raise ValueError('outcome phase polynomial term budget exceeded')
        self.cache[node] = result
        return result

    def xor_high(self, mask):
        for monomial, coefficient in self.parts(mask).items():
            value = self.f.xor(self.high.get(monomial, self.f.zero), coefficient)
            if value == self.f.zero:
                self.high.pop(monomial, None)
            else:
                self.high[monomial] = value
            if len(self.high) > self.term_limit:
                raise ValueError('outcome phase polynomial term budget exceeded')
        if len(self.cache) > 1024:
            self.cache.clear()

    def add(self, mask, k):
        if k not in (0, 2, 4, 6):
            raise ValueError('phase polynomial supports even Z/8 increments only')
        if k & 2:
            self.xor_high(self.low & mask)  # exact carry into the high bit
            self.low = self.f.xor(self.low, mask)
        if k & 4:
            self.xor_high(mask)

    def equal(self, word):
        if len(word) != 3:
            raise ValueError('requires a three-bit phase word')
        # Uniqueness of ANF: equality for every outcome iff every coefficient of
        # the difference vanishes. The returned BDD checks this for each lane.
        difference = dict(self.high)
        for monomial, coefficient in self.parts(word[2]).items():
            difference[monomial] = self.f.xor(difference.get(monomial, self.f.zero), coefficient)
        return (~word[0] & ~self.f.xor(self.low, word[1])
                & self.f.all(~coefficient for coefficient in difference.values()))


class PhaseBoolean:
    """Reuse Boolean/controller machinery; dispatch only the phase-word comparison."""
    def __init__(self, boolean):
        self.boolean = boolean

    def __getattr__(self, name):
        return getattr(self.boolean, name)

    def equal(self, a, b):
        if isinstance(a, OutcomePhase):
            return a.equal(b)
        if isinstance(b, OutcomePhase):
            return b.equal(a)
        return self.boolean.equal(a, b)


class Controller(exact.Controller):
    def __init__(self, doc, f, c, uniform, second, count):
        super().__init__(doc, f, c, uniform, second, count)
        self.phase = OutcomePhase(f)

    def phase_add(self, mask, k):
        self.phase.add(mask, k)


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
        f = PhaseBoolean(f)
        actual = Controller(doc, f, c, u, after, count)
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
                        for name in ('encoded_operator.py', 'artifacts.py', 'exact.py')},
        backend=f'dd.cudd {exact.dd.__version__}', phase_engine='even-Z8-outcome-ANF-v1',
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
