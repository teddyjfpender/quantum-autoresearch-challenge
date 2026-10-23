"""Independent binary-to-export and coefficient audit for the SA operator checker.

No Rust builder, simulator, exported coefficient count, or floating-point comparison is
used here. Unsupported binary programs fail closed. This is maintainer audit tooling,
not an alternative judge or a proof assistant.
"""
from __future__ import annotations

import hashlib
import json
import math
import pathlib
import struct
from fractions import Fraction as Q

NONE = 2**32 - 1
DISK_NONE = 2**64 - 1
RECORD = struct.Struct('<II6Q')


class PackedOps:
    """Read-only lowered program with bounded storage across forked BDD workers.

    Python JSON dictionaries for five million ops consume GB; reading them after fork
    dirties their reference-count pages in every worker. Store six u32s per op instead,
    yielding the same small dictionaries at the existing interpreter's interface.
    """
    record = struct.Struct('<6I')
    kinds = ('Pop', 'Reflect', 'Neg', 'X', 'Cx', 'Ccx', 'Swap', 'Phase', 'Hmr',
             'Reset', 'Bit', 'Push', 'Sys', 'Givens', 'SpinSwap')
    fields = ((), (), ('cond',), ('t', 'cond'), ('c', 't', 'cond'),
              ('a', 'b', 't', 'cond'), ('a', 'b', 'cond'),
              ('q0', 'q1', 'q2', 'k', 'cond'), ('t', 'bit', 'cond'), ('t',),
              ('bit', 'kind', 'cond'), ('bit',), ('q', 'z', 'c0', 'c1', 'cond'),
              ('p', 'q', 'reg', 'cond'), ('c', 'cond', 'dagger'))

    def __init__(self, ops):
        self.data = bytearray(self.record.size*len(ops))
        for index, op in enumerate(ops):
            if isinstance(op, str):
                kind, a = op, {}
            else:
                require(isinstance(op, dict) and len(op) == 1, 'unsupported packed op')
                kind, a = next(iter(op.items()))
                a = dict(a)
            require(kind in self.kinds, f'unsupported packed op {kind}')
            tag = self.kinds.index(kind)
            if kind == 'Phase':
                q = a.pop('q')
                require(len(q) == 3, 'invalid packed phase operands')
                a.update(zip(('q0', 'q1', 'q2'), q))
            if kind == 'Sys':
                ctrl = a.pop('ctrl')
                require(len(ctrl) == 2, 'invalid packed system controls')
                a.update(zip(('c0', 'c1'), ctrl))
            require(set(a) == set(self.fields[tag]), f'unsupported packed fields for {kind}')
            values = [a[name] for name in self.fields[tag]]
            require(all(isinstance(v, int) and 0 <= v <= NONE for v in values), 'invalid packed operand')
            self.record.pack_into(self.data, index*self.record.size,
                                  tag, *values, *([0]*(5-len(values))))

    def __len__(self):
        return len(self.data)//self.record.size

    @classmethod
    def unpack(cls, record):
        tag, *values = record
        kind = cls.kinds[tag]
        if tag < 2:
            return kind
        a = dict(zip(cls.fields[tag], values))
        if kind == 'Phase':
            a['q'] = [a.pop('q0'), a.pop('q1'), a.pop('q2')]
        if kind == 'Sys':
            a['ctrl'] = [a.pop('c0'), a.pop('c1')]
            a['z'] = bool(a['z'])
        if kind == 'SpinSwap':
            a['dagger'] = bool(a['dagger'])
        return {kind: a}

    def __getitem__(self, index):
        if not 0 <= index < len(self):
            raise IndexError(index)
        return self.unpack(self.record.unpack_from(self.data, index*self.record.size))

    def __iter__(self):
        return (self.unpack(record) for record in self.record.iter_unpack(self.data))


def require(ok, message):
    if not ok:
        raise ValueError(message)


class Reader:
    def __init__(self, data):
        self.data, self.at = data, 0

    def take(self, n):
        require(0 <= n <= len(self.data)-self.at, 'truncated binary input')
        out = self.data[self.at:self.at+n]
        self.at += n
        return out

    def integer(self, n=4):
        return int.from_bytes(self.take(n), 'little')

    def words(self, n):
        return list(struct.unpack('<' + str(n) + 'I', self.take(4*n)))

    def rational(self):
        sign = self.integer(1)
        parts = []
        for _ in range(2):
            n = self.integer()
            require(n <= 128, 'rational part exceeds binary format limit')
            parts.append(self.integer(n))
        require(parts[1] != 0, 'zero rational denominator')
        return Q(-parts[0] if sign == 1 else parts[0], parts[1])


def read_map(data, spec):
    r = Reader(data)
    require(r.take(8) == b'FEMOLMAP', 'bad lane-map magic')
    require(r.take(r.integer(2)) == b'sa-nested-alias-v1', 'unsupported lane-map family')
    ko, mo, ki, mi = r.words(4)
    require(max(ko, ki) <= 26 and max(mo, mi) <= 31
            and ko+mo+ki+mi+2 <= 63, 'unsupported alias widths')
    lam = r.rational()
    require(lam > 0 and lam.denominator & (lam.denominator-1) == 0
            and lam.denominator.bit_length()-1 <= 512, 'invalid dyadic normalization')

    def table(k, mu, items):
        require(items <= 1 << k, 'items do not fit alias table')
        keep, alt = r.words(1 << k), r.words(1 << k)
        require(all(v < 1 << mu for v in keep), 'keep threshold outside draw domain')
        require(all(v < items for v in alt), 'alias target outside item domain')
        require(not any(keep[items:]), 'padding alias bucket has nonzero keep')
        return dict(k=k, mu=mu, keep=keep, alt=alt)

    outer = table(ko, mo, spec['n'] + spec['r']*spec['c'])
    inner = [table(ki, mi, 2 if o < spec['n'] else spec['b']+1)
             for o in range(spec['n'] + spec['r']*spec['c'])]
    require(r.at == len(data), 'trailing lane-map bytes')
    return dict(lambda_decl=lam, outer=outer, inner_tables=inner,
                inner=dict(lo=ko+mo+1, width=ki+mi+1), uniform=ko+mo+ki+mi+2)


def counts(table, items):
    """Count every preimage by interval lengths, including keep=0 and padding."""
    out, draw = [0]*items, 1 << table['mu']
    for i, (keep, alt) in enumerate(zip(table['keep'], table['alt'])):
        if keep:
            out[i] += keep
        out[alt] += draw-keep
    require(sum(out) == 1 << (table['k']+table['mu']), 'alias counts are not normalized')
    return out


# Required / allowed operands, in disk order: c2, c1, target, classical target,
# classical condition, register. The compiler and the writer are not imported.
SHAPES = {
    0: ('', 'e'), 1: ('f', 'f'), 2: ('f', 'cdf'),
    3: ('d', 'de'), 4: ('d', 'de'), 5: ('d', 'de'),
    6: ('c', 'ce'), 7: ('c', 'ce'), 8: ('bc', 'bce'), 9: ('bc', 'bce'),
    10: ('bc', 'bce'), 11: ('c', 'c'), 12: ('cd', 'cde'),
    13: ('abc', 'abce'), 14: ('abc', 'abce'), 15: ('e', 'e'),
    16: ('', ''), 17: ('', 'abcdf'), 20: ('c', 'ce'), 21: ('c', 'ce'),
    30: ('bcf', 'bcef'), 31: ('f', 'f'), 32: ('c', 'ce'), 33: ('c', 'ce'),
    40: ('f', 'f'),
}


def bind_program(data, doc):
    """Stream-redecode every raw op and compare every lowered op, register and layout.

    Also check the actual reflection register and raw identical-copy structure. A digest
    match alone cannot establish that the JSON is a faithful lowering of those bytes.
    """
    require(data[:8] == b'FEMOOPS1' and len(data) >= 16, 'bad op-stream header')
    count = int.from_bytes(data[8:16], 'little')
    require(count <= 2_000_000_000 and len(data) == 16+56*count, 'op-stream length mismatch')
    system, uniform = doc['layout']['system'], doc['layout']['uniform']
    first, base = 1+system, 1+system+uniform
    registers, lowered, depth = [], 0, 0
    num_qubits, num_bits = base, 0
    begins, ends, reflects, outside_angles = [], [], [], []
    region = 0
    inner_wires = set(range(first+doc['inner']['lo'], first+uniform))

    def sys(q):
        return 1 <= q <= system

    def emit(op):
        nonlocal lowered
        require(lowered < len(doc['ops']) and doc['ops'][lowered] == op,
                f'binary/export lowering differs at lowered op {lowered}')
        lowered += 1

    for index, raw in enumerate(RECORD.iter_unpack(memoryview(data)[16:])):
        kind, pad, *fields = raw
        require(kind in SHAPES and pad == 0, f'unknown kind or padding at raw op {index}')
        require(all(v == DISK_NONE or v < NONE for v in fields), 'operand outside u32 domain')
        fields = [NONE if v == DISK_NONE else v for v in fields]
        present = {chr(97+i) for i, v in enumerate(fields) if v != NONE}
        required, allowed = map(set, SHAPES[kind])
        require(required <= present <= allowed, f'invalid operand shape at raw op {index}')
        c2, c1, t, bit, cond, reg = fields
        require(kind != 40 or reg <= 4, 'unknown segment code')
        qs = [q for q in (t, c1, c2) if q != NONE]
        require(len(set(qs)) == len(qs), 'aliased qubit operands')
        require(all(q < 1 << 20 for q in qs), 'qubit cap exceeded')
        require(all(b == NONE or b < 1 << 22 for b in (bit, cond)), 'classical-bit cap exceeded')
        require(kind == 40 or reg == NONE or reg < 1 << 20, 'register cap exceeded')
        num_qubits = max(num_qubits, max(qs, default=-1)+1)
        num_bits = max(num_bits, max((b for b in (bit, cond) if b != NONE), default=-1)+1)
        in_copy = region in (1, 3)
        if kind == 40 and reg == 3:
            require(region in (0, 2), 'unexpected inner begin')
            begins.append(index)
            region += 1
        elif kind == 40 and reg == 4:
            require(in_copy, 'unexpected inner end')
            ends.append(index)
            region += 1
        elif in_copy:
            require(kind not in (1, 2, 31, 40), 'bookkeeping or reflection inside a copy')
        elif kind not in (1, 2, 17, 31):
            require(not inner_wires.intersection(qs), 'inner register touched outside copies')
            if kind == 30:
                outside_angles.append(reg)

        if kind in (1, 2):
            while len(registers) <= reg:
                registers.append([])
            if kind == 2:
                require(t != NONE and bit == NONE and not sys(t) and len(registers[reg]) < 64,
                        'invalid register append')
                registers[reg].append(t)
            continue
        if kind in (17, 40):
            continue
        if kind == 31:
            require(depth == 0 and reg < len(registers), 'invalid reflection scope/register')
            reflects.append((index, reg))
            emit('Reflect')
            continue
        if kind == 15:
            depth += 1
        if kind == 16:
            depth -= 1
            require(depth >= 0, 'condition stack underflow')
        if kind in (11, 12):
            require(t >= base, 'control/uniform/system measurement or reset')
        if kind == 30:
            require(sys(c1) and sys(t) and c1 < t and reg < len(registers), 'invalid Givens')
            emit({'Givens': dict(p=c1-1, q=t-1, reg=reg, cond=cond)})
        elif kind in (32, 33):
            require(not sys(t), 'system-controlled spin swap')
            emit({'SpinSwap': dict(c=t, dagger=kind == 33, cond=cond)})
        elif any(sys(q) for q in qs):
            require(kind in (6, 7, 8, 9, 13, 14) and sum(sys(q) for q in qs) == 1,
                    'unsupported system operation')
            if kind in (6, 8, 13):
                require(sys(t), 'system used as quantum X control')
                q, ctrl = t, [c1, c2]
            elif kind == 7:
                q, ctrl = t, [NONE, NONE]
            else:
                triple = [t, c1, c2]
                q = next(q for q in triple if sys(q))
                ctrl = [v for v in triple if v != q]
            emit({'Sys': dict(q=q-1, z=kind in (7, 9, 14), ctrl=ctrl, cond=cond)})
        elif kind == 0:
            emit({'Neg': dict(cond=cond)})
        elif kind == 6:
            emit({'X': dict(t=t, cond=cond)})
        elif kind == 8:
            emit({'Cx': dict(c=c1, t=t, cond=cond)})
        elif kind == 13:
            emit({'Ccx': dict(a=c1, b=c2, t=t, cond=cond)})
        elif kind == 10:
            emit({'Swap': dict(a=c1, b=t, cond=cond)})
        elif kind in (7, 9, 14, 20, 21):
            emit({'Phase': dict(q=[t, c1, c2], k={20: 2, 21: 6}.get(kind, 4), cond=cond)})
        elif kind == 12:
            emit({'Hmr': dict(t=t, bit=bit, cond=cond)})
        elif kind == 11:
            emit({'Reset': dict(t=t)})
        elif kind in (3, 4, 5):
            emit({'Bit': dict(bit=bit, kind=kind-3, cond=cond)})
        elif kind == 15:
            emit({'Push': dict(bit=cond)})
        elif kind == 16:
            emit('Pop')
        else:
            raise ValueError(f'unsupported raw op {kind}')

    require(depth == 0 and lowered == len(doc['ops']), 'incomplete lowered stream')
    require(registers == doc['registers'], 'binary/export registers differ')
    require((num_qubits, num_bits) == (doc['layout']['num_qubits'], doc['layout']['num_bits']),
            'binary/export wire layout differs')
    require(len(begins) == len(ends) == 2 and len(reflects) == 1 and region == 4,
            'expected exactly two inner copies and one reflection')
    (rindex, rreg), = reflects
    require(ends[0]+1 == rindex == begins[1]-1, 'reflection is not the only op between copies')
    require(sorted(registers[rreg]) == sorted(inner_wires), 'reflection acts on wrong register')
    require(not any(inner_wires.intersection(registers[r]) for r in outside_angles),
            'angle register reads inner wires outside copies')
    def copy(begin, end):
        return memoryview(data)[16+56*(begin+1):16+56*end]
    require(copy(begins[0], ends[0]) == copy(begins[1], ends[1]), 'raw inner copies differ')
    return dict(status='proved', raw_operations=count, lowered_operations=lowered,
                reflection_register_checked=True, identical_raw_copies_checked=True)


def bind(directory, doc, spec):
    directory = pathlib.Path(directory)
    files = {}
    for key, name in [('ops', 'ops.bin'), ('lanemap', 'lanemap.bin'), ('family', 'family.out.json')]:
        files[key] = (directory/name).read_bytes()
        require(hashlib.sha256(files[key]).hexdigest() == doc['digests'][key], f'{name} digest differs')
    family = json.loads(files['family'])
    require(family['spec'] == doc['spec'], 'family/export spec differs')
    lane_map = read_map(files['lanemap'], spec)
    for key in ('outer', 'inner_tables', 'inner'):
        require(lane_map[key] == doc[key], f'binary/export {key} differs')
    require(lane_map['uniform'] == doc['layout']['uniform'], 'binary/export uniform width differs')
    return bind_program(files['ops'], doc), lane_map


def coefficient_audit(spec, lane_map, payload_bytes):
    """Independently expand all ordered pairs as exact rational coefficient records.

    The diagonal shift is retained explicitly. Coefficient L1 is an upper bound on an
    operator norm, not a measured ground-energy error, and a large bound is inconclusive.
    """
    n, b = spec['n'], spec['b']
    e = [abs(Q(x)) for x in spec['e']]
    wb = [abs(Q(x)) for x in spec['wb']]
    weights = [abs(Q(x)) for x in spec['weights']]
    sos = Q(struct.unpack_from('<d', payload_bytes, 36)[0])
    require(math.isfinite(float(sos)), 'nonfinite SOS constant')
    lam = sum(e) + sum((wb[q]+sum(weights[q*b:(q+1)*b]))**2/4 for q in range(len(wb)))
    identity = sos + sum(wb[q]**2/2 + sum(x*x for x in weights[q*b:(q+1)*b])/4
                         for q in range(len(wb)))
    e_sos = sos-sum(e)
    outer = counts(lane_map['outer'], n+spec['r']*spec['c'])
    to = 1 << (lane_map['outer']['k']+lane_map['outer']['mu'])
    declared = lane_map['lambda_decl']
    total_l1, signed, terms = Q(0), Q(0), 0
    target_diagonal, encoded_diagonal = Q(0), Q(0)
    for o, count in enumerate(outer):
        one_body = o < n
        q = o-n
        inner = lane_map['inner_tables'][o]
        m = counts(inner, 2 if one_body else b+1)
        ti = 1 << (inner['k']+inner['mu'])
        # Each tuple is (un-normalized ideal LCU weight, alias probability).
        # One-body generators have two outer-spin copies. Square spin is inner.
        components = ([(Q(1, 2), Q(v, ti)) for v in m] if one_body else
                      [(weights[q*b+j]/2, Q(m[j], 2*ti)) for j in range(b) for _ in range(2)]
                      + [(wb[q], Q(m[b], ti))])
        scale = e[o] if one_body else Q(1, 2)
        mass = sum(x for x, _ in components)
        # For one-body, target coefficient = |e|/4, not a normalized square weight.
        p = Q(count, 2*to) if one_body else Q(count, to)
        multiplicity = 2 if one_body else 1
        for j, (wj, pj) in enumerate(components):
            for k, (wk, pk) in enumerate(components):
                target = scale*wj*wk
                encoded = 2*declared*p*pj*pk
                if j == k:
                    target_diagonal += multiplicity*target
                    encoded_diagonal += multiplicity*encoded
                else:
                    delta = multiplicity*(encoded-target)
                    total_l1 += abs(delta)
                    signed += delta
                    terms += multiplicity
        require(mass > 0, 'zero-weight generator')
    require(target_diagonal+e_sos == identity, 'target diagonal expansion disagrees')
    # H_encoded = E_SOS + lambda_decl + A = E_SOS + sum encoded pairs.
    shift = encoded_diagonal-target_diagonal
    require(shift == 2*(declared-lam)-signed, 'diagonal coefficient identity disagrees')
    upper = total_l1+abs(shift)
    number = lambda x: dict(exact=str(x), display_Ha=float(x))
    return dict(status='proved', arithmetic='exact rational (binary64 inputs treated as dyadics)',
                ordered_off_diagonal_terms=terms, lambda_payload=number(lam),
                lambda_declared=number(declared), payload_identity=number(identity),
                E_SOS=number(e_sos), off_diagonal_coefficient_l1_Ha=number(total_l1),
                identity_shift_Ha=number(shift), encoded_to_payload_norm_upper_Ha=number(upper),
                rigorous_0p1_mHa_coefficient_rule_met=total_l1 <= Q(1, 10000),
                certifies_energy_error=False)
