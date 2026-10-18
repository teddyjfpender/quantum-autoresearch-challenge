#!/usr/bin/env python3
"""Explicit residual correction, certified sparse compression and resource plans.

This is maintainer research tooling, not a new judged circuit or an end-to-end
chemical-accuracy claim. See CORRECTED.md for the operator and cost derivations.
"""
from __future__ import annotations

import argparse
from fractions import Fraction
import hashlib
import itertools
import json
import math
import pathlib
import struct
import sys

import flint
from flint import arb, arb_mat, ctx
import numpy as np

from inputs import HERE, ROOT, factors, integrals, pairs, payload, sha256
from bounds import (ball, factor_rows, proposal_words, rational, residual,
                    rotation_bound, shifted_one_body, upper, qpe_plan)


def pair_paulis(n, p, q):
    """Four signed real Paulis averaging to A_pq/k_pq, interleaved spins.

    Each tuple means i**phase X**x Z**z (the Z acts first). Duplicate diagonal
    Paulis make both branches use exactly two uniform bits.
    """
    if not 0 <= q <= p < n:
        raise ValueError('invalid spatial pair')
    if p == q:
        return [(0, 1 << (2*p+s), 2) for s in range(2) for _ in range(2)]
    result = []
    for s in range(2):
        lo, hi = 2*q+s, 2*p+s
        endpoints = (1 << lo) | (1 << hi)
        between = ((1 << hi)-1) ^ ((1 << (lo+1))-1)
        result.extend([(endpoints, between, 0), (endpoints, between | endpoints, 2)])
    return result


def weighted_triangle(d, n):
    """Coefficients of norm-at-most-one Hermitian anticommutator terms."""
    k = [1 if p == q else 2 for p, q in pairs(n)]
    for a, ka in enumerate(k):
        for b in range(a+1):
            yield d[a, b] * ka*k[b] / (2 if a == b else 1)


def compress(d, n, drop_budget=Fraction(1, 5000), quant_budget=Fraction(1, 20000)):
    """Float sorting proposes a mask; Arb independently certifies its error.

    Quantize the weighted coefficients, not the unweighted tensor. The exact
    dyadic output multiplies a Hermitian operator of norm at most one.
    """
    drop_budget, quant_budget = Fraction(drop_budget), Fraction(quant_budget)
    if drop_budget < 0 or quant_budget <= 0:
        raise ValueError('invalid compression budgets')
    m = n*(n+1)//2
    count = m*(m+1)//2
    if d.nrows() != m or d.ncols() != m:
        raise ValueError('residual dimensions')
    approx = np.fromiter((float(x.mid()) for x in weighted_triangle(d, n)), float, count)
    if not np.all(np.isfinite(approx)):
        raise ValueError('nonfinite residual')
    order = np.argsort(np.abs(approx), kind='stable')
    cumulative = np.cumsum(np.abs(approx[order]))
    # Leave a little proposal slack. This does not enter the certified bound.
    how_many = int(np.searchsorted(cumulative, float(drop_budget)*0.999999, side='right'))
    drop = np.zeros(count, dtype=bool)
    drop[order[:how_many]] = True
    bits = max(0, math.ceil(math.log2(count/(2*float(quant_budget)))))
    while Fraction(count, 1 << (bits+1)) > quant_budget:
        bits += 1
    if bits > 50 or np.max(np.abs(approx), initial=0)*(1 << bits) >= 2**62:
        raise ValueError('quantization exceeds int64 proposal range')
    integer = np.rint(approx*(1 << bits)).astype(np.int64)
    full_error, kept_error, dropped, full_norm = (arb(0) for _ in range(4))
    for j, value in enumerate(weighted_triangle(d, n)):
        error = abs(value - arb(int(integer[j]))/(1 << bits))
        full_error += error
        full_norm += abs(value)
        if drop[j]:
            dropped += abs(value)
        else:
            kept_error += error
    if upper(dropped) > drop_budget or upper(full_error) > quant_budget:
        raise ValueError('uncertified compression proposal; no result accepted')
    full_integer = integer.copy()
    integer[drop] = 0
    return full_integer, integer, {
        'triangle_entries': count, 'nonzero_baseline_entries': int(np.count_nonzero(full_integer)),
        'nonzero_compressed_entries': int(np.count_nonzero(integer)),
        'proposed_dropped_entries': how_many,
        'fraction_entries_removed_display': 1-np.count_nonzero(integer)/count,
        'coefficient_fraction_bits': bits,
        'drop_budget_Ha': str(drop_budget), 'quantization_budget_Ha': str(quant_budget),
        'unquantized_correction_l1_upper_Ha': str(upper(full_norm)),
        'baseline_quantization_error_upper_Ha': str(upper(full_error)),
        'dropped_error_upper_Ha': str(upper(dropped)),
        'retained_quantization_error_upper_Ha': str(upper(kept_error)),
        'compressed_correction_error_upper_Ha': str(upper(dropped)+upper(kept_error)),
        'acceptance': 'outward rational Arb endpoints; floating sorting only proposes the mask',
    }


def alias(integer, fraction_bits, error_budget=Fraction(1, 20000)):
    """Build and exactly audit a padded integer Walker alias table.

    Apportion K*2^mu integer draws by floors, then give the remainder to the
    largest weight. Verify BOTH the distribution error and every alias count.
    """
    error_budget = Fraction(error_budget)
    if error_budget <= 0 or fraction_bits < 0:
        raise ValueError('invalid alias budget/precision')
    positions = np.flatnonzero(integer).astype(np.uint32)
    if not len(positions):
        raise ValueError('empty correction requires no alias circuit')
    weights = np.abs(integer[positions]).astype(np.int64)
    total = sum(map(int, weights))
    norm = Fraction(total, 1 << fraction_bits)
    k = (len(weights)-1).bit_length()
    size = 1 << k
    mu = 1
    while 2*norm/Fraction(1 << mu) > error_budget:
        mu += 1
    if k+mu >= 62:
        raise ValueError('integer alias counts exceed int64 range')
    draws, capacity = 1 << (k+mu), 1 << mu
    counts = np.zeros(size, dtype=np.int64)
    for j, weight in enumerate(weights):
        counts[j] = int(weight)*draws//total
    counts[int(np.argmax(weights))] += draws-sum(map(int, counts))
    numerator = sum(abs(int(counts[j])*total-int(weight)*draws) for j, weight in enumerate(weights))
    error = Fraction(numerator, (1 << fraction_bits)*draws)
    if error > error_budget:
        raise ValueError('alias distribution failed its exact error bound')
    remaining = counts.copy()
    small = list(map(int, np.flatnonzero(remaining < capacity)))
    large = list(map(int, np.flatnonzero(remaining >= capacity)))
    keep = np.full(size, capacity, dtype=np.int64)
    alternate = np.arange(size, dtype=np.uint32)
    while small and large:
        s, l = small.pop(), large.pop()
        keep[s], alternate[s] = remaining[s], l
        remaining[l] -= capacity-remaining[s]
        (small if remaining[l] < capacity else large).append(l)
    if any(remaining[j] != capacity for j in small+large):
        raise ValueError('alias did not close')
    reconstructed = keep.copy()
    np.add.at(reconstructed, alternate, capacity-keep)
    if not np.array_equal(reconstructed, counts):
        raise ValueError('alias histogram mismatch')
    digest = hashlib.sha256()
    for arr, dtype in ((positions, '<u4'), (integer[positions], '<i8'), (keep, '<i8'), (alternate, '<u4')):
        digest.update(arr.astype(dtype).tobytes())
    return (positions, keep, alternate), {
        'terms': len(positions), 'padded_buckets': size, 'address_bits': k,
        'keep_bits': mu, 'normalization_Ha': str(norm),
        'normalization_Ha_display': float(norm), 'alias_error_upper_Ha': str(error),
        'alias_error_budget_Ha': str(error_budget), 'integer_histogram_verified': True,
        'table_content_sha256': digest.hexdigest(),
    }


def lookup_cost(entries, width, blocks):
    """Unitary select-swap XOR lookup; no free measurement erasure assumed."""
    if min(entries, width, blocks) < 1 or blocks & (blocks-1):
        raise ValueError('invalid lookup dimensions')
    # Compute/undo unary ANDs, and select/unselect the output block.
    return 2*((entries+blocks-1)//blocks)+2*(blocks-1)*width


def resources(n, report):
    """Constructive bounds for correction P^dagger SELECT P only, NOT a walk.

    Two alias lookups, two row lookups, four single-pair Pauli dictionary
    lookups. Registers are deliberately overallocated; see CORRECTED.md.
    """
    m = n*(n+1)//2
    d = (m-1).bit_length()
    k, mu, size = report['address_bits'], report['keep_bits'], report['padded_buckets']
    # Inclusive keep value 2^mu requires mu+1 bits.
    tables = [(size, k+mu+1, 2), (report['terms'], 2*d+1, 2), (4*m, 4*n+1, 4)]
    # Two reversible comparisons (<=4(mu+1) each), two k-bit Fredkin banks,
    # anticommutation compute/uncompute, controlled Pauli masks, and phase flags.
    arithmetic = 8*(mu+1)+2*k+16*n+16
    fixed_qubits = 10*n+3*k+4*(mu+1)+2*d+20
    points = []
    choices = range(0, 13)
    for exponent in choices:
        blocks = 1 << exponent
        # Shared workspace; each XOR lookup fully uncomputes its scratch.
        counts = [lookup_cost(s, w, blocks) for s, w, _ in tables]
        peak = fixed_qubits + max(blocks*w+(s-1).bit_length() for s, w, _ in tables)
        cost = arithmetic+sum(c*mult for c, (_, _, mult) in zip(counts, tables))
        points.append({'blocks_per_lookup': blocks, 'toffoli_upper_bound': cost,
                       'logical_qubits_upper_bound_including_system': peak,
                       'toffoli_qubit_product_upper': cost*peak})
    # Also independently optimize each dictionary/table block count for T.
    bs = [min((1 << a for a in choices), key=lambda b: lookup_cost(s, w, b)) for s, w, _ in tables]
    cost = arithmetic+sum(mult*lookup_cost(s, w, b) for (s, w, mult), b in zip(tables, bs))
    peak = fixed_qubits+max(b*w+(s-1).bit_length() for (s, w, _), b in zip(tables, bs))
    independent = []
    for blocks in itertools.product((1 << a for a in choices), repeat=3):
        t = arithmetic+sum(mult*lookup_cost(s, w, b) for (s, w, mult), b in zip(tables, blocks))
        q = fixed_qubits+max(b*w+(s-1).bit_length() for (s, w, _), b in zip(tables, blocks))
        independent.append({'blocks_alias_row_dictionary': list(blocks), 'toffoli_upper_bound': t,
                            'logical_qubits_upper_bound_including_system': q,
                            'toffoli_qubit_product_upper': t*q})
    return {'scope': 'analytical constructive upper bounds for correction block encoding only; not emitted or symbolically verified',
            'gate_model': 'unitary Clifford+Toffoli, exact H uniform draws; reversible lookup without measurement savings',
            'unary': points[0], 'shared_block_sweep': points,
            'best_shared_product': min(points, key=lambda x: x['toffoli_qubit_product_upper']),
            'best_independent_product': min(independent, key=lambda x: x['toffoli_qubit_product_upper']),
            'independently_minimized_toffoli': {
                'blocks_alias_row_dictionary': bs, 'toffoli_upper_bound': cost,
                'logical_qubits_upper_bound_including_system': peak},
            'excludes': ['DFTHC base block', 'outer combination and its coefficient precision',
                         'qubitization reflection/control', 'state preparation and QPE',
                         'physical error correction and gate synthesis']}


def propose_payload(spec, us, hshift, const, beta, path):
    n = spec['n']
    eigs, vecs = np.linalg.eigh(np.array([[float(hshift[i, j].mid()) for j in range(n)] for i in range(n)]))
    ea = [a for j in range(n) for a in proposal_words([arb(float(vecs[i, j])) for i in range(n)], beta)]
    angles = [a for u in us for a in proposal_words(u, beta)]
    proposed = dict(spec, const=arb(float(const.mid())))
    proof = rotation_bound(proposed, us, hshift, const, ea, angles, beta, tuple(map(float, eigs)))
    out = bytearray(b'FEMOSAS1')
    out.extend(struct.pack('<7Id', 1, n, spec['r'], spec['b'], spec['c'], beta, spec['eta'], float(proposed['const'])))
    for vals, kind in ((eigs, 'd'), (ea, 'I'), (spec['wb'], 'd'), (spec['w'], 'd'), (angles, 'I')):
        out.extend(struct.pack('<'+str(len(vals))+kind, *vals))
    path.write_bytes(out)
    norm = sum((Fraction.from_float(abs(float(e))) for e in eigs), Fraction(0))
    b = spec['b']
    norm += sum(((Fraction.from_float(abs(wb))+sum((Fraction.from_float(abs(w)) for w in spec['w'][j*b:(j+1)*b]), Fraction(0)))**2/4 for j, wb in enumerate(spec['wb'])), Fraction(0))
    return proof, norm


def run(name, data, artifacts, precision=128):
    if precision < 96:
        raise ValueError('use at least 96-bit Arb precision')
    ctx.prec = precision
    source = json.loads((HERE/'sources.json').read_text())['instances'][name]
    directory = ROOT/'specs'/f'{name}-sa-v1'
    spec = payload(directory)
    params = factors(data/(name+'.pickle'), source, spec)
    print(f'{name}: target integrals and exact residual', file=sys.stderr, flush=True)
    h, g, core = integrals(data/source['integrals'], source, spec['n'], spec['eta'])
    rows, us = factor_rows(params, spec)
    hshift, const, hs = shifted_one_body(h, g, core, rows, params, spec)
    d = residual(rows, g, hs, spec['n'])
    del g, rows
    artifacts.mkdir(parents=True, exist_ok=True)
    basepath = artifacts/(name+'-corrected-base.bin')
    rotation, base_norm = propose_payload(spec, us, hshift, const, 26 if name == 'reiher' else 28, basepath)
    print(f'{name}: certifying all residual entries and sparse compression', file=sys.stderr, flush=True)
    full, compressed, proof = compress(d, spec['n'])
    del d
    scenarios = {}
    for label, coefficients in (('baseline', full), ('compressed', compressed)):
        print(f'{name}: constructing and exactly checking {label} alias table', file=sys.stderr, flush=True)
        arrays, enc = alias(coefficients, proof['coefficient_fraction_bits'])
        positions, keep, alternate = arrays
        path = artifacts/(name+'-'+label+'.npz')
        np.savez_compressed(path, positions=positions, coefficients=coefficients[positions],
                            keep=keep, alternate=alternate)
        correction = Fraction(proof['baseline_quantization_error_upper_Ha'] if label == 'baseline'
                              else proof['compressed_correction_error_upper_Ha'])
        structural = Fraction(rotation['upper_Ha'])+correction
        # The base lane map's exact 1-norm acceptance implies 2r <= 0.2 mHa.
        # Outer-combination 0.01 mHa is reserved, not implemented here.
        logical = structural+Fraction(enc['alias_error_upper_Ha'])+Fraction(1, 5000)+Fraction(1, 100000)
        total = logical+Fraction(1, 1000)
        scenarios[label] = {'encoding': enc, 'resource_model': resources(spec['n'], enc),
            'artifact_sha256': sha256(path),
            'explicit_hamiltonian_error_upper_Ha': str(structural),
            'explicit_hamiltonian_error_upper_mHa_display': float(structural*1000),
            'conditional_encoded_budget': {
                'rotation_preprocessing_Ha': rotation['upper_Ha'], 'correction_Ha': str(correction),
                'correction_alias_Ha': enc['alias_error_upper_Ha'],
                'base_nested_coefficients_reserved_Ha': '1/5000',
                'outer_combination_reserved_Ha': '1/100000', 'phase_estimation_reserved_Ha': '1/1000',
                'hamiltonian_total_upper_Ha': str(logical), 'total_with_QPE_Ha': str(total),
                'total_with_QPE_mHa_display': float(total*1000),
                'budget_fits_1_6_mHa': total <= Fraction(1, 625),
                'chemical_accuracy_certified': False,
                'missing': ['compiled/proved correction and combined block encoding',
                            'outer-combination rounding certificate', 'physical gate synthesis',
                            'ground-state preparation/selection', 'implemented QPE failure accounting']},
            'conditional_qpe_plan': qpe_plan(base_norm+Fraction(enc['normalization_Ha']))}
    return {'schema': 'femoco-corrected-compression-v1', 'instance': name,
        'hypothesis': 'sparse residual correction might retain the compact DFTHC resource advantage',
        'cheapest_refutation': 'count retained entries under a certified weighted entrywise error bound',
        'target': {'integrals_sha256': source['integrals_sha256'], 'factors_sha256': source['factors_sha256'],
                   'core_policy': source['core_policy'], 'electron_number': spec['eta'], 'spatial_orbitals': spec['n'],
                   'scope': 'exact FCIDUMP decimals, fixed electron sector; not complete experimental chemistry'},
        'arithmetic': {'python_flint': flint.__version__, 'flint': flint.__FLINT_VERSION__, 'numpy': np.__version__,
                       'python': sys.version.split()[0], 'precision_bits': precision,
                       'proposal_reproducibility': 'eigenpair bytes can vary with BLAS; every proposal is re-certified'},
        'checker_sha256': {p.name: sha256(p) for p in sorted(HERE.glob('*.py'))},
        'base_payload_sha256': sha256(basepath), 'base_normalization_Ha': str(base_norm),
        'rotation_and_preprocessing': rotation, 'compression': proof, 'scenarios': scenarios,
        'conclusion_scope': 'certified Hamiltonian approximations; correction resource estimates are analytical, not measured full walks'}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--instance', required=True, choices=('reiher', 'li'))
    p.add_argument('--data', required=True, type=pathlib.Path)
    p.add_argument('--artifacts', required=True, type=pathlib.Path)
    p.add_argument('--out', required=True, type=pathlib.Path)
    p.add_argument('--precision', type=int, default=128)
    args = p.parse_args()
    args.out.unlink(missing_ok=True)
    result = run(args.instance, args.data, args.artifacts, args.precision)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    temp = args.out.with_suffix('.tmp')
    temp.write_text(json.dumps(result, indent=2)+'\n')
    temp.replace(args.out)
    print(json.dumps({'instance': args.instance, 'compression': result['compression'],
                      'budgets_mHa': {k: v['conditional_encoded_budget']['total_with_QPE_mHa_display'] for k, v in result['scenarios'].items()}}))


if __name__ == '__main__':
    main()
