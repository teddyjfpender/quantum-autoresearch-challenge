//! GF(2) polynomials over immutable symbols, for the reflection check (`super`).
//!
//! A symbol is a Boolean variable: a primary (free) one, or a *sealed* one that names an older
//! polynomial (its definition). A monomial is a set of symbols (`x^2 = x`), interned to an id; a
//! polynomial is a set of monomials, held as a sorted list of ids, and addition is symmetric
//! difference. Sealed symbols are hash-consed on their definition, so equal definitions give
//! the same symbol, and a definition names older symbols only.
//!
//! Everything here is exact. The one judgement, [`Arena::is_zero`], answers `Zero` or `Nonzero`
//! only when it has established that, and `Unknown` otherwise. All work is counted in
//! [`Arena::work`] (monomial operations, never wall clock), so a run is deterministic; when the
//! count passes the limit the arena is `exhausted`, further products return zero, and the caller
//! must refuse to certify.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

pub type Sym = u32;
/// An interned monomial. [`ONE`] is the empty monomial, the constant 1.
pub type Mono = u32;
pub const ONE: Mono = 0;
const NO_SYM: u32 = u32::MAX;

/// What a symbol stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The value wire `q` (the control or a uniform bit) holds at the start of the step.
    Initial(u32),
    /// The value inner uniform wire `q` holds after the `Reflect`.
    Second(u32),
    /// The outcome of the `Hmr` at this op index.
    Outcome(usize),
    /// A name for an older polynomial ([`Arena::def`]).
    Sealed,
}

/// A polynomial: monomial ids, sorted ascending, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Poly(Vec<Mono>);

impl Poly {
    #[must_use]
    pub fn zero() -> Self {
        Self(Vec::new())
    }
    #[must_use]
    pub fn one() -> Self {
        Self(vec![ONE])
    }
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
    #[must_use]
    pub fn is_one(&self) -> bool {
        self.0 == [ONE]
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    #[must_use]
    pub fn terms(&self) -> &[Mono] {
        &self.0
    }
    /// Adds the monomial `m` (removes it if present).
    pub fn toggle(&mut self, m: Mono) {
        match self.0.binary_search(&m) {
            Ok(i) => {
                self.0.remove(i);
            }
            Err(i) => self.0.insert(i, m),
        }
    }
    /// From any list of monomials: those that occur an odd number of times.
    #[must_use]
    pub fn from_terms(mut v: Vec<Mono>) -> Self {
        v.sort_unstable();
        let mut out = Vec::with_capacity(v.len());
        let mut i = 0;
        while i < v.len() {
            let mut j = i;
            while j < v.len() && v[j] == v[i] {
                j += 1;
            }
            if (j - i) % 2 == 1 {
                out.push(v[i]);
            }
            i = j;
        }
        Self(out)
    }
    /// `self += o`.
    pub fn add(&mut self, o: &Self) {
        if o.0.is_empty() {
            return;
        }
        if o.0.len() * 16 < self.0.len() {
            for &m in &o.0 {
                self.toggle(m);
            }
            return;
        }
        let (a, b) = (&self.0, &o.0);
        let mut out = Vec::with_capacity(a.len() + b.len());
        let (mut i, mut j) = (0, 0);
        while i < a.len() && j < b.len() {
            match a[i].cmp(&b[j]) {
                std::cmp::Ordering::Less => {
                    out.push(a[i]);
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    out.push(b[j]);
                    j += 1;
                }
                std::cmp::Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
            }
        }
        out.extend_from_slice(&a[i..]);
        out.extend_from_slice(&b[j..]);
        self.0 = out;
    }
}

/// The answer of the zero test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The polynomial is 0 for every assignment of the primary symbols.
    Zero,
    /// It is 1 for some assignment.
    Nonzero,
    /// Not decided within the limits.
    Unknown,
}

/// Limits of one zero test.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Truth tables are used when the polynomial depends on at most this many primary symbols.
    pub table_bits: u32,
    /// Most monomials the expansion rung may hold.
    pub expand: usize,
}

/// Most `u64` words of truth tables one zero test may hold (128 MiB).
const TABLE_WORDS: usize = 1 << 24;
/// The expansion rung's first, cheap attempt holds at most this many monomials.
const QUICK_EXPAND: usize = 1 << 12;
/// Truth-table word operations per unit of work (a word operation is far cheaper than a
/// monomial operation).
const TABLE_WORDS_PER_TICK: usize = 16;
/// Most monomials the arena interns (each costs on the order of 100 bytes); past it the arena
/// is exhausted.
const MAX_MONOS: usize = 1 << 23;
/// Most terms one product may have.
const MAX_PRODUCT: usize = 1 << 26;

/// How many zero tests each rung decided.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rungs {
    pub syntactic: u64,
    pub truth_table: u64,
    pub expansion: u64,
    pub undecided: u64,
}

pub struct Arena {
    monos: Vec<Rc<[Sym]>>,
    index: HashMap<Rc<[Sym]>, Mono>,
    /// Per monomial: it names an `Outcome` symbol directly.
    outcome: Vec<bool>,
    /// Per monomial: its newest sealed symbol, or `NO_SYM`.
    top_sealed: Vec<Sym>,
    kinds: Vec<Kind>,
    defs: Vec<Option<Rc<Poly>>>,
    /// Per symbol: the monomial that is the symbol alone.
    single: Vec<Mono>,
    cons: HashMap<Poly, Sym>,
    sealed: u64,
    /// Per symbol: the zero test that last visited it.
    mark: Vec<u32>,
    epoch: u32,
    scratch: Vec<Sym>,
    pub work: u64,
    limit: u64,
    pub exhausted: bool,
    pub rungs: Rungs,
}

impl Arena {
    #[must_use]
    pub fn new(limit: u64) -> Self {
        let empty: Rc<[Sym]> = Rc::from(Vec::new());
        Self {
            monos: vec![empty.clone()],
            index: HashMap::from([(empty, ONE)]),
            outcome: vec![false],
            top_sealed: vec![NO_SYM],
            kinds: Vec::new(),
            defs: Vec::new(),
            single: Vec::new(),
            cons: HashMap::new(),
            sealed: 0,
            mark: Vec::new(),
            epoch: 0,
            scratch: Vec::new(),
            work: 0,
            limit,
            exhausted: false,
            rungs: Rungs::default(),
        }
    }

    /// Counts `n` units of work.
    pub fn tick(&mut self, n: usize) {
        self.work = self.work.saturating_add(n as u64);
        if self.work > self.limit {
            self.exhausted = true;
        }
    }

    #[must_use]
    pub fn symbols(&self) -> u64 {
        self.kinds.len() as u64
    }
    #[must_use]
    pub fn sealed(&self) -> u64 {
        self.sealed
    }
    #[must_use]
    pub fn kind(&self, s: Sym) -> Kind {
        self.kinds[s as usize]
    }
    /// The definition of a sealed symbol.
    #[must_use]
    pub fn def(&self, s: Sym) -> Option<Rc<Poly>> {
        self.defs[s as usize].clone()
    }
    #[must_use]
    pub fn syms(&self, m: Mono) -> Rc<[Sym]> {
        self.monos[m as usize].clone()
    }
    #[must_use]
    pub fn has_outcome(&self, m: Mono) -> bool {
        self.outcome[m as usize]
    }

    fn push_sym(&mut self, kind: Kind, def: Option<Rc<Poly>>) -> Sym {
        let s = self.kinds.len() as Sym;
        self.kinds.push(kind);
        self.defs.push(def);
        self.mark.push(0);
        self.single.push(0);
        let m = self.intern(&[s]);
        self.single[s as usize] = m;
        s
    }

    /// A new primary symbol.
    pub fn primary(&mut self, kind: Kind) -> Sym {
        debug_assert!(kind != Kind::Sealed);
        self.push_sym(kind, None)
    }

    /// The polynomial that is the symbol `s`.
    #[must_use]
    pub fn sym_poly(&self, s: Sym) -> Poly {
        Poly(vec![self.single[s as usize]])
    }

    /// The id of the monomial with exactly the symbols `syms` (sorted ascending, each once).
    pub fn intern(&mut self, syms: &[Sym]) -> Mono {
        if let Some(&m) = self.index.get(syms) {
            return m;
        }
        self.tick(1);
        if self.monos.len() >= MAX_MONOS {
            // Nothing computed after this is used: the caller refuses on `exhausted`.
            self.exhausted = true;
            return ONE;
        }
        let m = self.monos.len() as Mono;
        let rc: Rc<[Sym]> = Rc::from(syms);
        self.monos.push(rc.clone());
        self.index.insert(rc, m);
        let kinds = &self.kinds;
        self.outcome.push(
            syms.iter()
                .any(|&s| matches!(kinds[s as usize], Kind::Outcome(_))),
        );
        self.top_sealed.push(
            syms.iter()
                .rev()
                .copied()
                .find(|&s| kinds[s as usize] == Kind::Sealed)
                .unwrap_or(NO_SYM),
        );
        m
    }

    /// The product of two monomials: the union of their symbols.
    pub fn mono_mul(&mut self, a: Mono, b: Mono) -> Mono {
        if a == ONE || a == b {
            return b;
        }
        if b == ONE {
            return a;
        }
        let (x, y) = (
            self.monos[a as usize].clone(),
            self.monos[b as usize].clone(),
        );
        let mut out = std::mem::take(&mut self.scratch);
        out.clear();
        let (mut i, mut j) = (0, 0);
        while i < x.len() && j < y.len() {
            match x[i].cmp(&y[j]) {
                std::cmp::Ordering::Less => {
                    out.push(x[i]);
                    i += 1;
                }
                std::cmp::Ordering::Greater => {
                    out.push(y[j]);
                    j += 1;
                }
                std::cmp::Ordering::Equal => {
                    out.push(x[i]);
                    i += 1;
                    j += 1;
                }
            }
        }
        out.extend_from_slice(&x[i..]);
        out.extend_from_slice(&y[j..]);
        let m = self.intern(&out);
        self.scratch = out;
        m
    }

    /// `m` split into its `Outcome` symbols and the rest.
    pub fn split_outcome(&mut self, m: Mono) -> (Mono, Mono) {
        if !self.outcome[m as usize] {
            return (ONE, m);
        }
        let syms = self.monos[m as usize].clone();
        let is_out = |s: &Sym| matches!(self.kinds[*s as usize], Kind::Outcome(_));
        let out: Vec<Sym> = syms.iter().copied().filter(is_out).collect();
        let rest: Vec<Sym> = syms.iter().copied().filter(|s| !is_out(s)).collect();
        (self.intern(&out), self.intern(&rest))
    }

    /// `dst += src`, counted.
    pub fn add(&mut self, dst: &mut Poly, src: &Poly) {
        self.tick(src.len());
        dst.add(src);
    }

    /// The product `p q`. Zero once the arena is exhausted.
    pub fn mul(&mut self, p: &Poly, q: &Poly) -> Poly {
        if p.is_zero() || q.is_zero() {
            return Poly::zero();
        }
        if p.is_one() {
            return q.clone();
        }
        if q.is_one() {
            return p.clone();
        }
        let n = p.len().saturating_mul(q.len());
        self.tick(n);
        if n > MAX_PRODUCT {
            self.exhausted = true;
        }
        if self.exhausted {
            return Poly::zero();
        }
        let mut v = Vec::with_capacity(n);
        for &a in &p.0 {
            for &b in &q.0 {
                v.push(self.mono_mul(a, b));
            }
        }
        Poly::from_terms(v)
    }

    /// A symbol equal to the nonzero polynomial `p`: `p` itself when it is one symbol, else the
    /// sealed symbol whose definition is `p` (the same symbol for the same polynomial).
    pub fn seal(&mut self, p: &Poly) -> Sym {
        debug_assert!(!p.is_zero());
        if let [m] = p.0[..] {
            if let [s] = self.monos[m as usize][..] {
                return s;
            }
        }
        if let Some(&s) = self.cons.get(p) {
            return s;
        }
        self.tick(p.len());
        let s = self.push_sym(Kind::Sealed, Some(Rc::new(p.clone())));
        self.cons.insert(p.clone(), s);
        self.sealed += 1;
        s
    }

    /// Decides whether `p` is identically zero, in rungs, each sound:
    /// 1. no monomials: zero;
    /// 2. *expansion*: sealed symbols are replaced by their definitions, newest first, with
    ///    cancellation. If no sealed symbol is left the polynomial is in algebraic normal form
    ///    over free variables, which is zero exactly when it has no monomials. First with the
    ///    small limit `QUICK_EXPAND` on the monomials held;
    /// 3. *truth table*: if `p` depends, through its sealed symbols, on at most
    ///    `lim.table_bits` primary symbols, it is evaluated on every assignment of them
    ///    (complete);
    /// 4. expansion again with the limit `lim.expand`. Past it, or past the arena's work
    ///    limit, the answer is `Unknown`.
    ///
    /// The rungs agree wherever two of them decide, so their order affects cost only.
    pub fn is_zero(&mut self, p: &Poly, lim: &Limits) -> Verdict {
        if p.is_zero() {
            self.rungs.syntactic += 1;
            return Verdict::Zero;
        }
        let quick = self.expand(p, QUICK_EXPAND.min(lim.expand));
        if quick != Verdict::Unknown {
            self.rungs.expansion += 1;
            return quick;
        }
        if let Some((primaries, sealed)) = self.cone(p, lim.table_bits as usize) {
            let words = (1usize << primaries.len()).div_ceil(64);
            if sealed.len().saturating_mul(words) <= TABLE_WORDS {
                self.rungs.truth_table += 1;
                return self.table(p, &primaries, &sealed);
            }
        }
        let v = self.expand(p, lim.expand);
        match v {
            Verdict::Unknown => self.rungs.undecided += 1,
            _ => self.rungs.expansion += 1,
        }
        v
    }

    /// How many primary and how many sealed symbols `p` depends on (for messages only; zeros
    /// if the arena is exhausted on the way).
    pub fn support(&mut self, p: &Poly) -> (usize, usize) {
        self.cone(p, usize::MAX)
            .map_or((0, 0), |(primaries, sealed)| {
                (primaries.len(), sealed.len())
            })
    }

    /// The primary and the sealed symbols `p` depends on (each sorted ascending), or `None`
    /// when there are more than `max` primaries.
    fn cone(&mut self, p: &Poly, max: usize) -> Option<(Vec<Sym>, Vec<Sym>)> {
        self.epoch += 1;
        let epoch = self.epoch;
        let (mut primaries, mut sealed) = (Vec::new(), Vec::new());
        let mut stack: Vec<Rc<Poly>> = vec![Rc::new(p.clone())];
        while let Some(poly) = stack.pop() {
            self.tick(poly.len());
            if self.exhausted {
                return None;
            }
            for &m in poly.terms() {
                let syms = self.monos[m as usize].clone();
                for &s in syms.iter() {
                    if self.mark[s as usize] == epoch {
                        continue;
                    }
                    self.mark[s as usize] = epoch;
                    if let Some(d) = self.defs[s as usize].clone() {
                        sealed.push(s);
                        stack.push(d);
                    } else {
                        primaries.push(s);
                        if primaries.len() > max {
                            return None;
                        }
                    }
                }
            }
        }
        primaries.sort_unstable();
        sealed.sort_unstable();
        Some((primaries, sealed))
    }

    /// Rung 2: truth tables over `primaries`. A sealed symbol's table is computed from its
    /// definition, oldest first (a definition names older symbols only).
    fn table(&mut self, p: &Poly, primaries: &[Sym], sealed: &[Sym]) -> Verdict {
        const PATTERN: [u64; 6] = [
            0xAAAA_AAAA_AAAA_AAAA,
            0xCCCC_CCCC_CCCC_CCCC,
            0xF0F0_F0F0_F0F0_F0F0,
            0xFF00_FF00_FF00_FF00,
            0xFFFF_0000_FFFF_0000,
            0xFFFF_FFFF_0000_0000,
        ];
        let n = primaries.len();
        let words = (1usize << n).div_ceil(64);
        let mask = if n < 6 { (1u64 << (1 << n)) - 1 } else { !0 };
        let mut tables: HashMap<Sym, Vec<u64>> = HashMap::new();
        for (j, &s) in primaries.iter().enumerate() {
            let t = (0..words)
                .map(|w| match j {
                    0..6 => PATTERN[j] & mask,
                    _ if w >> (j - 6) & 1 == 1 => !0,
                    _ => 0,
                })
                .collect();
            tables.insert(s, t);
        }
        let eval = |a: &mut Self, tables: &HashMap<Sym, Vec<u64>>, poly: &Poly| -> Vec<u64> {
            let mut out = vec![0u64; words];
            let mut acc = vec![0u64; words];
            for &m in poly.terms() {
                let syms = a.monos[m as usize].clone();
                a.tick((words * syms.len().max(1)).div_ceil(TABLE_WORDS_PER_TICK));
                acc.fill(mask);
                for s in syms.iter() {
                    for (x, y) in acc.iter_mut().zip(&tables[s]) {
                        *x &= y;
                    }
                }
                for (o, x) in out.iter_mut().zip(&acc) {
                    *o ^= x;
                }
            }
            out
        };
        for &s in sealed {
            if self.exhausted {
                return Verdict::Unknown;
            }
            let Some(d) = self.defs[s as usize].clone() else {
                return Verdict::Unknown;
            };
            let t = eval(self, &tables, &d);
            tables.insert(s, t);
        }
        let t = eval(self, &tables, p);
        if self.exhausted {
            return Verdict::Unknown;
        }
        if t.iter().all(|&w| w == 0) {
            Verdict::Zero
        } else {
            Verdict::Nonzero
        }
    }

    /// `m` without the symbol `x`.
    fn without(&mut self, m: Mono, x: Sym) -> Mono {
        let syms = self.monos[m as usize].clone();
        let rest: Vec<Sym> = syms.iter().copied().filter(|&s| s != x).collect();
        self.intern(&rest)
    }

    /// Rung 3: substitution of sealed symbols, newest first.
    fn expand(&mut self, p: &Poly, limit: usize) -> Verdict {
        let mut set: BTreeSet<Mono> = p.terms().iter().copied().collect();
        // Monomials by their newest sealed symbol. A list may name monomials that have since
        // left `set`, or name one twice; both are filtered when the list is taken.
        let mut by_top: BTreeMap<Sym, Vec<Mono>> = BTreeMap::new();
        for &m in p.terms() {
            let t = self.top_sealed[m as usize];
            if t != NO_SYM {
                by_top.entry(t).or_default().push(m);
            }
        }
        while let Some((x, mut list)) = by_top.pop_last() {
            let Some(d) = self.defs[x as usize].clone() else {
                return Verdict::Unknown;
            };
            list.sort_unstable();
            list.dedup();
            for m in list {
                if !set.remove(&m) {
                    continue;
                }
                let rest = self.without(m, x);
                self.tick(d.len());
                if self.exhausted {
                    return Verdict::Unknown;
                }
                for &dm in d.terms() {
                    let n = self.mono_mul(rest, dm);
                    if !set.remove(&n) {
                        set.insert(n);
                        let t = self.top_sealed[n as usize];
                        if t != NO_SYM {
                            by_top.entry(t).or_default().push(n);
                        }
                    }
                }
                if set.len() > limit {
                    return Verdict::Unknown;
                }
            }
        }
        if set.is_empty() {
            Verdict::Zero
        } else {
            Verdict::Nonzero
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIM: Limits = Limits {
        table_bits: 20,
        expand: 1 << 19,
    };

    fn arena() -> (Arena, Vec<Poly>) {
        let mut a = Arena::new(u64::MAX);
        let v = (0..40)
            .map(|q| {
                let s = a.primary(Kind::Initial(q));
                a.sym_poly(s)
            })
            .collect();
        (a, v)
    }

    #[test]
    fn products_are_idempotent_and_sums_cancel() {
        let (mut a, v) = arena();
        let xy = a.mul(&v[0], &v[1]);
        assert_eq!(a.mul(&xy, &v[0]), xy);
        assert_eq!(a.mul(&xy, &xy), xy);
        let mut s = v[0].clone();
        s.add(&v[1]);
        // (x + y)^2 = x + y.
        assert_eq!(a.mul(&s, &s), s);
        s.add(&v[1]);
        assert_eq!(s, v[0]);
        s.add(&v[0]);
        assert!(s.is_zero());
        assert_eq!(a.mul(&Poly::one(), &xy), xy);
        assert!(a.mul(&Poly::zero(), &xy).is_zero());
    }

    #[test]
    fn sealing_is_hash_consed_and_transparent_to_the_zero_test() {
        let (mut a, v) = arena();
        let mut p = a.mul(&v[0], &v[1]);
        p.add(&v[2]);
        p.add(&Poly::one());
        let (s1, s2) = (a.seal(&p), a.seal(&p.clone()));
        assert_eq!(s1, s2);
        assert_eq!(a.kind(s1), Kind::Sealed);
        // One symbol is its own seal.
        let x = a.seal(&v[3]);
        assert_eq!(a.sym_poly(x), v[3]);
        // s + p = 0, by both rungs.
        let mut d = a.sym_poly(s1);
        d.add(&p);
        assert_eq!(a.is_zero(&d, &LIM), Verdict::Zero);
        assert_eq!(a.expand(&d, 1 << 10), Verdict::Zero);
        // s + p + x0 is x0: nonzero.
        d.add(&v[0]);
        assert_eq!(a.is_zero(&d, &LIM), Verdict::Nonzero);
        let (primaries, sealed) = a.cone(&d, 20).unwrap();
        assert_eq!((primaries.len(), sealed.len()), (3, 1));
        assert_eq!(a.table(&d, &primaries, &sealed), Verdict::Nonzero);
        assert_eq!(a.expand(&d, 1 << 10), Verdict::Nonzero);
        d.add(&v[0]);
        assert_eq!(a.table(&d, &primaries, &sealed), Verdict::Zero);
    }

    /// An identity the expansion cannot reach within a small limit but the truth table decides:
    /// `prod (x_i + 1)`, sealed stepwise, against the same product sealed in the other order.
    #[test]
    fn truth_table_decides_what_expansion_cannot() {
        let (mut a, v) = arena();
        let chain = |a: &mut Arena, order: &[usize]| {
            let mut acc = Poly::one();
            for &i in order {
                let mut f = v[i].clone();
                f.add(&Poly::one());
                let p = a.mul(&acc, &f);
                let s = a.seal(&p);
                acc = a.sym_poly(s);
            }
            acc
        };
        let up: Vec<usize> = (0..16).collect();
        let down: Vec<usize> = (0..16).rev().collect();
        let mut d = chain(&mut a, &up);
        d.add(&chain(&mut a, &down));
        assert!(!d.is_zero());
        assert_eq!(a.expand(&d, 1 << 8), Verdict::Unknown);
        assert_eq!(a.is_zero(&d, &LIM), Verdict::Zero);
        assert_eq!(a.expand(&d, 1 << 18), Verdict::Zero);
        // Drop one factor on one side: nonzero, found by the table.
        let mut e = chain(&mut a, &up);
        e.add(&chain(&mut a, &down[1..]));
        assert_eq!(a.is_zero(&e, &LIM), Verdict::Nonzero);
        // With too few table bits the test falls to the expansion and its limit.
        let small = Limits {
            table_bits: 8,
            expand: 1 << 8,
        };
        assert_eq!(a.is_zero(&d, &small), Verdict::Unknown);
    }

    /// The two complete rungs agree with direct evaluation on random sealed polynomials.
    #[test]
    fn zero_test_agrees_with_direct_evaluation() {
        let mut x = 0x1234_5678_9ABC_DEF1u64;
        let mut rnd = move |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        for round in 0..200u32 {
            let mut a = Arena::new(u64::MAX);
            let n: u32 = 3 + (round % 3);
            let mut pool: Vec<Poly> = (0..n)
                .map(|q| {
                    let s = a.primary(Kind::Initial(q));
                    a.sym_poly(s)
                })
                .collect();
            // Reference truth tables, one bit per assignment.
            let mut truth: Vec<u64> = (0..n)
                .map(|j| (0..1u64 << n).fold(0, |t, i| t | (i >> j & 1) << i))
                .collect();
            let full = (1u64 << (1 << n)) - 1;
            for _ in 0..24 {
                let (i, j, k) = (
                    rnd(pool.len() as u64) as usize,
                    rnd(pool.len() as u64) as usize,
                    rnd(pool.len() as u64) as usize,
                );
                let mut p = a.mul(&pool[i], &pool[j]);
                p.add(&pool[k]);
                let mut t = (truth[i] & truth[j]) ^ truth[k];
                if rnd(2) == 1 {
                    p.add(&Poly::one());
                    t ^= full;
                }
                if p.is_zero() {
                    continue;
                }
                let q = if rnd(2) == 1 {
                    let s = a.seal(&p);
                    a.sym_poly(s)
                } else {
                    p
                };
                pool.push(q);
                truth.push(t);
            }
            for _ in 0..40 {
                let (i, j) = (
                    rnd(pool.len() as u64) as usize,
                    rnd(pool.len() as u64) as usize,
                );
                let mut d = pool[i].clone();
                d.add(&pool[j]);
                let want = if truth[i] == truth[j] {
                    Verdict::Zero
                } else {
                    Verdict::Nonzero
                };
                assert_eq!(a.is_zero(&d, &LIM), want, "round {round}");
                assert_eq!(a.expand(&d, usize::MAX), want, "round {round}");
                let (primaries, sealed) = a.cone(&d, 20).unwrap();
                assert_eq!(a.table(&d, &primaries, &sealed), want, "round {round}");
            }
        }
    }

    #[test]
    fn work_limit_exhausts_instead_of_answering() {
        let mut a = Arena::new(50);
        let v: Vec<Poly> = (0..12)
            .map(|q| {
                let s = a.primary(Kind::Initial(q));
                a.sym_poly(s)
            })
            .collect();
        let mut acc = Poly::one();
        for x in &v {
            let mut f = x.clone();
            f.add(&Poly::one());
            acc = a.mul(&acc, &f);
        }
        assert!(a.exhausted);
        assert!(acc.is_zero());
    }
}
