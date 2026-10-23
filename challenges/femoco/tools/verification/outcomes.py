"""Exact Boolean functions: lane BDD coefficients and square-free outcome monomials.

Implements the Boolean interface used by exact.py without placing outcome variables in
wire BDDs. Addition is XOR; multiplication uses m*m=m. The monomials form a unique basis
of functions on {0,1}^k, so equality is coefficient equality for every lane and outcome.
"""
from __future__ import annotations

import exact
import weakref

EMPTY = frozenset()


class Polynomial:
    __slots__ = ('owner', 'terms', '__weakref__')

    def __init__(self, owner, terms):
        if len(terms) > owner.term_limit or any(len(m) > 128 for m in terms):
            raise ValueError('outcome polynomial term/degree budget exceeded')
        self.owner, self.terms = owner, terms

    def __eq__(self, other):
        return isinstance(other, Polynomial) and self.owner is other.owner and self.terms == other.terms

    def __invert__(self):
        return self.owner.xor(self, self.owner.one)

    def __and__(self, other):
        f = self.owner
        if not isinstance(other, Polynomial) or other.owner is not f:
            raise ValueError('mixed Boolean managers')
        if self == f.zero or other == f.zero:
            return f.zero
        if self == f.one:
            return other
        if other == f.one:
            return self
        if self == other:
            return self
        out = {}
        for a, x in self.terms.items():
            for b, y in other.terms.items():
                m = a | b
                f.accumulate(out, m, x & y)
        return Polynomial(f, out)

    def __or__(self, other):
        return self.owner.xor(self.owner.xor(self, other), self & other)


class DiagramInterface:
    """Only the BDD operations called by the inherited interpreter/word checker."""
    def __init__(self, owner):
        self._owner = weakref.ref(owner)

    @property
    def owner(self):
        owner = self._owner()
        if owner is None:
            raise ReferenceError('Boolean manager has been released')
        return owner

    def var(self, name):
        f = self.owner
        if name not in f.raw.bdd.vars:
            raise ValueError('undeclared Boolean variable')
        return (Polynomial(f, {frozenset((name,)): f.raw.one}) if name.startswith('m')
                else f.lane(f.raw.bdd.var(name)))

    def ite(self, cond, yes, no):
        return self.owner.xor(cond & yes, ~cond & no)

    def pick(self, node):
        """Construct an actual satisfying assignment for a nonzero polynomial.

        Fix a lane witnessing a nonzero coefficient, then select a nonzero monomial
        of minimum degree. Set exactly its outcomes to 1. No other surviving monomial
        is a subset, so the function evaluates to 1. Unlisted variables are don't-cares.
        """
        f = self.owner
        if node == f.zero:
            return None
        coefficient = next(iter(node.terms.values()))
        witness = f.raw.bdd.pick(coefficient)
        lane = {v: witness.get(v, False) for v in f.raw.bdd.vars if not v.startswith('m')}
        surviving = [m for m, c in node.terms.items() if f.raw.bdd.let(lane, c) == f.raw.one]
        chosen = min(surviving, key=len)
        support = set().union(*node.terms)
        return dict(lane, **{v: v in chosen for v in support})


class OutcomeBoolean(exact.Boolean):
    """Reuse word arithmetic, alias decoding, interpretation and normalization unchanged."""
    def __init__(self, raw, term_limit=65536):
        self.raw, self.term_limit = raw, term_limit
        self._zero = self._one = None
        self.bdd = DiagramInterface(self)

    @property
    def zero(self):
        value = self._zero() if self._zero else None
        if value is None:
            value = Polynomial(self, {})
            self._zero = weakref.ref(value)
        return value

    @property
    def one(self):
        value = self._one() if self._one else None
        if value is None:
            value = Polynomial(self, {EMPTY: self.raw.one})
            self._one = weakref.ref(value)
        return value

    def lane(self, node):
        if any(v.startswith('m') for v in node.support):
            raise ValueError('lane coefficient contains an outcome variable')
        return self.zero if node == self.raw.zero else Polynomial(self, {EMPTY: node})

    def accumulate(self, out, monomial, value):
        value = self.raw.xor(out.get(monomial, self.raw.zero), value)
        if value == self.raw.zero:
            out.pop(monomial, None)
        else:
            out[monomial] = value
        if len(out) > self.term_limit or len(monomial) > 128:
            raise ValueError('outcome polynomial term/degree budget exceeded')

    def xor(self, a, b):
        if not isinstance(a, Polynomial) or not isinstance(b, Polynomial) or a.owner is not self or b.owner is not self:
            raise ValueError('mixed Boolean managers')
        if a == b:
            return self.zero
        if a == self.zero:
            return b
        if b == self.zero:
            return a
        out = dict(a.terms)
        for monomial, value in b.terms.items():
            self.accumulate(out, monomial, value)
        return Polynomial(self, out)

    def add(self, a, b):
        """The inherited ripple adder, omitting its discarded final carry.

        For a measurement sign, the high bit can have thousands of monomials.
        Multiplying it into an unused overflow carry is both expensive and needless.
        The result is still exactly addition modulo 2**min(len(a), len(b)).
        """
        carry, out = self.zero, []
        width = min(len(a), len(b))
        for i, (x, y) in enumerate(zip(a, b)):
            xy = self.xor(x, y)
            out.append(self.xor(xy, carry))
            if i+1 < width:
                carry = (x & y) | (carry & xy)
        return tuple(out)
