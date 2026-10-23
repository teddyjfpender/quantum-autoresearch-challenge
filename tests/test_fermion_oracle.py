"""Exact Jordan-Wigner oracle independent of the BDD and Gaussian simulators.

Q[z]/(z^4+1), z=exp(i*pi/4), evaluates eight-angle gates without trigonometry or
tolerances. Spectator occupations and nonadjacent (2j,2j+2) rotations are retained.
These finite regression tests support the general algebraic derivation in OPERATOR.md;
they are not a proof for all dimensions/angle widths.
"""
import itertools
import pathlib
import sys
import unittest
from fractions import Fraction as Q

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] /
                       'challenges/femoco/tools/verification'))
try:
    import exact
except ImportError:
    exact = None


def add(a, b):
    return tuple(x+y for x, y in zip(a, b))


def scale(a, k):
    return tuple(k*x for x in a)


def mul(a, b):
    out = [Q(0)]*4
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            out[(i+j) % 4] += x*y*(-1 if i+j >= 4 else 1)
    return tuple(out)


ZERO, ONE = (Q(0),)*4, (Q(1), Q(0), Q(0), Q(0))


def root(a):
    return tuple(Q((-1 if a % 8 >= 4 else 1) if i == a % 4 else 0) for i in range(4))


def ladder(state, mode, creation):
    occupied = bool(state >> mode & 1)
    if occupied == creation:
        return None
    sign = -1 if (state & ((1 << mode)-1)).bit_count() % 2 else 1
    return state ^ (1 << mode), sign


def transfer(state, p, q):
    first = ladder(state, p, False)
    if first is None:
        return None
    second = ladder(first[0], q, True)
    return None if second is None else (second[0], first[1]*second[1])


def apply(events, basis):
    state = {basis: ONE}
    for event in events:
        out = {}
        def accumulate(dest, value):
            out[dest] = add(out.get(dest, ZERO), value)
        kind, *args = event
        for word, amplitude in state.items():
            if kind == 'G':
                p, q, a = args
                forward, backward = transfer(word, p, q), transfer(word, q, p)
                if forward is None and backward is None:
                    accumulate(word, amplitude)
                else:
                    cosine = scale(add(root(a), root(-a)), Q(1, 2))
                    sine = mul(scale(add(root(a), scale(root(-a), -1)), Q(1, 2)), root(-2))
                    accumulate(word, mul(amplitude, cosine))
                    dest, sign = forward if forward is not None else backward
                    accumulate(dest, scale(mul(amplitude, sine), sign if forward else -sign))
            elif kind == 'Z':
                accumulate(word, scale(amplitude, -1 if word >> args[0] & 1 else 1))
            elif kind == 'X':
                accumulate(word ^ (1 << args[0]), amplitude)
            elif kind == 'gamma':
                mode, odd = args
                sign = -1 if (word & ((1 << mode)-1)).bit_count() % 2 else 1
                value = scale(amplitude, sign)
                if odd:
                    value = mul(value, root(2 if word >> mode & 1 == 0 else -2))
                accumulate(word ^ (1 << mode), value)
            else:
                raise AssertionError(kind)
        state = {dest: a for dest, a in out.items() if a != ZERO}
    return state


@unittest.skipIf(exact is None, 'install tools/verification/requirements.txt')
class FermionOracleTest(unittest.TestCase):
    def test_nonadjacent_gate_normalization_includes_all_spectator_occupations(self):
        f = exact.Boolean(())
        for p, q in ((0, 2), (1, 3), (0, 3)):
            for a, b, zp, zq, x in itertools.product(range(8), range(8), range(2), range(2), range(2)):
                events = [('G', p, q, f.word(a, 3)), ('Z', p, f.word(zp, 1)[0]),
                          ('X', 0, f.word(x, 1)[0]), ('Z', q, f.word(zq, 1)[0]),
                          ('G', p, q, f.word(b, 3))]
                word, pending, phase = exact.normalize(f, events, 4, 3, f.one)
                def concrete(events):
                    out = []
                    for e in events:
                        if e[0] == 'G':
                            out.append(('G', e[1], e[2], sum((v == f.one) << i for i, v in enumerate(e[3]))))
                        elif e[-1] == f.one:
                            out.append(e[:-1])
                    return out
                normalized = concrete(word)+[('Z', j) for j, v in enumerate(pending) if v == f.one]
                for basis in range(16):
                    want = apply(concrete(events), basis)
                    got = apply(normalized, basis)
                    if phase == f.one:
                        got = {k: scale(v, -1) for k, v in got.items()}
                    self.assertEqual(got, want, (p, q, a, b, zp, zq, x, basis))

    def test_spin_layer_transports_both_majoranas_exactly(self):
        # F = product G_(2p,2p+1)(pi/2); verify F gamma_even F^dagger = gamma_odd_spin,
        # including the JW string in the odd-spin Majorana. Every Fock basis vector.
        for modes in (2, 4, 6):
            forward = [('G', p, p+1, 2) for p in range(0, modes, 2)]
            inverse = [('G', p, p+1, -2) for p in range(0, modes, 2)]
            for mode in range(0, modes, 2):
                for odd in (False, True):
                    for basis in range(1 << modes):
                        got = apply(inverse+[('gamma', mode, odd)]+forward, basis)
                        self.assertEqual(got, apply([('gamma', mode+1, odd)], basis))
            # Omitting the JW prefix in the spin transport is an actual operator error.
            self.assertNotEqual(apply(inverse+[('X', 0)]+forward, 1), apply([('X', 1)], 1))

    def test_long_chain_sign_change_and_partial_flip_mutant(self):
        # Two and three spatial rotations, on even spin modes; odd spins are spectators.
        for angles in ((1, 5), (3, 6), (7, 4), (1, 2, 5), (3, 7, 6)):
            modes = 2*(len(angles)+1)
            flipped = tuple((4-a) % 8 if j < len(angles)-1 else (a+4) % 8
                            for j, a in enumerate(angles))
            partial = flipped[:-1]+angles[-1:]
            def sandwich(words, pivot):
                inverse = [('G', 2*j, 2*j+2, -a) for j, a in reversed(list(enumerate(words)))]
                forward = [('G', 2*j, 2*j+2, a) for j, a in enumerate(words)]
                return inverse+[pivot]+forward
            for pivot in (('Z', 0), ('X', 0), ('gamma', 0, True)):
                different = False
                for basis in range(1 << modes):
                    want = apply(sandwich(angles, pivot), basis)
                    got = apply(sandwich(flipped, pivot), basis)
                    if pivot[0] != 'Z':
                        got = {k: scale(v, -1) for k, v in got.items()}
                    self.assertEqual(got, want, (angles, pivot, basis))
                    different |= apply(sandwich(partial, pivot), basis) != want
                self.assertTrue(different, (angles, pivot))


if __name__ == '__main__':
    unittest.main()
