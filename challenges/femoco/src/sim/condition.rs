//! Outcome coverage (`eval_circuit --outcome-coverage`; spec/SAMPLING.md section 2.3 and
//! spec/DESIGN.md section 9): which classically conditioned sites of a lowered op stream can
//! execute, how often each one did, and outcome streams that make a chosen site execute.
//!
//! A **site** is every `Push` and every other op with a condition of its own, numbered in stream
//! order. A site executes on a lane when its whole mask is 1 there: the base mask after the
//! `Push`, or `base & own condition` for a conditioned op.
//!
//! Four pieces, none of which runs outside the mode:
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
//! - [`qubit_outcomes`]: a second static pass, independent of the first, that finds the ops
//!   through which an outcome can act without a site: a qubit written under a classical
//!   condition holds a function of the outcomes, and an op that reads it (as a control, a phase
//!   operand, an angle bit, a measured or freed qubit) is steered by the outcomes although it
//!   carries no condition. Sites do not see such ops, so the mode refuses a stream that has one.
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
/// Most variables one equation may have; a longer one makes its value `Opaque`. Parities of
/// outcomes in the ledger's circuits are far shorter (their site tables do not change with this
/// cap).
pub const MAX_ROW: usize = 1 << 12;
/// Work budget of the static pass, in variable entries touched. Past it every value is
/// `Opaque`. It bounds the pass's time only; [`HELD_CAP`] and [`MAX_ROW`] bound its memory.
const WORK_CAP: u64 = 1 << 31;
/// Memory budget of the static pass, in [`weight`] units (one per variable entry, eight per
/// equation) summed over every classical bit and every open condition block. A value that
/// would take the total past it is stored as `Opaque`. With [`MAX_ROW`] this bounds what the
/// pass holds at about 64 MiB of variable entries whatever the stream (the per-bit and per-op
/// tables aside, which are linear in the stream), so a forged stream costs bounded time and
/// memory and can only be refused.
const HELD_CAP: u64 = 1 << 24;

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
    /// A value was dropped to `Opaque` because an equation passed [`MAX_ROW`] variables or the
    /// pass would have held more than its memory budget.
    pub memory_capped: bool,
    /// Most [`weight`] units the pass held at once (at most the memory budget).
    pub peak_held: u64,
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

/// What a value costs the memory budget: one unit per variable entry and eight per equation.
/// Every holder is charged in full, so shared systems are counted more than once and the sum
/// is an upper bound.
fn weight(v: &Value) -> u64 {
    match v {
        Value::Conj(s) => s.iter().map(|r| r.vars.len() as u64 + 8).sum(),
        Value::Zero | Value::Opaque => 0,
    }
}

/// What [`Pass::insert`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Insert {
    /// The row was added (or was implied already); the system is reduced.
    Done,
    /// The system became inconsistent.
    Inconsistent,
    /// An equation passed [`MAX_ROW`] variables; the system is to be discarded.
    TooLong,
}

struct Pass {
    bits: Vec<Value>,
    work: u64,
    capped: bool,
    /// [`weight`] of every value in `bits` and of every saved path condition.
    held: u64,
    peak_held: u64,
    memory_capped: bool,
    one: Rc<System>,
}

impl Pass {
    fn new() -> Self {
        Self {
            bits: Vec::new(),
            work: 0,
            capped: false,
            held: 0,
            peak_held: 0,
            memory_capped: false,
            one: Rc::new(System::new()),
        }
    }

    /// Charges `w` to the memory budget. `false`: it does not fit, and nothing was charged.
    fn hold(&mut self, w: u64) -> bool {
        if self.held + w > HELD_CAP {
            self.memory_capped = true;
            return false;
        }
        self.held += w;
        self.peak_held = self.peak_held.max(self.held);
        true
    }

    fn spend(&mut self, n: usize) {
        self.work += n as u64 + 1;
        if self.work > WORK_CAP {
            self.capped = true;
        }
    }

    fn bit(&self, b: u32) -> Value {
        self.bits.get(b as usize).cloned().unwrap_or(Value::Zero)
    }

    fn set(&mut self, b: u32, mut v: Value) {
        let i = b as usize;
        if self.bits.len() <= i {
            self.bits.resize(i + 1, Value::Zero);
        }
        self.held -= weight(&self.bits[i]);
        if !self.hold(weight(&v)) {
            v = Value::Opaque;
        }
        self.bits[i] = v;
    }

    /// Adds `row` to the reduced system `s`, keeping it reduced. No equation is left longer
    /// than [`MAX_ROW`]: the first that would be stops the insertion.
    fn insert(&mut self, s: &mut System, mut row: Row) -> Insert {
        for r in s.iter() {
            if row.vars.binary_search(&pivot(r)).is_ok() {
                self.work += (row.vars.len() + r.vars.len()) as u64;
                row.vars = xor_vars(&row.vars, &r.vars);
                row.rhs ^= r.rhs;
                if row.vars.len() > MAX_ROW {
                    self.memory_capped = true;
                    return Insert::TooLong;
                }
            }
        }
        if row.vars.is_empty() {
            return if row.rhs {
                Insert::Inconsistent
            } else {
                Insert::Done
            };
        }
        // The new pivot is below the pivot of any row that contains it, and `row` holds no
        // other pivot, so clearing it from those rows keeps every row's pivot and the form.
        let p = pivot(&row);
        for r in s.iter_mut() {
            if r.vars.binary_search(&p).is_ok() {
                self.work += (row.vars.len() + r.vars.len()) as u64;
                r.vars = xor_vars(&r.vars, &row.vars);
                r.rhs ^= row.rhs;
                if r.vars.len() > MAX_ROW {
                    self.memory_capped = true;
                    return Insert::TooLong;
                }
            }
        }
        let at = s.partition_point(|r| pivot(r) < p);
        s.insert(at, row);
        Insert::Done
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
                    match self.insert(&mut s, row.clone()) {
                        Insert::Done => {}
                        Insert::Inconsistent => return Value::Zero,
                        Insert::TooLong => return Value::Opaque,
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
                        _ if vars.len() > MAX_ROW => {
                            self.memory_capped = true;
                            Value::Opaque
                        }
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
    let mut p = Pass::new();
    let top = Value::Conj(Rc::clone(&p.one));
    let mut out = Analysis::default();
    // Cumulative path conditions: each entry is the conjunction of every bit pushed so far,
    // taken at the time of its `Push` (a later write to the bit does not change the mask).
    let mut stack: Vec<Value> = Vec::new();
    let mut base = top.clone();
    for (i, op) in ops.iter().enumerate() {
        p.spend(0);
        if matches!(op, SimOp::Pop) {
            base = stack.pop().unwrap_or_else(|| top.clone());
            p.held -= weight(&base);
            continue;
        }
        let (q, depth) = match *op {
            SimOp::Push { bit } => {
                // The saved condition is charged to the memory budget; one that does not fit
                // is saved as `Opaque`, which the block's end then restores.
                let saved = if p.hold(weight(&base)) {
                    base.clone()
                } else {
                    Value::Opaque
                };
                stack.push(saved);
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
    out.memory_capped = p.memory_capped;
    out.peak_held = p.peak_held;
    out
}

/// Classifies every site of `ops` (module docs).
#[must_use]
pub fn analyze(ops: &[SimOp]) -> Analysis {
    pass(ops, |_, _| {})
}

/// Why [`systems`] gave up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unplanned {
    /// More distinct systems than the cap.
    TooMany,
    /// The distinct systems together pass the memory budget of the static pass.
    TooLarge,
}

/// The distinct systems of the `Affine` sites `wanted` (ascending site ordinals), each with the
/// wanted sites that have it, in order of first appearance. Collection stops once there are
/// more than `max` systems or the systems collected weigh more than the pass's memory budget,
/// so memory is bounded whatever the stream.
///
/// # Errors
/// There are more than `max` distinct systems, or they are too large together.
pub fn systems(
    ops: &[SimOp],
    wanted: &[usize],
    max: usize,
) -> Result<Vec<(System, Vec<usize>)>, Unplanned> {
    let mut groups: Vec<(System, Vec<usize>)> = Vec::new();
    let mut index: HashMap<Rc<System>, usize> = HashMap::new();
    let mut next = 0;
    let mut held = 0u64;
    pass(ops, |site, s| {
        if wanted.get(next) != Some(&site) {
            return;
        }
        next += 1;
        if groups.len() > max || held > HELD_CAP {
            return;
        }
        let n = groups.len();
        let g = *index.entry(Rc::clone(s)).or_insert(n);
        if g == n {
            held += s.iter().map(|r| r.vars.len() as u64 + 8).sum::<u64>();
            groups.push(((**s).clone(), Vec::new()));
        }
        groups[g].1.push(site);
    });
    if held > HELD_CAP {
        return Err(Unplanned::TooLarge);
    }
    if groups.len() > max {
        return Err(Unplanned::TooMany);
    }
    Ok(groups)
}

/// One op that reads a qubit holding a function of the outcomes ([`qubit_outcomes`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutcomeRead {
    /// Index of the reading op in the lowered stream; `u32::MAX` for the end of the stream,
    /// whose per-lane checks read every qubit.
    pub op: u32,
    /// The qubit read.
    pub qubit: u32,
    /// Index, in the lowered stream, of the conditioned op the qubit's mark goes back to.
    pub source: u32,
}

/// What [`qubit_outcomes`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QubitOutcomes {
    /// Ops that write a non-system qubit while a classical condition is in force (their own or
    /// an enclosing block's).
    pub conditioned_writes: u64,
    /// Ops that read a marked qubit, the end of the stream counted once per qubit still marked.
    pub reads: u64,
    /// The first of them, in stream order (at most [`QubitOutcomes::KEPT`]).
    pub first: Vec<OutcomeRead>,
    kept: usize,
}

impl QubitOutcomes {
    /// Reads kept in `first`.
    pub const KEPT: usize = 8;
}

/// The mark of qubit `q` (`NONE`: not marked).
fn get(mark: &[u32], q: u32) -> u32 {
    mark.get(q as usize).copied().unwrap_or(NONE)
}

fn put(mark: &mut Vec<u32>, q: u32, v: u32) {
    if q == NONE || (v == NONE && mark.len() <= q as usize) {
        return;
    }
    if mark.len() <= q as usize {
        mark.resize(q as usize + 1, NONE);
    }
    mark[q as usize] = v;
}

/// Records that `op` reads `q` if `q` is marked. Returns `q`'s mark.
fn read(out: &mut QubitOutcomes, mark: &[u32], op: u32, q: u32) -> u32 {
    let source = if q == NONE { NONE } else { get(mark, q) };
    if source != NONE {
        out.reads += 1;
        if out.first.len() < out.kept {
            out.first.push(OutcomeRead {
                op,
                qubit: q,
                source,
            });
        }
    }
    source
}

/// Finds the ops that an outcome can steer without a site.
///
/// A non-system qubit is *marked* from the op that writes it while any classical condition is
/// in force (`X`, `CX`, `CCX`, `Swap`, `Hmr`, or a `Reflect` inside a block): from there on its
/// value may depend on the outcomes. A mark spreads to the target of an `X`-type gate one of
/// whose controls is marked and moves with a `Swap`. Only an `Hmr` with no condition in force
/// removes it, because that sets the qubit to 0 on every lane; `R` does not, because it checks
/// the qubit and leaves it as it is. An op *reads* a marked qubit when the qubit is one of its
/// controls, an operand of its phase, a bit of its angle register, the control of its
/// `SpinSwap`, the qubit it measures or frees, or an inner qubit at the `Reflect`; the checks at
/// the end of the stream read every qubit.
///
/// The rule uses no knowledge of what a condition evaluates to: a condition on a bit that is
/// always 1 marks as any other does. It is sound in one direction only. With no reads reported,
/// every qubit an op reads holds, on every lane and for every outcome record, a function of the
/// lane alone (by induction over the ops: an unmarked qubit is written only by ops that no
/// condition gates and whose controls are unmarked, and the system register never controls a
/// non-system qubit, `compile::lower_system`). A read reported is not a finding that the
/// circuit is wrong: a fix-up `X` conditioned on an outcome marks its qubit although the value
/// it leaves may well be the same for every outcome.
///
/// `registers` are the compiled registers (what `Givens` reads) and `inner` the inner uniform
/// qubits of a nested lane map (empty otherwise).
#[must_use]
pub fn qubit_outcomes(ops: &[SimOp], registers: &[Vec<u32>], inner: &[u32]) -> QubitOutcomes {
    marked_reads(ops, registers, inner, QubitOutcomes::KEPT)
}

/// [`qubit_outcomes`] keeping the first `kept` reads.
fn marked_reads(
    ops: &[SimOp],
    registers: &[Vec<u32>],
    inner: &[u32],
    kept: usize,
) -> QubitOutcomes {
    // Per qubit: the op its mark goes back to, or `NONE`.
    let mut mark: Vec<u32> = Vec::new();
    let mut out = QubitOutcomes {
        kept,
        ..QubitOutcomes::default()
    };
    let mut depth = 0usize;
    for (i, op) in ops.iter().enumerate() {
        let i = i as u32;
        let conditioned = depth > 0 || op.cond() != NONE;
        match *op {
            SimOp::Push { .. } => depth += 1,
            SimOp::Pop => depth = depth.saturating_sub(1),
            SimOp::Neg { .. } | SimOp::Bit { .. } | SimOp::SysS { .. } => {}
            SimOp::X { t, .. } => {
                if conditioned {
                    out.conditioned_writes += 1;
                    put(&mut mark, t, i);
                }
            }
            SimOp::Cx { c: a, t, .. } | SimOp::Ccx { a, t, .. } => {
                let b = match *op {
                    SimOp::Ccx { b, .. } => b,
                    _ => NONE,
                };
                // Both controls are read; the mark the target takes is the first one's.
                let from = [a, b].map(|c| read(&mut out, &mark, i, c));
                let from = if from[0] != NONE { from[0] } else { from[1] };
                if conditioned {
                    out.conditioned_writes += 1;
                    put(&mut mark, t, i);
                } else if from != NONE {
                    put(&mut mark, t, from);
                }
            }
            SimOp::Swap { a, b, .. } => {
                if conditioned {
                    out.conditioned_writes += 1;
                    put(&mut mark, a, i);
                    put(&mut mark, b, i);
                } else {
                    let (ma, mb) = (get(&mark, a), get(&mark, b));
                    put(&mut mark, a, mb);
                    put(&mut mark, b, ma);
                }
            }
            SimOp::Phase { q, .. } => {
                for x in q {
                    read(&mut out, &mark, i, x);
                }
            }
            SimOp::Sys { ctrl, .. } => {
                for x in ctrl {
                    read(&mut out, &mark, i, x);
                }
            }
            SimOp::Hmr { t, .. } => {
                read(&mut out, &mark, i, t);
                if conditioned {
                    out.conditioned_writes += 1;
                    put(&mut mark, t, i);
                } else {
                    put(&mut mark, t, NONE);
                }
            }
            SimOp::Reset { t } | SimOp::SpinSwap { c: t, .. } => {
                read(&mut out, &mark, i, t);
            }
            SimOp::Givens { reg, .. } => {
                for &x in registers.get(reg as usize).map_or(&[][..], Vec::as_slice) {
                    read(&mut out, &mark, i, x);
                }
            }
            SimOp::Reflect => {
                for &x in inner {
                    read(&mut out, &mark, i, x);
                    if conditioned {
                        put(&mut mark, x, i);
                    }
                }
                out.conditioned_writes += u64::from(conditioned);
            }
        }
    }
    for q in 0..mark.len() as u32 {
        read(&mut out, &mark, u32::MAX, q);
    }
    out
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
        let mut p = Pass::new();
        let mut s = System::new();
        for _ in 0..1 + r.below(6) {
            let vars: Vec<u32> = (0..nvars).filter(|_| r.below(3) == 0).collect();
            let row = Row {
                vars,
                rhs: r.below(2) == 1,
            };
            let mut t = s.clone();
            if p.insert(&mut t, row) == Insert::Done {
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
        let mut p = Pass::new();
        let row = |vars: &[u32], rhs| Row {
            vars: vars.to_vec(),
            rhs,
        };
        let rows = [row(&[0, 1], true), row(&[1, 2], false), row(&[0, 3], true)];
        let mut a = System::new();
        let mut b = System::new();
        for r in &rows {
            assert_eq!(p.insert(&mut a, r.clone()), Insert::Done);
        }
        for r in rows.iter().rev() {
            assert_eq!(p.insert(&mut b, r.clone()), Insert::Done);
        }
        assert_eq!(a, b);
        assert_eq!(canonical_bytes(&a), canonical_bytes(&b));
        // v0 ^ v2 = 1 follows from the first two rows; its negation is inconsistent.
        assert_eq!(p.insert(&mut a.clone(), row(&[0, 2], true)), Insert::Done);
        assert_eq!(a.len(), 3);
        assert_eq!(p.insert(&mut a, row(&[0, 2], false)), Insert::Inconsistent);
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

    /// The engines' semantics on qubits for one outcome vector: every qubit an op reads, as
    /// `(op, qubit, value)`, the end of the stream as op `u32::MAX`. `init` is the lane: the
    /// qubits' initial values.
    fn reads_of(ops: &[SimOp], init: u8, outcomes: u64) -> Vec<(u32, u32, bool)> {
        let mut q: Vec<bool> = (0..8).map(|k| init >> k & 1 == 1).collect();
        let mut bits = [false; 16];
        let mut stack = Vec::new();
        let (mut base, mut hmr) = (true, 0);
        let mut out = Vec::new();
        for (i, op) in ops.iter().enumerate() {
            let i = i as u32;
            let c = op.cond();
            let m = base && (c == NONE || bits[c as usize]);
            let mut see = |q: &[bool], x: u32| {
                if x != NONE {
                    out.push((i, x, q[x as usize]));
                }
            };
            match *op {
                SimOp::Push { bit } => {
                    stack.push(base);
                    base &= bits[bit as usize];
                }
                SimOp::Pop => base = stack.pop().unwrap_or(true),
                SimOp::X { t, .. } => q[t as usize] ^= m,
                SimOp::Cx { c, t, .. } => {
                    see(&q, c);
                    q[t as usize] ^= m && q[c as usize];
                }
                SimOp::Ccx { a, b, t, .. } => {
                    see(&q, a);
                    see(&q, b);
                    q[t as usize] ^= m && q[a as usize] && q[b as usize];
                }
                SimOp::Swap { a, b, .. } => {
                    if m {
                        q.swap(a as usize, b as usize);
                    }
                }
                SimOp::Phase { q: qs, .. } => qs.iter().for_each(|&x| see(&q, x)),
                SimOp::Sys { ctrl, .. } => ctrl.iter().for_each(|&x| see(&q, x)),
                SimOp::Hmr { t, bit, .. } => {
                    see(&q, t);
                    if m {
                        bits[bit as usize] = outcomes >> hmr & 1 == 1;
                        q[t as usize] = false;
                    }
                    hmr += 1;
                }
                SimOp::Reset { t } | SimOp::SpinSwap { c: t, .. } => see(&q, t),
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
        out.extend((0..8).map(|x| (u32::MAX, x, q[x as usize])));
        out
    }

    /// A random program over eight qubits and a few classical bits, with every op kind that
    /// reads or writes a non-system qubit, conditioned or not, inside and outside blocks.
    fn random_qubit_program(r: &mut Rng) -> Vec<SimOp> {
        let nbits = 2 + r.below(4) as u32;
        let (mut ops, mut depth, mut hmr) = (Vec::new(), 0, 0);
        // Most ops carry no condition, so that some programs have no marked read at all.
        let cond = |r: &mut Rng| {
            if r.below(4) == 0 {
                r.below(u64::from(nbits)) as u32
            } else {
                NONE
            }
        };
        // Three distinct qubits.
        let three = |r: &mut Rng| {
            let a = r.below(8) as u32;
            let b = (a + 1 + r.below(7) as u32) % 8;
            let c = (0..8).find(|&x| x != a && x != b).unwrap_or(0);
            (a, b, (c + r.below(6) as u32) % 8)
        };
        for _ in 0..4 + r.below(24) {
            let (a, b, c) = three(r);
            let c = if c == a || c == b { NONE } else { c };
            let bit = r.below(u64::from(nbits)) as u32;
            match r.below(14) {
                0 | 1 if hmr < 6 => {
                    hmr += 1;
                    ops.push(SimOp::Hmr {
                        t: a,
                        bit,
                        cond: cond(r),
                    });
                }
                2 => ops.push(SimOp::X {
                    t: a,
                    cond: cond(r),
                }),
                3 | 4 => ops.push(SimOp::Cx {
                    c: a,
                    t: b,
                    cond: cond(r),
                }),
                5 if c != NONE => ops.push(SimOp::Ccx {
                    a,
                    b,
                    t: c,
                    cond: cond(r),
                }),
                6 => ops.push(SimOp::Swap {
                    a,
                    b,
                    cond: cond(r),
                }),
                7 => ops.push(SimOp::Phase {
                    q: [a, b, c],
                    k: 4,
                    cond: cond(r),
                }),
                8 => ops.push(SimOp::Sys {
                    q: 0,
                    z: r.below(2) == 0,
                    ctrl: [a, c],
                    cond: cond(r),
                }),
                9 => ops.push(SimOp::Reset { t: a }),
                10 => ops.push(SimOp::SpinSwap {
                    c: a,
                    cond: cond(r),
                    dagger: false,
                }),
                11 if depth < 3 => {
                    depth += 1;
                    ops.push(SimOp::Push { bit });
                }
                12 if depth > 0 => {
                    depth -= 1;
                    ops.push(SimOp::Pop);
                }
                _ => ops.push(SimOp::Bit {
                    bit,
                    kind: r.below(3) as u8,
                    cond: cond(r),
                }),
            }
        }
        ops.extend((0..depth).map(|_| SimOp::Pop));
        ops
    }

    /// Soundness of [`qubit_outcomes`]: over every outcome vector of random programs and several
    /// lanes, a qubit whose value at a read differs between two outcome vectors is reported as a
    /// read of a marked qubit. So a program with no reads reported has none that depends on the
    /// outcomes. The converse is not asserted: a marked qubit may hold the same value for every
    /// outcome.
    #[test]
    fn every_outcome_dependent_qubit_read_is_reported() {
        let mut r = Rng(0x2545_f491_4f6c_dd1d);
        let (mut dependent, mut clean, mut clean_with_writes, mut spurious) = (0, 0, 0, 0u64);
        for _ in 0..6000 {
            let ops = random_qubit_program(&mut r);
            let found = marked_reads(&ops, &[], &[], usize::MAX);
            assert_eq!(found.reads as usize, found.first.len());
            let short = qubit_outcomes(&ops, &[], &[]);
            assert_eq!(
                (short.reads, short.conditioned_writes),
                (found.reads, found.conditioned_writes)
            );
            assert_eq!(
                short.first[..],
                found.first[..found.first.len().min(QubitOutcomes::KEPT)]
            );
            let reported: std::collections::HashSet<(u32, u32)> =
                found.first.iter().map(|x| (x.op, x.qubit)).collect();
            let n = ops
                .iter()
                .filter(|o| matches!(o, SimOp::Hmr { .. }))
                .count();
            let mut varies = std::collections::HashSet::new();
            for _ in 0..4 {
                let init = r.below(256) as u8;
                let first = reads_of(&ops, init, 0);
                for o in 1..1u64 << n {
                    let run = reads_of(&ops, init, o);
                    assert_eq!(run.len(), first.len());
                    for (x, y) in run.iter().zip(&first) {
                        assert_eq!((x.0, x.1), (y.0, y.1));
                        if x.2 != y.2 {
                            varies.insert((x.0, x.1));
                        }
                    }
                }
            }
            for v in &varies {
                assert!(
                    reported.contains(v),
                    "op {} reads q{}, which depends on the outcomes, unreported: {ops:?}",
                    v.0,
                    v.1
                );
            }
            dependent += usize::from(!varies.is_empty());
            clean += usize::from(found.reads == 0);
            clean_with_writes += usize::from(found.reads == 0 && found.conditioned_writes > 0);
            spurious += (reported.len() - varies.len()) as u64;
        }
        // The fuzz reaches programs with dependent reads, programs with none reported, and
        // reports that are over-approximations.
        assert!(
            dependent > 1000 && clean > 300 && spurious > 1000,
            "{dependent} {clean} {clean_with_writes} {spurious}"
        );
    }

    /// The rules the fuzz does not reach, and the shape of the laundering the pass exists for.
    #[test]
    fn marks_follow_the_stated_rules() {
        let hmr = |t, bit| SimOp::Hmr { t, bit, cond: NONE };
        let x_if = |t, cond| SimOp::X { t, cond };
        // Two outcomes copied into qubits, their AND in qubit 12, a phase controlled on it.
        let laundered = [
            hmr(20, 0),
            hmr(21, 1),
            x_if(10, 0),
            x_if(11, 1),
            SimOp::Ccx {
                a: 10,
                b: 11,
                t: 12,
                cond: NONE,
            },
            SimOp::Phase {
                q: [12, 1, NONE],
                k: 4,
                cond: NONE,
            },
        ];
        let got = qubit_outcomes(&laundered, &[], &[]);
        assert_eq!(got.conditioned_writes, 2);
        // The CCX reads both copies, the phase reads the AND, and three qubits stay marked.
        assert_eq!(got.reads, 6);
        let at = |i: usize| (got.first[i].op, got.first[i].qubit, got.first[i].source);
        assert_eq!([at(0), at(1), at(2)], [(4, 10, 2), (4, 11, 3), (5, 12, 2)]);
        assert_eq!(at(3), (u32::MAX, 10, 2));
        // The same ops with the conditions removed: nothing is marked.
        let plain: Vec<SimOp> = laundered
            .iter()
            .map(|o| match *o {
                SimOp::X { t, .. } => x_if(t, NONE),
                o => o,
            })
            .collect();
        let none = qubit_outcomes(&plain, &[], &[]);
        assert_eq!((none.conditioned_writes, none.reads), (0, 0));
        // A conditioned write that nothing reads is still read by the checks at the end.
        let left = qubit_outcomes(&[hmr(20, 0), x_if(10, 0)], &[], &[]);
        assert_eq!((left.reads, left.first[0].op), (1, u32::MAX));
        // A block marks as an own condition does; an unconditioned Hmr removes the mark (and is
        // itself a read); R does not.
        let block = |tail: SimOp| {
            let ops = [
                hmr(20, 0),
                SimOp::Push { bit: 0 },
                x_if(10, NONE),
                SimOp::Pop,
                tail,
            ];
            qubit_outcomes(&ops, &[], &[]).reads
        };
        assert_eq!(block(hmr(10, 1)), 1);
        assert_eq!(block(SimOp::Reset { t: 10 }), 2);
        // A marked angle bit is read by its Givens, and a marked inner qubit by the Reflect.
        let givens = SimOp::Givens {
            p: 0,
            q: 1,
            reg: 0,
            cond: NONE,
        };
        let ops = [hmr(20, 0), x_if(10, 0), givens, SimOp::Reflect];
        let got = qubit_outcomes(&ops, &[vec![9, 10]], &[10, 11]);
        assert_eq!((got.first[0].op, got.first[1].op), (2, 3));
        assert_eq!(got.reads, 3);
    }

    /// `n` outcomes in bits `0..n`, then bit `n` as their parity.
    fn parity(n: u32) -> Vec<SimOp> {
        let mut ops: Vec<SimOp> = (0..n)
            .map(|b| SimOp::Hmr {
                t: 0,
                bit: b,
                cond: NONE,
            })
            .collect();
        ops.extend((0..n).map(|b| SimOp::Bit {
            bit: n,
            kind: 0,
            cond: b,
        }));
        ops
    }

    /// An equation longer than `MAX_ROW` is `Opaque`, never truncated; one of exactly that
    /// length is still followed.
    #[test]
    fn long_parities_become_opaque() {
        for (n, want) in [
            (MAX_ROW as u32, Class::Affine),
            (MAX_ROW as u32 + 1, Class::Opaque),
        ] {
            let mut ops = parity(n);
            ops.push(SimOp::Neg { cond: n });
            let a = analyze(&ops);
            let last = a.sites.last().unwrap();
            assert_eq!(last.class, want, "{n} variables");
            assert_eq!(a.memory_capped, want == Class::Opaque);
            assert!(!a.capped);
        }
        // Two equations short enough on their own, one of which grows past the limit when the
        // other is inserted: `A` is the parity of outcome `p` and the 3,000 after it, `B` of
        // the 3,000 before `p` and `p`. In the block on `A` and `B`, reducing `A`'s row by
        // `B`'s leaves 6,000 variables.
        let half = 3000u32;
        let (p, n) = (half, 2 * half + 1);
        let hmr = |bit| SimOp::Hmr {
            t: 0,
            bit,
            cond: NONE,
        };
        let into = |bit, from: std::ops::RangeInclusive<u32>| {
            from.map(move |cond| SimOp::Bit { bit, kind: 0, cond })
        };
        let mut ops: Vec<SimOp> = (0..n).map(hmr).collect();
        ops.extend(into(n, p..=2 * half));
        ops.extend(into(n + 1, 0..=p));
        ops.extend([SimOp::Push { bit: n }, SimOp::Push { bit: n + 1 }]);
        ops.extend([SimOp::Pop, SimOp::Pop]);
        let a = analyze(&ops);
        let blocks: Vec<(Class, u8)> = a.sites[a.sites.len() - 2..]
            .iter()
            .map(|s| (s.class, s.rank))
            .collect();
        assert_eq!(blocks, [(Class::Affine, 1), (Class::Opaque, 0)]);
        assert!(a.memory_capped);
    }

    /// A forged stream cannot make the pass hold more than its memory budget: many bits, each
    /// with its own long equation, are followed until the budget is full and are `Opaque` from
    /// there on; a deep stack of large path conditions is charged in the same way.
    #[test]
    fn the_pass_holds_at_most_its_memory_budget() {
        let n = MAX_ROW as u32 - 1;
        let copies = 3 * (HELD_CAP / u64::from(n)) as u32 / 2;
        let mut ops = parity(n);
        for k in 0..copies {
            // b = parity, then b ^= 1: a row of its own, not shared with the parity bit.
            let bit = n + 1 + k;
            ops.push(SimOp::Bit {
                bit,
                kind: 0,
                cond: n,
            });
            ops.push(SimOp::Bit {
                bit,
                kind: 0,
                cond: NONE,
            });
        }
        let first_site = ops.iter().filter(|o| o.is_site()).count();
        ops.extend((0..copies).map(|k| SimOp::Neg { cond: n + 1 + k }));
        let a = analyze(&ops);
        assert!(a.memory_capped && !a.capped);
        assert!(
            a.peak_held <= HELD_CAP && a.peak_held > HELD_CAP / 2,
            "{}",
            a.peak_held
        );
        let class = |k: u32| a.sites[first_site + k as usize].class;
        assert_eq!(
            (class(0), class(copies - 1)),
            (Class::Affine, Class::Opaque)
        );
        assert!(a.sites.iter().all(|s| s.class != Class::Dead));

        // Two bits with long disjoint equations, pushed alternately: every push saves a path
        // condition of two long rows.
        let mut ops = parity(n);
        let shift = n + 1;
        ops.extend(parity(n).into_iter().map(|o| match o {
            SimOp::Hmr { t, bit, cond } => SimOp::Hmr {
                t,
                bit: bit + shift,
                cond,
            },
            SimOp::Bit { bit, kind, cond } => SimOp::Bit {
                bit: bit + shift,
                kind,
                cond: cond + shift,
            },
            o => o,
        }));
        let depth = 3 * (HELD_CAP / (2 * u64::from(n))) as usize / 2;
        let first_site = ops.iter().filter(|o| o.is_site()).count();
        for d in 0..depth {
            ops.push(SimOp::Push {
                bit: if d % 2 == 0 { n } else { n + shift },
            });
        }
        ops.extend((0..depth).map(|_| SimOp::Pop));
        ops.push(SimOp::Neg { cond: n });
        let a = analyze(&ops);
        assert!(a.memory_capped && !a.capped);
        assert!(a.peak_held <= HELD_CAP, "{}", a.peak_held);
        // Every push is followed (the condition in force is never dropped, only saved copies),
        // and the site after the last pop is followed again.
        let ranks: Vec<(Class, u8)> = a.sites[first_site..]
            .iter()
            .map(|s| (s.class, s.rank))
            .collect();
        assert_eq!(ranks[0], (Class::Affine, 1));
        assert!(ranks[1..depth].iter().all(|&x| x == (Class::Affine, 2)));
        assert_eq!(ranks[depth], (Class::Affine, 1));
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
