#!/usr/bin/env python3
"""Symbolic check of a spin-swap/chain SA circuit against its pinned spec.

What a passing run shows: in the evaluator's lowered-op model, for every control, uniform
and second-pass input and every measurement outcome, the circuit's classical controller
restores its registers and its system gate word equals the reference word built from
sa.bin, up to the stated gate identities, with the expected scalar phase.

What it rests on (README.md, "Trust base"): the evaluator's lowering and export, the
gate identities in `normalize`, the spin-swap gate taken as an opaque inverse pair, the
reflection as an interface, this file's transcription of the spec, and CUDD. It is not a
machine-checked proof and does not address coefficient magnitudes, which the evaluator's
rounding rule checks.

Reduced ordered BDDs quantify every input and outcome bit. The address partitions are a
decomposition of the whole domain, not a sample. Unsupported structure fails closed.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import multiprocessing
import pathlib
import struct
import time
from concurrent.futures import ProcessPoolExecutor

import dd
from dd import cudd

SCHEMA = 'femoco-exact-report-v2'
CLAIM = ('For every input and measurement outcome in input_domain, in the evaluator\'s '
         'lowered-op model: registers are restored and the system gate word equals the '
         'reference word of the pinned spec with the expected scalar phase. Rests on the '
         'trust base in tools/verification/README.md; not a machine-checked proof.')
# The three files the evaluator hashed into the export, by their names in a circuit directory.
CIRCUIT_FILES = {'ops': 'ops.bin', 'lanemap': 'lanemap.bin', 'family': 'family.out.json'}

NONE = 2**32 - 1


class Unproved(Exception):
    pass


def payload(path, doc):
    data = pathlib.Path(path).read_bytes()
    if hashlib.sha256(data).hexdigest() != doc['digests']['spec']:
        raise ValueError('sa.bin digest differs from the evaluator export')
    if data[:8] != b'FEMOSAS1':
        raise ValueError('unsupported payload magic')
    version, n, r, b, c, beta, electrons = struct.unpack_from('<7I', data, 8)
    if (version != 1 or not 2 <= n <= 1024 or not 3 <= beta <= 32
            or min(r, b, c) < 1 or electrons > 2*n):
        raise ValueError('unsupported payload dimensions')
    if (2*n, b, beta) != (doc['layout']['system'], doc['b'], doc['beta']):
        raise ValueError('payload and export dimensions differ')
    at = 44  # header plus sos_const

    def read(code, count):
        nonlocal at
        out = struct.unpack_from('<' + str(count) + code, data, at)
        at += struct.calcsize(code) * count
        return out

    e = read('d', n)
    ea = read('I', n*(n-1))
    wb = read('d', r*c)
    weights = read('d', r*c*b)
    angles = read('I', r*b*(n-1))
    if at != len(data) or any(a >= 1 << beta for a in (*ea, *angles)):
        raise ValueError('malformed payload angles or length')
    if not all(math.isfinite(v) for v in (*e, *wb, *weights)):
        raise ValueError('nonfinite payload coefficient')
    return dict(n=n, r=r, b=b, c=c, beta=beta, e=e, ea=ea, wb=wb,
                weights=weights, angles=angles)


class Boolean:
    def __init__(self, names):
        self.bdd = cudd.BDD()
        self.bdd.configure(reordering=False, max_memory=2*1024**3)
        self.bdd.declare(*names)
        self.zero, self.one = self.bdd.false, self.bdd.true

    def xor(self, a, b):
        return self.bdd.apply('xor', a, b)

    def all(self, xs):
        v = self.one
        for x in xs:
            v &= x
        return v

    def word(self, value, width):
        return tuple(self.one if value >> i & 1 else self.zero for i in range(width))

    def add(self, a, b):
        carry, out = self.zero, []
        for x, y in zip(a, b):
            xy = self.xor(x, y)
            out.append(self.xor(xy, carry))
            carry = (x & y) | (carry & xy)
        return tuple(out)

    def neg(self, a):
        return self.add(tuple(~x for x in a), self.word(1, len(a)))

    def mux(self, cond, yes, no):
        return tuple(self.bdd.ite(cond, x, y) for x, y in zip(yes, no))

    def equal(self, a, b):
        return self.all(~self.xor(x, y) for x, y in zip(a, b))

    def less(self, a, b):
        lt = self.zero
        for x, y in zip(a, b):
            lt = (~x & y) | (~self.xor(x, y) & lt)
        return lt

    def lookup(self, values, index, width):
        """Constant table, composed with arbitrary Boolean index functions."""
        if len(values) > 1 << len(index):
            raise ValueError('lookup does not fit index')
        values = tuple(values) + (0,) * ((1 << len(index)) - len(values))

        def bit_tree(v, k, bit):
            while k >= 0 and index[k] in (self.zero, self.one):
                half = len(v)//2
                v = v[half:] if index[k] == self.one else v[:half]
                k -= 1
            first = (v[0] >> bit) & 1
            if all((x >> bit) & 1 == first for x in v):
                return self.one if first else self.zero
            half = len(v)//2
            return self.bdd.ite(index[k], bit_tree(v[half:], k-1, bit),
                                bit_tree(v[:half], k-1, bit))

        result = tuple(bit_tree(values, len(index)-1, bit) for bit in range(width))
        del bit_tree  # break the recursive closure before releasing the CUDD manager
        return result

    def require(self, condition, label):
        if condition != self.one:
            witness = self.bdd.pick(~condition)
            raise Unproved(json.dumps({'obligation': label, 'witness': witness}))


def variables(doc, shard_bits, shard):
    ko, mo = doc['outer']['k'], doc['outer']['mu']
    ki, mi = doc['inner_tables'][0]['k'], doc['inner_tables'][0]['mu']
    n, w, lo = doc['layout']['uniform'], doc['inner']['width'], doc['inner']['lo']
    if not 0 <= shard_bits <= ko or not 0 <= shard < 1 << shard_bits:
        raise ValueError('invalid exhaustive partition')
    # Address bits before comparator draws; each draw most significant first.
    order = ['c', f'u{ko+mo}', f'u{n-1}', f'a{w-1}']
    order += [f'u{i}' for i in range(ko-1, -1, -1)]
    order += [f'u{i}' for i in range(ko+mo-1, ko-1, -1)]
    order += [f'u{i}' for i in range(lo+ki-1, lo-1, -1)]
    order += [f'u{i}' for i in range(lo+ki+mi-1, lo+ki-1, -1)]
    order += [f'a{i}' for i in range(ki-1, -1, -1)]
    order += [f'a{i}' for i in range(ki+mi-1, ki-1, -1)]
    count = sum(isinstance(o, dict) and 'Hmr' in o for o in doc['ops'])
    order += [f'm{i}' for i in range(count)]
    if len(set(order)) != len(order) or len(order) != 1+n+w+count:
        raise ValueError('unsupported alias register layout')
    f = Boolean(order)
    u = [f.bdd.var(f'u{i}') for i in range(n)]
    for bit in range(shard_bits):
        u[ko-shard_bits+bit] = f.one if shard >> bit & 1 else f.zero
    return f, f.bdd.var('c'), u, [f.bdd.var(f'a{i}') for i in range(w)], count


class Controller:
    def __init__(self, doc, f, c, uniform, second, count):
        self.doc, self.f = doc, f
        self.system = doc['layout']['system']
        self.first = 1 + self.system
        self.q = [f.zero] * doc['layout']['num_qubits']
        self.bits = [f.zero] * doc['layout']['num_bits']
        self.q[0] = c
        self.q[self.first:self.first+len(uniform)] = uniform
        self.c, self.uniform, self.second = c, uniform, second
        self.phase = f.word(0, 3)
        self.events = [[], []]
        self.pass_id, self.hmr, self.count = 0, 0, count

    def phase_add(self, mask, k):
        self.phase = self.f.add(self.phase, tuple(mask if k >> i & 1 else self.f.zero
                                                 for i in range(3)))

    def wires(self, *ids):
        """Wires a classical gate may touch: the control and everything after the system
        qubits. The system wires are tracked as gate words, never as Boolean state."""
        for v in ids:
            if v != NONE and not (v == 0 or self.first <= v < len(self.q)):
                raise ValueError(f'classical gate on system or unknown wire {v}')

    def system_qubits(self, *ids):
        for v in ids:
            if not 0 <= v < self.system:
                raise ValueError(f'system gate on unknown qubit {v}')

    def run(self):
        f, q, bits = self.f, self.q, self.bits
        mask, stack = f.one, []
        for i, op in enumerate(self.doc['ops']):
            if op == 'Pop':
                mask = stack.pop()
                continue
            if op == 'Reflect':
                f.require(mask, 'unconditional reflection')
                if self.pass_id != 0 or stack:
                    raise ValueError('unsupported reflection structure')
                lo, w = self.doc['inner']['lo'], self.doc['inner']['width']
                f.require(f.equal(q[self.first+lo:self.first+lo+w], self.uniform[lo:lo+w]),
                          'inner register at reflection')
                q[self.first+lo:self.first+lo+w] = self.second
                self.pass_id = 1
                continue
            if not isinstance(op, dict) or len(op) != 1:
                raise ValueError('unsupported lowered operation')
            kind, a = next(iter(op.items()))
            cond = a.get('cond', NONE)
            m = mask if cond == NONE else mask & bits[cond]
            if kind == 'Push':
                stack.append(mask)
                mask &= bits[a['bit']]
            elif kind in ('X', 'Cx', 'Ccx'):
                self.wires(a['t'], a.get('c', NONE), a.get('a', NONE), a.get('b', NONE))
                if kind == 'Cx': m &= q[a['c']]
                if kind == 'Ccx': m &= q[a['a']] & q[a['b']]
                if m != f.zero:
                    q[a['t']] = m if q[a['t']] == f.zero else f.xor(q[a['t']], m)
            elif kind == 'Swap':
                x, y = a['a'], a['b']
                self.wires(x, y)
                delta = m & f.xor(q[x], q[y])
                q[x], q[y] = f.xor(q[x], delta), f.xor(q[y], delta)
            elif kind == 'Phase':
                self.wires(*a['q'])
                self.phase_add(m & f.all(q[v] for v in a['q'] if v != NONE), a['k'])
            elif kind == 'Neg':
                self.phase_add(m, 4)
            elif kind == 'Hmr':
                self.wires(a['t'])
                outcome = f.bdd.var(f'm{self.hmr}')
                self.hmr += 1
                self.phase_add(m & q[a['t']] & outcome, 4)
                q[a['t']] &= ~m
                bits[a['bit']] = f.bdd.ite(m, outcome, bits[a['bit']])
            elif kind == 'Reset':
                self.wires(a['t'])
                f.require(~(m & q[a['t']]), f'clean reset at op {i}')
            elif kind == 'Bit':
                k, v = a['kind'], bits[a['bit']]
                if k not in (0, 1, 2): raise ValueError('invalid bit operation')
                bits[a['bit']] = f.xor(v, m) if k == 0 else v & ~m if k == 1 else v | m
            elif kind == 'Sys':
                self.wires(*a['ctrl'])
                self.system_qubits(a['q'])
                m &= f.all(q[v] for v in a['ctrl'] if v != NONE)
                self.events[self.pass_id].append(('Z' if a['z'] else 'X', a['q'], m))
            elif kind == 'SpinSwap':
                self.wires(a['c'])
                self.events[self.pass_id].append(('F', a['dagger'], m & q[a['c']]))
            elif kind == 'Givens':
                width = self.doc['beta']
                reg = self.doc['registers'][a['reg']]
                self.wires(*reg[:width])
                self.system_qubits(a['p'], a['q'])
                angle = tuple(m & q[v] for v in reg[:width])
                angle += (f.zero,) * (width-len(angle))
                self.events[self.pass_id].append(('G', a['p'], a['q'], angle))
            else:
                raise ValueError(f'unsupported exact-proof opcode {kind}')
        if stack or self.pass_id != 1 or self.hmr != self.count:
            raise ValueError('incomplete controller execution')
        expected = list(self.uniform)
        lo, w = self.doc['inner']['lo'], self.doc['inner']['width']
        expected[lo:lo+w] = self.second
        f.require(~f.xor(q[0], self.c), 'control restoration')
        f.require(f.equal(q[self.first:self.first+len(expected)], expected), 'uniform restoration')
        f.require(f.all(~v for v in q[self.first+len(expected):]), 'final clean ancillas')


def decode(doc, f, uniform, second):
    outer, tables = doc['outer'], doc['inner_tables']
    ko, mo = outer['k'], outer['mu']
    ki, mi = tables[0]['k'], tables[0]['mu']
    if any((t['k'], t['mu']) != (ki, mi) for t in tables):
        raise ValueError('nonuniform inner table shape')
    bucket = tuple(uniform[:ko])
    keep = f.lookup(outer['keep'], bucket, mo+1)
    draw = tuple(uniform[ko:ko+mo]) + (f.zero,)
    item = f.mux(f.less(draw, keep), bucket, f.lookup(outer['alt'], bucket, ko))
    keeps = [v for t in tables for v in t['keep']]
    alts = [v for t in tables for v in t['alt']]
    lo = doc['inner']['lo']
    decoded = []
    for word in (uniform[lo:], second):
        idx = tuple(word[:ki])
        flat = idx + item
        keep = f.lookup(keeps, flat, mi+1)
        draw = tuple(word[ki:ki+mi]) + (f.zero,)
        term = f.mux(f.less(draw, keep), idx, f.lookup(alts, flat, ki))
        decoded.append((term, word[ki+mi]))
    return item, uniform[ko+mo], decoded


def normalize(f, events, system, beta, enable):
    """Exact identities: ZG(theta)=G(-theta)Z on one endpoint;
    G(theta+pi)=Z_p Z_q G(theta); ZX=-XZ. G gates on the same
    pair add angles. The resulting word is sufficient, not a universal normal form.
    """
    pending = [f.zero]*system
    phase, out = f.zero, []
    for event in events:
        kind = event[0]
        if kind == 'Z':
            _, q, mask = event
            pending[q] = f.xor(pending[q], mask & enable)
        elif kind == 'X':
            _, q, mask = event
            mask &= enable
            phase = f.xor(phase, pending[q] & mask)
            if mask != f.zero: out.append(('X', q, mask))
        elif kind == 'G':
            _, p, q, angle = event
            angle = tuple(v & enable for v in angle)
            angle = f.mux(f.xor(pending[p], pending[q]), f.neg(angle), angle)
            pending[p] = f.xor(pending[p], angle[-1])
            pending[q] = f.xor(pending[q], angle[-1])
            angle = angle[:-1] + (f.zero,)
            if all(v == f.zero for v in angle): continue
            if out and out[-1][:3] == ('G', p, q):
                prior = out.pop()[3]
                angle = f.add(prior, angle)
                pending[p] = f.xor(pending[p], angle[-1])
                pending[q] = f.xor(pending[q], angle[-1])
                angle = angle[:-1] + (f.zero,)
            if any(v != f.zero for v in angle): out.append(('G', p, q, angle))
        else:
            raise ValueError(f'unsupported system word {kind}')
    return out, pending, phase


def same_word(f, a, b):
    if len(a) != len(b): return f.zero
    equal = f.one
    for x, y in zip(a, b):
        if x[0] != y[0] or x[1:-1] != y[1:-1]: return f.zero
        equal &= f.equal(x[-1], y[-1]) if x[0] == 'G' else ~f.xor(x[-1], y[-1])
    return equal


def quantum(doc, spec, f, c, uniform, second, actual):
    item, outer_spin, decoded = decode(doc, f, uniform, second)
    ko, ki = len(item), len(decoded[0][0])
    n, b, copies, beta = spec['n'], spec['b'], spec['c'], spec['beta']
    ob = f.less(item, f.word(n, ko))
    expected_phase = f.zero
    variants = []
    for pass_id, ((term, inner_spin), events) in enumerate(zip(decoded, actual.events)):
        ident = ~ob & f.equal(term, f.word(b, ki))
        active = c & ~ident
        spin = f.bdd.ite(ob, outer_spin, inner_spin)
        x = c & ob
        z = c & ((ob & term[0]) | (~ob & ~ident))
        if (len(events) < 2 or events[0][:2] != ('F', True)
                or events[-1][:2] != ('F', False)
                or any(e[0] == 'F' for e in events[1:-1])):
            raise Unproved('system word is not one spin-swap sandwich per pass')
        f.require(~(active & f.xor(events[0][2], spin)), 'selected spin on entry')
        f.require(~(active & f.xor(events[-1][2], spin)), 'selected spin on exit')
        f.require(~f.xor(events[0][2], events[-1][2]), 'inverse spin swaps')

        # Inactive branches must cancel algebraically even when their angles are padding.
        off, off_z, off_phase = normalize(f, events[1:-1], 2*n, beta, ~active)
        if off: raise Unproved('inactive system word did not reduce to identity')
        f.require(f.all(~v for v in off_z), 'inactive diagonal identity')
        expected_phase = f.xor(expected_phase, off_phase)

        got, got_z, got_phase = normalize(f, events[1:-1], 2*n, beta, active)
        flat = term + item
        # One-body ignores the inner item for its network. Square identity is inactive.
        def values(j):
            out = []
            for o in range(n + spec['r']*copies):
                for t in range(1 << ki):
                    out.append(spec['ea'][o*(n-1)+j] if o < n else
                               spec['angles'][((o-n)//copies*b+t)*(n-1)+j] if t < b else 0)
            return out
        angles = [f.lookup(values(j), flat, beta) for j in range(n-1)]
        # The vector u and -u give the same square. A one-body Majorana changes
        # sign, accounted for explicitly in the global scalar phase below. The sign-normalized
        # form is accepted for one-rotation networks only (N = 2), where adding pi to the
        # angle is the whole change and the tests cover it; longer chains must match the
        # payload form or stay unproved.
        matched = None
        for normalized in ((False, True) if n == 2 else (False,)):
            flip = angles[-1][-1] if normalized else f.zero
            aa = [f.mux(flip, f.add(angle, f.word(1 << (beta-1), beta)), angle)
                  for angle in angles]
            reference = [('G', 2*j, 2*j+2, f.neg(aa[j])) for j in reversed(range(n-1))]
            reference += [('Z', 0, z), ('X', 0, x)]
            reference += [('G', 2*j, 2*j+2, aa[j]) for j in range(n-1)]
            want, want_z, want_phase = normalize(f, reference, 2*n, beta, active)
            if same_word(f, got, want) & f.equal(got_z, want_z) == f.one:
                matched = normalized
                expected_phase = f.xor(expected_phase, f.xor(got_phase, want_phase))
                expected_phase = f.xor(expected_phase, x & flip)
                break
        if matched is None:
            raise Unproved(f'pass {pass_id}: exact network/Pauli word differs from spec')
        variants.append('vector-sign-normalized' if matched else 'payload')
        signs = []
        for o in range(n + spec['r']*copies):
            for t in range(1 << ki):
                sign = (bool(t & 1) and ((spec['e'][o] >= 0) != bool(pass_id))) if o < n else (
                    spec['wb'][o-n] < 0 if t == b else
                    spec['weights'][(o-n)*b+t] >= 0 if t < b else False)
                signs.append(int(sign))
        expected_phase = f.xor(expected_phase, c & f.lookup(signs, flat, 1)[0])
    f.require(f.equal(actual.phase, (f.zero, f.zero, expected_phase)),
              'exact scalar phase, including every independent HMR outcome')
    return variants


def prove_partition(doc, spec, shard_bits, shard):
    start = time.monotonic()
    f, c, u, after, count = variables(doc, shard_bits, shard)
    actual = Controller(doc, f, c, u, after, count)
    actual.run()
    controller_seconds = time.monotonic()-start
    variants = quantum(doc, spec, f, c, u, after, actual)
    return {'partition': shard, 'status': 'proved', 'network_forms': variants,
            'controller_seconds': round(controller_seconds, 3),
            'seconds': round(time.monotonic()-start, 3)}


_worker_input = None


def worker(shard):
    doc, spec, shard_bits = _worker_input
    try:
        return prove_partition(doc, spec, shard_bits, shard)
    except (Unproved, ValueError, MemoryError) as error:
        return {'partition': shard, 'status': 'unproved', 'reason': str(error)}


def bind_circuit(doc, directory):
    """The export's digests must be those of the circuit files in `directory`."""
    for key, name in CIRCUIT_FILES.items():
        have = hashlib.sha256((pathlib.Path(directory) / name).read_bytes()).hexdigest()
        if have != doc['digests'].get(key):
            raise ValueError(f'{name} is not the file the evaluator export was made from')


def certify(doc, spec, input_sha256, partition_bits=8, partition=None, jobs=1, progress=None):
    """Run the partitions and return the report. `certified` is true only when every
    partition of the whole domain was run and proved; a single partition never certifies."""
    if doc.get('schema') != 'femoco-symbolic-input-v1':
        raise ValueError('requires a trusted evaluator export')
    if 'rotation_widths' not in doc or doc['rotation_widths'] is not None:
        raise ValueError('requires an explicit untapered rotation-width export')
    if not 0 <= partition_bits <= doc['outer']['k'] or jobs < 1:
        raise ValueError('invalid partition bits or worker count')
    total = 1 << partition_bits
    if partition is not None and not 0 <= partition < total:
        raise ValueError('invalid partition')
    started = time.monotonic()
    report = {'schema': SCHEMA, 'claim': CLAIM, 'spec': doc['spec'], 'digests': doc['digests'],
              'input_sha256': input_sha256,
              'checker_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
              'backend': f'dd.cudd {dd.__version__}', 'partition_bits': partition_bits,
              'partition_domain': 'most significant outer alias bucket bits',
              'lowered_operations': len(doc['ops']),
              'input_domain': {'controls': 2, 'uniform_bits': doc['layout']['uniform'],
                               'second_pass_bits': doc['inner']['width'],
                               'independent_measurement_bits': sum(isinstance(o, dict) and 'Hmr' in o
                                                                  for o in doc['ops'])},
              'partitions': [], 'certified': False}
    partitions = range(total) if partition is None else [partition]
    global _worker_input
    _worker_input = (doc, spec, partition_bits)
    if jobs == 1:
        results = map(worker, partitions)
        pool = None
    else:
        # fork shares the read-only lowered program. Each worker creates its own CUDD
        # manager after the fork; none is shared.
        pool = ProcessPoolExecutor(max_workers=jobs, mp_context=multiprocessing.get_context('fork'))
        results = pool.map(worker, partitions)
    try:
        for result in results:
            report['partitions'].append(result)
            if progress is not None:
                progress(report, result)
    finally:
        if pool is not None:
            pool.shutdown()
    proved = [r['partition'] for r in report['partitions'] if r['status'] == 'proved']
    report['certified'] = partition is None and proved == list(range(total))
    report['wall_seconds'] = round(time.monotonic()-started, 1)
    return report


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('input', type=pathlib.Path)
    p.add_argument('--payload', type=pathlib.Path, required=True)
    p.add_argument('--circuit', type=pathlib.Path,
                   help='directory with ops.bin, lanemap.bin and family.out.json; their digests '
                        'must equal the export\'s')
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--partition-bits', type=int, default=8)
    p.add_argument('--partition', type=int, help='one partition only; never a full certificate')
    p.add_argument('--jobs', type=int, default=1)
    args = p.parse_args()
    args.out.unlink(missing_ok=True)
    raw = args.input.read_bytes()
    doc = json.loads(raw)
    if args.circuit is not None:
        bind_circuit(doc, args.circuit)
    spec = payload(args.payload, doc)

    def progress(report, result):
        args.out.write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(result), flush=True)

    report = certify(doc, spec, hashlib.sha256(raw).hexdigest(), args.partition_bits,
                     args.partition, args.jobs, progress)
    report['circuit_files_checked'] = args.circuit is not None
    args.out.write_text(json.dumps(report, indent=2)+'\n')
    return 0 if all(r['status'] == 'proved' for r in report['partitions']) else 2


if __name__ == '__main__':
    raise SystemExit(main())
