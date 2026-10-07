"""Pinned scientific inputs; no execution of published Python or unrestricted pickle."""
from __future__ import annotations

import hashlib
import io
import json
import math
import pathlib
import pickle
import re
import struct

import numpy as np
from flint import arb, arb_mat

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def sha256(path):
    h = hashlib.sha256()
    with pathlib.Path(path).open('rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()


def checked(path, expected):
    if sha256(path) != expected:
        raise ValueError(f'input SHA-256 mismatch: {pathlib.Path(path).name}')
    return pathlib.Path(path)


def _jax_array(fun, args, state, extra):
    if fun is not np._core.multiarray._reconstruct:
        raise ValueError('unsupported array reconstruction')
    out = fun(*args)
    out.__setstate__(state)
    if out.dtype.kind != 'f' or out.dtype.itemsize not in (4, 8):
        raise ValueError('unsupported scientific array dtype')
    return out


class ArrayReader(pickle.Unpickler):
    def find_class(self, module, name):
        allowed = {
            ('jax._src.array', '_reconstruct_array'): _jax_array,
            ('numpy.core.multiarray', '_reconstruct'): np._core.multiarray._reconstruct,
            ('numpy.core.numeric', '_frombuffer'): np._core.numeric._frombuffer,
            ('numpy', 'ndarray'): np.ndarray,
            ('numpy', 'dtype'): np.dtype,
        }
        if (module, name) not in allowed:
            raise ValueError(f'unsupported pickle global: {module}.{name}')
        return allowed[module, name]


def factors(path, source, spec):
    data = checked(path, source['factors_sha256']).read_bytes()
    params = ArrayReader(io.BytesIO(data)).load()['params_best']
    n, r, b, c = (spec[k] for k in ('n', 'r', 'b', 'c'))
    shapes = {'U': (r, b, n), 'W': (r, b, c), 'H2_I': (r, c),
              'Hsym_lower': (n*(n+1)//2,), 'H1_I': ()}
    for k, shape in shapes.items():
        a = np.asarray(params[k])
        if a.shape != shape or a.dtype != np.dtype('float32') or not np.isfinite(a).all():
            raise ValueError(f'invalid published factor {k}')
    if not np.array_equal(params['W'].transpose(0, 2, 1).ravel(), spec['w']):
        raise ValueError('published W differs from circuit payload')
    if not np.array_equal(params['H2_I'].ravel(), spec['wb']):
        raise ValueError('published H2_I differs from circuit payload')
    return params


def payload(directory):
    directory = pathlib.Path(directory)
    meta = json.loads((directory / 'spec.json').read_text())
    data = checked(directory / 'sa.bin', meta['payload']['sha256']).read_bytes()
    if data[:8] != b'FEMOSAS1':
        raise ValueError('unsupported payload')
    version, n, r, b, c, beta, eta = struct.unpack_from('<7I', data, 8)
    if version != 1 or not 2 <= n <= 256 or min(r, b, c) < 1 or not 3 <= beta <= 32:
        raise ValueError('invalid payload dimensions')
    if (n, r, b, c, beta, eta) != tuple(meta[k] for k in
            ('spatial_orbitals', 'R', 'B', 'C', 'rotation_bits', 'electrons')):
        raise ValueError('metadata and payload dimensions differ')
    if not 0 <= eta <= 2*n:
        raise ValueError('invalid electron sector')
    at = 36

    def read(code, count):
        nonlocal at
        out = struct.unpack_from('<' + str(count) + code, data, at)
        at += struct.calcsize(code) * count
        return out

    const, = read('d', 1)
    e, ea = read('d', n), read('I', n*(n-1))
    wb, w = read('d', r*c), read('d', r*c*b)
    angles = read('I', r*b*(n-1))
    if at != len(data) or any(a >= 1 << beta for a in (*ea, *angles)):
        raise ValueError('invalid payload length/angles')
    if not all(math.isfinite(v) for v in (const, *e, *wb, *w)):
        raise ValueError('nonfinite coefficient')
    return dict(n=n, r=r, b=b, c=c, beta=beta, eta=eta, const=arb(const),
                e=e, ea=ea, wb=wb, w=w, angles=angles, meta=meta)


def pair(p, q):
    p, q = max(p, q), min(p, q)
    return p*(p+1)//2 + q


def pairs(n):
    return [(p, q) for p in range(n) for q in range(p+1)]


def integrals(path, source, n, eta):
    """Exact decimal FCIDUMP target, enclosed by Arb (not float32 JAX inputs).

    Chemists' (pq|rs), packed symmetric spatial pairs. Only the fixed, hashed
    restricted-orbital files are accepted. Li's scalar core is deliberately zero.
    """
    checked(path, source['integrals_sha256'])
    m = n*(n+1)//2
    g, h, core = arb_mat(m, m), arb_mat(n, n), arb(0)
    with pathlib.Path(path).open() as f:
        header = ''
        for line in f:
            header += line
            if '&END' in line:
                break
        if not re.search(rf'\bNORB\s*=\s*{n}\s*,', header) or not re.search(rf'\bNELEC\s*=\s*{eta}\s*,', header):
            raise ValueError('FCIDUMP sector/dimensions mismatch')
        for line in f:
            value, *indices = line.split()
            if len(indices) != 4:
                raise ValueError('invalid FCIDUMP entry')
            v = arb(value.replace('D', 'E'))
            p, q, s, t = map(int, indices)
            if not all(0 <= x <= n for x in (p, q, s, t)) or not v.is_finite():
                raise ValueError('invalid FCIDUMP coefficient/index')
            if min(p, q, s, t) > 0:
                a, b = pair(p-1, q-1), pair(s-1, t-1)
                g[a, b] = g[b, a] = v
            elif p > 0 and q > 0 and s == t == 0:
                h[p-1, q-1] = h[q-1, p-1] = v
            elif p == q == s == t == 0:
                core = v
            else:
                raise ValueError('unsupported FCIDUMP record')
    if source['core_policy'] == 'zero':
        core = arb(0)
    elif source['core_policy'] != 'fcidump':
        raise ValueError('unknown core policy')
    return h, g, core
