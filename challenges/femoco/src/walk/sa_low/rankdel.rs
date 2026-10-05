//! Lever `q<b>`: **rank-scheduled angle delivery** on a
//! split one-hot (`G = 2` or `3` groups).
//!
//! **Setting.** The RPREP one-hot holds the lane's leaf `e` as a slot one-hot `h` and exclusive
//! group bits `g_i` (`onehot.rs`, "Split one-hot"). Every qubit value in SELECT is a function of
//! the leaf; over GF(2) the leaf functions are a vector space and the *base* `B0 = span{h_s, g_i}`
//! is live for free. Rotation `j`'s register must hold the stored word `W_j` (one function per
//! bit) at its `Givens`. Today's gadgets (`onehot::transition_by`, lever `z`) pay one Toffoli per
//! register bit and transition: every transition re-creates the group-dependent part of the words.
//!
//! **Idea.** The words' group-dependent parts span a small space: `D' = dim(span(W) + B0) - dim B0`
//! (210 on Reiher G = 3, against 795 bit-rotations per pass). Hold a subspace `X` of it live and
//! assemble each word by CNOTs (free). A copy's rotation sequence `52 .. 0, 0 .. 52` is cut into
//! **windows**; window `F` holds the span of its rotations' words (`dim F <= b`, the budget). At a
//! window switch `A -> B` the intersection `F_A ∩ F_B` is kept (Gaussian elimination by CNOTs), the
//! rest of `F_A` is measured out (Clifford fixups on the base), and `F_B / (F_A ∩ F_B)` is bought:
//! **one Toffoli per dimension** (a pair product `(g_1 ^ X(h))(g_2 ^ Y(h))`, the exclusivity
//! algebra of lever `z`, gives any group-dependent function at one Toffoli; G = 2: `g_1 Y(h)` with
//! the parity formed in place on a slot qubit). Since two `b`-dimensional windows of a
//! `D'`-dimensional space share at least `2 b - D'` dimensions, a switch costs at most `D' - b`.
//! The window boundaries are chosen by dynamic programming on the spec's own words ([`plan`]).
//!
//! **Registers.** The angle register `R` (the Givens register) and `h_B` held qubits `H`, with
//! `H + span(W_t) = F` (mod `B0`) for every rotation `t` of the window (a random complement, checked).
//! Inside a window a transition `W_t -> W_t'` is a CNOT-only retarget of `R` against `B0 + H`
//! ([`retarget`]: both row spaces equal `F / (B0 + H)`, so an invertible CNOT map exists). Live
//! qubits across SELECT: the one-hot plus `|R| + max h_B` (about `b + 1`).
//!
//! **Erasure.** Every qubit this gadget writes holds a known leaf function `f`; it is X-measured and
//! `(-1)^(m f)` is applied as `Z(h_s)` and `CZ(g_i, h_s)` from `f = f_0(h) ^ sum_i g_i f_i(h)`
//! (every leaf function has this form on a valid one-hot): 0 Toffolis.
//!
//! **Cost** (Toffolis per copy) = the plan's buys: the first window's dimension plus every switch's
//! `dim F_B - dim(F_A ∩ F_B)`. Static and exact (no outcome gating).
#![allow(clippy::needless_range_loop)]
use super::onehot::Hot;
use crate::circuit::{Bit, Builder, Qubit};

// Deliberate faults for the mutant tests (`tests_c1`, `rankdel::tests`): 90 a buy's group fan
// skipped, 91 one fixup term of a measured unload skipped, 92 a retarget's base fix skipped on bit 0,
// 93 the first CNOT that gathers the kept intersection not emitted, 94 the write's reachable
// non-leaf states left out of the care set; 120 the triple's shared CNOT skipped,
// 121 the triple's `f2` written into the first register; 133 the Majorana cut
// (`park`) not emitted (130-132 belong to lever `.e`).
fn fault_is(k: u8) -> bool {
    super::onehot::fault_is(k)
}

type V = Vec<u64>;

fn zero(w: usize) -> V {
    vec![0; w]
}
fn get(v: &[u64], i: usize) -> bool {
    v[i / 64] >> (i % 64) & 1 == 1
}
fn set(v: &mut [u64], i: usize) {
    v[i / 64] |= 1 << (i % 64);
}
fn xor(a: &mut [u64], b: &[u64]) {
    a.iter_mut().zip(b).for_each(|(x, y)| *x ^= y);
}
fn is_zero(v: &[u64]) -> bool {
    v.iter().all(|&x| x == 0)
}
fn lowest(v: &[u64]) -> Option<usize> {
    v.iter()
        .enumerate()
        .find(|(_, &x)| x != 0)
        .map(|(i, &x)| i * 64 + x.trailing_zeros() as usize)
}
fn ones(v: &[u64]) -> impl Iterator<Item = usize> + '_ {
    v.iter().enumerate().flat_map(|(i, &x)| {
        (0..64)
            .filter(move |b| x >> b & 1 == 1)
            .map(move |b| i * 64 + b)
    })
}

/// A fully reduced GF(2) row basis (pivot = lowest set bit, zero in every other row) whose rows
/// carry a mask over *sources* (the qubits whose functions sum to the row).
#[derive(Clone, Debug)]
pub struct Elim {
    rows: Vec<(usize, V, V)>,
    mw: usize,
}

impl Elim {
    fn new(mw: usize) -> Self {
        Self {
            rows: Vec::new(),
            mw: mw.max(1),
        }
    }
    /// `(residue, mask)`: `v = residue ^ sum of the mask's sources`; the residue is canonical.
    fn reduce(&self, v: &[u64]) -> (V, V) {
        let mut r = v.to_vec();
        let mut m = zero(self.mw);
        for (p, row, rm) in &self.rows {
            if get(&r, *p) {
                xor(&mut r, row);
                xor(&mut m, rm);
            }
        }
        (r, m)
    }
    fn residue(&self, v: &[u64]) -> V {
        let mut r = v.to_vec();
        for (p, row, _) in &self.rows {
            if get(&r, *p) {
                xor(&mut r, row);
            }
        }
        r
    }
    /// Inserts `v` (source mask `m`); false if it was already in the span.
    fn insert(&mut self, v: &[u64], m: &[u64]) -> bool {
        let (r, mut rm) = self.reduce(v);
        let Some(p) = lowest(&r) else { return false };
        let mut mm = m.to_vec();
        mm.resize(self.mw, 0);
        xor(&mut rm, &mm);
        for (_, row, m2) in &mut self.rows {
            if get(row, p) {
                xor(row, &r);
                xor(m2, &rm);
            }
        }
        self.rows.push((p, r, rm));
        true
    }
    fn dim(&self) -> usize {
        self.rows.len()
    }
}

/// The split one-hot's states as GF(2) coordinates: `(group, slot)` and `(group, none)` (no slot
/// set). Every tracked function is exact on the *care* states: every leaf's state and every state
/// the RPREP write can produce ([`reachable`], by enumerating its inputs; all states when that is
/// not available). A state without a leaf reads angle 0 (as `onehot::transition_by` delivers). A
/// state no lane can hold is a don't-care (masked to 0 in every vector), which keeps the span
/// small.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Coordinates: `groups * (e1 + 1)`.
    pub n: usize,
    pub words: usize,
    pub e1: usize,
    pub groups: usize,
    /// `at[group][slot]`: the leaf there.
    pub at: Vec<Vec<Option<usize>>>,
    /// The care coordinates.
    care: V,
}

impl Layout {
    /// `reach[g][s]` (`s = e1`: no slot): the states a lane can hold ([`reachable`]); `None`:
    /// every state.
    #[must_use]
    pub fn of(hot: &Hot, n_leaves: usize, reach: Option<&[Vec<bool>]>) -> Self {
        let groups = hot.g.len() + 1;
        let mut at = vec![vec![None; hot.e1]; groups];
        for e in 0..n_leaves {
            let (s, g) = hot.place(e);
            at[g][s] = Some(e);
        }
        let n = groups * (hot.e1 + 1);
        let words = n.div_ceil(64).max(1);
        let mut care = zero(words);
        for g in 0..groups {
            for s in 0..=hot.e1 {
                let r = reach.is_none_or(|r| r[g][s]) && !fault_is(94);
                if r || (s < hot.e1 && at[g][s].is_some()) || (g == 0 && s == hot.e1) {
                    set(&mut care, g * (hot.e1 + 1) + s);
                }
            }
        }
        Self {
            n,
            words,
            e1: hot.e1,
            groups,
            at,
            care,
        }
    }
    fn mask(&self, mut v: V) -> V {
        v.iter_mut().zip(&self.care).for_each(|(x, c)| *x &= c);
        v
    }
    /// Coordinate of `(group, slot)`; `slot = e1` is "no slot".
    fn idx(&self, g: usize, s: usize) -> usize {
        g * (self.e1 + 1) + s
    }
    /// The function `state -> bit k of value(leaf)` (0 where there is no leaf).
    fn bits(&self, value: &dyn Fn(usize) -> u64, k: usize) -> V {
        let mut v = zero(self.words);
        for g in 0..self.groups {
            for s in 0..self.e1 {
                if self.at[g][s].is_some_and(|e| value(e) >> k & 1 == 1) {
                    set(&mut v, self.idx(g, s));
                }
            }
        }
        v
    }
    fn slot_fn(&self, s: usize) -> V {
        let mut v = zero(self.words);
        for g in 0..self.groups {
            set(&mut v, self.idx(g, s));
        }
        self.mask(v)
    }
    fn group_fn(&self, i: usize) -> V {
        let mut v = zero(self.words);
        for s in 0..=self.e1 {
            set(&mut v, self.idx(i, s));
        }
        self.mask(v)
    }
    /// The base `span{h_s, g_i}` with source masks over `[slots .., group bits .., extra]`.
    fn base(&self, extra: usize) -> Elim {
        let mut el = Elim::new((self.e1 + self.groups - 1 + extra).div_ceil(64));
        for s in 0..self.e1 {
            let mut m = zero(el.mw);
            set(&mut m, s);
            el.insert(&self.slot_fn(s), &m);
        }
        for i in 1..self.groups {
            let mut m = zero(el.mw);
            set(&mut m, self.e1 + i - 1);
            el.insert(&self.group_fn(i), &m);
        }
        el
    }
}

/// The delivery plan of one copy: rotation order `seq` (`V^dagger` then `V`), window starts
/// (positions in `seq`) and each window's span (residues mod the base).
#[derive(Clone, Debug)]
pub struct Plan {
    pub seq: Vec<usize>,
    pub starts: Vec<usize>,
    pub spans: Vec<Vec<V>>,
    /// Toffolis per copy (the buys).
    pub buys: usize,
    /// `D'`: the dimension of all words mod the base.
    pub dprime: usize,
    /// Largest window dimension.
    pub max_dim: usize,
    /// Mode 1 ([`plan_belady`]): every position's held span (empty for windows).
    pub per_pos: Vec<Vec<V>>,
    /// `q<b>_1.<c>`: the span held across the Majorana, cut before position
    /// `.0` (the first rotation of `V`) to at most `c` dimensions.
    pub park: Option<(usize, Vec<V>)>,
}

fn word_residues(
    lay: &Layout,
    base: &Elim,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
) -> Vec<Vec<V>> {
    (0..rots)
        .map(|j| {
            (0..width)
                .map(|k| base.residue(&lay.bits(&|e| angle(e, j), k)))
                .filter(|r| !is_zero(r))
                .collect()
        })
        .collect()
}

fn span_dim(vs: &[V], mw: usize) -> usize {
    let mut el = Elim::new(mw);
    let m = zero(1);
    vs.iter().filter(|v| el.insert(v, &m)).count()
}

/// Window plan by dynamic programming over contiguous windows of `seq` with span dimension at
/// most `budget`: minimises the total buys. Panics if one rotation's words exceed the budget.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn plan(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
) -> Plan {
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    let m = seq.len();
    let dprime = span_dim(&nres.concat(), 1);
    // Window spans seq[i..j] with dim <= budget: basis vectors and dimension.
    let mut win: Vec<Vec<(usize, Vec<V>)>> = vec![Vec::new(); m]; // win[i][j - i - 1]
    for i in 0..m {
        let mut el = Elim::new(1);
        let mut vs: Vec<V> = Vec::new();
        let z = zero(1);
        for &t in &seq[i..m] {
            for v in &nres[t] {
                if el.insert(v, &z) {
                    vs.push(v.clone());
                }
            }
            if vs.len() > budget {
                break;
            }
            win[i].push((vs.len(), vs.clone()));
        }
        assert!(
            !win[i].is_empty(),
            "lever q: one rotation's words need more than the budget {budget}"
        );
    }
    let w = |i: usize, j: usize| win[i].get(j - i - 1);
    // sums[i][h][l]: dim(F_[i-h-1, i) + F_[i, i+l+1)), incrementally over l.
    let mut f: Vec<Vec<usize>> = (0..m).map(|i| vec![usize::MAX; win[i].len()]).collect();
    let mut from: Vec<Vec<usize>> = (0..m).map(|i| vec![0; win[i].len()]).collect();
    let inf = usize::MAX;
    let z = zero(1);
    for l in 0..win[0].len() {
        f[0][l] = win[0][l].0;
    }
    for i in 1..m {
        for h in (0..i).rev() {
            let Some((dhi, vhi)) = w(h, i) else { break };
            let fh = f[h][i - h - 1];
            if fh == inf {
                continue;
            }
            let mut el = Elim::new(1);
            vhi.iter().for_each(|v| {
                el.insert(v, &z);
            });
            for l in 0..win[i].len() {
                for v in &nres[seq[i + l]] {
                    el.insert(v, &z);
                }
                let dij = win[i][l].0;
                let inter = dhi + dij - el.dim();
                let c = fh + dij - inter;
                if c < f[i][l] {
                    f[i][l] = c;
                    from[i][l] = h;
                }
            }
        }
    }
    let (mut bi, mut bc) = (0, inf);
    for i in 0..m {
        if let Some(&c) = f[i].get(m - i - 1) {
            if c < bc {
                (bi, bc) = (i, c);
            }
        }
    }
    let mut starts = vec![bi];
    let mut j = m;
    let mut i = bi;
    while i > 0 {
        let h = from[i][j - i - 1];
        j = i;
        i = h;
        starts.push(i);
    }
    starts.reverse();
    let mut ends = starts[1..].to_vec();
    ends.push(m);
    let spans: Vec<Vec<V>> = starts
        .iter()
        .zip(&ends)
        .map(|(&a, &bnd)| w(a, bnd).unwrap().1.clone())
        .collect();
    let max_dim = spans.iter().map(Vec::len).max().unwrap_or(0);
    Plan {
        seq,
        starts,
        spans,
        buys: bc,
        dprime,
        max_dim,
        per_pos: Vec::new(),
        park: None,
    }
}

/// Mode 1: the plan of [`belady`] (one held span per position of the copy's sequence); with
/// `park > 0` the span held across the Majorana (between position `rots - 1`, the
/// last of `V^dagger`, and `rots`, the first of `V`) is cut to at most `park` dimensions.
#[must_use]
pub fn plan_belady(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
    park: usize,
    rollout: usize,
) -> Plan {
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    assert!(
        nres.iter().all(|n| span_dim(n, 1) <= budget),
        "lever q: one rotation's words need more than the budget {budget}"
    );
    let dprime = span_dim(&nres.concat(), 1);
    let cut = (park > 0).then_some((rots, park));
    let (xs, parked, buys) = if rollout > 0 {
        belady_rollout(&nres, &seq, budget, cut, rollout)
    } else {
        belady(&nres, &seq, budget, cut)
    };
    // The run count with the parked span as a position of its own equals the buys.
    let mut xs2 = xs.clone();
    if let Some(k) = &parked {
        xs2.insert(rots, k.clone());
    }
    let (starts, spans, b2) = runs(&xs2);
    assert_eq!(b2, buys);
    let max_dim = xs.iter().map(Vec::len).max().unwrap_or(0);
    Plan {
        seq,
        starts,
        spans,
        buys,
        dprime,
        max_dim,
        per_pos: xs,
        park: parked.map(|k| (rots, k)),
    }
}

/// Recorded row operations to the reduced row echelon form: `(from, to)` means row `to ^= row
/// from`; `piv[c]` the row holding pivot column `c` (the RREF rows are unique for a row space).
type Rref = (Vec<(usize, usize)>, Vec<(usize, usize)>, Vec<V>);

fn rref_ops(rows: &[V]) -> Rref {
    let mut r = rows.to_vec();
    let mut used = vec![false; r.len()];
    let mut ops = Vec::new();
    let mut piv = Vec::new();
    loop {
        let best = (0..r.len())
            .filter(|&i| !used[i])
            .filter_map(|i| lowest(&r[i]).map(|c| (c, i)))
            .min();
        let Some((c, i)) = best else { break };
        used[i] = true;
        piv.push((c, i));
        let pr = r[i].clone();
        for k in 0..r.len() {
            if k != i && get(&r[k], c) {
                xor(&mut r[k], &pr);
                ops.push((i, k));
            }
        }
    }
    (ops, piv, r)
}

/// The delivery state of one copy.
pub struct Deliver<'a> {
    lay: Layout,
    plan: Plan,
    angle: &'a dyn Fn(usize, usize) -> u64,
    reg: Vec<Qubit>,
    /// Exact leaf functions of the register bits.
    reg_f: Vec<V>,
    /// Held qubits and their exact functions.
    held: Vec<(Qubit, V)>,
    pos: usize,
    win: usize,
    /// The base plus the held qubits, with source masks over `[slots, groups, held]`.
    o: Option<Elim>,
    rng: u64,
    /// Directions bought.
    pub buys: usize,
    /// Toffolis emitted (`buys` for `G <= 3`; a `G >= 4` direction costs its
    /// pair products, `(G - 1) / 2` on average with [`buy_two`]).
    pub toffolis: usize,
    /// Whether [`Self::park`] has cut the held span at the Majorana gap.
    parked: bool,
}

impl<'a> Deliver<'a> {
    /// A copy's delivery over `reg` (the Givens register, `|0>`), with `budget` the largest window
    /// dimension.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hot: &Hot,
        n_leaves: usize,
        reg: &[Qubit],
        angle: &'a dyn Fn(usize, usize) -> u64,
        rots: usize,
        budget: usize,
        mode: u8,
        reach: Option<&[Vec<bool>]>,
        park: usize,
    ) -> Self {
        assert!(
            !hot.g.is_empty(),
            "lever q needs a split one-hot (at least 2 groups)"
        );
        assert!(
            park == 0 || mode == 1,
            "lever q: the Majorana budget needs mode 1"
        );
        let lay = Layout::of(hot, n_leaves, reach);
        let plan = if mode == 1 {
            plan_belady(&lay, angle, rots, reg.len(), budget, park, 0)
        } else {
            plan(&lay, angle, rots, reg.len(), budget)
        };
        Self {
            reg_f: vec![zero(lay.words); reg.len()],
            lay,
            plan,
            angle,
            reg: reg.to_vec(),
            held: Vec::new(),
            pos: 0,
            win: 0,
            o: None,
            rng: 0x2545_F491_4F6C_DD1D,
            buys: 0,
            toffolis: 0,
            parked: false,
        }
    }

    #[must_use]
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The register and held qubits that hold a nonzero function (the delivery's live rows).
    #[must_use]
    pub fn live(&self) -> Vec<Qubit> {
        self.reg
            .iter()
            .zip(&self.reg_f)
            .filter(|(_, f)| !is_zero(f))
            .map(|(q, _)| *q)
            .chain(self.held.iter().filter(|h| !is_zero(&h.1)).map(|h| h.0))
            .collect()
    }

    fn target(&self, j: usize) -> Vec<V> {
        (0..self.reg.len())
            .map(|k| self.lay.bits(&|e| (self.angle)(e, j), k))
            .collect()
    }

    /// Loads the register for the next rotation of the copy's sequence (call before its Givens).
    pub fn step(&mut self, b: &mut Builder, hot: &Hot, j: usize) {
        assert_eq!(self.plan.seq[self.pos], j, "lever q: rotation order");
        if let Some((gap, _)) = &self.plan.park {
            assert!(
                self.pos != *gap || self.parked,
                "lever q: the Majorana budget needs park() before V"
            );
        }
        if !self.plan.per_pos.is_empty() {
            self.step_belady(b, hot, j);
        } else if self.plan.starts.get(self.win) == Some(&self.pos) {
            self.switch(b, hot);
            self.win += 1;
        } else if self.plan.seq[self.pos - 1] != j {
            let tgt = self.target(j);
            let o = self.o.as_ref().expect("window open");
            let tres: Vec<V> = tgt.iter().map(|t| o.residue(t)).collect();
            let src = self.sources(hot);
            let rq = self.reg.clone();
            let mut cur = std::mem::take(&mut self.reg_f);
            let ex: Vec<Option<V>> = tgt.into_iter().map(Some).collect();
            retarget(b, &rq, &mut cur, &tres, &ex, o, &src, &self.funcs(hot));
            self.reg_f = cur;
        }
        self.pos += 1;
    }

    /// Mode 1: one position of [`plan_belady`]. The pool (register and held rows) spans the
    /// previous held span `P`; `K = P ∩ X` is kept: every row's image in `P / K` is eliminated by
    /// CNOTs from a few pivot rows, which are measured out (Clifford fixups), with any row left
    /// redundant. Then each register bit `k` gets a row holding `W_t[k]` exactly: bought (one pair
    /// product) when its residue is outside the pool's span, else gathered by CNOTs into an
    /// unassigned row of its expression (a fresh row if it depends on bits already placed), plus a
    /// base fix; the rows are swapped into the register's qubits.
    #[allow(clippy::too_many_lines)]
    fn step_belady(&mut self, b: &mut Builder, hot: &Hot, j: usize) {
        let w = self.lay.words;
        let z = zero(1);
        let base = self.lay.base(0);
        let x_new = self.plan.per_pos[self.pos].clone();
        let nreg = self.reg.len();
        let mut q: Vec<Qubit> = self.reg.clone();
        q.extend(self.held.iter().map(|h| h.0));
        let mut f: Vec<V> = std::mem::take(&mut self.reg_f);
        f.extend(self.held.drain(..).map(|h| h.1));
        // (1) Keep K = span(pool) ∩ X: eliminate the images in span(pool) / K.
        self.keep_only(b, hot, &q, &mut f, &x_new);
        // (2) Place every register bit.
        let tgt = self.target(j);
        let mut owner: Vec<Option<usize>> = vec![None; q.len()]; // row -> bit
        let mut src: Vec<Qubit> = hot.q.iter().chain(hot.g.iter()).copied().collect();
        src.truncate(self.lay.e1 + self.lay.groups - 1);
        let sf = {
            let saved = std::mem::take(&mut self.held);
            let v = self.funcs(hot);
            self.held = saved;
            v
        };
        // With G >= 4 the step's buys are found first (greedily in bit order,
        // as the loop below would find them) and emitted two at a time ([`buy_two`], lever `Z`'s
        // triple) into free rows; the loop below then only gathers.
        if self.lay.groups >= 4 {
            let mut er = Elim::new(1);
            for fi in &f {
                if !is_zero(fi) {
                    er.insert(&base.residue(fi), &z);
                }
            }
            let need: Vec<(usize, V)> = (0..nreg)
                .map(|k| (k, base.residue(&tgt[k])))
                .filter(|(_, tk)| er.insert(tk, &z))
                .collect();
            let mut taken = vec![false; q.len()];
            let mut rows = Vec::with_capacity(need.len());
            for &(k, _) in &need {
                let i = if !taken[k] && is_zero(&f[k]) {
                    Some(k)
                } else {
                    (0..q.len()).find(|&i| !taken[i] && is_zero(&f[i]))
                };
                let i = i.unwrap_or_else(|| {
                    q.push(b.alloc());
                    f.push(zero(w));
                    owner.push(None);
                    taken.push(false);
                    q.len() - 1
                });
                taken[i] = true;
                rows.push(i);
            }
            for (c, two) in need.chunks(2).enumerate() {
                let r0 = rows[2 * c];
                if let [(_, ta), (_, tb)] = two {
                    let r1 = rows[2 * c + 1];
                    let (ca, cb) = (comps_of(&self.lay, ta), comps_of(&self.lay, tb));
                    let (fa, fb, nt) = buy_two(b, hot, &self.lay, &ca, &cb, (q[r0], q[r1]));
                    f[r0] = fa;
                    f[r1] = fb;
                    self.buys += 2;
                    self.toffolis += nt;
                } else {
                    let (fa, nt) = buy(b, hot, &self.lay, &two[0].1, q[r0]);
                    f[r0] = fa;
                    self.buys += 1;
                    self.toffolis += nt;
                }
            }
        }
        for k in 0..nreg {
            let tk = base.residue(&tgt[k]);
            // A free row: the register's own qubit if free, else any, else a new qubit.
            let free_row = |f: &Vec<V>, owner: &Vec<Option<usize>>| -> Option<usize> {
                if owner[k].is_none() && is_zero(&f[k]) {
                    return Some(k);
                }
                (0..f.len()).find(|&i| owner[i].is_none() && is_zero(&f[i]))
            };
            let mw = q.len().div_ceil(64).max(1);
            let mut er = Elim::new(mw);
            for i in 0..q.len() {
                if !is_zero(&f[i]) {
                    let mut m = zero(mw);
                    set(&mut m, i);
                    er.insert(&base.residue(&f[i]), &m);
                }
            }
            let (r, mask) = er.reduce(&tk);
            let row = if !is_zero(&r) {
                let i = free_row(&f, &owner).unwrap_or_else(|| {
                    q.push(b.alloc());
                    f.push(zero(w));
                    owner.push(None);
                    q.len() - 1
                });
                let (fi, nt) = buy(b, hot, &self.lay, &tk, q[i]);
                f[i] = fi;
                self.buys += 1;
                self.toffolis += nt;
                i
            } else {
                let rows: Vec<usize> = ones(&mask).collect();
                let pick = if rows.contains(&k) && owner[k].is_none() {
                    Some(k)
                } else {
                    rows.iter().copied().find(|&i| owner[i].is_none())
                };
                let i = pick.unwrap_or_else(|| {
                    free_row(&f, &owner).unwrap_or_else(|| {
                        q.push(b.alloc());
                        f.push(zero(w));
                        owner.push(None);
                        q.len() - 1
                    })
                });
                for &r2 in rows.iter().filter(|&&r2| r2 != i) {
                    b.cx(q[r2], q[i]);
                    let fr = f[r2].clone();
                    xor(&mut f[i], &fr);
                }
                i
            };
            owner[row] = Some(k);
            // Base fix: the row holds W_t[k] exactly.
            let mut delta = tgt[k].clone();
            xor(&mut delta, &f[row]);
            let (r2, m2) = base.reduce(&delta);
            assert!(is_zero(&r2), "lever q: placed row off its residue");
            if !(fault_is(92) && k == 0) {
                for s2 in ones(&m2) {
                    b.cx(src[s2], q[row]);
                    xor(&mut f[row], &sf[s2]);
                }
            }
        }
        // (3) Swap each bit's row onto its register qubit.
        for k in 0..nreg {
            let row = owner.iter().position(|o| *o == Some(k)).unwrap();
            if row != k {
                b.swap(q[row], q[k]);
                f.swap(row, k);
                owner.swap(row, k);
            }
        }
        self.reg_f = f[..nreg].to_vec();
        for i in nreg..q.len() {
            if is_zero(&f[i]) {
                b.free(q[i]);
            } else {
                self.held.push((q[i], f[i].clone()));
            }
        }
    }

    /// Keeps `K = span(pool) ∩ X` in the pool rows `q` (functions `f`): every row's image in
    /// `span(pool) / K` is eliminated by CNOTs from a few pivot rows, which are measured out
    /// (Clifford fixups on the one-hot), with any row left redundant or a pure base function.
    fn keep_only(&self, b: &mut Builder, hot: &Hot, q: &[Qubit], f: &mut [V], x_new: &[V]) {
        let w = self.lay.words;
        let z = zero(1);
        let base = self.lay.base(0);
        let res: Vec<V> = f.iter().map(|v| base.residue(v)).collect();
        let kb = inter_of(&sum_of(&res, &[]), x_new);
        let mut ek = Elim::new(1);
        kb.iter().for_each(|v| {
            ek.insert(v, &z);
        });
        let mut img: Vec<V> = res.iter().map(|r| ek.residue(r)).collect();
        let mut piv: Vec<(usize, usize)> = Vec::new(); // (column, row)
        let mut dropped_one = false;
        for i in 0..q.len() {
            while let Some(c) = lowest(&img[i]) {
                let Some(&(_, pr)) = piv.iter().find(|x| x.0 == c) else {
                    piv.push((c, i));
                    break;
                };
                if !fault_is(93) || dropped_one {
                    b.cx(q[pr], q[i]);
                }
                dropped_one = true;
                let (fp, ip) = (f[pr].clone(), img[pr].clone());
                xor(&mut f[i], &fp);
                xor(&mut img[i], &ip);
            }
        }
        let mut gone: Vec<usize> = piv.iter().map(|x| x.1).collect();
        // Rows now in K that are redundant (or pure base functions) go too.
        let mut el = Elim::new(1);
        for i in 0..q.len() {
            if gone.contains(&i) || is_zero(&f[i]) {
                continue;
            }
            if !el.insert(&base.residue(&f[i]), &z) {
                gone.push(i);
            }
        }
        for &i in &gone {
            let fi = std::mem::replace(&mut f[i], zero(w));
            if !is_zero(&fi) {
                let m = crate::walk::common::lookup::hmr_keep(b, q[i]);
                phase(b, hot, &self.lay, &fi, m);
                super::onehot::reset_keep(b, q[i]);
            }
        }
    }

    /// `q<b>_1.<c>`: call between `V^dagger` and the Majorana. Cuts the pool to the
    /// plan's parked span (at most `c` rows live; the rest measured out, 0 Toffolis); the first
    /// load of `V` buys back what the parked span lacks. No-op without the Majorana budget.
    pub fn park(&mut self, b: &mut Builder, hot: &Hot) {
        let Some((gap, k)) = self.plan.park.clone() else {
            return;
        };
        assert_eq!(self.pos, gap, "lever q: park() between V^dagger and V");
        let nreg = self.reg.len();
        let mut q: Vec<Qubit> = self.reg.clone();
        q.extend(self.held.iter().map(|h| h.0));
        let mut f: Vec<V> = std::mem::take(&mut self.reg_f);
        f.extend(self.held.drain(..).map(|h| h.1));
        if !fault_is(133) {
            self.keep_only(b, hot, &q, &mut f, &k);
        }
        self.reg_f = f[..nreg].to_vec();
        for i in nreg..q.len() {
            if is_zero(&f[i]) {
                b.free(q[i]);
            } else {
                self.held.push((q[i], f[i].clone()));
            }
        }
        self.parked = true;
    }

    /// Measures every register and held qubit out (Clifford fixups): the copy's end.
    pub fn finish(&mut self, b: &mut Builder, hot: &Hot) {
        assert_eq!(self.pos, self.plan.seq.len(), "lever q: copy not complete");
        for k in 0..self.reg.len() {
            let f = std::mem::replace(&mut self.reg_f[k], zero(self.lay.words));
            if !is_zero(&f) {
                let m = crate::walk::common::lookup::hmr_keep(b, self.reg[k]);
                phase(b, hot, &self.lay, &f, m);
            }
            super::onehot::reset_keep(b, self.reg[k]);
        }
        for (q, f) in std::mem::take(&mut self.held) {
            if is_zero(&f) {
                b.free(q);
            } else {
                let m = b.hmr(q);
                phase(b, hot, &self.lay, &f, m);
            }
        }
        self.o = None;
    }

    fn sources(&self, hot: &Hot) -> Vec<Qubit> {
        hot.q
            .iter()
            .chain(hot.g.iter())
            .copied()
            .chain(self.held.iter().map(|h| h.0))
            .collect()
    }

    /// Functions of [`Self::sources`].
    fn funcs(&self, _hot: &Hot) -> Vec<V> {
        let mut out: Vec<V> = (0..self.lay.e1).map(|s| self.lay.slot_fn(s)).collect();
        out.extend((1..self.lay.groups).map(|i| self.lay.group_fn(i)));
        out.extend(self.held.iter().map(|h| h.1.clone()));
        out
    }

    /// Window switch at `self.pos`: keep the intersection with the new span, measure the rest,
    /// buy the new directions, choose the held complement and load the register.
    #[allow(clippy::too_many_lines)]
    fn switch(&mut self, b: &mut Builder, hot: &Hot) {
        let w = self.lay.words;
        let base = self.lay.base(0);
        let fb: Vec<V> = self.plan.spans[self.win].clone();
        let ends: Vec<usize> = self.plan.starts[1..]
            .iter()
            .copied()
            .chain([self.plan.seq.len()])
            .collect();
        let window: Vec<usize> = self.plan.seq[self.pos..ends[self.win]].to_vec();
        // Pool: register rows then held rows.
        let mut pool_q: Vec<Qubit> = self.reg.clone();
        pool_q.extend(self.held.iter().map(|h| h.0));
        let mut pool_f: Vec<V> = std::mem::take(&mut self.reg_f);
        pool_f.extend(self.held.drain(..).map(|h| h.1));
        let nreg = self.reg.len();
        // K = span(pool residues) ∩ F_B (Zassenhaus).
        let k_basis = {
            let mut el = Elim::new(1);
            let z1 = zero(1);
            let cat = |a: &V, bb: &V| -> V { a.iter().chain(bb.iter()).copied().collect() };
            for f in &pool_f {
                let r = base.residue(f);
                el.insert(&cat(&r, &r), &z1);
            }
            for v in &fb {
                el.insert(&cat(v, &zero(w)), &z1);
            }
            el.rows
                .iter()
                .filter(|(_, v, _)| is_zero(&v[..w]))
                .map(|(_, v, _)| v[w..].to_vec())
                .collect::<Vec<V>>()
        };
        // Bring a basis of K into `kept` rows by CNOTs among the pool rows.
        let mut kept = vec![false; pool_q.len()];
        let mut dropped = false;
        for y in &k_basis {
            let mut el = Elim::new(pool_q.len().div_ceil(64));
            for (i, f) in pool_f.iter().enumerate() {
                let mut m = zero(pool_q.len().div_ceil(64));
                set(&mut m, i);
                el.insert(&base.residue(f), &m);
            }
            let (r, mask) = el.reduce(y);
            assert!(is_zero(&r), "lever q: K not in the pool span");
            let i = ones(&mask)
                .find(|&i| !kept[i])
                .expect("lever q: K vector in the kept span");
            for jj in ones(&mask).filter(|&jj| jj != i).collect::<Vec<_>>() {
                if !fault_is(93) || dropped {
                    b.cx(pool_q[jj], pool_q[i]);
                }
                dropped = true;
                let fj = pool_f[jj].clone();
                xor(&mut pool_f[i], &fj);
            }
            kept[i] = true;
        }
        // Measure every other row out.
        for i in 0..pool_q.len() {
            if kept[i] || is_zero(&pool_f[i]) {
                continue;
            }
            let f = std::mem::replace(&mut pool_f[i], zero(w));
            let m = crate::walk::common::lookup::hmr_keep(b, pool_q[i]);
            phase(b, hot, &self.lay, &f, m);
            super::onehot::reset_keep(b, pool_q[i]);
        }
        // Buy F_B / K: one pair product each, into a free row (register first, then new qubits).
        let mut el = Elim::new(1);
        let z1 = zero(1);
        for i in 0..pool_q.len() {
            if kept[i] {
                el.insert(&base.residue(&pool_f[i]), &z1);
            }
        }
        for v in &fb {
            if el.reduce(v).0.iter().all(|&x| x == 0) {
                continue;
            }
            el.insert(v, &z1);
            let free = (0..pool_q.len()).find(|&i| !kept[i] && is_zero(&pool_f[i]));
            let i = free.unwrap_or_else(|| {
                pool_q.push(b.alloc());
                pool_f.push(zero(w));
                kept.push(false);
                pool_q.len() - 1
            });
            let (fi, nt) = buy(b, hot, &self.lay, v, pool_q[i]);
            pool_f[i] = fi;
            kept[i] = true;
            self.buys += 1;
            self.toffolis += nt;
        }
        // The held complement: random vectors of F_B with H + span(W_t) = F_B for each t.
        let tres: Vec<Vec<V>> = window
            .iter()
            .map(|&t| {
                self.target(t)
                    .iter()
                    .map(|v| base.residue(v))
                    .collect::<Vec<V>>()
            })
            .collect();
        let hvec = complement(&fb, &tres, &mut self.rng);
        let hb = hvec.len();
        // Rows: register (exact targets), then held rows (residue targets), extras to zero.
        let nh = pool_q.len() - nreg;
        for _ in nh..hb {
            pool_q.push(b.alloc());
            pool_f.push(zero(w));
        }
        let first = self.plan.seq[self.pos];
        let tgt = self.target(first);
        let mut tres_rows: Vec<V> = tgt.iter().map(|v| base.residue(v)).collect();
        let mut exact: Vec<Option<V>> = tgt.into_iter().map(Some).collect();
        for i in 0..pool_q.len() - nreg {
            if i < hb {
                tres_rows.push(hvec[i].clone());
                exact.push(None);
            } else {
                tres_rows.push(zero(w));
                exact.push(Some(zero(w)));
            }
        }
        let mut src: Vec<Qubit> = hot.q.iter().chain(hot.g.iter()).copied().collect();
        src.truncate(self.lay.e1 + self.lay.groups - 1);
        let bfuncs = {
            let saved = std::mem::take(&mut self.held);
            let f = self.funcs(hot);
            self.held = saved;
            f
        };
        retarget(
            b,
            &pool_q,
            &mut pool_f,
            &tres_rows,
            &exact,
            &base,
            &src,
            &bfuncs,
        );
        // Split back; free the zero extras.
        self.reg_f = pool_f[..nreg].to_vec();
        for i in nreg..pool_q.len() {
            if i - nreg < hb {
                self.held.push((pool_q[i], pool_f[i].clone()));
            } else {
                assert!(is_zero(&pool_f[i]));
                b.free(pool_q[i]);
            }
        }
        // The window's elimination: base plus held, masks over [slots, groups, held].
        let mut o = self.lay.base(self.held.len());
        for (k, (_, f)) in self.held.iter().enumerate() {
            let mut m = zero(o.mw);
            set(&mut m, self.lay.e1 + self.lay.groups - 1 + k);
            o.insert(f, &m);
        }
        self.o = Some(o);
    }
}

/// A subspace `H` of `span(fb)` with `H + span(ts) = span(fb)` for every window rotation's words
/// `ts` (all inside `span(fb)`), of the largest dimension found: `H = ker(pi)` for a random map
/// `pi` onto `n` bits that is onto on every `span(ts)`; `n` starts at the smallest word rank and
/// drops by one after 4,000 failed draws (each step costs one held qubit).
fn complement(fb: &[V], tres: &[Vec<V>], rng: &mut u64) -> Vec<V> {
    let d = fb.len();
    let cw = d.div_ceil(64).max(1);
    let mut el = Elim::new(cw);
    for (i, v) in fb.iter().enumerate() {
        let mut m = zero(cw);
        set(&mut m, i);
        assert!(el.insert(v, &m), "lever q: window basis dependent");
    }
    let coords: Vec<Vec<V>> = tres
        .iter()
        .map(|ts| {
            ts.iter()
                .filter(|v| !is_zero(v))
                .map(|v| {
                    let (r, m) = el.reduce(v);
                    assert!(is_zero(&r), "lever q: word outside its window");
                    m
                })
                .collect()
        })
        .collect();
    let rho = coords.iter().map(|cs| span_dim(cs, 1)).min().unwrap_or(0);
    let mut next = || {
        *rng ^= *rng << 13;
        *rng ^= *rng >> 7;
        *rng ^= *rng << 17;
        *rng
    };
    let parity = |a: &[u64], c: &[u64]| {
        a.iter()
            .zip(c)
            .fold(0u32, |x, (p, q)| x ^ (p & q).count_ones())
            & 1
    };
    for n in (0..=rho).rev() {
        for _ in 0..4000 {
            let pi: Vec<V> = (0..n)
                .map(|_| {
                    let mut v: V = (0..cw).map(|_| next()).collect();
                    if !d.is_multiple_of(64) {
                        v[cw - 1] &= (1u64 << (d % 64)) - 1;
                    }
                    v
                })
                .collect();
            let image = |c: &V| -> V {
                let mut x = 0u64;
                for (i, p) in pi.iter().enumerate() {
                    x |= u64::from(parity(p, c)) << i;
                }
                vec![x]
            };
            let onto = coords.iter().all(|cs| {
                let im: Vec<V> = cs.iter().map(image).collect();
                span_dim(&im, 1) == n
            });
            if !onto {
                continue;
            }
            // ker(pi) in coordinates, then as leaf vectors.
            let mut ke = Elim::new(cw);
            let mut out = Vec::new();
            for j in 0..d {
                let mut e = zero(cw);
                set(&mut e, j);
                let img = image(&e);
                let (r, m) = ke.reduce(&img);
                if is_zero(&r) {
                    let mut v = zero(fb[0].len());
                    let mut mm = m;
                    xor(&mut mm, &e);
                    for i in ones(&mm) {
                        xor(&mut v, &fb[i]);
                    }
                    out.push(v);
                } else {
                    ke.insert(&img, &e);
                }
            }
            assert_eq!(out.len(), d - n);
            return out;
        }
    }
    unreachable!("n = 0 always succeeds")
}

/// One Toffoli: a fresh `|0>` qubit `t` gets a function `f` with `f = v` modulo the base. G = 3:
/// `(g_1 ^ X(h))(g_2 ^ Y(h))` with `Y` group 1's and `X` group 2's per-slot difference from group
/// 0 (the parities XORed into the group bits in place and undone). G = 2: `g_1 Y(h)` with `Y(h)`
/// formed in place on one slot qubit of `Y`. Returns `f` exactly.
fn buy(b: &mut Builder, hot: &Hot, lay: &Layout, v: &[u64], t: Qubit) -> (V, usize) {
    let val = |g: usize, s: usize| get(v, lay.idx(g, s));
    let none = lay.e1;
    // Per-slot group differences with the pure group terms (the no-slot states) taken out, so
    // that `f - v` is in the base.
    let d = |g: usize, s: usize| val(g, s) ^ val(0, s) ^ val(g, none) ^ val(0, none);
    let mut f = zero(lay.words);
    if lay.groups >= 4 {
        let comps: Vec<Vec<bool>> = (0..lay.groups)
            .map(|g| (0..lay.e1).map(|s| g > 0 && d(g, s)).collect())
            .collect();
        return buy_general(b, hot, lay, &comps, t);
    }
    let y: Vec<bool> = (0..lay.e1).map(|s| d(1, s)).collect();
    if lay.groups == 3 {
        let x: Vec<bool> = (0..lay.e1).map(|s| d(2, s)).collect();
        let (a, c) = (hot.g[0], hot.g[1]);
        let fan = |b: &mut Builder| {
            for s in 0..lay.e1 {
                if x[s] && !fault_is(90) {
                    b.cx(hot.q[s], a);
                }
                if y[s] {
                    b.cx(hot.q[s], c);
                }
            }
        };
        fan(b);
        b.ccx(a, c, t);
        fan(b);
        for g in 0..3 {
            for s in 0..lay.e1 {
                if ((g == 1) ^ x[s]) && ((g == 2) ^ y[s]) {
                    set(&mut f, lay.idx(g, s));
                }
            }
        }
        f = lay.mask(f);
    } else {
        let ys: Vec<usize> = (0..lay.e1).filter(|&s| y[s]).collect();
        let Some((&s0, rest)) = ys.split_first() else {
            return (f, 0);
        };
        let fan = |b: &mut Builder| {
            if !fault_is(90) {
                rest.iter().for_each(|&s| b.cx(hot.q[s], hot.q[s0]));
            }
        };
        fan(b);
        b.ccx(hot.g[0], hot.q[s0], t);
        fan(b);
        for &s in &ys {
            set(&mut f, lay.idx(1, s));
        }
        f = lay.mask(f);
    }
    (f, 1)
}

/// The per-group components of `v`: `comps[g][s]` is group `g`'s per-slot
/// difference from group 0 with the pure group terms (the no-slot states) taken out, so that any
/// function with these components equals `v` modulo the base. `comps[0]` is all false.
fn comps_of(lay: &Layout, v: &[u64]) -> Vec<Vec<bool>> {
    let val = |g: usize, s: usize| get(v, lay.idx(g, s));
    let none = lay.e1;
    (0..lay.groups)
        .map(|g| {
            (0..lay.e1)
                .map(|s| g > 0 && (val(g, s) ^ val(0, s) ^ val(g, none) ^ val(0, none)))
                .collect()
        })
        .collect()
}

/// One pair product into `t` (any `G >= 3`): group `ga` gets component `ua`
/// and group `gc` component `uc`, every other group 0 (modulo the base). With exclusive group
/// bits, `(g_ga ^ X(h))(g_gc ^ Y(h)) = g_ga Y ^ g_gc X ^ (X Y)(h)`, so `X = uc` is XORed into
/// `g_ga` and `Y = ua` into `g_gc` (parities of slot qubits, undone after). A single component
/// (`uc` all zero) is the same product with `X = 0`: `g_ga (g_gc ^ Y) = g_ga Y`. Returns the
/// product's exact function. One Toffoli.
fn pair_product(
    b: &mut Builder,
    hot: &Hot,
    lay: &Layout,
    (ga, ua): (usize, &[bool]),
    (gc, uc): (usize, &[bool]),
    t: Qubit,
) -> V {
    assert!(ga != gc && ga >= 1 && gc >= 1, "lever q: pair groups");
    let (a, c) = (hot.g[ga - 1], hot.g[gc - 1]);
    let fan = |b: &mut Builder| {
        for s in 0..lay.e1 {
            if uc[s] && !fault_is(90) {
                b.cx(hot.q[s], a);
            }
            if ua[s] {
                b.cx(hot.q[s], c);
            }
        }
    };
    fan(b);
    b.ccx(a, c, t);
    fan(b);
    let mut f = zero(lay.words);
    for g in 0..lay.groups {
        for s in 0..lay.e1 {
            if ((g == ga) ^ uc[s]) && ((g == gc) ^ ua[s]) {
                set(&mut f, lay.idx(g, s));
            }
        }
    }
    lay.mask(f)
}

fn any(u: &[bool]) -> bool {
    u.iter().any(|&x| x)
}

/// The nonzero groups of `comps`, in pairs (the last alone when their number is odd).
fn pairs_of(comps: &[Vec<bool>], groups: &[usize]) -> Vec<(usize, Option<usize>)> {
    let gs: Vec<usize> = groups.iter().copied().filter(|&g| any(&comps[g])).collect();
    gs.chunks(2).map(|c| (c[0], c.get(1).copied())).collect()
}

/// Emits the pair products of `pairs` into `t`; returns the summed exact function.
fn emit_pairs(
    b: &mut Builder,
    hot: &Hot,
    lay: &Layout,
    comps: &[Vec<bool>],
    pairs: &[(usize, Option<usize>)],
    t: Qubit,
) -> V {
    let z: Vec<bool> = vec![false; lay.e1];
    let mut f = zero(lay.words);
    for &(ga, gc) in pairs {
        // A lone component: any other group bit carries the parity (its own component is 0).
        let (gc, uc) = match gc {
            Some(gc) => (gc, comps[gc].as_slice()),
            None => (if ga == 1 { 2 } else { 1 }, z.as_slice()),
        };
        let p = pair_product(b, hot, lay, (ga, &comps[ga]), (gc, uc), t);
        xor(&mut f, &p);
    }
    f
}

/// A buy for `G >= 4`: the nonzero components in pairs, one Toffoli per pair
/// and per lone component (`ceil(nonzero / 2)`, at most `ceil((G - 1) / 2)`). Returns the exact
/// function and the Toffolis.
fn buy_general(
    b: &mut Builder,
    hot: &Hot,
    lay: &Layout,
    comps: &[Vec<bool>],
    t: Qubit,
) -> (V, usize) {
    let all: Vec<usize> = (1..lay.groups).collect();
    let pairs = pairs_of(comps, &all);
    (emit_pairs(b, hot, lay, comps, &pairs, t), pairs.len())
}

/// Two buys at once for `G >= 4`: with an odd number `G - 1` of group
/// components, the last three groups `p, q, r` form lever `Z`'s triple, shared by the two
/// directions `a` and `b`: `f3 = (p: b_p, q: a_q)` into both registers (a CNOT copies it), then
/// `f1 = (p: a_p ^ b_p, r: a_r)` into `a`'s and `f2 = (q: a_q ^ b_q, r: b_r)` into `b`'s, so that
/// `f3 ^ f1 = (a_p, a_q, a_r)` and `f3 ^ f2 = (b_p, b_q, b_r)`: three Toffolis for the two
/// directions' last three components, `(G - 1) / 2` per direction in all. The other groups are
/// paired per direction. The triple is used only when it saves Toffolis over two
/// [`buy_general`]s (all-zero products are skipped). Returns both exact functions and the
/// Toffolis.
fn buy_two(
    b: &mut Builder,
    hot: &Hot,
    lay: &Layout,
    ca: &[Vec<bool>],
    cb: &[Vec<bool>],
    (ta, tb): (Qubit, Qubit),
) -> (V, V, usize) {
    let g = lay.groups;
    let all: Vec<usize> = (1..g).collect();
    let single = pairs_of(ca, &all).len() + pairs_of(cb, &all).len();
    if (g - 1) % 2 == 1 && g >= 4 {
        let (p, q, r) = (g - 3, g - 2, g - 1);
        let head: Vec<usize> = (1..p).collect();
        let (pa, pb) = (pairs_of(ca, &head), pairs_of(cb, &head));
        let xorv = |x: &[bool], y: &[bool]| -> Vec<bool> {
            x.iter().zip(y).map(|(u, v)| u ^ v).collect::<Vec<bool>>()
        };
        let (apb, aqb) = (xorv(&ca[p], &cb[p]), xorv(&ca[q], &cb[q]));
        let n3 = usize::from(any(&cb[p]) || any(&ca[q]));
        let n1 = usize::from(any(&apb) || any(&ca[r]));
        let n2 = usize::from(any(&aqb) || any(&cb[r]));
        let tri = pa.len() + pb.len() + n3 + n1 + n2;
        if tri < single {
            let mut fa = zero(lay.words);
            let mut fb = zero(lay.words);
            if n3 == 1 {
                let f3 = pair_product(b, hot, lay, (p, &cb[p]), (q, &ca[q]), ta);
                if !fault_is(120) {
                    b.cx(ta, tb);
                }
                xor(&mut fa, &f3);
                xor(&mut fb, &f3);
            }
            if n1 == 1 {
                let f1 = pair_product(b, hot, lay, (p, &apb), (r, &ca[r]), ta);
                xor(&mut fa, &f1);
            }
            if n2 == 1 {
                // Mutant 121 writes `f2` into `a`'s register.
                let t2 = if fault_is(121) { ta } else { tb };
                let f2 = pair_product(b, hot, lay, (q, &aqb), (r, &cb[r]), t2);
                xor(&mut fb, &f2);
            }
            xor(&mut fa, &emit_pairs(b, hot, lay, ca, &pa, ta));
            xor(&mut fb, &emit_pairs(b, hot, lay, cb, &pb, tb));
            return (fa, fb, tri);
        }
    }
    let (fa, na) = buy_general(b, hot, lay, ca, ta);
    let (fb, nb) = buy_general(b, hot, lay, cb, tb);
    (fa, fb, na + nb)
}

/// Under outcome `m`: `(-1)^f(state)` as `Z(h_s)`, `Z(g_i)` and `CZ(g_i, h_s)` from
/// `f = sum_s a_s h_s ^ sum_i b_i g_i ^ sum_(i,s) c_is g_i h_s` (exact on every exclusive state;
/// the empty state's value is 0 for every function here).
fn phase(b: &mut Builder, hot: &Hot, lay: &Layout, f: &[u64], m: Bit) {
    let val = |g: usize, s: usize| get(f, lay.idx(g, s));
    let none = lay.e1;
    assert!(
        !val(0, none),
        "lever q: a function nonzero on the empty state"
    );
    b.push_condition(m);
    let mut skipped = false;
    for s in 0..lay.e1 {
        if val(0, s) {
            if fault_is(91) && !skipped {
                skipped = true;
            } else {
                b.z(hot.q[s]);
            }
        }
    }
    for g in 1..lay.groups {
        if val(g, none) {
            b.z(hot.g[g - 1]);
        }
        for s in 0..lay.e1 {
            if val(g, s) ^ val(0, s) ^ val(g, none) {
                b.cz(hot.g[g - 1], hot.q[s]);
            }
        }
    }
    b.pop_condition();
}

/// Moves rows `qs` (exact functions `cur`) to target residues `tres` modulo the elimination `o`
/// (sources `src` with functions `sf`) by CNOTs among the rows, swaps and CNOTs from the
/// sources; rows with `exact = Some(t)` end holding exactly `t`. The row spaces of the current
/// and target residues must be equal (then an invertible CNOT map exists: both reduce to the same
/// reduced row echelon form).
#[allow(clippy::too_many_arguments)]
fn retarget(
    b: &mut Builder,
    qs: &[Qubit],
    cur: &mut [V],
    tres: &[V],
    exact: &[Option<V>],
    o: &Elim,
    src: &[Qubit],
    sf: &[V],
) {
    let n = qs.len();
    let cres: Vec<V> = cur.iter().map(|f| o.residue(f)).collect();
    let (ops_c, piv_c, r_c) = rref_ops(&cres);
    let (ops_t, piv_t, r_t) = rref_ops(tres);
    {
        let mut a: Vec<(usize, &V)> = piv_c.iter().map(|&(c, i)| (c, &r_c[i])).collect();
        let mut bb: Vec<(usize, &V)> = piv_t.iter().map(|&(c, i)| (c, &r_t[i])).collect();
        a.sort_by_key(|x| x.0);
        bb.sort_by_key(|x| x.0);
        assert!(a == bb, "lever q: retarget row spaces differ");
    }
    for &(j, k) in &ops_c {
        b.cx(qs[j], qs[k]);
        let fj = cur[j].clone();
        xor(&mut cur[k], &fj);
    }
    // Permutation: the row holding pivot c moves to where the target's pivot c sits; zero rows
    // fill the rest.
    let mut dest = vec![usize::MAX; n];
    let mut taken = vec![false; n];
    for &(c, i) in &piv_c {
        let t = piv_t.iter().find(|x| x.0 == c).unwrap().1;
        dest[i] = t;
        taken[t] = true;
    }
    let mut free_t = (0..n).filter(|&t| !taken[t]);
    for d in &mut dest {
        if *d == usize::MAX {
            *d = free_t.next().unwrap();
        }
    }
    // Apply by swaps: position p holds row `at[p]`.
    let mut at: Vec<usize> = (0..n).collect();
    let mut pos_of: Vec<usize> = (0..n).collect();
    for t in 0..n {
        // The row whose destination is t.
        let row = (0..n).find(|&r| dest[r] == t).unwrap();
        let p = pos_of[row];
        if p != t {
            b.swap(qs[p], qs[t]);
            cur.swap(p, t);
            let other = at[t];
            at.swap(p, t);
            pos_of[row] = t;
            pos_of[other] = p;
        }
    }
    for &(j, k) in ops_t.iter().rev() {
        b.cx(qs[j], qs[k]);
        let fj = cur[j].clone();
        xor(&mut cur[k], &fj);
    }
    for i in 0..n {
        let Some(t) = &exact[i] else { continue };
        let mut delta = t.clone();
        xor(&mut delta, &cur[i]);
        let (r, mask) = o.reduce(&delta);
        assert!(is_zero(&r), "lever q: retarget residue left over");
        if fault_is(92) && i == 0 {
            continue;
        }
        for s in ones(&mask) {
            b.cx(src[s], qs[i]);
            xor(&mut cur[i], &sf[s]);
        }
        debug_assert!(cur[i] == *t);
    }
}

/// The one-hot states (`[group][slot]`, slot `e1` = none) the RPREP write `ops` can leave: a
/// classical simulation of the write (X, CX, CCX, Swap; resets and measurements zero their qubit;
/// phases ignored) over every assignment of its `inputs` that `valid` accepts (the index values a
/// lane can carry into the write), and every value of any other qubit the write reads before
/// writing it. `None` (every state is then a care state) if a bit-changing op is conditioned, more
/// than 20 bits are free, or a state is not an exclusive one-hot.
#[must_use]
pub fn reachable(
    ops: &[crate::circuit::Op],
    inputs: &[Qubit],
    valid: &dyn Fn(u64) -> bool,
    hot: &Hot,
) -> Option<Vec<Vec<bool>>> {
    use crate::circuit::{OperationType as K, NONE};
    let mut free: Vec<u32> = inputs.iter().map(|q| q.0).collect();
    let mut written: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut depth = 0usize;
    for op in ops {
        match op.kind {
            K::PushCondition => depth += 1,
            K::PopCondition => depth -= 1,
            K::X | K::CX | K::CCX | K::Swap => {
                if depth > 0 || op.c_condition != NONE {
                    return None;
                }
                let mut reads = vec![];
                if op.kind != K::X {
                    reads.push(op.q_control1);
                }
                if op.kind == K::CCX {
                    reads.push(op.q_control2);
                }
                if op.kind == K::Swap {
                    reads.push(op.q_target);
                }
                for q in reads {
                    if !written.contains(&q) && !free.contains(&q) {
                        free.push(q);
                    }
                }
                written.insert(op.q_target);
                if op.kind == K::Swap {
                    written.insert(op.q_control1);
                }
            }
            K::R | K::Hmr => {
                written.insert(op.q_target);
            }
            _ => {}
        }
    }
    if free.len() > 20 {
        return None;
    }
    let groups = hot.g.len() + 1;
    let mut reach = vec![vec![false; hot.e1 + 1]; groups];
    let top = ops
        .iter()
        .flat_map(|o| [o.q_target, o.q_control1, o.q_control2])
        .chain(hot.q.iter().chain(&hot.g).map(|q| q.0))
        .filter(|&q| q != NONE)
        .max()
        .unwrap_or(0) as usize;
    let mut v = vec![false; top + 1];
    let ni = inputs.len();
    for a in 0u32..1 << free.len() {
        if !valid(u64::from(a) & ((1u64 << ni) - 1)) {
            continue;
        }
        v.iter_mut().for_each(|x| *x = false);
        for (i, &q) in free.iter().enumerate() {
            v[q as usize] = a >> i & 1 == 1;
        }
        for op in ops {
            let (t, c1, c2) = (
                op.q_target as usize,
                op.q_control1 as usize,
                op.q_control2 as usize,
            );
            match op.kind {
                K::X => v[t] ^= true,
                K::CX => v[t] ^= v[c1],
                K::CCX => v[t] ^= v[c1] && v[c2],
                K::Swap => v.swap(t, c1),
                K::R | K::Hmr => v[t] = false,
                _ => {}
            }
        }
        let slots: Vec<usize> = (0..hot.e1).filter(|&s| v[hot.q[s].0 as usize]).collect();
        let gs: Vec<usize> = (0..hot.g.len())
            .filter(|&i| v[hot.g[i].0 as usize])
            .collect();
        if slots.len() > 1 || gs.len() > 1 {
            return None;
        }
        let g = gs.first().map_or(0, |&i| i + 1);
        let sl = slots.first().copied().unwrap_or(hot.e1);
        reach[g][sl] = true;
    }
    Some(reach)
}

/// The segmented lower bound **inside the delivery model** (one-hot base free, any quotient
/// vector one Toffoli, at most `budget` held dimensions): for every segment `I` of the copy's
/// sequence, `buys(I) >= dim(span of I's words mod the base) - budget` (the first segment starts
/// empty), maximised over partitions by dynamic programming (TN-R1's argument in this model).
#[must_use]
pub fn model_bound(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
) -> usize {
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    let m = seq.len();
    let mut rank = vec![vec![0usize; m + 1]; m + 1];
    for j in 0..m {
        let mut el = Elim::new(1);
        let z = zero(1);
        let mut d = 0;
        for i in j..m {
            d += nres[seq[i]].iter().filter(|v| el.insert(v, &z)).count();
            rank[j][i + 1] = d;
        }
    }
    let mut best = vec![0usize; m + 1];
    for i in 1..=m {
        best[i] = best[i - 1];
        for j in 0..i {
            let off = if j == 0 { 0 } else { budget };
            best[i] = best[i].max(best[j] + rank[j][i].saturating_sub(off));
        }
    }
    best[m]
}

/// **The linear-factor bound** (theorem for the class below; numbers from the
/// spec's own words). Class LF: during a copy's chain passes every Toffoli's two inputs are
/// parities of the split one-hot's qubits (slot qubits `h_s`, group bits `g_i`) and constants;
/// anything else is free (CNOTs from any qubit, swaps, measurements with Clifford fixups). Every
/// construction here (`onehot::transition_by` with `z` / `Z` / `Y`, lever `q`'s buys) is in LF.
///
/// Lemma. On a full slot `s` (every group holds a leaf there) a factor is `x_s ^ a_g` on state
/// `(g, s)`, so a product's group differences `f(g, s) ^ f(0, s) = (b_g ^ b_0) x_s ^ (a_g ^ a_0) y_s
/// ^ const_g`: as a `(G - 1) x |P|` matrix over the full slots `P` its rows lie in `span{x, y,
/// 1_P}`. So `T` Toffolis give rows in a space of dimension at most `2 T + 1` (with `1_P`).
///
/// Bound. In a segment `I` of the copy's chain, every word bit read is (on the leaves) a live
/// function at `I`'s start (at most `budget` non-base dimensions, each with at most `G - 1` rows)
/// plus outputs made in `I` plus a base function (no rows). With `r(I)` the dimension of the span
/// of `1_P` and every row of every word bit of `I`'s rotations: `T(I) >= ceil((r(I) - 1 - (G - 1)
/// budget) / 2)`, and at least the union-span term `dim(span of I's words mod the base) - budget`
/// (the first segment starts with nothing held), whichever is larger; maximised over partitions
/// of the copy's sequence by dynamic programming. For
/// `G <= 3` it is the delivery model's bound ([`model_bound`]) restricted to full slots; for
/// `G >= 4` it carries the factor `(G - 1) / 2` per direction that the pair products pay. Only
/// leaf states enter (exact for any circuit that is right on the leaves).
#[must_use]
pub fn lf_bound(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
) -> usize {
    let g = lay.groups;
    let full: Vec<usize> = (0..lay.e1)
        .filter(|&s| (0..g).all(|gi| lay.at[gi][s].is_some()))
        .collect();
    let pw = full.len().div_ceil(64).max(1);
    // Rows of rotation t: for each bit and group gi >= 1, s -> W(gi, s) ^ W(0, s) on P.
    let rows: Vec<Vec<V>> = (0..rots)
        .map(|t| {
            let mut out = Vec::new();
            for k in 0..width {
                let v = lay.bits(&|e| angle(e, t), k);
                for gi in 1..g {
                    let mut r = zero(pw);
                    for (x, &s) in full.iter().enumerate() {
                        if get(&v, lay.idx(gi, s)) ^ get(&v, lay.idx(0, s)) {
                            set(&mut r, x);
                        }
                    }
                    out.push(r);
                }
            }
            out
        })
        .collect();
    let mut ones_p = zero(pw);
    for x in 0..full.len() {
        set(&mut ones_p, x);
    }
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    let m = seq.len();
    let z = zero(1);
    let mut rank = vec![vec![0usize; m + 1]; m + 1];
    for j in 0..m {
        let mut el = Elim::new(1);
        el.insert(&ones_p, &z);
        for i in j..m {
            for r in &rows[seq[i]] {
                el.insert(r, &z);
            }
            rank[j][i + 1] = el.dim() - 1;
        }
    }
    // The union-span term per segment (each Toffoli adds at most one dimension modulo the base;
    // `model_bound`'s), whichever is larger.
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let mut unit = vec![vec![0usize; m + 1]; m + 1];
    for j in 0..m {
        let mut el = Elim::new(1);
        let mut d = 0;
        for i in j..m {
            d += nres[seq[i]].iter().filter(|v| el.insert(v, &z)).count();
            unit[j][i + 1] = d;
        }
    }
    let held = (g - 1) * budget;
    let mut best = vec![0usize; m + 1];
    for i in 1..=m {
        best[i] = best[i - 1];
        for j in 0..i {
            let lf = rank[j][i].saturating_sub(held).div_ceil(2);
            let off = if j == 0 { 0 } else { budget };
            let un = unit[j][i].saturating_sub(off);
            best[i] = best[i].max(best[j] + lf.max(un));
        }
    }
    best[m]
}

/// A basis of `span(a) + span(b)`.
fn sum_of(a: &[V], b: &[V]) -> Vec<V> {
    let mut el = Elim::new(1);
    let z = zero(1);
    a.iter()
        .chain(b.iter())
        .filter(|v| el.insert(v, &z))
        .cloned()
        .collect()
}

/// A basis of `span(a) ∩ span(b)` (Zassenhaus).
fn inter_of(a: &[V], b: &[V]) -> Vec<V> {
    let Some(w) = a.first().or(b.first()).map(Vec::len) else {
        return Vec::new();
    };
    let mut el = Elim::new(1);
    let z = zero(1);
    for v in a {
        el.insert(&[v.as_slice(), v.as_slice()].concat(), &z);
    }
    for v in b {
        el.insert(&[v.as_slice(), &zero(w)].concat(), &z);
    }
    el.rows
        .iter()
        .filter(|(_, v, _)| is_zero(&v[..w]))
        .map(|(_, v, _)| v[w..].to_vec())
        .collect()
}

/// The furthest-future keep schedule (Belady's rule for subspaces), with the budget respected at
/// every moment (drop before buy). Before rotation `p`'s load the held span `prev` is cut to
/// `K = prev ∩ (N_t + S_(p+1..p+j))` for the largest `j` with `dim(K + N_t) <= budget` (`S` the
/// span of the following rotations' words), filled from the next level; then `N_t` is bought
/// beyond `K`. With `park = Some((g, c))` the span is first cut before position `g`
/// to `prev ∩ S_(g..g+j)` with dimension at most `c`, by the same rule with nothing bought.
/// Returns each position's held span, the parked span and the total buys.
fn belady(
    nres: &[Vec<V>],
    seq: &[usize],
    budget: usize,
    park: Option<(usize, usize)>,
) -> (Vec<Vec<V>>, Option<Vec<V>>, usize) {
    belady_from(nres, seq, budget, park, 0, Vec::new())
}

/// The cut before the Majorana gap: `prev` cut to at most `c` dimensions by the
/// furthest-future rule.
fn park_cut(prev: &[V], nres: &[Vec<V>], rest: &[usize], c: usize) -> Vec<V> {
    if prev.len() > c {
        furthest(prev, Vec::new(), Vec::new(), nres, rest, c)
    } else {
        prev.to_vec()
    }
}

/// Belady's keep before position `p`'s load (`prev` the held span, `nt` the word's span).
fn belady_keep(
    prev: &[V],
    nt: &[V],
    nres: &[Vec<V>],
    seq: &[usize],
    p: usize,
    budget: usize,
) -> Vec<V> {
    if sum_of(prev, nt).len() <= budget {
        return prev.to_vec();
    }
    let k0 = inter_of(prev, nt);
    let room = budget - nt.len() + k0.len();
    furthest(prev, k0, nt.to_vec(), nres, &seq[p + 1..], room)
}

/// [`belady`] from position `p0` with held span `prev`: the spans of positions `p0..`, the
/// parked span (when the gap is at or after `p0`) and the buys from `p0` on.
fn belady_from(
    nres: &[Vec<V>],
    seq: &[usize],
    budget: usize,
    park: Option<(usize, usize)>,
    p0: usize,
    mut prev: Vec<V>,
) -> (Vec<Vec<V>>, Option<Vec<V>>, usize) {
    let mut xs = Vec::with_capacity(seq.len() - p0);
    let mut buys = 0;
    let mut parked = None;
    for (p, &t) in seq.iter().enumerate().skip(p0) {
        if let Some((g, c)) = park {
            if p == g {
                prev = park_cut(&prev, nres, &seq[p..], c);
                parked = Some(prev.clone());
            }
        }
        let nt = sum_of(&nres[t], &[]);
        let keep = belady_keep(&prev, &nt, nres, seq, p, budget);
        let x = sum_of(&keep, &nt);
        buys += x.len() - keep.len();
        xs.push(x.clone());
        prev = x;
    }
    (xs, parked, buys)
}

/// Analysis (not a circuit mode; [`rollout_buys`]): **rollout-improved furthest-future keep** (the pilot method on
/// [`belady`]). Wherever the held span overflows (and at the Majorana cut), several keeps are
/// tried: Belady's, fills of the first level that does not fit taken from the next word alone or
/// at random, and the same at every nearer level; each is scored by its buys plus [`belady`]'s
/// buys for the rest of the copy from there, and the cheapest is taken (Belady's on a tie). Every
/// keep contains `prev ∩ N_t` and is inside `prev`, so the spans satisfy the same invariants as
/// mode 1 and the total is never above it.
fn belady_rollout(
    nres: &[Vec<V>],
    seq: &[usize],
    budget: usize,
    park: Option<(usize, usize)>,
    tries: usize,
) -> (Vec<Vec<V>>, Option<Vec<V>>, usize) {
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut prev: Vec<V> = Vec::new();
    let mut xs = Vec::with_capacity(seq.len());
    let mut buys = 0;
    let mut parked = None;
    let score = |p: usize, held: &[V], at_gap: bool| -> usize {
        // Buys from position p on with `held` already cut (at the gap) or before p's keep.
        if at_gap {
            belady_from(nres, seq, budget, None, p, held.to_vec()).2
        } else {
            belady_from(
                nres,
                seq,
                budget,
                park.filter(|g| g.0 >= p),
                p,
                held.to_vec(),
            )
            .2
        }
    };
    for (p, &t) in seq.iter().enumerate() {
        if let Some((g, c)) = park {
            if p == g {
                let base = park_cut(&prev, nres, &seq[p..], c);
                let mut best = (score(p, &base, true), base);
                if prev.len() > c {
                    for cand in fills(&prev, &[], nres, &seq[p..], c, tries, &mut rng) {
                        let sc = score(p, &cand, true);
                        if sc < best.0 {
                            best = (sc, cand);
                        }
                    }
                }
                prev = best.1;
                parked = Some(prev.clone());
            }
        }
        let nt = sum_of(&nres[t], &[]);
        let bel = belady_keep(&prev, &nt, nres, seq, p, budget);
        let keep = if sum_of(&prev, &nt).len() <= budget {
            bel
        } else {
            let k0 = inter_of(&prev, &nt);
            let room = budget - nt.len() + k0.len();
            let eval = |k: &[V]| -> usize {
                let x = sum_of(k, &nt);
                let rest = if p + 1 < seq.len() {
                    score(p + 1, &x, false)
                } else {
                    0
                };
                x.len() - k.len() + rest
            };
            let mut best = (eval(&bel), bel);
            for cand in fills(&prev, &nt, nres, &seq[p + 1..], room, tries, &mut rng) {
                let sc = eval(&cand);
                if sc < best.0 {
                    best = (sc, cand);
                }
            }
            best.1
        };
        let x = sum_of(&keep, &nt);
        buys += x.len() - keep.len();
        xs.push(x.clone());
        prev = x;
    }
    (xs, parked, buys)
}

/// Candidate keeps of at most `room` dimensions inside `prev`, each containing `prev ∩ span(nt)`:
/// for every level `j` of the future (`span(nt) + N(rest[0..j])`) whose intersection with `prev`
/// fits, that intersection filled up to `room` from the next level's intersection, by the next
/// word alone first, then by `tries` random combinations.
fn fills(
    prev: &[V],
    nt: &[V],
    nres: &[Vec<V>],
    rest: &[usize],
    room: usize,
    tries: usize,
    rng: &mut u64,
) -> Vec<Vec<V>> {
    let z = zero(1);
    let mut out: Vec<Vec<V>> = Vec::new();
    let mut fut: Vec<V> = nt.to_vec();
    let mut keep = inter_of(prev, &fut);
    if keep.len() > room {
        return out;
    }
    let next = |r: &mut u64| {
        *r ^= *r << 13;
        *r ^= *r >> 7;
        *r ^= *r << 17;
        *r
    };
    for &u in rest.iter().take(24) {
        let word = sum_of(&nres[u], &[]);
        let fut2 = sum_of(&fut, &word);
        let k = inter_of(prev, &fut2);
        // Fills of `keep` from `k` (the next level), to `room`.
        let mut el = Elim::new(1);
        keep.iter().for_each(|v| {
            el.insert(v, &z);
        });
        let comp: Vec<V> = k
            .iter()
            .filter(|v| el.clone().insert(v, &z))
            .cloned()
            .collect();
        let mut comp_basis: Vec<V> = Vec::new();
        {
            let mut e2 = el.clone();
            for v in &comp {
                if e2.insert(v, &z) {
                    comp_basis.push(v.clone());
                }
            }
        }
        let need = room - keep.len();
        if need > 0 && !comp_basis.is_empty() {
            // The next word alone first.
            let own = inter_of(prev, &word);
            let mut c1 = keep.clone();
            let mut e1 = el.clone();
            for v in own.iter().chain(comp_basis.iter()) {
                if c1.len() >= room {
                    break;
                }
                if e1.insert(v, &z) {
                    c1.push(v.clone());
                }
            }
            out.push(c1);
            for _ in 0..tries {
                let mut c2 = keep.clone();
                let mut e3 = el.clone();
                for _ in 0..4 * comp_basis.len() {
                    if c2.len() >= room {
                        break;
                    }
                    let mut v = zero(comp_basis[0].len());
                    let r = next(rng);
                    for (i, b) in comp_basis.iter().enumerate() {
                        if r >> (i % 64) & 1 == 1 {
                            xor(&mut v, b);
                        }
                    }
                    if !is_zero(&v) && e3.insert(&v, &z) {
                        c2.push(v);
                    }
                }
                out.push(c2);
            }
        }
        if k.len() > room {
            break;
        }
        keep = k;
        fut = fut2;
        if keep.len() == room {
            out.push(keep.clone());
            break;
        }
    }
    out
}

/// The furthest-future part of `prev` that fits in `room` dimensions: starting from `keep`
/// (`prev ∩ fut`), grows `fut` by the words of `rest` in order and keeps `prev ∩ fut` while it
/// fits, then fills up to `room` from the first level that does not fit.
fn furthest(
    prev: &[V],
    mut keep: Vec<V>,
    mut fut: Vec<V>,
    nres: &[Vec<V>],
    rest: &[usize],
    room: usize,
) -> Vec<V> {
    let z = zero(1);
    for &u in rest {
        fut = sum_of(&fut, &nres[u]);
        let k = inter_of(prev, &fut);
        if k.len() <= room {
            keep = k;
            if keep.len() == room {
                break;
            }
        } else {
            let mut el = Elim::new(1);
            keep.iter().for_each(|v| {
                el.insert(v, &z);
            });
            for v in &k {
                if keep.len() >= room {
                    break;
                }
                if el.insert(v, &z) {
                    keep.push(v.clone());
                }
            }
            break;
        }
    }
    keep
}

/// Runs of equal spans: `(starts, spans, total buys)`.
fn runs(xs: &[Vec<V>]) -> (Vec<usize>, Vec<Vec<V>>, usize) {
    let mut starts = Vec::new();
    let mut spans: Vec<Vec<V>> = Vec::new();
    let mut buys = 0;
    for (p, x) in xs.iter().enumerate() {
        let same = spans
            .last()
            .is_some_and(|l: &Vec<V>| l.len() == x.len() && sum_of(l, x).len() == x.len());
        if !same {
            buys += spans
                .last()
                .map_or(x.len(), |l| x.len() - inter_of(l, x).len());
            starts.push(p);
            spans.push(x.clone());
        }
    }
    (starts, spans, buys)
}

/// Model: [`belady`]'s buys per copy.
#[must_use]
pub fn belady_buys(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
) -> usize {
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    let (xs, _, buys) = belady(&nres, &seq, budget, None);
    assert_eq!(runs(&xs).2, buys);
    buys
}

/// Mode 1's buys per copy with the Majorana cut `park` (0 none), for analysis.
#[must_use]
pub fn belady_park_buys(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
    park: usize,
) -> usize {
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    belady(&nres, &seq, budget, (park > 0).then_some((rots, park))).2
}

/// The buys per copy of mode 1 (furthest-future keep) and mode 2 (with `tries`
/// random fills per decision), with the Majorana cut `park` (0 none), for analysis.
#[must_use]
pub fn rollout_buys(
    lay: &Layout,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
    park: usize,
    tries: usize,
) -> (usize, usize) {
    let base = lay.base(0);
    let nres = word_residues(lay, &base, angle, rots, width);
    let seq: Vec<usize> = (0..rots).rev().chain(0..rots).collect();
    let cut = (park > 0).then_some((rots, park));
    let b1 = belady(&nres, &seq, budget, cut).2;
    let b2 = belady_rollout(&nres, &seq, budget, cut, tries).2;
    (b1, b2)
}

/// Static cost: buys per copy and the largest window, for a layout and budget (no circuit).
#[must_use]
pub fn plan_cost(
    hot: &Hot,
    n_leaves: usize,
    angle: &dyn Fn(usize, usize) -> u64,
    rots: usize,
    width: usize,
    budget: usize,
    reach: Option<&[Vec<bool>]>,
) -> (usize, usize, usize) {
    let lay = Layout::of(hot, n_leaves, reach);
    let p = plan(&lay, angle, rots, width, budget);
    (p.buys, p.max_dim, p.dprime)
}

#[cfg(test)]
mod tests {
    //! Exhaustive gadget tests: for every leaf of small random angle tables (G = 2 and 3, slots
    //! with missing groups, budgets from one rotation's rank to the whole span), the register
    //! holds each rotation's angle at its position of the copy sequence, every ancilla ends `|0>`
    //! with phase `+1` for every measurement-outcome seed, an empty one-hot lane stays clean, the
    //! Toffolis equal the plan's buys, and mutants 90-93 leave a wrong angle or a dirty lane.
    use super::{Deliver, Hot};
    use crate::circuit::Builder;

    thread_local! {
        /// The plan mode the gadget tests use (0 windows, 1 furthest-future keep).
        static MODE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
        /// The Majorana budget the gadget tests use (0 off).
        static PARK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    use crate::walk::sa_low::onehot::FAULT;
    use crate::walk::shared::testsim::Sim;

    fn table(seed: u64, leaves: usize, rots: usize, bits: usize) -> Vec<Vec<u64>> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..leaves)
            .map(|_| {
                (0..rots)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x % (1 << bits)
                    })
                    .collect()
            })
            .collect()
    }

    /// Runs one copy for leaf `e` (`None`: an empty one-hot); returns whether every load was
    /// right and the end clean, and the Toffolis and the plan's buys.
    #[allow(clippy::too_many_arguments)]
    fn copy(
        n: usize,
        groups: usize,
        tab: &[Vec<u64>],
        bits: usize,
        budget: usize,
        e: Option<usize>,
        seed: u64,
        fault: u8,
    ) -> (bool, u64, usize) {
        copy_state(n, groups, tab, bits, budget, e, 0, seed, fault)
    }

    /// [`copy`] with `lonely`: on an empty one-hot, group bit `lonely` (if `>= 1`) set alone (a
    /// state the model covers although no lane is known to reach it).
    #[allow(clippy::too_many_arguments)]
    fn copy_state(
        n: usize,
        groups: usize,
        tab: &[Vec<u64>],
        bits: usize,
        budget: usize,
        e: Option<usize>,
        lonely: usize,
        seed: u64,
        fault: u8,
    ) -> (bool, u64, usize) {
        let rots = tab[0].len();
        let mut b = Builder::new(1);
        b.declare_uniform(1);
        let e1 = n.div_ceil(groups);
        let hot = Hot {
            q: b.alloc_n(e1),
            g: b.alloc_n(groups - 1),
            e1,
            paired: true,
            fold: None,
            shared: false,
            sandwich: false,
            rec: None,
        };
        let reg = b.alloc_n(bits);
        let angle = |l: usize, j: usize| tab[l][j];
        let mut sim = Sim::new(&b, seed);
        if let Some(e) = e {
            sim.set(hot.q[e % e1], true);
            if e / e1 > 0 {
                sim.set(hot.g[e / e1 - 1], true);
            }
        } else if lonely > 0 {
            sim.set(hot.g[lonely - 1], true);
        }
        FAULT.with(|c| c.set(fault));
        let mut d = Deliver::new(
            &hot,
            n,
            &reg,
            &angle,
            rots,
            budget,
            MODE.with(std::cell::Cell::get),
            None,
            PARK.with(std::cell::Cell::get),
        );
        let seq = d.plan().seq.clone();
        let mut ok = true;
        for (p, &j) in seq.iter().enumerate() {
            let park = PARK.with(std::cell::Cell::get);
            if park > 0 && p == rots {
                // The cut at the Majorana gap. At most `park` rows stay live, and
                // every other register qubit is |0> on this lane.
                let start = b.ops().len();
                d.park(&mut b, &hot);
                sim.run(&b.ops()[start..]);
                let live = d.live();
                ok &= live.len() <= park;
                ok &= reg
                    .iter()
                    .filter(|q| !live.contains(q))
                    .all(|&q| sim.read(&[q]) == 0);
            }
            let start = b.ops().len();
            d.step(&mut b, &hot, j);
            sim.run(&b.ops()[start..]);
            let want = e.map_or(0, |e| tab[e][j]);
            ok &= sim.read(&reg) == want;
        }
        let start = b.ops().len();
        d.finish(&mut b, &hot);
        FAULT.with(|c| c.set(0));
        sim.run(&b.ops()[start..]);
        if let Some(e) = e {
            sim.set(hot.q[e % e1], false);
            if e / e1 > 0 {
                sim.set(hot.g[e / e1 - 1], false);
            }
        } else if lonely > 0 {
            sim.set(hot.g[lonely - 1], false);
        }
        ok &= std::panic::catch_unwind(|| sim.assert_clean()).is_ok();
        if groups <= 3 {
            assert_eq!(d.toffolis, d.buys, "G <= 3: one Toffoli per buy");
        } else if (groups - 1) % 2 == 1 && fault == 0 && MODE.with(std::cell::Cell::get) == 1 {
            // The triple pays: fewer than ceil((G - 1) / 2) Toffolis per bought direction.
            assert!(
                d.toffolis < d.buys * (groups / 2),
                "G {groups}: triple unused"
            );
        }
        (ok, sim.toffolis, d.toffolis)
    }

    #[test]
    fn rank_delivery_is_exact_for_every_leaf() {
        let bits = 5;
        for (n, groups, rots) in [
            (7usize, 2usize, 4usize),
            (11, 3, 5),
            (13, 3, 6),
            (10, 2, 6),
            (17, 3, 4),
            (23, 3, 7),
            // G = 4 .. 7 (paired buys, lever `Z`'s triple).
            (13, 4, 5),
            (19, 4, 6),
            (21, 5, 5),
            (23, 6, 4),
            (20, 7, 4),
        ] {
            let tab = table((n * 131 + groups * 7 + rots) as u64, n, rots, bits);
            for (budget, mode) in [bits, bits + 2, 2 * bits, 3 * bits, 64]
                .into_iter()
                .flat_map(|b| [(b, 0u8), (b, 1)])
            {
                MODE.with(|c| c.set(mode));
                let mut toff = None;
                for e in (0..n).map(Some).chain([None]) {
                    for seed in 1..4u64 {
                        let (ok, t, buys) = copy(n, groups, &tab, bits, budget, e, seed, 0);
                        assert!(
                            ok,
                            "n {n} G {groups} budget {budget} leaf {e:?} seed {seed}"
                        );
                        assert_eq!(t as usize, buys, "Toffolis = the gadget's count");
                        // Toffolis do not depend on the lane.
                        assert_eq!(*toff.get_or_insert(t), t);
                    }
                }
                for g in 1..groups {
                    let (ok, _, _) = copy_state(n, groups, &tab, bits, budget, None, g, 5, 0);
                    assert!(ok, "n {n} G {groups} budget {budget}: lonely group bit {g}");
                }
            }
        }
    }

    #[test]
    fn rank_delivery_mutants_are_caught() {
        let bits = 5;
        for (fault, mode) in [90u8, 91, 92, 93]
            .into_iter()
            .flat_map(|f| [(f, 0u8), (f, 1)])
            .chain([(120, 1), (121, 1)])
        {
            MODE.with(|c| c.set(mode));
            let mut caught = false;
            for (n, groups, rots) in [
                (11usize, 3usize, 5usize),
                (10, 2, 6),
                (13, 3, 6),
                (17, 3, 4),
                (23, 3, 7),
                (13, 4, 5),
                (21, 5, 5),
                (23, 6, 4),
            ] {
                let tab = table((n * 131 + groups * 7 + rots) as u64, n, rots, bits);
                for budget in [bits + 1, bits + 2, bits + 4, 2 * bits, 3 * bits] {
                    for e in 0..n {
                        for seed in 1..4u64 {
                            caught |= !copy(n, groups, &tab, bits, budget, Some(e), seed, fault).0;
                        }
                    }
                }
            }
            assert!(caught, "mutant {fault} (mode {mode}) not caught");
        }
    }
    /// `q<b>_1.<c>`: with the Majorana budget, every leaf still gets every angle,
    /// the lane ends clean for every outcome seed, at most `c` rows are live across the gap, the
    /// Toffolis equal the plan's buys, and the cut costs at most `dim(X) - c` extra buys over the
    /// uncut plan; a budget at or above the held span changes nothing.
    #[test]
    fn rank_delivery_park_is_exact_for_every_leaf() {
        let bits = 5;
        MODE.with(|c| c.set(1));
        for (n, groups, rots) in [
            (7usize, 2usize, 4usize),
            (11, 3, 5),
            (13, 3, 6),
            (10, 2, 6),
            (17, 3, 4),
            (23, 3, 7),
            (13, 4, 5),
            (21, 5, 5),
        ] {
            let tab = table((n * 131 + groups * 7 + rots) as u64, n, rots, bits);
            for budget in [bits, bits + 2, 2 * bits, 3 * bits] {
                PARK.with(|c| c.set(0));
                let (_, _, free) = copy(n, groups, &tab, bits, budget, Some(0), 1, 0);
                for park in 1..=budget.min(2 * bits) {
                    PARK.with(|c| c.set(park));
                    let mut toff = None;
                    for e in (0..n).map(Some).chain([None]) {
                        for seed in 1..4u64 {
                            let (ok, t, buys) = copy(n, groups, &tab, bits, budget, e, seed, 0);
                            assert!(
                                ok,
                                "n {n} G {groups} budget {budget} park {park} leaf {e:?} seed {seed}"
                            );
                            assert_eq!(t as usize, buys, "Toffolis = the gadget's count");
                            assert_eq!(*toff.get_or_insert(t), t);
                        }
                    }
                    let t = toff.unwrap() as usize;
                    if groups <= 3 {
                        assert!(t >= free, "the cut never saves buys");
                        assert!(
                            t <= free + budget.saturating_sub(park),
                            "n {n} G {groups} budget {budget} park {park}: {t} vs {free}"
                        );
                    }
                    if park >= budget {
                        assert_eq!(t, free, "a budget above the span changes nothing");
                    }
                    for g in 1..groups {
                        let (ok, _, _) = copy_state(n, groups, &tab, bits, budget, None, g, 5, 0);
                        assert!(ok, "n {n} G {groups} park {park}: lonely group bit {g}");
                    }
                }
            }
        }
        PARK.with(|c| c.set(0));
        MODE.with(|c| c.set(0));
    }

    /// Mutant 133 (the cut not emitted: the register and held rows stay live across the gap) is
    /// caught by the live-row check; 91 and 93 inside the cut leave a wrong angle or a dirty lane.
    #[test]
    fn rank_delivery_park_mutants_are_caught() {
        let bits = 5;
        MODE.with(|c| c.set(1));
        for fault in [133u8, 91, 93] {
            let mut caught = false;
            for (n, groups, rots) in [
                (11usize, 3usize, 5usize),
                (13, 3, 6),
                (23, 3, 7),
                (10, 2, 6),
            ] {
                let tab = table((n * 131 + groups * 7 + rots) as u64, n, rots, bits);
                for budget in [bits + 2, 2 * bits] {
                    for park in [1, 3, bits] {
                        PARK.with(|c| c.set(park));
                        for e in 0..n {
                            for seed in 1..3u64 {
                                caught |=
                                    !copy(n, groups, &tab, bits, budget, Some(e), seed, fault).0;
                            }
                        }
                    }
                }
            }
            assert!(caught, "park mutant {fault} not caught");
        }
        PARK.with(|c| c.set(0));
        MODE.with(|c| c.set(0));
    }
}
