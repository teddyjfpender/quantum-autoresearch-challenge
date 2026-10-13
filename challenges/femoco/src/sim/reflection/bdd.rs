//! Reduced ordered binary decision diagrams, for the last complete rung of the zero test
//! (`super::poly::Arena::is_zero`).
//!
//! A node is `(level, lo, hi)`: the function is `lo` where the variable of that level is 0 and
//! `hi` where it is 1. Levels increase from the root down, `lo != hi` in every node, and a
//! node is stored once, so two functions over the same variables are equal exactly when their
//! node ids are equal. In particular a function is identically zero exactly when its id is
//! [`FALSE`]. That canonicity is the whole of what the zero test uses.
//!
//! The two operations, AND and XOR, are the textbook `apply`, run on an explicit stack so the
//! depth of a diagram is not the depth of the call stack. Results are remembered in a table of
//! fixed size that forgets on collision, which costs time and never changes a result. Every
//! miss is counted through the caller's `tick`. The store has a hard node limit: past it the
//! manager is `overflowed`, results are meaningless, and the caller must not read them (it
//! answers `Unknown`). [`Bdd::collect`] drops the nodes the caller no longer names.
pub type Node = u32;
pub const FALSE: Node = 0;
pub const TRUE: Node = 1;
/// The level of the two terminals: below every variable.
const LEAF: u32 = u32::MAX;
/// An empty slot of the operation tables.
const EMPTY: (Node, Node, Node) = (u32::MAX, 0, 0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Op {
    And,
    Xor,
}

enum Task {
    Call(Node, Node),
    Join(u32, Node, Node),
}

fn mix(a: u32, b: u32, c: u32) -> usize {
    let x = (u64::from(a) << 32 | u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ u64::from(c).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    ((x ^ x >> 29).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 20) as usize
}

pub struct Bdd {
    nodes: Vec<(u32, Node, Node)>,
    /// Open addressing over node ids; 0 is an empty slot (a terminal is never entered).
    unique: Vec<Node>,
    /// Per operation: `(f, g, f op g)`, one entry per slot, overwritten on collision.
    memo: [Vec<(Node, Node, Node)>; 2],
    limit: usize,
    pub overflowed: bool,
}

impl Bdd {
    /// A manager that holds at most `limit` nodes.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            nodes: vec![(LEAF, FALSE, FALSE), (LEAF, TRUE, TRUE)],
            unique: vec![0; 1 << 10],
            memo: [Vec::new(), Vec::new()],
            limit: limit.max(2),
            overflowed: false,
        }
    }

    /// Nodes held, the two terminals included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 2
    }

    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }

    fn level(&self, f: Node) -> u32 {
        self.nodes[f as usize].0
    }

    /// Enters node `n` in the unique table (it is not there).
    fn enter(&mut self, n: Node) {
        let (level, lo, hi) = self.nodes[n as usize];
        let mask = self.unique.len() - 1;
        let mut at = mix(level, lo, hi) & mask;
        while self.unique[at] != 0 {
            at = (at + 1) & mask;
        }
        self.unique[at] = n;
    }

    /// Rebuilds the unique table for the nodes held, at most half full.
    fn rehash(&mut self) {
        let size = (self.nodes.len() * 2).next_power_of_two().max(1 << 10);
        self.unique.clear();
        self.unique.resize(size, 0);
        for n in 2..self.nodes.len() {
            self.enter(n as Node);
        }
    }

    fn mk(&mut self, level: u32, lo: Node, hi: Node) -> Node {
        if lo == hi {
            return lo;
        }
        let mask = self.unique.len() - 1;
        let mut at = mix(level, lo, hi) & mask;
        loop {
            match self.unique[at] {
                0 => break,
                n if self.nodes[n as usize] == (level, lo, hi) => return n,
                _ => at = (at + 1) & mask,
            }
        }
        if self.nodes.len() >= self.limit {
            self.overflowed = true;
            return FALSE;
        }
        let n = self.nodes.len() as Node;
        self.nodes.push((level, lo, hi));
        if self.nodes.len() * 2 > self.unique.len() {
            self.rehash();
        } else {
            self.unique[at] = n;
        }
        n
    }

    /// The variable of `level` (`level < u32::MAX`).
    pub fn var(&mut self, level: u32) -> Node {
        debug_assert!(level != LEAF);
        self.mk(level, FALSE, TRUE)
    }

    /// The result of `op` when it follows from the operands' ids alone.
    fn shortcut(op: Op, f: Node, g: Node) -> Option<Node> {
        match op {
            Op::And if f == FALSE || g == FALSE => Some(FALSE),
            Op::And if f == TRUE || f == g => Some(g),
            Op::And if g == TRUE => Some(f),
            Op::Xor if f == g => Some(FALSE),
            Op::Xor if f == FALSE => Some(g),
            Op::Xor if g == FALSE => Some(f),
            _ => None,
        }
    }

    /// `f op g`. `tick` is called once per table miss; when it returns `false` (the caller is
    /// out of work) the manager is marked overflowed and the result is meaningless.
    pub fn apply(
        &mut self,
        op: Op,
        f: Node,
        g: Node,
        tick: &mut (impl FnMut() -> bool + ?Sized),
    ) -> Node {
        let which = usize::from(op == Op::Xor);
        // The tables grow with the store: a quarter of the node limit each, at most 2^22 slots.
        let want = (self.nodes.len() / 2)
            .next_power_of_two()
            .clamp(1 << 10, 1 << 22);
        for m in &mut self.memo {
            if m.len() < want {
                m.clear();
                m.resize(want, EMPTY);
            }
        }
        let mask = self.memo[which].len() - 1;
        let mut tasks = vec![Task::Call(f, g)];
        let mut done: Vec<Node> = Vec::new();
        while let Some(task) = tasks.pop() {
            if self.overflowed {
                return FALSE;
            }
            match task {
                Task::Call(f, g) => {
                    let (f, g) = if f <= g { (f, g) } else { (g, f) };
                    if let Some(r) = Self::shortcut(op, f, g) {
                        done.push(r);
                        continue;
                    }
                    let (a, b, r) = self.memo[which][mix(f, g, 0) & mask];
                    if (a, b) == (f, g) {
                        done.push(r);
                        continue;
                    }
                    if !tick() {
                        self.overflowed = true;
                        return FALSE;
                    }
                    let (lf, lg) = (self.level(f), self.level(g));
                    let level = lf.min(lg);
                    let (f0, f1) = if lf == level {
                        let (_, lo, hi) = self.nodes[f as usize];
                        (lo, hi)
                    } else {
                        (f, f)
                    };
                    let (g0, g1) = if lg == level {
                        let (_, lo, hi) = self.nodes[g as usize];
                        (lo, hi)
                    } else {
                        (g, g)
                    };
                    tasks.push(Task::Join(level, f, g));
                    tasks.push(Task::Call(f1, g1));
                    tasks.push(Task::Call(f0, g0));
                }
                Task::Join(level, f, g) => {
                    // The 0-branch was called last, so it finished first.
                    let (Some(hi), Some(lo)) = (done.pop(), done.pop()) else {
                        self.overflowed = true;
                        return FALSE;
                    };
                    let r = self.mk(level, lo, hi);
                    self.memo[which][mix(f, g, 0) & mask] = (f, g, r);
                    done.push(r);
                }
            }
        }
        match done[..] {
            [r] if !self.overflowed => r,
            _ => {
                self.overflowed = true;
                FALSE
            }
        }
    }

    /// Whether `f` depends on the variable of `level`: whether its diagram has a node of that
    /// level. In a reduced diagram a variable labels a node exactly when the function changes
    /// with it somewhere.
    #[must_use]
    pub fn reads(&self, f: Node, level: u32) -> bool {
        let mut seen = vec![false; f as usize + 1];
        seen[f as usize] = true;
        // Children have smaller ids, so one pass from `f` down visits every reachable node.
        for n in (2..=f as usize).rev() {
            if !seen[n] {
                continue;
            }
            let (l, lo, hi) = self.nodes[n];
            if l == level {
                return true;
            }
            // Levels grow towards the leaves: below a deeper node the variable cannot occur.
            if l < level {
                (seen[lo as usize], seen[hi as usize]) = (true, true);
            }
        }
        false
    }

    /// One assignment on which `f` is 1, as `(level, value)` for the variables on the path;
    /// the others are free. Empty for the constant 1. `f` must not be [`FALSE`].
    #[must_use]
    pub fn witness(&self, mut f: Node) -> Vec<(u32, bool)> {
        let mut path = Vec::new();
        while self.level(f) != LEAF {
            let (level, lo, hi) = self.nodes[f as usize];
            // A node other than FALSE has a path to TRUE through either child that is not FALSE.
            let up = lo == FALSE;
            path.push((level, up));
            f = if up { hi } else { lo };
        }
        path
    }

    /// The fraction of all assignments on which `f` is 1.
    #[must_use]
    pub fn density(&self, f: Node) -> f64 {
        let mut d = vec![0.0f64; f as usize + 1];
        if f >= TRUE {
            d[TRUE as usize] = 1.0;
        }
        for n in 2..=f as usize {
            let (_, lo, hi) = self.nodes[n];
            d[n] = (d[lo as usize] + d[hi as usize]) / 2.0;
        }
        d[f as usize]
    }

    /// Keeps the nodes reachable from `roots` and drops the rest. Ids change: every root is
    /// rewritten to its new id, and any other id the caller holds is dead.
    pub fn collect<'a>(&mut self, roots: impl Iterator<Item = &'a mut Node>) {
        let roots: Vec<&mut Node> = roots.collect();
        let mut live = vec![false; self.nodes.len()];
        (live[0], live[1]) = (true, true);
        for r in &roots {
            live[**r as usize] = true;
        }
        // A node's children have smaller ids, so one pass from the newest marks everything.
        for n in (2..self.nodes.len()).rev() {
            if live[n] {
                let (_, lo, hi) = self.nodes[n];
                (live[lo as usize], live[hi as usize]) = (true, true);
            }
        }
        let mut to = vec![0 as Node; self.nodes.len()];
        let mut kept = 0usize;
        for n in 0..self.nodes.len() {
            if !live[n] {
                continue;
            }
            let (level, lo, hi) = self.nodes[n];
            self.nodes[kept] = if n < 2 {
                (level, lo, hi)
            } else {
                (level, to[lo as usize], to[hi as usize])
            };
            to[n] = kept as Node;
            kept += 1;
        }
        self.nodes.truncate(kept);
        self.nodes.shrink_to(kept.max(1 << 10));
        for r in roots {
            *r = to[*r as usize];
        }
        for m in &mut self.memo {
            m.fill(EMPTY);
        }
        self.rehash();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> impl FnMut(u64) -> u64 {
        let mut x = seed;
        move |n| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        }
    }

    /// The truth table of `f` over the first `n` levels, one bit per assignment.
    fn table(b: &Bdd, f: Node, n: u32) -> u64 {
        (0..1u64 << n).fold(0, |t, i| {
            let mut x = f;
            while b.level(x) != LEAF {
                let (level, lo, hi) = b.nodes[x as usize];
                x = if i >> level & 1 == 1 { hi } else { lo };
            }
            t | u64::from(x == TRUE) << i
        })
    }

    /// Equal functions get equal ids, and both operations agree with truth tables.
    #[test]
    fn apply_is_canonical_and_agrees_with_truth_tables() {
        let mut rnd = rng(0x0BDD_5EED_1234_5678);
        let mut go = || true;
        for _ in 0..300 {
            let n = 2 + rnd(4) as u32;
            let mut b = Bdd::new(1 << 16);
            let mut pool: Vec<(Node, u64)> = vec![(FALSE, 0), (TRUE, (1u64 << (1 << n)) - 1)];
            for level in 0..n {
                let v = b.var(level);
                pool.push((v, table(&b, v, n)));
            }
            for _ in 0..60 {
                let (f, tf) = pool[rnd(pool.len() as u64) as usize];
                let (g, tg) = pool[rnd(pool.len() as u64) as usize];
                let (op, want) = if rnd(2) == 0 {
                    (Op::And, tf & tg)
                } else {
                    (Op::Xor, tf ^ tg)
                };
                let r = b.apply(op, f, g, &mut go);
                assert!(!b.overflowed);
                assert_eq!(table(&b, r, n), want);
                assert_eq!(
                    b.density(r),
                    f64::from(want.count_ones()) / f64::from(1u32 << n)
                );
                if r != FALSE {
                    let at = b
                        .witness(r)
                        .iter()
                        .fold(0u64, |i, &(l, v)| i | u64::from(v) << l);
                    assert_eq!(want >> at & 1, 1);
                }
                for level in 0..n {
                    let flipped =
                        (0..1u64 << n).fold(0u64, |t, i| t | (want >> (i ^ 1 << level) & 1) << i);
                    assert_eq!(b.reads(r, level), flipped != want);
                }
                // Canonical: the same function never has two ids.
                for &(h, th) in &pool {
                    assert_eq!(h == r, th == want);
                }
                if pool.iter().all(|&(h, _)| h != r) {
                    pool.push((r, want));
                }
            }
        }
    }

    /// A diagram as deep as it has variables does not use the call stack.
    #[test]
    fn deep_diagrams_do_not_recurse() {
        let mut b = Bdd::new(1 << 22);
        let mut go = || true;
        let n = 200_000u32;
        let (mut parity, mut all) = (FALSE, TRUE);
        // Deepest variable first, so each step adds nodes at the root only.
        for level in (0..n).rev() {
            let v = b.var(level);
            parity = b.apply(Op::Xor, parity, v, &mut go);
            all = b.apply(Op::And, all, v, &mut go);
        }
        assert!(!b.overflowed);
        assert_eq!(b.apply(Op::Xor, parity, parity, &mut go), FALSE);
        assert_ne!(b.apply(Op::And, parity, all, &mut go), parity);
    }

    /// Past the node limit, and when the caller runs out of work, the manager says so.
    #[test]
    fn limits_overflow_instead_of_answering() {
        let build = |b: &mut Bdd, tick: &mut dyn FnMut() -> bool| {
            // The hidden weighted bit function over interleaved-badly variables: many nodes.
            let mut acc = FALSE;
            for i in 0..12u32 {
                let (x, y) = (b.var(i), b.var(i + 12));
                let p = b.apply(Op::And, x, y, &mut *tick);
                acc = b.apply(Op::Xor, acc, p, &mut *tick);
            }
            acc
        };
        let mut b = Bdd::new(1 << 20);
        let f = build(&mut b, &mut || true);
        assert!(!b.overflowed && f != FALSE && b.len() > 4_000);
        let mut small = Bdd::new(1_000);
        build(&mut small, &mut || true);
        assert!(small.overflowed);
        let mut left = 100u32;
        let mut starved = Bdd::new(1 << 20);
        build(&mut starved, &mut || {
            left = left.saturating_sub(1);
            left > 0
        });
        assert!(starved.overflowed);
    }

    /// Collection keeps exactly what the roots name, and the kept functions are unchanged.
    #[test]
    fn collection_keeps_the_roots_and_their_functions() {
        let mut rnd = rng(0x00C0_11EC_7000_0001);
        let mut go = || true;
        for _ in 0..100 {
            let n = 5u32;
            let mut b = Bdd::new(1 << 16);
            let mut pool: Vec<Node> = (0..n).map(|level| b.var(level)).collect();
            for _ in 0..80 {
                let f = pool[rnd(pool.len() as u64) as usize];
                let g = pool[rnd(pool.len() as u64) as usize];
                let op = if rnd(2) == 0 { Op::And } else { Op::Xor };
                pool.push(b.apply(op, f, g, &mut go));
            }
            let mut kept: Vec<Node> = pool.iter().copied().filter(|_| rnd(4) == 0).collect();
            let want: Vec<u64> = kept.iter().map(|&f| table(&b, f, n)).collect();
            let before = b.len();
            b.collect(kept.iter_mut());
            assert!(b.len() <= before);
            let got: Vec<u64> = kept.iter().map(|&f| table(&b, f, n)).collect();
            assert_eq!(got, want);
            // Still canonical: rebuilding a kept function finds the kept node.
            for (i, &f) in kept.iter().enumerate() {
                for (j, &g) in kept.iter().enumerate() {
                    assert_eq!(f == g, want[i] == want[j]);
                    let x = b.apply(Op::Xor, f, g, &mut go);
                    assert_eq!(x == FALSE, want[i] == want[j]);
                    assert_eq!(table(&b, x, n), want[i] ^ want[j]);
                }
            }
            // Everything left is reachable from a root.
            let size = b.len();
            b.collect(kept.iter_mut());
            assert!(b.len() <= size);
            let mut none: Vec<Node> = Vec::new();
            b.collect(none.iter_mut());
            assert!(b.is_empty());
        }
    }
}
