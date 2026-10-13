//! Outcome coverage (`eval_circuit --outcome-coverage`; spec/SAMPLING.md section 2.3 and
//! spec/DESIGN.md section 9): which classically conditioned sites of a lowered op stream can
//! execute, how often each one did, and outcome streams that make a chosen site execute.
//!
//! A **site** is every `Push` and every other op with a condition of its own, numbered in stream
//! order. A site executes on a lane when its whole mask is 1 there: the base mask after the
//! `Push`, or `base & own condition` for a conditioned op.
//!
//! Three pieces, none of which runs outside the mode:
//!
//! - [`SiteCounts`] / [`CoverageSink`]: the engines count, per site, the lanes on which it
//!   executed, split by the sampled control bit. The counts come from the masks that gate the
//!   ops, so they do not depend on anything else in this module.
//! - [`analyze`]: a static pass that follows each classical bit as a function of the `Hmr`
//!   outcomes (variable `j` is the outcome of the `j`-th `Hmr` op in stream order, the position
//!   both engines read the outcome stream at). A bit is `Zero`, a conjunction of GF(2)-affine
//!   forms, or `Opaque`. Every rule is exact or yields `Opaque`, so a site classed
//!   [`Class::Dead`] executes for no outcome vector and a site classed [`Class::Affine`]
//!   executes exactly on the solutions of its system.
//! - [`Directed`]: an outcome stream that is uniform on the solution space of one system. Free
//!   variables keep their raw stream bits; each pivot is set from the earlier ones.
//!
//! The static pass grants one thing only: a `Dead` site needs no executions. Everything else it
//! says is checked against the engines' counts by `score::coverage`, which refuses on any
//! disagreement.
use super::compile::SimOp;
use crate::circuit::NONE;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Control-1 executions every site that is not proved unreachable must have.
pub const COVERAGE_MIN: u64 = 256;
/// Lanes of one directed stratum.
pub const STRATUM_LANES: usize = 1024;
/// Most directed strata a run may need; more refuses the run.
pub const MAX_STRATA: usize = 1024;
/// Most rows a tracked system may have; a deeper conjunction is `Opaque`.
pub const MAX_RANK: usize = 64;
/// Work budget of the static pass, in variable entries touched. Past it every value is
/// `Opaque`, so a forged stream costs bounded time and can only be refused.
const WORK_CAP: u64 = 1 << 31;

/// One equation over the outcome variables: the XOR of `vars` equals `rhs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Row {
    /// Variable ordinals, ascending.
    pub vars: Vec<u32>,
    pub rhs: bool,
}

/// A consistent linear system over GF(2) in reduced echelon form: each row's pivot is its
/// highest variable, no row contains another row's pivot, and rows ascend by pivot. The form is
/// unique for a solution set. The empty system holds for every outcome vector.
pub type System = Vec<Row>;

/// The bytes a stratum id commits to: `u32` row count, then per row one byte `rhs`, a `u32`
/// variable count and the variables as `u32`, all little-endian.
#[must_use]
pub fn canonical_bytes(s: &[Row]) -> Vec<u8> {
    let mut out = (s.len() as u32).to_le_bytes().to_vec();
    for r in s {
        out.push(u8::from(r.rhs));
        out.extend_from_slice(&(r.vars.len() as u32).to_le_bytes());
        for v in &r.vars {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Whether `outcomes` (bit `j` of the vector is variable `j`) solves `s`.
#[must_use]
pub fn satisfies(s: &[Row], outcome: &dyn Fn(u32) -> bool) -> bool {
    s.iter()
        .all(|r| r.vars.iter().fold(false, |a, &v| a ^ outcome(v)) == r.rhs)
}

/// A classical bit (or a path condition) as a function of the outcome variables.
#[derive(Clone, Debug)]
enum Value {
    /// 0 for every outcome vector.
    Zero,
    /// 1 exactly on the solutions of the system.
    Conj(Rc<System>),
    /// Not tracked: any function.
    Opaque,
}

/// A site's static class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Proved unreachable: its condition is 0 for every outcome vector.
    Dead,
    /// Executes exactly on the solutions of a system of this rank, so with probability
    /// `2^-rank` under uniform outcomes.
    Affine,
    /// Not tracked.
    Opaque,
}

/// One site of the lowered stream.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    /// Index of the op in the lowered stream (`Compiled::ops`).
    pub op: u32,
    /// Conditions in force on the site: enclosing `Push`es plus the site's own.
    pub depth: u16,
    pub class: Class,
    /// Rank of the system (`Affine` only; 0 otherwise).
    pub rank: u8,
}

/// What [`analyze`] found.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub sites: Vec<Site>,
    /// `Hmr` ops in the stream: the number of outcome variables.
    pub hmr_ops: usize,
    /// The work budget ran out; later sites are `Opaque`.
    pub capped: bool,
}

impl SimOp {
    /// The op's own condition bit, or `NONE` (`Push` carries its bit separately).
    #[must_use]
    pub fn cond(&self) -> u32 {
        match *self {
            Self::Neg { cond }
            | Self::X { cond, .. }
            | Self::Cx { cond, .. }
            | Self::Ccx { cond, .. }
            | Self::Swap { cond, .. }
            | Self::Phase { cond, .. }
            | Self::Hmr { cond, .. }
            | Self::Bit { cond, .. }
            | Self::Sys { cond, .. }
            | Self::SysS { cond, .. }
            | Self::Givens { cond, .. }
            | Self::SpinSwap { cond, .. } => cond,
            Self::Reset { .. } | Self::Push { .. } | Self::Pop | Self::Reflect => NONE,
        }
    }

    /// Whether the op is a site (module docs).
    #[must_use]
    pub fn is_site(&self) -> bool {
        matches!(self, Self::Push { .. }) || self.cond() != NONE
    }
}

/// Number of sites in `ops`.
#[must_use]
pub fn site_count(ops: &[SimOp]) -> usize {
    ops.iter().filter(|o| o.is_site()).count()
}

/// Symmetric difference of two ascending lists.
fn xor_vars(a: &[u32], b: &[u32]) -> Vec<u32> {
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
    out
}

fn pivot(r: &Row) -> u32 {
    r.vars.last().copied().unwrap_or(0)
}

struct Pass {
    bits: Vec<Value>,
    work: u64,
    capped: bool,
    one: Rc<System>,
}

impl Pass {
    fn spend(&mut self, n: usize) {
        self.work += n as u64 + 1;
        if self.work > WORK_CAP {
            self.capped = true;
        }
    }

    fn bit(&self, b: u32) -> Value {
        self.bits.get(b as usize).cloned().unwrap_or(Value::Zero)
    }

    fn set(&mut self, b: u32, v: Value) {
        let i = b as usize;
        if self.bits.len() <= i {
            self.bits.resize(i + 1, Value::Zero);
        }
        self.bits[i] = v;
    }

    /// Adds `row` to the reduced system `s`, keeping it reduced. `false`: the system became
    /// inconsistent.
    fn insert(&mut self, s: &mut System, mut row: Row) -> bool {
        for r in s.iter() {
            if row.vars.binary_search(&pivot(r)).is_ok() {
                self.work += (row.vars.len() + r.vars.len()) as u64;
                row.vars = xor_vars(&row.vars, &r.vars);
                row.rhs ^= r.rhs;
            }
        }
        if row.vars.is_empty() {
            return !row.rhs;
        }
        // The new pivot is below the pivot of any row that contains it, and `row` holds no
        // other pivot, so clearing it from those rows keeps every row's pivot and the form.
        let p = pivot(&row);
        for r in s.iter_mut() {
            if r.vars.binary_search(&p).is_ok() {
                self.work += (row.vars.len() + r.vars.len()) as u64;
                r.vars = xor_vars(&r.vars, &row.vars);
                r.rhs ^= row.rhs;
            }
        }
        let at = s.partition_point(|r| pivot(r) < p);
        s.insert(at, row);
        true
    }

    /// The conjunction of two values (exact, or `Opaque`).
    fn conj(&mut self, a: &Value, b: &Value) -> Value {
        match (a, b) {
            (Value::Zero, _) | (_, Value::Zero) => Value::Zero,
            (Value::Opaque, _) | (_, Value::Opaque) => Value::Opaque,
            (Value::Conj(x), Value::Conj(y)) => {
                if x.is_empty() || Rc::ptr_eq(x, y) {
                    return b.clone();
                }
                if y.is_empty() {
                    return a.clone();
                }
                if self.capped {
                    return Value::Opaque;
                }
                let mut s: System = (**x).clone();
                self.spend(s.iter().map(|r| r.vars.len()).sum());
                for row in y.iter() {
                    if !self.insert(&mut s, row.clone()) {
                        return Value::Zero;
                    }
                    self.spend(0);
                    if s.len() > MAX_RANK || self.capped {
                        return Value::Opaque;
                    }
                }
                Value::Conj(Rc::new(s))
            }
        }
    }

    /// `v` as one affine form, when it is one: `Zero`, the constant 1 or a single row. The form
    /// is `XOR(vars) ^ rhs ^ 1`, so `Zero` is `([], true)` and the constant 1 is `([], false)`.
    fn single(v: &Value) -> Option<Row> {
        match v {
            Value::Zero => Some(Row {
                vars: Vec::new(),
                rhs: true,
            }),
            Value::Conj(s) if s.is_empty() => Some(Row {
                vars: Vec::new(),
                rhs: false,
            }),
            Value::Conj(s) if s.len() == 1 => Some(s[0].clone()),
            _ => None,
        }
    }

    /// `Hmr` into `b` on the outcomes where `q` holds: `b = q ? v_j : b`.
    fn hmr(&mut self, b: u32, j: u32, q: &Value) {
        let fresh = Row {
            vars: vec![j],
            rhs: true,
        };
        let new = match (q, self.bit(b)) {
            (Value::Zero, _) => return,
            (Value::Conj(s), _) if s.is_empty() => Value::Conj(Rc::new(vec![fresh])),
            // `v_j` is a new variable above every other: the system stays reduced.
            (Value::Conj(s), Value::Zero) if s.len() < MAX_RANK && !self.capped => {
                let mut t: System = (**s).clone();
                self.spend(t.iter().map(|r| r.vars.len()).sum());
                t.push(fresh);
                Value::Conj(Rc::new(t))
            }
            _ => Value::Opaque,
        };
        self.set(b, new);
    }

    /// `BitInvert` of `b` where `q` holds: `b ^= q`.
    fn invert(&mut self, b: u32, q: &Value) {
        let old = self.bit(b);
        let new = match (q, &old) {
            (Value::Zero, _) => return,
            (_, Value::Opaque) | (Value::Opaque, _) => Value::Opaque,
            (_, Value::Zero) => q.clone(),
            _ => match (Self::single(&old), Self::single(q)) {
                (Some(x), Some(y)) if !self.capped => {
                    self.spend(x.vars.len() + y.vars.len());
                    let vars = xor_vars(&x.vars, &y.vars);
                    // (X ^ rx ^ 1) ^ (Y ^ ry ^ 1) = (X ^ Y) ^ (rx ^ ry ^ 1) ^ 1.
                    let rhs = !(x.rhs ^ y.rhs);
                    match (vars.is_empty(), rhs) {
                        (true, true) => Value::Zero,
                        (true, false) => Value::Conj(Rc::clone(&self.one)),
                        _ => Value::Conj(Rc::new(vec![Row { vars, rhs }])),
                    }
                }
                _ => Value::Opaque,
            },
        };
        self.set(b, new);
    }

    /// `BitStore0` / `BitStore1` of `b` where `q` holds: `b = q ? c : b`.
    fn store(&mut self, b: u32, c: bool, q: &Value) {
        let old = self.bit(b);
        let same = match &old {
            Value::Zero => !c,
            Value::Conj(s) => c && s.is_empty(),
            Value::Opaque => false,
        };
        if same || matches!(q, Value::Zero) {
            return;
        }
        let constant = if c {
            Value::Conj(Rc::clone(&self.one))
        } else {
            Value::Zero
        };
        let new = match q {
            Value::Conj(s) if s.is_empty() => constant,
            _ => Value::Opaque,
        };
        self.set(b, new);
    }
}

/// Runs the pass. `on_site(ordinal, system)` is called for every `Affine` site.
fn pass(ops: &[SimOp], mut on_site: impl FnMut(usize, &Rc<System>)) -> Analysis {
    let one = Rc::new(System::new());
    let mut p = Pass {
        bits: Vec::new(),
        work: 0,
        capped: false,
        one: Rc::clone(&one),
    };
    let top = Value::Conj(one);
    let mut out = Analysis::default();
    // Cumulative path conditions: each entry is the conjunction of every bit pushed so far,
    // taken at the time of its `Push` (a later write to the bit does not change the mask).
    let mut stack: Vec<Value> = Vec::new();
    let mut base = top.clone();
    for (i, op) in ops.iter().enumerate() {
        p.spend(0);
        if matches!(op, SimOp::Pop) {
            base = stack.pop().unwrap_or_else(|| top.clone());
            continue;
        }
        let (q, depth) = match *op {
            SimOp::Push { bit } => {
                stack.push(base.clone());
                base = p.conj(&base, &p.bit(bit));
                (base.clone(), stack.len())
            }
            _ if op.cond() != NONE => (p.conj(&base, &p.bit(op.cond())), stack.len() + 1),
            _ => (base.clone(), stack.len()),
        };
        if op.is_site() {
            let (class, rank) = match &q {
                Value::Zero => (Class::Dead, 0),
                Value::Conj(s) => {
                    on_site(out.sites.len(), s);
                    (Class::Affine, s.len() as u8)
                }
                Value::Opaque => (Class::Opaque, 0),
            };
            out.sites.push(Site {
                op: i as u32,
                depth: u16::try_from(depth).unwrap_or(u16::MAX),
                class,
                rank,
            });
        }
        match *op {
            SimOp::Hmr { bit, .. } => {
                p.hmr(bit, out.hmr_ops as u32, &q);
                out.hmr_ops += 1;
            }
            SimOp::Bit { bit, kind: 0, .. } => p.invert(bit, &q),
            SimOp::Bit { bit, kind, .. } => p.store(bit, kind != 1, &q),
            _ => {}
        }
    }
    out.capped = p.capped;
    out
}

/// Classifies every site of `ops` (module docs).
#[must_use]
pub fn analyze(ops: &[SimOp]) -> Analysis {
    pass(ops, |_, _| {})
}

/// [`systems`] found more distinct systems than its cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooMany;

/// The distinct systems of the `Affine` sites `wanted` (ascending site ordinals), each with the
/// wanted sites that have it, in order of first appearance. Collection stops once there are
/// more than `max` systems, so memory is bounded whatever the stream.
///
/// # Errors
/// There are more than `max` distinct systems.
pub fn systems(
    ops: &[SimOp],
    wanted: &[usize],
    max: usize,
) -> Result<Vec<(System, Vec<usize>)>, TooMany> {
    let mut groups: Vec<(System, Vec<usize>)> = Vec::new();
    let mut index: HashMap<Rc<System>, usize> = HashMap::new();
    let mut next = 0;
    pass(ops, |site, s| {
        if wanted.get(next) != Some(&site) {
            return;
        }
        next += 1;
        if groups.len() > max {
            return;
        }
        let n = groups.len();
        let g = *index.entry(Rc::clone(s)).or_insert(n);
        if g == n {
            groups.push(((**s).clone(), Vec::new()));
        }
        groups[g].1.push(site);
    });
    if groups.len() > max {
        return Err(TooMany);
    }
    Ok(groups)
}

enum Event {
    /// Keep this free variable's words in the history slot.
    Record(usize),
    /// Replace this pivot's words by the row's value.
    Pivot(usize),
}

/// A system in the form the engines apply while they read the outcome stream: at a pivot's
/// `Hmr` the stream words are replaced by `rhs ^ XOR(the row's other variables)`. Those are
/// free variables with lower ordinals, so their words were read earlier and are kept. The
/// result is uniform on the system's solution space: free variables are the raw stream bits
/// and the pivots are determined.
pub struct Directed {
    /// `(Hmr ordinal, what to do there)`, ascending.
    events: Vec<(u32, Event)>,
    /// Per row: `rhs` and the history slots of its free variables.
    rows: Vec<(bool, Vec<usize>)>,
    slots: usize,
}

impl Directed {
    /// # Panics
    /// If `s` is not in reduced form (a row's pivot appears in another row).
    #[must_use]
    pub fn new(s: &[Row]) -> Self {
        let mut slot: HashMap<u32, usize> = HashMap::new();
        let mut events = Vec::new();
        let mut rows = Vec::new();
        let pivots: Vec<u32> = s.iter().map(pivot).collect();
        for (i, r) in s.iter().enumerate() {
            let free = &r.vars[..r.vars.len().saturating_sub(1)];
            assert!(
                !r.vars.is_empty() && free.iter().all(|v| !pivots.contains(v)),
                "system not in reduced form"
            );
            let deps = free
                .iter()
                .map(|&v| {
                    let n = slot.len();
                    *slot.entry(v).or_insert_with(|| {
                        events.push((v, Event::Record(n)));
                        n
                    })
                })
                .collect();
            rows.push((r.rhs, deps));
            events.push((pivots[i], Event::Pivot(i)));
        }
        events.sort_by_key(|e| e.0);
        Self {
            events,
            rows,
            slots: slot.len(),
        }
    }
}

/// One run's position in a [`Directed`] system: `width` words per variable (a batch or a pass).
pub struct DirectedRun<'a> {
    d: &'a Directed,
    width: usize,
    ordinal: u32,
    next: usize,
    hist: Vec<u64>,
}

impl<'a> DirectedRun<'a> {
    #[must_use]
    pub fn new(d: &'a Directed, width: usize) -> Self {
        Self {
            d,
            width,
            ordinal: 0,
            next: 0,
            hist: vec![0; d.slots * width],
        }
    }

    /// Called once per `Hmr` op in stream order with the outcome words just read from the
    /// stream; leaves them as the outcomes to use.
    #[inline]
    pub fn fix(&mut self, words: &mut [u64]) {
        let j = self.ordinal;
        self.ordinal += 1;
        let Some((at, ev)) = self.d.events.get(self.next) else {
            return;
        };
        if *at != j {
            return;
        }
        self.next += 1;
        let w = self.width;
        match *ev {
            Event::Record(slot) => self.hist[slot * w..(slot + 1) * w].copy_from_slice(words),
            Event::Pivot(row) => {
                let (rhs, deps) = &self.d.rows[row];
                words.fill(if *rhs { u64::MAX } else { 0 });
                for &slot in deps {
                    for (x, h) in words.iter_mut().zip(&self.hist[slot * w..(slot + 1) * w]) {
                        *x ^= h;
                    }
                }
            }
        }
    }
}

/// One batch's (or pass's) per-site execution counts, `[control-1, control-0]`.
pub struct SiteCounts {
    /// The lanes whose sampled control bit is 1.
    c1: Vec<u64>,
    counts: Vec<[u64; 2]>,
    next: usize,
}

impl SiteCounts {
    /// Counts the next site: `m` is its mask (a subset of the active lanes).
    #[inline]
    pub fn add(&mut self, m: &[u64]) {
        let (mut all, mut one) = (0u64, 0u64);
        for (a, c) in m.iter().zip(&self.c1) {
            all += u64::from(a.count_ones());
            one += u64::from((a & c).count_ones());
        }
        if let Some(n) = self.counts.get_mut(self.next) {
            n[0] += one;
            n[1] += all - one;
        }
        self.next += 1;
    }

    /// Marks lane `lane` as control-1.
    pub fn set_control_one(&mut self, lane: usize) {
        self.c1[lane / 64] |= 1 << (lane % 64);
    }
}

/// Collects the engines' per-site counts for one engine call. Sums commute, so the totals do
/// not depend on thread scheduling.
pub struct CoverageSink {
    sites: usize,
    total: Mutex<Vec<[u64; 2]>>,
    pool: Mutex<Vec<Vec<[u64; 2]>>>,
    mismatch: AtomicBool,
}

impl CoverageSink {
    #[must_use]
    pub fn new(sites: usize) -> Self {
        Self {
            sites,
            total: Mutex::new(vec![[0; 2]; sites]),
            pool: Mutex::new(Vec::new()),
            mismatch: AtomicBool::new(false),
        }
    }

    /// Fresh counts for a batch or pass of `width` words.
    #[must_use]
    pub fn begin(&self, width: usize) -> SiteCounts {
        let counts = self
            .pool
            .lock()
            .ok()
            .and_then(|mut p| p.pop())
            .unwrap_or_else(|| vec![[0; 2]; self.sites]);
        SiteCounts {
            c1: vec![0; width],
            counts,
            next: 0,
        }
    }

    /// Adds a run's counts to the totals. `complete`: the run reached the end of the stream, so
    /// it must have passed exactly this sink's number of sites.
    pub fn commit(&self, mut c: SiteCounts, complete: bool) {
        if complete && c.next != self.sites {
            self.mismatch.store(true, Ordering::Relaxed);
        }
        match self.total.lock() {
            Ok(mut t) => {
                for (a, b) in t.iter_mut().zip(c.counts.iter_mut()) {
                    a[0] += b[0];
                    a[1] += b[1];
                    *b = [0; 2];
                }
            }
            Err(_) => self.mismatch.store(true, Ordering::Relaxed),
        }
        if let Ok(mut p) = self.pool.lock() {
            p.push(c.counts);
        }
    }

    /// Drops a run's counts (the run is repeated elsewhere).
    pub fn discard(&self, mut c: SiteCounts) {
        c.counts.fill([0; 2]);
        if let Ok(mut p) = self.pool.lock() {
            p.push(c.counts);
        }
    }

    /// The totals.
    ///
    /// # Errors
    /// An engine run passed a different number of sites than the sink was built for.
    pub fn finish(self) -> Result<Vec<[u64; 2]>, String> {
        if self.mismatch.load(Ordering::Relaxed) {
            return Err(format!(
                "outcome-coverage: internal error: the engine did not pass exactly the {} sites \
                 the static pass counted; refusing",
                self.sites
            ));
        }
        self.total
            .into_inner()
            .map_err(|_| "outcome-coverage: internal error: poisoned counts; refusing".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64*.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// The engines' classical semantics on one outcome vector: per site, whether it executes.
    fn interpret(ops: &[SimOp], outcomes: u64) -> Vec<bool> {
        let mut bits = [false; 16];
        let mut stack = Vec::new();
        let mut base = true;
        let mut hmr = 0;
        let mut out = Vec::new();
        for op in ops {
            match *op {
                SimOp::Push { bit } => {
                    stack.push(base);
                    base &= bits[bit as usize];
                    out.push(base);
                }
                SimOp::Pop => base = stack.pop().unwrap_or(true),
                _ => {
                    let c = op.cond();
                    let m = base && (c == NONE || bits[c as usize]);
                    if c != NONE {
                        out.push(m);
                    }
                    match *op {
                        SimOp::Hmr { bit, .. } => {
                            if m {
                                bits[bit as usize] = outcomes >> hmr & 1 == 1;
                            }
                            hmr += 1;
                        }
                        SimOp::Bit { bit, kind, .. } if m => {
                            bits[bit as usize] = match kind {
                                0 => !bits[bit as usize],
                                1 => false,
                                _ => true,
                            };
                        }
                        _ => {}
                    }
                }
            }
        }
        out
    }

    fn random_program(r: &mut Rng) -> Vec<SimOp> {
        let nbits = 2 + r.below(5) as u32;
        let len = 4 + r.below(28);
        let (mut ops, mut depth, mut hmr) = (Vec::new(), 0, 0);
        let cond = |r: &mut Rng| {
            if r.below(2) == 0 {
                NONE
            } else {
                r.below(u64::from(nbits)) as u32
            }
        };
        for _ in 0..len {
            let bit = r.below(u64::from(nbits)) as u32;
            match r.below(10) {
                0..=2 if hmr < 10 => {
                    hmr += 1;
                    ops.push(SimOp::Hmr {
                        t: 0,
                        bit,
                        cond: cond(r),
                    });
                }
                3..=5 => ops.push(SimOp::Bit {
                    bit,
                    kind: if r.below(4) == 0 {
                        1 + r.below(2) as u8
                    } else {
                        0
                    },
                    cond: cond(r),
                }),
                6 if depth < 4 => {
                    depth += 1;
                    ops.push(SimOp::Push { bit });
                }
                7 if depth > 0 => {
                    depth -= 1;
                    ops.push(SimOp::Pop);
                }
                _ => ops.push(SimOp::Neg { cond: cond(r) }),
            }
        }
        ops.extend((0..depth).map(|_| SimOp::Pop));
        ops
    }

    /// Soundness of the static pass: over every outcome vector of random classical programs, a
    /// `Dead` site never executes and an `Affine` site executes exactly on the solutions of its
    /// system, which number `2^(n - rank)`.
    #[test]
    fn static_pass_is_exact_on_random_programs() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut dead, mut affine, mut opaque, mut deep) = (0, 0, 0, 0);
        for _ in 0..4000 {
            let ops = random_program(&mut r);
            let a = analyze(&ops);
            let n = a.hmr_ops;
            assert!(n <= 10 && !a.capped);
            assert_eq!(a.sites.len(), site_count(&ops));
            let wanted: Vec<usize> = (0..a.sites.len())
                .filter(|&i| a.sites[i].class == Class::Affine)
                .collect();
            let groups = systems(&ops, &wanted, usize::MAX).unwrap();
            let mut system: HashMap<usize, &System> = HashMap::new();
            for (s, members) in &groups {
                for &m in members {
                    system.insert(m, s);
                }
            }
            let runs: Vec<Vec<bool>> = (0..1u64 << n).map(|o| interpret(&ops, o)).collect();
            for (i, site) in a.sites.iter().enumerate() {
                let truth = |o: usize| runs[o][i];
                match site.class {
                    Class::Dead => {
                        dead += 1;
                        assert!(
                            (0..1 << n).all(|o| !truth(o)),
                            "dead site executes: {ops:?}"
                        );
                    }
                    Class::Affine => {
                        affine += 1;
                        deep += usize::from(site.rank >= 2);
                        let s = system[&i];
                        assert_eq!(s.len(), usize::from(site.rank));
                        let mut count = 0;
                        for o in 0..1usize << n {
                            let sat = satisfies(s, &|v| o >> v & 1 == 1);
                            assert_eq!(sat, truth(o), "site {i} of {ops:?} at outcomes {o:b}");
                            count += usize::from(sat);
                        }
                        assert_eq!(count, 1 << (n - s.len()));
                    }
                    Class::Opaque => opaque += 1,
                }
            }
        }
        // The fuzz reaches every class, and conjunctions.
        assert!(dead > 500 && affine > 5000 && opaque > 500 && deep > 500);
    }

    fn random_system(r: &mut Rng, nvars: u32) -> System {
        let mut p = Pass {
            bits: Vec::new(),
            work: 0,
            capped: false,
            one: Rc::new(System::new()),
        };
        let mut s = System::new();
        for _ in 0..1 + r.below(6) {
            let vars: Vec<u32> = (0..nvars).filter(|_| r.below(3) == 0).collect();
            let row = Row {
                vars,
                rhs: r.below(2) == 1,
            };
            let mut t = s.clone();
            if p.insert(&mut t, row) {
                s = t;
            }
        }
        s
    }

    /// The directed stream: every lane's outcomes solve the system, and the free variables are
    /// the raw stream words.
    #[test]
    fn directed_outcomes_solve_the_system() {
        let mut r = Rng(7);
        for width in [8usize, 16] {
            for _ in 0..300 {
                let nvars = 12;
                let s = random_system(&mut r, nvars);
                let d = Directed::new(&s);
                let mut run = DirectedRun::new(&d, width);
                let pivots: Vec<u32> = s.iter().map(pivot).collect();
                let mut used = Vec::new();
                for j in 0..nvars {
                    let raw: Vec<u64> = (0..width).map(|_| r.next()).collect();
                    let mut w = raw.clone();
                    run.fix(&mut w);
                    assert!(pivots.contains(&j) || w == raw, "free variable changed");
                    used.push(w);
                }
                for lane in 0..64 * width {
                    let at = |v: u32| used[v as usize][lane / 64] >> (lane % 64) & 1 == 1;
                    assert!(satisfies(&s, &at), "lane {lane} of {s:?}");
                }
            }
        }
    }

    #[test]
    fn canonical_form_is_unique_and_inconsistency_is_found() {
        let mut p = Pass {
            bits: Vec::new(),
            work: 0,
            capped: false,
            one: Rc::new(System::new()),
        };
        let row = |vars: &[u32], rhs| Row {
            vars: vars.to_vec(),
            rhs,
        };
        let rows = [row(&[0, 1], true), row(&[1, 2], false), row(&[0, 3], true)];
        let mut a = System::new();
        let mut b = System::new();
        for r in &rows {
            assert!(p.insert(&mut a, r.clone()));
        }
        for r in rows.iter().rev() {
            assert!(p.insert(&mut b, r.clone()));
        }
        assert_eq!(a, b);
        assert_eq!(canonical_bytes(&a), canonical_bytes(&b));
        // v0 ^ v2 = 1 follows from the first two rows; its negation is inconsistent.
        assert!(p.insert(&mut a.clone(), row(&[0, 2], true)));
        assert_eq!(a.len(), 3);
        assert!(!p.insert(&mut a, row(&[0, 2], false)));
    }

    /// A conjunction deeper than `MAX_RANK` is `Opaque`, never `Dead` or a truncated system.
    #[test]
    fn deep_conjunctions_become_opaque() {
        let n = MAX_RANK as u32 + 4;
        let mut ops = Vec::new();
        for b in 0..n {
            ops.push(SimOp::Hmr {
                t: 0,
                bit: b,
                cond: NONE,
            });
        }
        ops.extend((0..n).map(|bit| SimOp::Push { bit }));
        let a = analyze(&ops);
        for (i, s) in a.sites.iter().enumerate() {
            if i < MAX_RANK {
                assert_eq!((s.class, usize::from(s.rank)), (Class::Affine, i + 1));
            } else {
                assert_eq!(s.class, Class::Opaque);
            }
        }
    }

    #[test]
    fn sink_sums_and_detects_a_site_count_mismatch() {
        let sink = CoverageSink::new(2);
        for _ in 0..3 {
            let mut c = sink.begin(1);
            c.set_control_one(0);
            c.set_control_one(1);
            c.add(&[0b0111]);
            c.add(&[0b1000]);
            sink.commit(c, true);
        }
        let d = sink.begin(1);
        sink.discard(d);
        assert_eq!(sink.finish().unwrap(), vec![[6, 3], [0, 3]]);
        let sink = CoverageSink::new(2);
        let mut c = sink.begin(1);
        c.add(&[1]);
        sink.commit(c, true);
        assert!(sink.finish().unwrap_err().contains("internal error"));
    }
}
