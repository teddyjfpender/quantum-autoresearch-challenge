#!/usr/bin/env python3
"""Recompute interval certificates, then fail closed on chemical-accuracy claims."""
from __future__ import annotations

import argparse
import base64
from fractions import Fraction
import json
import pathlib
import struct
import sys

import flint
from flint import arb, arb_mat, ctx

from inputs import HERE, ROOT, factors, integrals, payload, sha256
from bounds import (ball, determinant_energy, determinant_witnesses, enclosure,
                    factor_rows, fit_norm, precision_sweep, projector, qpe_plan,
                    rational, residual, rotation_bound, shifted_one_body, upper)


def budget(fit, rotation, coefficient=Fraction(1, 5000), phase=Fraction(1, 1000)):
    components = dict(fit=Fraction(fit), rotation_and_preprocessing=Fraction(rotation),
                      coefficient=Fraction(coefficient), phase_estimation=Fraction(phase))
    if any(v < 0 for v in components.values()):
        raise ValueError('negative error bound')
    total = sum(components.values())
    return {'components_Ha': {k: str(v) for k, v in components.items()},
            'total_upper_Ha': str(total), 'total_upper_mHa_display': float(total*1000),
            'threshold_Ha': '1/625', 'energy_budget_met': total <= Fraction(1, 625),
            'chemical_accuracy_certified': False,
            'scope': 'conditional on an accepted rigorous lane map, correct logical circuit, and successful ground-state phase estimation',
            'missing_unconditional_obligations': ['ground-state preparation/selection probability',
                                                'implemented phase-estimation and gate-synthesis error/failure accounting'],
            'coefficient_rule': '2 * (exact coefficient 1-norm <= 1/10000 Ha); includes nested-walk identity correction'}


def variational(spec, directory):
    cert = json.loads((directory/'certificate.json').read_text())
    if cert['spec_json_sha256'] != sha256(directory/'spec.json') or cert['payload_sha256'] != sha256(directory/'sa.bin'):
        raise ValueError('variational witness belongs to another spec')
    state = cert['ground_energy_upper_bound']['state']
    columns, blobs, occupations = [], [], []
    for spin in ('alpha', 'beta'):
        shape = state[f'c_{spin}_shape']
        if len(shape) != 2 or shape[0] != spec['n'] or not 0 <= shape[1] <= spec['n']:
            raise ValueError('invalid orbital dimensions')
        data = base64.b64decode(state[f'c_{spin}_b64'], validate=True)
        if len(data) != shape[0]*shape[1]*8:
            raise ValueError('invalid orbital byte count')
        vals = struct.unpack('<' + str(shape[0]*shape[1]) + 'd', data)
        a = arb_mat(shape[0], shape[1], [arb(v) for v in vals])
        if any(not a[i, j].is_finite() for i in range(shape[0]) for j in range(shape[1])):
            raise ValueError('nonfinite orbital')
        columns.append(projector(a))
        blobs.append(data)
        occupations.append(shape[1])
    if sum(occupations) != spec['eta']:
        raise ValueError('variational witness is in wrong electron sector')
    import hashlib
    digest = hashlib.sha256(b''.join(blobs)).hexdigest()
    if digest != state['orbitals_sha256']:
        raise ValueError('orbital witness digest mismatch')
    energy = determinant_energy(spec, *columns)
    es = sum((Fraction.from_float(abs(v)) for v in spec['e']), Fraction(0))
    # The payload scalar is an exact binary64 dyadic.
    sos = rational(spec['const']) - es
    gap = upper(energy) - sos
    if gap < 0:
        raise ValueError('variational energy below SOS lower bound')
    return {'determinant_energy': enclosure(energy), 'ground_energy_upper_Ha': str(upper(energy)),
            'SOS_lower_Ha': str(sos), 'gap_upper_Ha': str(gap),
            'occupations': occupations, 'orbitals_sha256': digest,
            'floating_point_allowance_Ha': '0',
            'scope': 'stored Hamiltonian only; does not establish target-Hamiltonian fidelity'}


def certify(name, data, precision=128):
    if precision < 96:
        raise ValueError('use at least 96-bit interval arithmetic')
    ctx.prec = precision
    sources = json.loads((HERE/'sources.json').read_text())
    source = sources['instances'][name]
    directory = ROOT/'specs'/f'{name}-sa-v1'
    spec = payload(directory)
    params = factors(data/(name+'.pickle'), source, spec)
    print(f'{name}: reading pinned target integrals', file=sys.stderr, flush=True)
    h, g, core = integrals(data/source['integrals'], source, spec['n'], spec['eta'])
    rows, us = factor_rows(params, spec)
    hshift, const, hs = shifted_one_body(h, g, core, rows, params, spec)
    print(f'{name}: enclosing complete DFTHC residual', file=sys.stderr, flush=True)
    d = residual(rows, g, hs, spec['n'])
    fit = fit_norm(d, spec['n'])
    witnesses = determinant_witnesses(d, spec['n'], spec['eta'])
    del d, g
    rotation = rotation_bound(spec, us, hshift, const)
    print(f'{name}: validating rotation proposals and Slater witness', file=sys.stderr, flush=True)
    sweep = precision_sweep(spec, us, hshift, const, (20, 24, 26, 28))
    variational_report = variational(spec, directory)
    norm = sum((Fraction.from_float(abs(e)) for e in spec['e']), Fraction(0))
    b = spec['b']
    norm += sum(((Fraction.from_float(abs(wb)) + sum((Fraction.from_float(abs(w))
                  for w in spec['w'][i*b:(i+1)*b]), Fraction(0)))**2/4
                 for i, wb in enumerate(spec['wb'])), Fraction(0))
    if norm != Fraction(spec['meta']['lambda']['exact']):
        raise ValueError('normalization metadata mismatch')
    gap = Fraction(variational_report['gap_upper_Ha'])
    effective = upper(ball(gap*(2*norm-gap)).sqrt()) if gap <= norm else norm
    variational_report['spectral_amplification_effective_norm_upper_Ha'] = str(effective)
    result = {
        'schema': 'femoco-energy-certificate-v1', 'instance': name,
        'arithmetic': {'library': 'python-flint', 'version': flint.__version__, 'precision_bits': precision,
                       'acceptance_uses': 'exact rational outward endpoints; display floats never decide acceptance'},
        'target': {'integrals_sha256': source['integrals_sha256'], 'integrals': source['integrals'],
                   'coefficient_interpretation': 'exact FCIDUMP decimal values',
                   'core_policy': source['core_policy'], 'electrons': spec['eta'],
                   'spatial_orbitals': spec['n'], 'sector': 'fixed electron number, all spin sectors',
                   'experimental_molecule_error_certified': False},
        'inputs': {'factors_sha256': source['factors_sha256'], 'spec_json_sha256': sha256(directory/'spec.json'),
                   'sa_bin_sha256': sha256(directory/'sa.bin'),
                   'slater_certificate_sha256': sha256(directory/'certificate.json'),
                   'sources_sha256': sha256(HERE/'sources.json')},
        'checker_sha256': {p.name: sha256(p) for p in sorted(HERE.glob('*.py'))},
        'factorization': {'operator_norm_upper_Ha': str(upper(fit)),
                          'triangle_bound_expression_enclosure': enclosure(fit),
                          'enclosure_scope': 'interval for the triangle-bound expression, not an interval for the operator norm',
                          'witnesses': witnesses,
                          'chemical_accuracy_operator_norm_excluded_even_after_scalar_shift':
                              Fraction(witnesses['any_scalar_shift_norm_lower_Ha']) > Fraction(1, 625),
                          'ground_energy_accuracy_decided': False},
        'rotation_and_preprocessing': rotation,
        'rotation_precision_proposals': sweep,
        'stored_hamiltonian_variational_certificate': variational_report,
        'conditional_qpe_plan': qpe_plan(norm),
        'end_to_end': budget(upper(fit), rotation['upper_Ha']),
        'full_quantum_equivalence_scope': 'See PR #7; this checker proves Hamiltonian inequalities, not circuit equivalence.',
    }
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--instance', choices=('reiher', 'li'), required=True)
    p.add_argument('--data', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--precision', type=int, default=128)
    p.add_argument('--require-chemical-accuracy', action='store_true',
                   help='exit nonzero unless the full unconditional claim has been established')
    args = p.parse_args()
    # Never leave an old success report after a failed recomputation.
    args.out.unlink(missing_ok=True)
    result = certify(args.instance, args.data, args.precision)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.out.with_suffix(args.out.suffix+'.tmp')
    temporary.write_text(json.dumps(result, indent=2)+'\n')
    temporary.replace(args.out)
    print(json.dumps({'instance': args.instance, 'energy_budget_met': result['end_to_end']['energy_budget_met'],
                      'chemical_accuracy_certified': result['end_to_end']['chemical_accuracy_certified'],
                      'shift_independent_fit_norm_lower_mHa': result['factorization']['witnesses']['any_scalar_shift_norm_lower_mHa_display']}))
    return 2 if args.require_chemical_accuracy and not result['end_to_end']['chemical_accuracy_certified'] else 0


if __name__ == '__main__':
    raise SystemExit(main())
