"""Validated finite-dimensional inequalities; all proof arithmetic uses Arb.

NumPy is used only for proposed higher-precision eigenvectors. Every proposal is
subsequently checked by the same interval residual calculation.
"""
from __future__ import annotations

from fractions import Fraction
import math

from flint import arb, arb_mat

from inputs import pair, pairs


def rational(x):
    """Serialize an exact Arb endpoint without a float/decimal round trip."""
    if not x.is_finite() or not x.is_exact():
        raise ValueError('expected a finite exact endpoint')
    mantissa, exponent = x.man_exp()
    return Fraction(int(mantissa)) * Fraction(2)**int(exponent)


def upper(x):
    return rational(x.upper())


def lower(x):
    return rational(x.lower())


def ball(q):
    q = Fraction(q)
    return arb(q.numerator) / q.denominator


def dyadic_upper(q, bits=192):
    """Round a rational upward to a multiple of 2^-bits.

    Sums of thousands of certified quotients would otherwise carry products of
    unrelated denominators; outward dyadic rounding keeps every bound valid.
    """
    q = Fraction(q)
    return Fraction(-((-q.numerator*(1 << bits))//q.denominator), 1 << bits)


def enclosure(x):
    lo, hi = lower(x), upper(x)
    return {'lower_Ha': str(lo), 'upper_Ha': str(hi),
            'display_Ha': float(x.mid()), 'radius_Ha': float(x.rad())}


def trace(a):
    return sum((a[i, i] for i in range(a.nrows())), arb(0))


def trace_product(a, b):
    return sum((a[i, j]*b[j, i] for i in range(a.nrows())
                for j in range(a.ncols())), arb(0))


def symmetric(v, n):
    out = arb_mat(n, n)
    for k, (p, q) in enumerate(pairs(n)):
        out[p, q] = out[q, p] = v[k]
    return out


def network(words, beta):
    """Givens chain applied to orbital 0, with the payload's +sin convention."""
    out, tail = [], arb(1)
    for word in words:
        angle = arb.pi() * (2*int(word)) / (1 << beta)
        out.append(tail * angle.cos())
        tail *= angle.sin()
    return out + [tail]


def vectors(words, count, n, beta):
    return [network(words[i*(n-1):(i+1)*(n-1)], beta) for i in range(count)]


def normalized(raw):
    values = [arb(float(x)) for x in raw]
    norm = sum((x*x for x in values), arb(0)).sqrt()
    if not norm > 0:
        raise ValueError('zero or unresolved factor norm')
    return [x/norm for x in values]


def factor_rows(params, spec):
    n, r, b, c = (spec[k] for k in ('n', 'r', 'b', 'c'))
    us = [normalized(params['U'][i, j]) for i in range(r) for j in range(b)]
    # These matrices are only RC by N(N+1)/2, not the full many-body matrix.
    blocks = []
    for i in range(r):
        products = arb_mat([[us[i*b+j][p]*us[i*b+j][q] for p, q in pairs(n)]
                            for j in range(b)])
        weights = arb_mat([[arb(float(params['W'][i, j, k])) for j in range(b)]
                           for k in range(c)])
        blocks.append(weights * products)
    rows = arb_mat(r*c, n*(n+1)//2)
    for i, block in enumerate(blocks):
        for j in range(c):
            for k in range(rows.ncols()):
                rows[i*c+j, k] = block[j, k]
    return rows, us


def shifted_one_body(h, g, core, rows, params, spec):
    """Construct the ideal DFTHC Hamiltonian algebraically, before eigenpairs."""
    n, eta = spec['n'], spec['eta']
    hdf = arb_mat(h)
    for p in range(n):
        for q in range(n):
            hdf[p, q] += sum((g[pair(k, k), pair(p, q)]
                              - g[pair(p, k), pair(k, q)]/2 for k in range(n)), arb(0))
    cdf = core + trace(hdf) - sum((g[pair(p, p), pair(q, q)]
                                  for p in range(n) for q in range(n)), arb(0))/2
    hshift = arb_mat(hdf)
    hi = arb(float(params['H1_I']))
    hs = [arb(float(x)) for x in params['Hsym_lower']]
    for a, (p, q) in enumerate(pairs(n)):
        value = hdf[p, q] + (n-eta)*hs[a]/2
        value -= sum((arb(wb)*rows[k, a] for k, wb in enumerate(spec['wb'])), arb(0))
        if p == q:
            value += hi
        hshift[p, q] = hshift[q, p] = value
    const = cdf + (n-eta)*hi - sum((arb(x)**2 for x in spec['wb']), arb(0))/2
    return hshift, const, hs


def residual(rows, g, hs, n):
    d = rows.transpose()*rows - g
    diag = [pair(p, p) for p in range(n)]
    for p in diag:
        for a in range(d.ncols()):
            d[p, a] -= hs[a]/2
            d[a, p] -= hs[a]/2
    return d


def fit_norm(d, n):
    """R=1/2 sum_ab D_ab A_a A_b; ||A_pp||<=1, ||A_pq||<=2.

    Packed pairs collect BOTH ordered off-diagonal E_pq terms. Symmetry lets us
    visit only one triangle, preserving the factor of 1/2 on diagonal entries.
    """
    weights = [1 if p == q else 2 for p, q in pairs(n)]
    total = arb(0)
    for a, wa in enumerate(weights):
        total += abs(d[a, a]) * wa*wa/2
        for b in range(a):
            total += abs(d[a, b]) * wa*weights[b]
    return total


def fit_expectation(d, alpha, beta, n):
    for occ in (alpha, beta):
        if len(set(occ)) != len(occ) or any(not 0 <= x < n for x in occ):
            raise ValueError('invalid determinant occupation')
    a, b = set(alpha), set(beta)
    z = [int(p in a)+int(p in b)-1 for p in range(n)]
    mean = sum((d[pair(p, p), pair(q, q)]*z[p]*z[q]
                for p in range(n) for q in range(n)), arb(0))
    variance = sum((d[pair(p, q), pair(p, q)]
                    for occupied in (a, b) for p in occupied
                    for q in range(n) if q not in occupied), arb(0))
    return (mean+variance)/2


def determinant_witnesses(d, n, eta):
    na, nb = (eta+1)//2, eta//2
    choices = [('prefix', list(range(n))), ('suffix', list(reversed(range(n)))),
               ('even-first', list(range(0, n, 2))+list(range(1, n, 2))),
               ('odd-first', list(range(1, n, 2))+list(range(0, n, 2)))]
    states = []
    for name, order in choices:
        alpha, beta = order[:na], order[:nb]
        value = fit_expectation(d, alpha, beta, n)
        states.append({'name': name, 'alpha': alpha, 'beta': beta,
                       'fit_expectation': enclosure(value)})
    lo = min(Fraction(s['fit_expectation']['upper_Ha']) for s in states)
    hi = max(Fraction(s['fit_expectation']['lower_Ha']) for s in states)
    # For EVERY scalar shift s, max_i |<i|R-sI|i>| >= (max d_i-min d_i)/2.
    obstruction = max(Fraction(0), (hi-lo)/2)
    return {'states': states, 'any_scalar_shift_norm_lower_Ha': str(obstruction),
            'any_scalar_shift_norm_lower_mHa_display': float(obstruction*1000),
            'ground_energy_error_lower_bound': None}


def nuclear_norm_upper(a):
    """Certified upper bound on the nuclear norm of a real ball matrix.

    Three valid bounds are enclosed and the smallest is returned: the entrywise
    l1 sum, sqrt(N) times the Frobenius norm, and an approximate-diagonalization
    certificate. For any invertible Q,
        ||A||_* <= ||Q^T A Q||_* / sigma_min(Q)^2
               <= sum_jk |(Q^T A Q)_jk| / (1 - ||Q^T Q - I||_F),
    valid whenever the denominator is positive. Q is a float64 eigenvector
    proposal used only as exact dyadic data; the enclosure never trusts it.
    A ball enclosing a nonnegative value can extend below zero. Taking its
    upper endpoint before sqrt is sound and avoids NaN for an exact-zero
    residual whose trigonometric enclosure has a tiny radius.
    """
    import numpy as np
    n = a.nrows()
    if a.ncols() != n:
        raise ValueError('square matrix expected')
    entry = sum((abs(a[p, q]) for p in range(n) for q in range(n)), arb(0))
    frob = (n*sum((a[p, q]**2 for p in range(n) for q in range(n)), arb(0))).upper().sqrt()
    best = min(upper(entry), upper(frob))
    mid = np.array([[float(a[p, q].mid()) for q in range(n)] for p in range(n)])
    if not np.all(np.isfinite(mid)):
        return best
    _, vecs = np.linalg.eigh((mid+mid.T)/2)
    q = arb_mat(n, n, [float(x) for x in vecs.ravel()])
    rotated = q.transpose()*a*q
    l1 = sum((abs(rotated[j, k]) for j in range(n) for k in range(n)), arb(0))
    gram = q.transpose()*q
    for j in range(n):
        gram[j, j] -= 1
    defect = upper(sum((gram[j, k]**2 for j in range(n) for k in range(n)), arb(0)).upper().sqrt())
    if defect < 1:
        best = min(best, dyadic_upper(upper(l1)/(1-defect)))
    return best


def one_body_norm(a):
    # Eigenvalues lambda_j give sum lambda_j (n_j,up+n_j,down-1), whose
    # Fock-space norm is exactly the nuclear norm of the symmetric matrix.
    return nuclear_norm_upper(a)


def spectral_fit_norm(d, n):
    """Second certified bound on ||R||, through a signed square factorization.

    A float64 eigendecomposition proposes rows w_t = sqrt|lam_t| l_t and signs.
    With exact dyadic copies, D = sum_t s_t w_t w_t^T + Delta exactly, so
        R = (1/2) sum_t s_t G_t^2 + (1/2) sum_ab Delta_ab A_a A_b,
        G_t = sum_pq W_t,pq Et_pq,   ||G_t|| <= ||W_t||_*,
    where W_t is the symmetric spatial unpacking of w_t. Delta is enclosed and
    bounded by the entrywise rule; the remainder is not assumed small.
    """
    import numpy as np
    m = d.nrows()
    if d.ncols() != m or m != n*(n+1)//2:
        raise ValueError('residual dimensions')
    mid = np.array([[float(d[i, j].mid()) for j in range(m)] for i in range(m)])
    if not np.all(np.isfinite(mid)):
        raise ValueError('nonfinite residual')
    values, vectors_ = np.linalg.eigh((mid+mid.T)/2)
    rows = vectors_*np.sqrt(np.abs(values))
    remainder = arb_mat(d)
    for sign in (-1, 1):
        subset = rows[:, values*sign > 0]
        if subset.shape[1]:
            w = arb_mat(m, subset.shape[1], [float(x) for x in subset.ravel()])
            remainder -= sign*(w*w.transpose())
            del w
    tail = upper(fit_norm(remainder, n))
    del remainder
    squares = Fraction(0)
    ij = pairs(n)
    for t in range(m):
        unpacked = arb_mat(n, n)
        for a, (p, q) in enumerate(ij):
            unpacked[p, q] = unpacked[q, p] = arb(float(rows[a, t]))
        nuc = nuclear_norm_upper(unpacked)
        squares += nuc*nuc/2
    squares, tail = dyadic_upper(squares, 128), dyadic_upper(tail, 128)
    total = squares+tail
    return {'upper_Ha': str(total), 'upper_mHa_display': float(total*1000),
            'signed_squares_upper_Ha': str(squares), 'remainder_triangle_upper_Ha': str(tail),
            'factors': m, 'positive_factors': int(np.sum(values > 0)),
            'rule': '(1/2) sum_t ||W_t||_*^2 + (1/2) sum_ab |Delta_ab| k_a k_b; eigensolver output is data, never trusted'}


def rotation_bound(spec, us, hshift, const, ea=None, angles=None, beta=None, eigenvalues=None):
    n, r, b, c = (spec[k] for k in ('n', 'r', 'b', 'c'))
    beta = spec['beta'] if beta is None else beta
    vs = vectors(spec['ea'] if ea is None else ea, n, n, beta)
    quantized = vectors(spec['angles'] if angles is None else angles, r*b, n, beta)
    es = spec['e'] if eigenvalues is None else eigenvalues
    hhat = arb_mat(n, n)
    for p in range(n):
        for q in range(p+1):
            hhat[p, q] = hhat[q, p] = sum((arb(e)*v[p]*v[q] for e, v in zip(es, vs)), arb(0))
    eps = []
    for u, v in zip(us, quantized):
        dot = sum((x*y for x, y in zip(u, v)), arb(0))
        radicand = (1-dot*dot).upper()
        if radicand < 0:
            raise ValueError('unit vector overlap inconsistent')
        eps.append(2*radicand.sqrt())
    square = Fraction(0)
    for i in range(r):
        exact = arb_mat([[us[i*b+t][p] for t in range(b)] for p in range(n)])
        rounded = arb_mat([[quantized[i*b+t][p] for t in range(b)] for p in range(n)])
        for j in range(c):
            k = i*c+j
            weights = spec['w'][k*b:(k+1)*b]
            # Triangle: sum_b |w_b| ||Et(u_b)-Et(v_b)||, each at most 2 sqrt(1-(u.v)^2).
            delta = sum((abs(arb(w))*eps[i*b+t] for t, w in enumerate(weights)), arb(0))
            # The same difference is the centered one-body operator of the
            # symmetric matrix sum_b w_b (u_b u_b^T - v_b v_b^T), so its norm is
            # also at most that matrix's nuclear norm. Both bounds are valid.
            scale = arb_mat(b, b)
            for t, w in enumerate(weights):
                scale[t, t] = arb(w)
            generator = exact*scale*exact.transpose() - rounded*scale*rounded.transpose()
            delta = min(upper(delta), nuclear_norm_upper(generator))
            size = abs(arb(spec['wb'][k])) + sum((abs(arb(w)) for w in weights), arb(0))
            # ||O||, ||Ohat|| <= S, so ||O^2-Ohat^2||/2 <= S delta.
            square += upper(size)*delta
    square = dyadic_upper(square, 128)
    one = dyadic_upper(one_body_norm(hhat-hshift), 128)
    scalar = abs(spec['const']-const)
    total = one + square + upper(scalar)
    return {'upper_Ha': str(total), 'upper_mHa_display': float(total*1000),
            'one_body_upper_Ha': str(one), 'squares_upper_Ha': str(square),
            'scalar_upper_Ha': str(upper(scalar)), 'rotation_bits': beta}


def proposal_words(values, beta):
    """Untrusted binary64 proposal. Quantized networks are validated afterwards."""
    v = [float(x.mid()) for x in values]
    out = []
    for j in range(len(v)-1):
        tail = math.sqrt(sum(x*x for x in v[j+1:]))
        if j == len(v)-2:
            tail = v[-1]
        theta = math.atan2(tail, v[j])
        out.append(round(theta*(1 << beta)/(2*math.pi)) % (1 << beta))
    return out


def precision_sweep(spec, us, hshift, const, bits=(20, 24, 28)):
    import numpy as np
    n = spec['n']
    # Eigensolver output is data, never a certificate. Residuals check it.
    eigs, eigvecs = np.linalg.eigh(np.array([[float(hshift[i, j].mid())
                                            for j in range(n)] for i in range(n)]))
    ev = [[arb(float(eigvecs[i, j])) for i in range(n)] for j in range(n)]
    out = []
    for beta in bits:
        ea = [a for v in ev for a in proposal_words(v, beta)]
        angles = [a for v in us for a in proposal_words(v, beta)]
        result = rotation_bound(spec, us, hshift, const, ea, angles, beta,
                                tuple(float(e) for e in eigs))
        result['implemented_circuit'] = False
        result['resource_counts'] = None
        out.append(result)
    return out


def projector(columns):
    # Stored binary64 columns need not be orthonormal. This is the exact Slater
    # projector of their span, enclosed using validated matrix inversion.
    return columns * (columns.transpose()*columns).inv() * columns.transpose()


def determinant_energy(spec, pa, pb):
    n, r, b, c = (spec[k] for k in ('n', 'r', 'b', 'c'))
    ev = vectors(spec['ea'], n, n, spec['beta'])
    us = vectors(spec['angles'], r*b, n, spec['beta'])
    energy = arb(spec['const'])
    for e, u in zip(spec['e'], ev):
        occupation = sum((u[p]*(pa[p, q]+pb[p, q])*u[q]
                          for p in range(n) for q in range(n)), arb(0))
        energy += arb(e)*(occupation-1)
    for i in range(r):
        products = arb_mat([[u[p]*u[q] for p, q in pairs(n)] for u in us[i*b:(i+1)*b]])
        w = arb_mat([[arb(x) for x in spec['w'][(i*c+j)*b:(i*c+j+1)*b]] for j in range(c)])
        rows = w*products
        for j in range(c):
            l = symmetric([rows[j, a] for a in range(rows.ncols())], n)
            mean = arb(spec['wb'][i*c+j]) - trace(l)
            variance = arb(0)
            for p in (pa, pb):
                pl = p*l
                mean += trace(pl)
                variance += trace_product(pl, l) - trace_product(pl, pl)
            energy += (mean*mean+variance)/2
    return energy


def qpe_tail_bound(tail, bits):
    """Upper bound on the probability that m-bit textbook QPE of an exact
    eigenstate lands more than `tail` bins (circular) from the true phase.

    With delta in [0,1) the fractional bin offset and l the signed bin error,
    p(l) = sin^2(pi delta) / (2^2m sin^2(pi y)), y = (l-delta)/2^m, |y| <= 1/2.
    Far region |y| >= 1/8: sin(pi y) >= 2y gives p(l) <= 16/2^2m, at most 2^m
    outcomes, total <= 2^(4-m). Near region: sin x >= x - x^3/6 gives
    p(l) <= c/(l-delta)^2 with c = 1/(pi^2 (1-pi^2/384)^2). Failure needs
    |l-delta| > tail, so l >= tail+1 or l <= -tail, and the integral bound on
    those two sums gives c (1/(tail-delta) + 1/(tail-1+delta)), a convex
    function of delta, at most c (1/tail + 1/(tail-1)). The classical
    1/[2(tail-1)] bound (Jordan's inequality alone) is also returned.
    """
    if tail < 2 or bits < 1:
        raise ValueError('invalid tail/register')
    pi = arb.pi()
    c = 1/(pi*pi*(1-pi*pi/384)**2)
    refined = upper(c*ball(Fraction(1, tail)+Fraction(1, tail-1))) + Fraction(16, 1 << bits)
    return min(refined, Fraction(1, 2*(tail-1)))


def qpe_plan(normalization, energy_error=Fraction(1, 1000), failure=Fraction(1, 100)):
    """Conservative textbook QPE plan, conditional on an exact eigenstate.

    The register is the smallest one for which some tail length L satisfies
    both qpe_tail_bound(L, m) <= failure and 2 pi Lambda L / 2^m <= error:
    E=offset+Lambda*cos(theta) is Lambda-Lipschitz, so an outcome within L
    bins has energy error at most the second quantity. No spectral-
    amplification speedup or preparation/synthesis success is assumed here.
    """
    normalization, energy_error, failure = map(Fraction, (normalization, energy_error, failure))
    if normalization <= 0 or energy_error <= 0 or not 0 < failure < 1:
        raise ValueError('invalid QPE parameters')
    widest = math.ceil(1/(2*failure)) + 1
    bits = 1
    while True:
        bits += 1
        for tail in range(2, widest+1):
            if qpe_tail_bound(tail, bits) <= failure and \
                    upper(2*arb.pi()*ball(normalization)*tail/(1 << bits)) <= energy_error:
                return {'energy_error_Ha': str(energy_error), 'failure_probability': str(failure),
                        'normalization_Ha': str(normalization),
                        'phase_register_qubits': bits, 'controlled_walk_uses': (1 << bits)-1,
                        'tail_bins': tail, 'tail_probability_upper': str(qpe_tail_bound(tail, bits)),
                        'tail_rule': 'min(c(1/L+1/(L-1))+2^(4-m), 1/[2(L-1)]), c=1/(pi^2(1-pi^2/384)^2)',
                        'conditional_on': 'exact eigenstate and exact controlled walk/QFT',
                        'includes_state_preparation_or_gate_synthesis': False}
