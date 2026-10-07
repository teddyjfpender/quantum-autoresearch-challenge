#!/usr/bin/env python3
"""Executable hierarchical signed-square walk and exact table/word verification.

The IR's primitives are clean QROAM (with feed-forward erasure), comparisons,
Fredkin, logical integer-angle Givens, controlled Z, and uniform reflections.
This is an operator-level certificate, not a fully lowered physical-gate proof.
"""
from __future__ import annotations
import argparse
from dataclasses import dataclass
from fractions import Fraction as F
import hashlib
import json
import math
import pathlib
import struct
import sys
import numpy as np
from flint import arb, ctx
from bounds import ball, upper, qpe_plan
from inputs import HERE, ROOT, sha256


def frac(x):
    return F.from_float(float(x))


def dyadic_upper(q, bits=128):
    """Exact outward rounding avoids products of thousands of odd denominators."""
    q = F(q)
    return F(-((-q.numerator*(1 << bits))//q.denominator), 1 << bits)


def read_base(path):
    data = path.read_bytes()
    if data[:8] != b'FEMOSAS1':
        raise ValueError('base magic')
    version, n, r, b, c, beta, eta = struct.unpack_from('<7I', data, 8)
    if version != 1 or not 2 <= n <= 1024 or not 3 <= beta <= 32:
        raise ValueError('base dimensions')
    pos = 36
    def take(count, dtype):
        nonlocal pos
        x = np.frombuffer(data, dtype=dtype, count=count, offset=pos).copy()
        pos += x.nbytes
        return x
    const = float(take(1, '<f8')[0])
    e = take(n, '<f8')
    ea = take(n*(n-1), '<u4').reshape(n, n-1)
    wb, w = take(r*c, '<f8'), take(r*c*b, '<f8').reshape(r*c, b)
    angles = take(r*b*(n-1), '<u4').reshape(r*b, n-1)
    if pos != len(data) or not all(np.all(np.isfinite(a)) for a in (e, wb, w)):
        raise ValueError('invalid base payload')
    return dict(n=n, r=r, b=b, c=c, beta=beta, eta=eta, const=const, e=e, ea=ea, wb=wb, w=w, angles=angles)


@dataclass
class Row:
    square: bool
    sign: int
    weights: list[F]
    networks: list[int]  # -1 denotes I; other entries denote -Z(u,s).

    def size(self):
        return sum(map(abs, self.weights), F(0))

    def mass(self):
        return self.size()**2/4 if self.square else self.size()


def assemble(base, correction=None):
    n = base['n']
    beta = max(base['beta'], 26 if correction is not None else base['beta'])
    networks = [np.array(w, dtype=np.uint32) << (beta-base['beta']) for w in base['ea']]
    rows = [Row(False, 1, [frac(e)], [j]) for j, e in enumerate(base['e']) if e]
    start = len(networks)
    networks.extend(np.array(w, dtype=np.uint32) << (beta-base['beta']) for w in base['angles'])
    for j, (wb, weights) in enumerate(zip(base['wb'], base['w'])):
        nets = [start+(j//base['c'])*base['b']+b for b in range(base['b'])]+[-1]
        rows.append(Row(True, 1, list(map(frac, weights))+[frac(wb)], nets))
    if correction is not None:
        for sign, es, angles in zip(correction['signs'], correction['eigenvalues'], correction['angles']):
            start = len(networks)
            networks.extend(np.array(w, dtype=np.uint32) << (beta-26) for w in angles)
            rows.append(Row(True, -int(sign), list(map(frac, es)), list(range(start, start+n))))
    rows = [r for r in rows if r.mass()]
    if any(r.sign not in (-1, 1) for r in rows):
        raise ValueError('invalid square sign')
    return rows, np.array(networks, dtype=np.uint32), beta


def alias(weights, k, mu):
    """Exact apportionment followed by Walker construction, no float decisions."""
    if mu < 1 or k < 0 or len(weights) > 1 << k or any(w < 0 for w in weights):
        raise ValueError('alias dimensions/weights')
    norm = sum(weights, F(0))
    if norm <= 0:
        raise ValueError('zero alias mass')
    size, capacity = 1 << k, 1 << mu
    draws = size*capacity
    counts = [int(w*draws//norm) for w in weights]+[0]*(size-len(weights))
    counts[max(range(len(weights)), key=lambda j: weights[j])] += draws-sum(counts)
    work = counts.copy()
    small = [j for j, v in enumerate(work) if v < capacity]
    large = [j for j, v in enumerate(work) if v >= capacity]
    keep, alt = [capacity]*size, list(range(size))
    while small and large:
        s, l = small.pop(), large.pop()
        keep[s], alt[s] = work[s], l
        work[l] -= capacity-work[s]
        (small if work[l] < capacity else large).append(l)
    if any(work[j] != capacity for j in small+large):
        raise ValueError('alias closure')
    error = sum((abs(F(counts[j], draws)-w/norm) for j, w in enumerate(weights)), F(0))
    return {'k': k, 'mu': mu, 'keep': keep, 'alt': alt, 'counts': counts, 'l1_error': str(error)}


def histogram(table):
    size, cap = 1 << table['k'], 1 << table['mu']
    if len(table['keep']) != size or len(table['alt']) != size:
        raise ValueError('lookup length')
    counts = [0]*size
    for j, (keep, alt) in enumerate(zip(table['keep'], table['alt'])):
        if not 0 <= keep <= cap or not 0 <= alt < size:
            raise ValueError('invalid alias word')
        counts[j] += keep
        counts[alt] += cap-keep
    return counts


def inner_code(second=False):
    # The second SELECT is enabled only on square rows. PREPARE/Givens cancel
    # on an inactive SELECT, so no controlled arbitrary rotation is required.
    return [
        ['load', 'inner_alias'], ['compare', 'inner_draw', 'keep'], ['fredkin', 'inner_record'],
        ['load', 'angles'], ['spin_swap', 'outer_if_one_else_inner'], ['givens', -1],
        ['select_z', 'control_and_square' if second else 'control'], ['givens', 1],
        ['spin_swap_inverse', 'outer_if_one_else_inner'], ['erase', 'angles'],
        ['fredkin_inverse', 'inner_record'], ['compare_inverse', 'inner_draw', 'keep'],
        ['erase', 'inner_alias']]


def compile_program(rows, networks, beta, mu=24):
    outer_k = max(1, (len(rows)-1).bit_length())
    inner_k = max(1, (max(map(lambda r: len(r.weights), rows))-1).bit_length())
    outer = alias([r.mass() for r in rows], outer_k, mu)
    inner = [alias(list(map(abs, r.weights)), inner_k, mu) for r in rows]
    program = {'schema': 'signed-square-walk-ir-v1', 'beta': beta,
        'outer_bits': outer_k+mu+1, 'inner_bits': inner_k+mu+1,
        'body': [['load', 'outer_alias'], ['compare', 'outer_draw', 'keep'], ['fredkin', 'outer_record'],
                 ['inner', inner_code()], ['reflect', 'inner_uniform', 'control_and_square'],
                 ['inner', inner_code(True)], ['row_sign', 'control'],
                 ['fredkin_inverse', 'outer_record'], ['compare_inverse', 'outer_draw', 'keep'],
                 ['erase', 'outer_alias'], ['reflect', 'all_uniform', 'control']],
        'primitive_contract': 'clean QROAM/erase with phase fixup; logical integer-angle Givens; exact uniform reflections',
        'network_sha256': hashlib.sha256(networks.astype('<u4').tobytes()).hexdigest()}
    netbits = max(1, len(networks).bit_length())
    def outer_record(j):
        if j >= len(rows):
            return 0
        return j | int(rows[j].square) << outer_k | int(rows[j].sign < 0) << (outer_k+1)
    def inner_record(row, j):
        if j >= len(row.weights):
            return 1  # harmless identity on an unreachable padding branch
        net, w = row.networks[j], row.weights[j]
        neg = (w < 0) if net < 0 else (w > 0)
        return ((net+1) << 2) | (int(neg) << 1) | int(net < 0)
    wo, wi = outer_k+2, netbits+2
    program['outer_words'] = [keep | outer_record(j) << (mu+1) | outer_record(alt) << (mu+1+wo)
                              for j, (keep, alt) in enumerate(zip(outer['keep'], outer['alt']))]
    program['inner_words'] = [[keep | inner_record(row, j) << (mu+1) | inner_record(row, alt) << (mu+1+wi)
                              for j, (keep, alt) in enumerate(zip(table['keep'], table['alt']))]
                             for row, table in zip(rows, inner)]
    program['rows'] = [{'square': r.square, 'sign': r.sign, 'weights': list(map(str, r.weights)), 'networks': r.networks} for r in rows]
    return program, outer, inner


def decode_word(word, draw, mu, record_bits):
    cut = word & ((1 << (mu+1))-1)
    shift = mu+1+(record_bits if draw >= cut else 0)
    return (word >> shift) & ((1 << record_bits)-1)


def interpret_inner(code):
    """Reduce a concrete conjugation/cleanup program to a system gate word.

    Tracks resource ownership and adjoints, rather than trusting a declared
    operator. A missing cleanup, wrong angle direction, selector or spin fails.
    """
    stack, word, enable = [], [], None
    for op in code:
        kind = op[0]
        if kind == 'load':
            stack.append(op[1])
        elif kind == 'erase':
            if not stack or stack.pop() != op[1]:
                raise ValueError('dirty or mismatched lookup erasure')
        elif kind in ('compare', 'fredkin'):
            stack.append(tuple(op))
        elif kind.endswith('_inverse') and kind != 'spin_swap_inverse':
            if not stack or stack.pop() != tuple([kind[:-8]]+op[1:]):
                raise ValueError('unpaired classical computation')
        elif kind in ('spin_swap', 'spin_swap_inverse', 'givens', 'select_z'):
            word.append(op)
        else:
            raise ValueError('unsupported inner primitive')
    if stack:
        raise ValueError('live scratch at inner boundary')
    if len(word) != 5 or word[0] != ['spin_swap', 'outer_if_one_else_inner'] or word[1] != ['givens', -1] or word[3] != ['givens', 1] or word[4] != ['spin_swap_inverse', 'outer_if_one_else_inner']:
        raise ValueError('incorrect rotated-Pauli conjugation')
    if word[2][0] != 'select_z' or word[2][1] not in ('control', 'control_and_square'):
        raise ValueError('incorrect SELECT condition')
    return word[2][1]


def execute_projected(program, z_operators, control=1):
    """Exact small-instance execution from emitted words and reflection entries.

    z_operators[network][spin] are exact involution matrices. This interpreter
    does not use target coefficients or the checker's 2 B^2-I rewrite.
    """
    if control not in (0, 1):
        raise ValueError('control must be a bit')
    identity = z_operators[0][0]*z_operators[0][0]
    zero = 0*identity
    outer, inner = program['outer_table'], program['inner_tables']
    ko, mu = outer['k'], outer['mu']
    netbits = max(1, len(z_operators).bit_length())
    if ko+mu > 10 or max(t['k']+t['mu'] for t in inner) > 7:
        raise ValueError('dense oracle is restricted to small circuits')
    first = interpret_inner(program['body'][3][1])
    second = interpret_inner(program['body'][5][1])
    cache = {}
    result = zero
    for bucket in range(1 << ko):
        for draw in range(1 << mu):
            record = decode_word(program['outer_words'][bucket], draw, mu, ko+2)
            row = record & ((1 << ko)-1)
            square = bool(record >> ko & 1)
            sign = -1 if control and record >> (ko+1) & 1 else 1
            for outer_spin in range(2):
                key = (row, outer_spin, control)
                if key not in cache:
                    table = inner[row]
                    matrices = []
                    for b in range(1 << table['k']):
                        for d in range(1 << table['mu']):
                            rec = decode_word(program['inner_words'][row][b], d, table['mu'], netbits+2)
                            for spin in range(2):
                                m = identity if rec & 1 else z_operators[(rec >> 2)-1][spin if square else outer_spin]
                                matrices.append(-m if rec >> 1 & 1 else m)
                    size = len(matrices)
                    applied_a = [m if control and (first == 'control' or square) else identity for m in matrices]
                    applied_b = [m if control and (second == 'control' or square) else identity for m in matrices]
                    value = zero
                    for a, ma in enumerate(applied_a):
                        for b, mb in enumerate(applied_b):
                            reflection = (F(2, size)-int(a == b)) if control and square else F(int(a == b))
                            if reflection:
                                from flint import fmpq
                                weight = reflection/size
                                value += fmpq(weight.numerator, weight.denominator)*(mb*ma)
                    cache[key] = value
                result += sign*cache[key]
    return result/(2*(1 << (ko+mu)))


def verify(program, rows, networks, outer, inner, *, reference_beta):
    """All-row exact coefficient proof and compositional operator-word check.

    V R V reduces to 2 B^2-I for every system input, including noncommuting
    rotated Paulis; negative rows change both the square AND its scalar offset.
    """
    if program['schema'] != 'signed-square-walk-ir-v1' or program['beta'] != reference_beta:
        raise ValueError('incorrect schema or certified rotation precision')
    if not rows or len(inner) != len(rows):
        raise ValueError('missing inner table')
    if any(t['k'] != inner[0]['k'] or t['mu'] != outer['mu'] for t in inner):
        raise ValueError('inconsistent uniform register dimensions')
    if program['network_sha256'] != hashlib.sha256(networks.astype('<u4').tobytes()).hexdigest():
        raise ValueError('network digest')
    if np.any(networks.astype(np.uint64) >= 1 << program['beta']):
        raise ValueError('rotation outside declared width')
    if program['outer_bits'] != outer['k']+outer['mu']+1 or program['inner_bits'] != inner[0]['k']+inner[0]['mu']+1:
        raise ValueError('incorrect reflection register')
    if program['rows'] != [{'square': r.square, 'sign': r.sign, 'weights': list(map(str, r.weights)), 'networks': r.networks} for r in rows]:
        raise ValueError('program target rows differ')
    body = program['body']
    if len(body) != 11 or body[:3] != [['load', 'outer_alias'], ['compare', 'outer_draw', 'keep'], ['fredkin', 'outer_record']] or body[6:] != [['row_sign', 'control'], ['fredkin_inverse', 'outer_record'], ['compare_inverse', 'outer_draw', 'keep'], ['erase', 'outer_alias'], ['reflect', 'all_uniform', 'control']]:
        raise ValueError('incorrect outer combination or walk reflection')
    if body[3][0] != 'inner' or interpret_inner(body[3][1]) != 'control' or body[5][0] != 'inner' or interpret_inner(body[5][1]) != 'control_and_square':
        raise ValueError('incorrect first/second pass')
    if body[4] != ['reflect', 'inner_uniform', 'control_and_square']:
        raise ValueError('incorrect signed-square reflection')
    counts = histogram(outer)
    if counts != outer['counts'] or any(counts[len(rows):]):
        raise ValueError('outer distribution mismatch')
    norm = sum((r.mass() for r in rows), F(0))
    draw_o = 1 << (outer['k']+outer['mu'])
    error_o = sum((abs(norm*F(counts[j], draw_o)-r.mass()) for j, r in enumerate(rows)), F(0))
    inner_error = F(0)
    offset = F(0)
    symbolic_pairs = 0
    checked_entries = len(counts)
    ko, mu = outer['k'], outer['mu']
    if len(program['outer_words']) != len(counts) or len(program['inner_words']) != len(rows):
        raise ValueError('missing emitted lookup words')
    for bucket, word in enumerate(program['outer_words']):
        if word & ((1 << (mu+1))-1) != outer['keep'][bucket] or word >> (mu+1+2*(ko+2)):
            raise ValueError('bad outer keep/word width')
        for use_alt, j in enumerate((bucket, outer['alt'][bucket])):
            record = (word >> (mu+1+use_alt*(ko+2))) & ((1 << (ko+2))-1)
            if j >= len(rows):
                if record != 0:
                    raise ValueError('nonzero outer padding')
                continue
            if record & ((1 << ko)-1) != j or bool(record >> ko & 1) != rows[j].square or (-1 if record >> (ko+1) & 1 else 1) != rows[j].sign:
                raise ValueError('wrong emitted outer row/sign')
    for j, (r, tab) in enumerate(zip(rows, inner)):
        have = histogram(tab)
        if have != tab['counts'] or any(have[len(r.weights):]) or len(r.weights) != len(r.networks):
            raise ValueError('inner distribution/record mismatch')
        if r.sign not in (-1, 1) or any(not -1 <= net < len(networks) for net in r.networks):
            raise ValueError('bad signed row or network')
        width = max(1, len(networks).bit_length())+2
        words = program['inner_words'][j]
        if len(words) != len(have):
            raise ValueError('missing inner lookup')
        for bucket, word in enumerate(words):
            if word & ((1 << (tab['mu']+1))-1) != tab['keep'][bucket] or word >> (tab['mu']+1+2*width):
                raise ValueError('bad inner keep/width')
            for use_alt, t in enumerate((bucket, tab['alt'][bucket])):
                record = (word >> (tab['mu']+1+use_alt*width)) & ((1 << width)-1)
                if t >= len(r.weights):
                    if record != 1:
                        raise ValueError('bad inner padding')
                    continue
                net, w = r.networks[t], r.weights[t]
                if (record >> 2)-1 != net or bool(record & 1) != (net < 0) or bool(record >> 1 & 1) != ((w < 0) if net < 0 else (w > 0)):
                    raise ValueError('wrong emitted orbital/sign/identity')
        draw = 1 << (tab['k']+tab['mu'])
        delta = sum((abs(F(have[t], draw)-abs(w)/r.size()) for t, w in enumerate(r.weights)), F(0))
        if not r.square and (len(r.weights) != 1 or r.networks[0] < 0 or r.sign != 1):
            raise ValueError('one-body row must be a single signed orbital')
        if r.square:
            # Norms of both B and Bhat are <=1: ||2 Bhat²-2 B²||<=4 delta.
            inner_error += dyadic_upper(4*norm*F(counts[j], draw_o)*delta)
            offset += r.sign*r.mass()
            symbolic_pairs += (2*len(r.weights))**2
        else:
            symbolic_pairs += 2
        checked_entries += len(have)
    return {'verified': True, 'level': 'exact compositional operator-word proof and exhaustive table histograms',
        'scope': 'all system states, both controls, every encoded row; logical primitive contracts are trusted',
        'fully_lowered_gate_certificate': False, 'checked_alias_buckets': checked_entries,
        'ordered_operator_pairs_covered_by_identity': symbolic_pairs,
        'outer_error_upper_Ha': str(error_o), 'inner_error_upper_Ha': str(inner_error),
        'coefficient_error_upper_Ha': str(error_o+inner_error),
        'normalization_Ha': str(norm), 'chebyshev_offset_Ha': str(offset),
        'identity': 'projected block = sum_one mass*B + sum_square sign*mass*(2*B^2-I); add const+sum_square sign*mass'}


def erase_cost(length):
    h = min(range((length-1).bit_length()+1), key=lambda h: 2*(1 << h)+(length+(1 << h)-1)//(1 << h))
    return 2*(1 << h)+(length+(1 << h)-1)//(1 << h), 1 << h


def lookup_pair(length, width, blocks):
    # Existing clean QROAM load plus exact phase-fixup erasure of the output.
    e, workspace = erase_cost(length)
    return (length+blocks-1)//blocks+(blocks-1)*width+2*e, max(blocks*width, workspace)


def compile_resources(program, rows, networks, n, outer, inner):
    """Expand every IR stage into conservative logical gate/space charges."""
    ko, ki, mu = outer['k'], inner[0]['k'], outer['mu']
    netbits = max(1, len(networks).bit_length())
    outer_width = mu+1+2*(ko+2)
    inner_width = mu+1+2*(netbits+2)
    angle_width = (n-1)*program['beta']
    tables = [(1 << ko, outer_width, 1), (len(rows)*(1 << ki), inner_width, 2),
              (len(networks)+1, angle_width, 2)]
    # Two passes, each G† Z G with spin swap and its inverse. Per-table alias
    # comparator and its inverse <=4(mu+1), selected-record swaps twice.
    givens = 4*(n-1)
    rotations = givens*2*program['beta']
    spin = 4*(n+1)
    alias_arithmetic = 3*8*(mu+1)+2*(ko+2)+4*(netbits+2)
    reflections = 2*(program['inner_bits']+1)+2*(program['outer_bits']+program['inner_bits']+1)
    select_flags = 32
    fixed_t = rotations+spin+alias_arithmetic+reflections+select_flags
    # Uniforms, system/control, phase gradient, live table outputs, compare
    # carries, selected spin/row controls. Lookup scratch is reused sequentially.
    fixed_q = 2*n+1+program['outer_bits']+program['inner_bits']+program['beta']+outer_width+inner_width+angle_width+3*(mu+1)+32
    choices = [1 << a for a in range(13)]
    selected = [min(choices, key=lambda b: lookup_pair(l, w, b)[0]) for l, w, _ in tables]
    def point(blocks):
        values = [lookup_pair(l, w, b) for (l, w, _), b in zip(tables, blocks)]
        t = fixed_t+sum(mult*v[0] for v, (_, _, mult) in zip(values, tables))
        q = fixed_q+max(v[1] for v in values)
        return {'blocks_outer_inner_angles': list(blocks), 'controlled_walk_toffoli_upper': t,
                'logical_qubits_upper_including_system': q, 'TQ_upper': t*q,
                'lookup_toffoli_by_stage': [mult*v[0] for v, (_, _, mult) in zip(values, tables)]}
    # For each memory ceiling choose every table's best feasible implementation.
    frontier = []
    for ceiling in sorted({lookup_pair(l, w, b)[1] for l, w, _ in tables for b in choices}):
        feasible = [[b for b in choices if lookup_pair(l, w, b)[1] <= ceiling] for l, w, _ in tables]
        if all(feasible):
            bs = [min(bs, key=lambda b: lookup_pair(l, w, b)[0]) for (l, w, _), bs in zip(tables, feasible)]
            p = point(bs)
            if not frontier or p['controlled_walk_toffoli_upper'] < frontier[-1]['controlled_walk_toffoli_upper']:
                frontier.append(p)
    return {'scope': 'compiled hierarchical controlled walk; conservative primitive expansion, not a flattened gate-count measurement',
        'givens_count': givens, 'givens_charge': '2*beta Toffolis per logical Givens (Low convention)',
        'rotation_toffoli': rotations, 'non_lookup_toffoli': fixed_t,
        'table_dimensions': [{'entries': l, 'word_bits': w, 'calls_load_and_erase': mult} for l, w, mult in tables],
        'min_toffoli': point(selected), 'min_TQ': min(frontier, key=lambda p: p['TQ_upper']), 'frontier': frontier,
        'includes': ['outer and inner preparation/unpreparation', 'angle lookups and rotations',
                     'signed SELECT', 'both controlled reflections', 'phase-fixup erasure', 'phase-gradient register'],
        'excludes': ['ground-state preparation', 'physical gate-synthesis error overhead', 'QFT synthesis']}


def low_comparison(name, norm, step):
    target = next(t for t in json.loads((ROOT/'targets.json').read_text())['targets'] if t['track'] == name)
    low_norm = F(str(target['lambdaEffPublished']))
    low_step = target['toffoliPublished']
    sigma = F(1, 1000)
    query = math.ceil(upper(arb.pi()*ball(norm)/(2*ball(sigma))))
    low_query = math.ceil(upper(arb.pi()*ball(low_norm)/(2*ball(sigma))))
    plan = qpe_plan(norm)
    return {'paper_convention': {'scope': 'same pi*lambda/(2 sigma) resource convention; not a matched chemical-accuracy guarantee',
        'sigma_Ha': str(sigma), 'our_ordinary_normalization_Ha': str(norm), 'low_effective_normalization_Ha': str(low_norm),
        'our_queries': query, 'low_queries': low_query, 'our_total_toffoli_upper': query*step,
        'low_total_toffoli_from_printed_parameters': low_query*low_step,
        'total_cost_ratio_display': query*step/(low_query*low_step),
        'break_even_step_toffoli_at_our_normalization': str(F(low_query*low_step, query)),
        'continuous_break_even_normalization_at_our_step_Ha': str(low_norm*low_step/step),
        'source': 'Low et al. PRX 15, 041016, Table V; https://doi.org/10.1103/pb2g-j9cw'},
        'conditional_99_percent_QPE': dict(plan, controlled_walk_toffoli_upper=step,
            controlled_walk_total_toffoli_upper=step*plan['controlled_walk_uses'])}


def run(name, artifacts, out, mu=24, verify_only=False):
    ctx.prec = 128
    out.unlink(missing_ok=True)
    cert = json.loads((ROOT/'rigorous'/f'signed-{name}.json').read_text())
    for file, digest in cert['checker_sha256'].items():
        if sha256(HERE/file) != digest:
            raise ValueError('stale numerical certificate checker')
    base_path, data_path = artifacts/(name+'-base.bin'), artifacts/(name+'-signed.npz')
    if sha256(base_path) != cert['base_payload_sha256'] or sha256(data_path) != cert['signed_data_sha256']:
        raise ValueError('unbound circuit input')
    base = read_base(base_path)
    with np.load(data_path, allow_pickle=False) as correction:
        rows, nets, beta = assemble(base, correction)
    path = artifacts/(name+'-combined-program.json')
    network_path = artifacts/(name+'-combined-networks.npy')
    if verify_only:
        program = json.loads(path.read_text())
        emitted_nets = np.load(network_path, allow_pickle=False)
        if not np.array_equal(emitted_nets, nets):
            raise ValueError('emitted networks differ from certified input')
        outer, inner = program['outer_table'], program['inner_tables']
        mu = outer['mu']
    else:
        program, outer, inner = compile_program(rows, nets, beta, mu)
        program['outer_table'], program['inner_tables'] = outer, inner
    proof = verify(program, rows, nets, outer, inner, reference_beta=beta)
    if F(proof['normalization_Ha']) != F(cert['combined_normalization_Ha']):
        raise ValueError('normalization mismatch')
    resources = compile_resources(program, rows, nets, base['n'], outer, inner)
    if not verify_only:
        path.write_text(json.dumps(program, separators=(',', ':'))+'\n')
        np.save(network_path, nets, allow_pickle=False)
    # Verify the emitted artifact, not just transient compiler objects.
    emitted = json.loads(path.read_text())
    proof = verify(emitted, rows, nets, emitted['outer_table'], emitted['inner_tables'], reference_beta=beta)
    total = F(cert['hamiltonian_error_upper_Ha'])+F(proof['coefficient_error_upper_Ha'])+F(1, 1000)
    result = {'schema': 'femoco-combined-signed-walk-v1', 'instance': name,
        'operator_certificate_sha256': sha256(ROOT/'rigorous'/f'signed-{name}.json'),
        'program_sha256': sha256(path), 'network_sha256': program['network_sha256'],
        'checker_sha256': sha256(pathlib.Path(__file__)),
        'rows': len(rows), 'networks': len(nets), 'beta': beta, 'alias_keep_bits': mu,
        'verification': proof, 'resources': resources,
        'offset_Ha': str(frac(base['const'])+F(proof['chebyshev_offset_Ha'])),
        'budget': {'operator_Ha': cert['hamiltonian_error_upper_Ha'],
                   'encoding_Ha': proof['coefficient_error_upper_Ha'], 'QPE_Ha': '1/1000',
                   'total_upper_Ha': str(total), 'total_mHa_display': float(total*1000),
                   'remaining_systematic_margin_Ha': str(F(1, 625)-total),
                   'fits_1_6_mHa': total <= F(1, 625), 'chemical_accuracy_certified': False,
                   'missing': ['physical synthesis accuracy including phase-gradient preparation',
                               'ground-state preparation/selection', 'implemented QPE/QFT failure accounting']},
        'comparison': low_comparison(name, F(proof['normalization_Ha']), resources['min_toffoli']['controlled_walk_toffoli_upper'])}
    if not result['budget']['fits_1_6_mHa']:
        raise ValueError('combined budget failed')
    out.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'instance': name, 'lambda': float(F(proof['normalization_Ha'])),
                     'resources': resources['min_toffoli'], 'budget_mHa': float(total*1000)}), flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--instance', required=True, choices=('reiher', 'li'))
    p.add_argument('--artifacts', required=True, type=pathlib.Path)
    p.add_argument('--out', required=True, type=pathlib.Path)
    p.add_argument('--mu', type=int, default=24)
    p.add_argument('--verify-only', action='store_true', help='check existing emitted artifacts without recompiling or overwriting them')
    args = p.parse_args()
    run(args.instance, args.artifacts, args.out, args.mu, args.verify_only)
