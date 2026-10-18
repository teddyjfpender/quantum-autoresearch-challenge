#!/usr/bin/env python3
"""Propose and certify a signed double factorization of the DFTHC residual."""
from __future__ import annotations
import argparse
from fractions import Fraction
import json
import pathlib
import sys
import time

import numpy as np
import flint
from flint import arb, arb_mat, ctx
from inputs import HERE, ROOT, factors, integrals, pairs, payload, sha256
from bounds import (ball, factor_rows, fit_norm, network, one_body_norm,
                    proposal_words, residual, shifted_one_body, upper)
from corrected import propose_payload


def propose(d, n, budget=0.00008, beta=26):
    """Numerical proposal only. The actual certificate never trusts eigh/BLAS."""
    m = d.nrows()
    a = np.array([[float(d[i, j].mid()) for j in range(m)] for i in range(m)])
    values, vectors = np.linalg.eigh(a)
    order = np.argsort(np.abs(values))[::-1]
    values, vectors = values[order], vectors[:, order]
    k = np.array([1 if p == q else 2 for p, q in pairs(n)])
    lo, hi = m//2, m
    while hi-lo > 8:
        rank = (lo+hi)//2
        r = a-(vectors[:, :rank]*values[:rank])@vectors[:, :rank].T
        loss = .5*np.sum(np.abs(r)*k[:, None]*k[None, :])
        if loss < budget:
            hi = rank
        else:
            lo = rank
    rows = (vectors[:, :hi]*np.sqrt(np.abs(values[:hi]))).T
    signs = np.sign(values[:hi]).astype(np.int8)
    eigenvalues, angles = [], []
    ij = np.tril_indices(n)
    for row in rows:
        matrix = np.zeros((n, n))
        matrix[ij] = row
        matrix[ij[1], ij[0]] = row
        ev, u = np.linalg.eigh(matrix)
        eigenvalues.append(ev)
        angles.append([proposal_words([arb(float(x)) for x in u[:, j]], beta) for j in range(n)])
    return rows, signs, np.array(eigenvalues), np.array(angles, dtype=np.uint32)


def certify_factorization(d, rows, signs, n):
    if rows.shape[1] != n*(n+1)//2 or len(signs) != len(rows) or not np.all(np.isin(signs, (-1, 1))):
        raise ValueError('invalid signed factor proposal')
    if not np.all(np.isfinite(rows)):
        raise ValueError('nonfinite factor')
    remainder = arb_mat(d)
    for sign in (-1, 1):
        subset = rows[signs == sign]
        if len(subset):
            a = arb_mat([[arb(float(x)) for x in row] for row in subset])
            remainder -= sign*(a.transpose()*a)
            del a
    return upper(fit_norm(remainder, n))


def certify_rotations(rows, signs, eigenvalues, angles, n, beta=26):
    """Direct matrix residual: includes eigensolver AND angle quantization errors."""
    if eigenvalues.shape != (len(rows), n) or angles.shape != (len(rows), n, n-1):
        raise ValueError('invalid rotation proposal dimensions')
    if not np.all(np.isfinite(eigenvalues)) or np.any(angles >= 1 << beta):
        raise ValueError('nonfinite eigenvalue or out-of-range angle')
    total = Fraction(0)
    normalization = Fraction(0)
    signed_offset = Fraction(0)
    maximum = Fraction(0)
    ij = list(pairs(n))
    for row, sign, es, words in zip(rows, signs, eigenvalues, angles):
        matrix = arb_mat(n, n)
        for x, (p, q) in zip(row, ij):
            matrix[p, q] = matrix[q, p] = arb(float(x))
        # Columns are exact normalized integer-angle networks, enclosed by Arb.
        columns = [network(w, beta) for w in words]
        u = arb_mat([[columns[j][i] for j in range(n)] for i in range(n)])
        ue = arb_mat([[u[i, j]*arb(float(es[j])) for j in range(n)] for i in range(n)])
        delta = one_body_norm(matrix-ue*u.transpose())
        size = sum((Fraction.from_float(abs(float(x))) for x in es), Fraction(0))
        # ||Lhat|| <= size; ||L|| <= size+delta. Half-square perturbation.
        term = size*delta+delta*delta/2
        total += term
        maximum = max(maximum, delta)
        normalization += size*size/4
        # Correction is MINUS the original signed residual.
        signed_offset -= int(sign)*size*size/4
    return {'operator_error_upper_Ha': str(total), 'operator_error_mHa_display': float(total*1000),
            'normalization_Ha': str(normalization), 'normalization_Ha_display': float(normalization),
            'correction_chebyshev_offset_Ha': str(signed_offset),
            'maximum_generator_error_Ha': str(maximum), 'beta': beta}


def run(name, data, artifacts, out, proposal=None, precision=128):
    if precision < 96:
        raise ValueError('minimum interval precision is 96 bits')
    ctx.prec = precision
    out.unlink(missing_ok=True)
    artifacts.mkdir(parents=True, exist_ok=True)
    source = json.loads((HERE/'sources.json').read_text())['instances'][name]
    spec = payload(ROOT/'specs'/f'{name}-sa-v1')
    params = factors(data/(name+'.pickle'), source, spec)
    print(f'{name}: recomputing exact target residual', file=sys.stderr, flush=True)
    h, g, core = integrals(data/source['integrals'], source, spec['n'], spec['eta'])
    original_rows, us = factor_rows(params, spec)
    hshift, const, hs = shifted_one_body(h, g, core, original_rows, params, spec)
    d = residual(original_rows, g, hs, spec['n'])
    del g, original_rows
    base_path = artifacts/(name+'-base.bin')
    base_proof, base_norm = propose_payload(spec, us, hshift, const, 26 if name == 'reiher' else 28, base_path)
    print(f'{name}: proposing signed factors', file=sys.stderr, flush=True)
    if proposal is None:
        rows, signs, es, angles = propose(d, spec['n'])
    else:
        # A saved numerical proposal is untrusted and passes identical checks.
        with np.load(proposal, allow_pickle=False) as a:
            rows, signs, es = a['rows'], a['signs'].astype(np.int8), a['e']
            angles = np.array([[proposal_words([arb(float(x)) for x in mat[:, j]], 26)
                                for j in range(spec['n'])] for mat in a['u']], dtype=np.uint32)
    path = artifacts/(name+'-signed.npz')
    np.savez_compressed(path, rows=rows, signs=signs, eigenvalues=es, angles=angles)
    print(f'{name}: interval-checking {len(rows)} signed factors', file=sys.stderr, flush=True)
    fit = certify_factorization(d, rows, signs, spec['n'])
    del d
    if fit > Fraction(1, 10000):
        raise ValueError(f'signed factorization fails 0.1 mHa allocation: {float(fit)*1000}')
    print(f'{name}: interval-checking {len(rows)*spec["n"]} rotation networks', file=sys.stderr, flush=True)
    rotations = certify_rotations(rows, signs, es, angles, spec['n'])
    if Fraction(rotations['operator_error_upper_Ha']) > Fraction(1, 10000):
        raise ValueError('correction rotations exceed 0.1 mHa allocation')
    total = fit+Fraction(rotations['operator_error_upper_Ha'])+Fraction(base_proof['upper_Ha'])
    result = {'schema': 'femoco-signed-correction-certificate-v1', 'instance': name,
        'target': {'spatial_orbitals': spec['n'], 'electrons': spec['eta'],
                   'integrals_sha256': source['integrals_sha256'], 'factors_sha256': source['factors_sha256'],
                   'core_policy': source['core_policy'], 'sector': 'fixed electron number, all spin sectors'},
        'arithmetic': {'precision_bits': precision, 'python_flint': flint.__version__,
                       'flint': flint.__FLINT_VERSION__, 'numpy': np.__version__, 'python': sys.version.split()[0]},
        'factor_rank': len(rows), 'positive_residual_factors': int(np.sum(signs > 0)),
        'negative_residual_factors': int(np.sum(signs < 0)),
        'factorization_error_upper_Ha': str(fit), 'factorization_error_mHa_display': float(fit*1000),
        'correction_rotations': rotations, 'base_rotation_preprocessing': base_proof,
        'base_normalization_Ha': str(base_norm), 'combined_normalization_Ha': str(base_norm+Fraction(rotations['normalization_Ha'])),
        'hamiltonian_error_upper_Ha': str(total), 'hamiltonian_error_mHa_display': float(total*1000),
        'base_payload_sha256': sha256(base_path), 'signed_data_sha256': sha256(path),
        'checker_sha256': {x: sha256(HERE/x) for x in ('signed_df.py', 'bounds.py', 'inputs.py', 'corrected.py')},
        'chemical_accuracy_certified': False,
        'scope': 'numerical operator certificate; logical combined implementation is checked separately'}
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k: result[k] for k in ('instance', 'factor_rank', 'factorization_error_mHa_display', 'hamiltonian_error_mHa_display')}), flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--instance', required=True, choices=('reiher', 'li'))
    p.add_argument('--data', required=True, type=pathlib.Path)
    p.add_argument('--artifacts', required=True, type=pathlib.Path)
    p.add_argument('--out', required=True, type=pathlib.Path)
    p.add_argument('--proposal', type=pathlib.Path)
    p.add_argument('--precision', type=int, default=128)
    args = p.parse_args()
    run(args.instance, args.data, args.artifacts, args.out, args.proposal, args.precision)
