//! The reflection check: a static certificate that a nested walk step is Hermitian on every
//! lane and for every measurement-outcome record (spec/SAMPLING.md section 4, spec/DESIGN.md
//! section 16.6). Opt-in (`eval_circuit --reflection-check`); it never reads a sampled lane.
//!
//! **Method ("symbolic lanes").** The op stream is executed once under the lane semantics of
//! `super::lanes`, with every non-system wire and every classical bit a Boolean function, held
//! as a reduced ordered binary decision diagram (`bdd`), of free variables: one per initial
//! control and uniform bit, one per inner uniform bit after the `Reflect`, one per `Hmr`
//! outcome. Each rule of `Engine::exec` is an exact identity of those semantics, so at every op
//! a wire's diagram is the wire's value on every lane and outcome record. A diagram is a
//! canonical form: two functions are equal exactly when their diagrams are the same node. So a
//! wire is 0 on every lane exactly when it is the node `FALSE`, and a value that depends on an
//! outcome only in form (a fix-up that cancels) does not depend on it in the diagram at all.
//!
//! **What a certificate establishes**, in this order (`Report`):
//! 1. *Registers restored.* Every `R` frees a qubit that is 0; at the `Reflect` the inner
//!    register holds its initial value; at the end the control and the outer uniform bits hold
//!    their initial values, the inner register its second-pass value, and every other
//!    non-system wire is 0.
//! 2. *Outcome independent.* The lane's sign is `(-1)^phi`; `phi`, and every control, angle and
//!    sign of the lane's system ops in normal form, is a function that no outcome can change.
//!    So the lane operator `L(g, a, b)` on the system register is a function of the lane alone
//!    (`g`: control and outer bits, `a`, `b`: the inner value before and after the `Reflect`).
//! 3. *Hermitian.* `L(g, a, b)^dagger = L(g, b, a)` as an identity in the variables. With 1
//!    this is Hermiticity of the step `C = sum_g sum_{a,b} (2 / 2^w - [a = b]) |g, b><g, a| (x)
//!    L(g, a, b)` (spec/DESIGN.md section 16.2) on the space where every ancilla is 0.
//!
//! The comparison of 3 writes the lane's system ops in a normal form: `Z`s are moved to the end
//! (a symbolic `Z` frame), leaving the `X`s, `Givens` and `SpinSwap`s in order. It uses three
//! operator identities and the `X`/`Z` anticommutation sign: `Z_p G_pq(t) Z_p = G_pq(-t)` (and
//! the same for `Z_q`); `Z_k` commutes with `G_pq` for `k` outside `{p, q}`; `F Z_2p F^dagger =
//! Z_{2p+1}` for the `SpinSwap` layer `F`. tests/reflection.rs checks them against dense
//! matrices. The adjoint's normal form must then equal, element by element, the normal form
//! with the first- and second-pass inner variables exchanged.
//!
//! **Fail closed.** The result is `certified`, or `not certified` with the first obstruction.
//! Anything outside the rules is an obstruction: `S`/`Sdg`, a rotation under a classical
//! condition, an outcome that reaches a system op, a wire that is not restored, a normal-form
//! mismatch, an exhausted work budget or node store, a lane map that is not nested. Stages 1
//! and 2 are decided exactly (an obstruction there names a lane and outcome record). Stage 3
//! is incomplete on purpose: for example two system ops that commute are not reordered, and an
//! angle register that holds `2^beta - a` is not recognised as `-a`. "Not certified" is
//! therefore not a finding that the step is not Hermitian.
//!
//! **Not established.** That the step encodes the spec's operator: a step can be Hermitian and
//! wrong, and that stays a sampled statement. The certificate also says nothing about executed
//! gate counts.
pub mod bdd;

use super::compile::{Layout, MAX_BITS, MAX_QUBITS};
use super::nested::Inner;
use crate::circuit::{Op, OperationType as K, NONE};
use bdd::{Bdd, Node, Op as B, FALSE, TRUE};
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

/// The rule set's name and version. The rules of `Engine::exec`, the normal form and the
/// variable order belong to the version: change any of them and this string must change.
pub const VERSION: &str = "reflection-v3";
const NO_LEVEL: u32 = u32::MAX;
/// Nodes visited per unit of work when the store is collected.
const COLLECT_PER_TICK: usize = 16;
/// The store is not collected below this many nodes.
const COLLECT_FLOOR: usize = 1 << 16;

/// A product of measurement outcomes: their levels, ascending. Empty is the constant 1.
type Mono = Rc<[u32]>;
/// A function as `sum_mu mu * f_mu` over products `mu` of outcomes, each `f_mu` a function of
/// the lane bits alone and not 0. The form is unique (the algebraic normal form in the
/// outcomes), so the function is independent of every outcome exactly when no product other
/// than the empty one is present.
type Parts = Vec<(Mono, Node)>;
/// New coefficients of the lane's sign, to be committed.
type Signs = Vec<(Mono, Sum)>;

/// Deterministic limits (counts of decision-diagram steps and nodes; never wall clock).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Total work of the run: decision-diagram steps, plus one per op.
    pub work: u64,
    /// Most decision-diagram nodes held at once.
    pub bdd_nodes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            work: 1 << 38,
            bdd_nodes: 1 << 28,
        }
    }
}

/// Why the check did not certify: the first rule the op stream does not meet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obstruction {
    /// The op it is attributed to; `None` for a condition on the whole stream.
    pub op: Option<usize>,
    /// That op's kind name.
    pub name: Option<&'static str>,
    pub kind: &'static str,
    pub reason: String,
}

impl std::fmt::Display for Obstruction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.op, self.name) {
            (Some(i), Some(n)) => write!(f, "op {i} ({n}): ")?,
            (Some(i), None) => write!(f, "op {i}: ")?,
            _ => {}
        }
        write!(f, "{} [{}]", self.reason, self.kind)
    }
}

/// What the run saw and proved before it stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Ops executed symbolically (all of them when certified).
    pub ops: u64,
    pub measurements: u64,
    /// `R` ops whose qubit was proven 0.
    pub resets_proved: u64,
    /// Measurement outcomes the lane operator and the lane's sign were proven independent of:
    /// every one, once stage 2 is established.
    pub outcomes_proved: u64,
    /// System ops in the lane operator: `X`/`Z` (controlled or not), `Givens`, `SpinSwap`.
    pub system_ops: u64,
    pub givens: u64,
    pub spin_swaps: u64,
    /// Elements of the normal form compared with the adjoint's.
    pub compared: u64,
    /// Most decision-diagram nodes held at once.
    pub nodes_peak: u64,
    /// Times the node store was collected.
    pub collections: u64,
}

/// The result of [`certify`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub certified: bool,
    pub obstruction: Option<Obstruction>,
    /// Stage 1 of the module doc. `false` means not established.
    pub registers_restored: bool,
    /// Stage 2.
    pub outcome_independent: bool,
    /// Stage 3.
    pub hermitian: bool,
    pub counts: Counts,
    /// Boolean variables: lane bits and measurement outcomes.
    pub symbols: u64,
    pub work: u64,
    pub budget: Budget,
}

impl Report {
    /// A refusal before any op was read.
    #[must_use]
    pub fn refused(budget: Budget, kind: &'static str, reason: &str) -> Self {
        Self {
            certified: false,
            obstruction: Some(Obstruction {
                op: None,
                name: None,
                kind,
                reason: reason.to_string(),
            }),
            registers_restored: false,
            outcome_independent: false,
            hermitian: false,
            counts: Counts::default(),
            symbols: 0,
            work: 0,
            budget,
        }
    }

    /// `certified`, or `not certified: <first obstruction>`.
    #[must_use]
    pub fn summary(&self) -> String {
        match &self.obstruction {
            None if self.certified => "certified".to_string(),
            None => "not certified".to_string(),
            Some(o) => format!("not certified: {o}"),
        }
    }

    /// `metrics.reflection` of `score.json`.
    #[must_use]
    pub fn json(&self) -> serde_json::Value {
        let c = &self.counts;
        json!({
            "version": VERSION,
            "rule_set": "symbolic lanes: every non-system wire as a reduced ordered binary decision diagram over the lane's bits and the measurement outcomes; the system ops in a Z-frame normal form compared with their adjoint under exchange of the first- and second-pass inner values",
            "document": "spec/SAMPLING.md section 4, spec/DESIGN.md section 16.6",
            "status": if self.certified { "certified" } else { "not certified" },
            "certified": self.certified,
            "obstruction": self.obstruction.as_ref().map(|o| json!({
                "op": o.op,
                "op_kind": o.name,
                "kind": o.kind,
                "reason": o.reason,
            })),
            "registers_restored": self.registers_restored,
            "outcome_independent": self.outcome_independent,
            "hermitian": self.hermitian,
            "counts": {
                "ops": c.ops,
                "measurements": c.measurements,
                "resets_proved_zero": c.resets_proved,
                "outcomes_proved_independent": c.outcomes_proved,
                "system_ops": c.system_ops,
                "givens": c.givens,
                "spin_swaps": c.spin_swaps,
                "normal_form_elements_compared": c.compared,
            },
            "diagrams": {
                "variables": self.symbols,
                "nodes_peak": c.nodes_peak,
                "node_limit": self.budget.bdd_nodes,
                "collections": c.collections,
            },
            "work": self.work,
            "work_budget": self.budget.work,
            "statement": if self.certified {
                "Proved for every lane and every measurement-outcome record, without sampling: the step restores its registers, its system operator does not depend on the outcomes, and the step is Hermitian on the space where every ancilla is 0. Does not establish that the step encodes the spec's operator; that is the sampled check."
            } else {
                "Not proven. The check stopped at the first obstruction; false entries above mean not established, and this is not a finding that the step is not Hermitian."
            },
        })
    }
}

/// One system op of the lane operator, with symbolic controls.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Sys {
    /// `Z` on system qubit `q` where `c` is 1.
    Z { q: usize, c: Node },
    /// `X` on system qubit `q` where `c` is 1.
    X { q: usize, c: Node },
    /// `G_{p,q}((-1)^sign 2 pi angle / 2^beta)`, the angle register bit by bit.
    G {
        p: usize,
        q: usize,
        angle: Vec<Node>,
        sign: Node,
    },
    /// The `SpinSwap` layer `F` (`F^dagger` when `dagger`) where `c` is 1.
    F { dagger: bool, c: Node },
}

impl Sys {
    fn nodes_mut(&mut self) -> Vec<&mut Node> {
        match self {
            Sys::Z { c, .. } | Sys::X { c, .. } | Sys::F { c, .. } => vec![c],
            Sys::G { angle, sign, .. } => angle.iter_mut().chain(std::iter::once(sign)).collect(),
        }
    }

    fn nodes(&self) -> Vec<Node> {
        match self {
            Sys::Z { c, .. } | Sys::X { c, .. } | Sys::F { c, .. } => vec![*c],
            Sys::G { angle, sign, .. } => angle.iter().copied().chain([*sign]).collect(),
        }
    }

    fn what(&self) -> &'static str {
        match self {
            Sys::Z { .. } => "a system Z's control",
            Sys::X { .. } => "a system X's control",
            Sys::G { .. } => "a Givens angle or its sign",
            Sys::F { .. } => "a SpinSwap's control",
        }
    }
}

/// A sum of functions, added up lazily. Slot `k` holds the sum of about `2^k` of the terms, or
/// `FALSE` for none, and two sums of the same size are added when they meet (a binary counter).
/// A register that a table lookup fills one entry at a time is then built in work
/// proportional to its final diagram times the logarithm of the table size, where adding each
/// entry to the whole would cost the product of the two.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sum(Vec<Node>);

impl Sum {
    fn of(f: Node) -> Self {
        Self(vec![f])
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|&n| n == FALSE)
    }
}

/// A list of system ops as `(-1)^sign * Z^frame * (last of list) ... (first of list)`.
struct Normal {
    list: Vec<(usize, Sys)>,
    frame: Vec<Node>,
    sign: Node,
}

impl Normal {
    fn new(system: usize) -> Self {
        Self {
            list: Vec::new(),
            frame: vec![FALSE; system],
            sign: FALSE,
        }
    }
}

/// A difference between two normal forms: its kind, what differs, the two functions and the
/// op it is attributed to.
type Mismatch = (&'static str, String, (Node, Node), Option<usize>);

/// Where the next op of a normal form comes from.
#[derive(Clone, Copy)]
enum Source {
    /// The op at this position of the pass's log.
    Log(usize),
    /// The op at this position of the adjoint of `forms[0]`.
    Adjoint(usize),
}

struct Engine<'a> {
    b: Bdd,
    work: u64,
    limit: u64,
    /// The work limit is passed: nothing computed after that is used.
    spent: bool,
    l: Layout,
    first: u32,
    inner: Vec<u32>,
    registers: &'a [Vec<u32>],
    /// Per wire below `first`: the level of its `Initial` variable (`NO_LEVEL` for a system
    /// qubit).
    rank: Vec<u32>,
    /// Per inner wire, in `inner` order: the level of its `Second` variable.
    second: Vec<u32>,
    /// The level of the first measurement outcome. Outcomes take the levels from here on, in
    /// the order of their `Hmr`s.
    outcome_base: u32,
    /// Per outcome: the op index of its `Hmr`.
    outcomes: Vec<usize>,
    /// Per non-system wire: its value.
    val: Vec<Sum>,
    /// Per classical bit: its value, a function of the outcomes.
    cl: Vec<Node>,
    /// The condition stack: per open block the product of its bit (as it was at the push) and
    /// every enclosing block's.
    stack: Vec<Node>,
    /// The lane's sign is `(-1)^Phi`, `Phi = sum_mu mu * phi[mu]` over products `mu` of
    /// outcomes; `phi[1]` is the part with no outcome. Held apart because a measurement's
    /// fix-up may come long after it, and the sum of thousands of pending `outcome * value`
    /// terms has no small diagram.
    phi: BTreeMap<Mono, Sum>,
    /// Per node that depends on outcomes alone: its algebraic normal form. Emptied when the
    /// store is collected (ids change).
    anf: HashMap<Node, Rc<Vec<Mono>>>,
    /// The system ops of the pass that is running, in stream order.
    log: Vec<(usize, Sys)>,
    /// Normal forms: of `L(g, a, b)`, of `L(g, b, a)`, of the adjoint of the first.
    forms: Vec<Normal>,
    /// The pass that is running starts the inner register on its second-pass variables and
    /// gives it the first-pass ones at the `Reflect`.
    swapped: bool,
    reflected: bool,
    collect_at: usize,
    counts: Counts,
    registers_restored: bool,
    outcome_independent: bool,
    hermitian: bool,
}

/// Test hook (tests/reflection.rs): the normal form and the adjoint on system ops whose
/// controls are constants, so that both can be compared with dense matrices. Not used by the
/// check itself; it runs the same `Engine::normal` and `Engine::adjoint`.
#[doc(hidden)]
pub mod probe {
    use super::{Budget, Engine, Inner, Layout, Source, Step, Sys, FALSE, TRUE};

    /// A system op that executes: `G`'s angle is `(-1)^minus * angle`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Gate {
        X(usize),
        Z(usize),
        G {
            p: usize,
            q: usize,
            angle: u64,
            minus: bool,
        },
        F {
            dagger: bool,
        },
        /// A `SpinSwap` whose control is 0: the identity, which stays in the list.
        Idle {
            dagger: bool,
        },
    }

    /// An operator as `(-1)^sign * Z^frame * (last of list) ... (first of list)`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Normal {
        pub list: Vec<Gate>,
        pub frame: Vec<bool>,
        pub sign: bool,
    }

    const BITS: usize = 16;

    fn constant(b: bool) -> super::Node {
        if b {
            TRUE
        } else {
            FALSE
        }
    }

    fn lift(g: &Gate) -> Sys {
        match *g {
            Gate::X(q) => Sys::X { q, c: TRUE },
            Gate::Z(q) => Sys::Z { q, c: TRUE },
            Gate::G { p, q, angle, minus } => Sys::G {
                p,
                q,
                angle: (0..BITS).map(|k| constant(angle >> k & 1 == 1)).collect(),
                sign: constant(minus),
            },
            Gate::F { dagger } => Sys::F { dagger, c: TRUE },
            Gate::Idle { dagger } => Sys::F { dagger, c: FALSE },
        }
    }

    fn lower(s: &Sys) -> Gate {
        match s {
            Sys::X { q, .. } => Gate::X(*q),
            Sys::Z { q, .. } => Gate::Z(*q),
            Sys::G { p, q, angle, sign } => Gate::G {
                p: *p,
                q: *q,
                angle: (0..angle.len()).fold(0, |a, k| a | u64::from(angle[k] == TRUE) << k),
                minus: *sign == TRUE,
            },
            Sys::F { dagger, c } if *c == TRUE => Gate::F { dagger: *dagger },
            Sys::F { dagger, .. } => Gate::Idle { dagger: *dagger },
        }
    }

    /// The normal form of the time-ordered `seq` on `n` system qubits, and the normal form of
    /// its adjoint as the check builds it.
    #[must_use]
    pub fn normal(n: usize, seq: &[Gate]) -> (Normal, Normal) {
        let l = Layout {
            system: n,
            uniform: 0,
        };
        let mut e = Engine::new(&[], &l, Inner { lo: 0, width: 0 }, &[], Budget::default());
        e.log = seq.iter().map(|g| (0, lift(g))).collect();
        e.forms.push(super::Normal::new(n));
        for j in 0..e.log.len() {
            assert!(matches!(e.fold(0, Source::Log(j)), Ok(Step::Done)));
        }
        e.forms.push(super::Normal::new(n));
        e.forms.push(super::Normal::new(n));
        for j in 0..n + e.forms[0].list.len() {
            assert!(matches!(e.fold(2, Source::Adjoint(j)), Ok(Step::Done)));
        }
        let pack = |f: &super::Normal, sign: bool| Normal {
            list: f.list.iter().map(|(_, s)| lower(s)).collect(),
            frame: f.frame.iter().map(|&x| x == TRUE).collect(),
            sign,
        };
        let (nf, back) = (&e.forms[0], &e.forms[2]);
        (
            pack(nf, nf.sign == TRUE),
            pack(back, (nf.sign == TRUE) != (back.sign == TRUE)),
        )
    }
}

fn obstruct(at: Option<(usize, &Op)>, kind: &'static str, reason: String) -> Obstruction {
    Obstruction {
        op: at.map(|(i, _)| i),
        name: at.map(|(_, op)| op.kind.name()),
        kind,
        reason,
    }
}

/// Runs the reflection check on `ops`.
///
/// `inner` is the nested lane map's inner uniform register; `None` (a lane map that is not
/// nested) is refused, as is every lane map whose `uniform_after` is not the identity, which
/// the caller must map to `None`. `registers` are the compiled registers (`Compiled::registers`:
/// each register's final contents, which is what the simulator's `Givens` reads). The caller
/// should have compiled `ops` first (`super::compile`), which includes the structural check of
/// `super::nested`; this function nevertheless refuses whatever it does not recognise.
#[must_use]
pub fn certify(
    ops: &[Op],
    l: &Layout,
    inner: Option<Inner>,
    registers: &[Vec<u32>],
    budget: Budget,
) -> Report {
    let Some(inner) = inner else {
        return Report::refused(
            budget,
            "unsupported-lane-map",
            "the rule set covers nested lane maps whose control-1 lanes keep their uniform value",
        );
    };
    if budget.bdd_nodes > 1 << 28 || l.first_ancilla() >= MAX_QUBITS {
        return Report::refused(budget, "budget", "limits outside the supported range");
    }
    if inner
        .lo
        .checked_add(inner.width)
        .is_none_or(|end| end > l.uniform)
    {
        return Report::refused(
            budget,
            "structure",
            "the inner register is not inside the uniform register",
        );
    }
    let mut e = Engine::new(ops, l, inner, registers, budget);
    let obstruction = e.run(ops).err();
    Report {
        certified: obstruction.is_none() && e.hermitian,
        obstruction,
        registers_restored: e.registers_restored,
        outcome_independent: e.outcome_independent,
        hermitian: e.hermitian,
        counts: e.counts,
        symbols: u64::from(e.outcome_base) + e.outcomes.len() as u64,
        work: e.work,
        budget,
    }
}

/// What an attempt at an op or at the end of the stream came to.
enum Step {
    Done,
    /// The node store or the work limit was passed before anything was committed.
    Full,
}

impl<'a> Engine<'a> {
    /// The variable order (it affects cost only: any order gives a canonical form). Lane bits
    /// first, in the order the op stream first names their wires, then the second-pass inner
    /// bits in the same relative order, then the outcomes in stream order. A fix-up reads its
    /// outcomes after the lane bits that select it, which keeps its diagram narrow.
    fn new(
        ops: &[Op],
        l: &Layout,
        inner: Inner,
        registers: &'a [Vec<u32>],
        budget: Budget,
    ) -> Self {
        let first = l.first_ancilla();
        let mut rank = vec![NO_LEVEL; first as usize];
        let mut next = 0u32;
        let lane = |q: u32| q < first && l.sys(q).is_none();
        let used = ops.iter().flat_map(Op::qubits).chain(0..first);
        for q in used {
            if lane(q) && rank[q as usize] == NO_LEVEL {
                rank[q as usize] = next;
                next += 1;
            }
        }
        let inner = inner.qubits(l);
        let mut by_rank: Vec<u32> = inner.clone();
        by_rank.sort_unstable_by_key(|&q| rank[q as usize]);
        let second = inner
            .iter()
            .map(|q| next + by_rank.iter().position(|x| x == q).unwrap_or(0) as u32)
            .collect();
        Self {
            b: Bdd::new(budget.bdd_nodes),
            work: 0,
            limit: budget.work,
            spent: false,
            l: *l,
            first,
            outcome_base: next + inner.len() as u32,
            inner,
            registers,
            rank,
            second,
            outcomes: Vec::new(),
            val: Vec::new(),
            cl: Vec::new(),
            stack: Vec::new(),
            phi: BTreeMap::new(),
            anf: HashMap::new(),
            log: Vec::new(),
            forms: Vec::new(),
            swapped: false,
            reflected: false,
            collect_at: COLLECT_FLOOR,
            counts: Counts::default(),
            registers_restored: false,
            outcome_independent: false,
            hermitian: false,
        }
    }

    fn ap(&mut self, op: B, f: Node, g: Node) -> Node {
        let (work, limit, spent) = (&mut self.work, self.limit, &mut self.spent);
        self.b.apply(op, f, g, &mut || {
            *work += 1;
            *spent |= *work > limit;
            !*spent
        })
    }

    fn and(&mut self, f: Node, g: Node) -> Node {
        self.ap(B::And, f, g)
    }

    fn xor(&mut self, f: Node, g: Node) -> Node {
        self.ap(B::Xor, f, g)
    }

    /// `s + f`.
    fn plus(&mut self, s: &Sum, mut f: Node) -> Sum {
        let mut slots = s.0.clone();
        let mut k = 0;
        while f != FALSE {
            if k == slots.len() {
                slots.push(FALSE);
            }
            if slots[k] == FALSE {
                slots[k] = f;
                break;
            }
            f = self.xor(f, slots[k]);
            slots[k] = FALSE;
            k += 1;
        }
        Sum(slots)
    }

    /// The function `s` adds up to.
    fn total(&mut self, s: &Sum) -> Node {
        s.0.iter().fold(FALSE, |t, &n| self.xor(t, n))
    }

    /// The value of the wire `q`. The wire's sum is replaced by its total, which is the same
    /// function, when that is sound.
    fn rd(&mut self, q: u32) -> Node {
        let s = &self.val[q as usize];
        let mut terms = s.0.iter().filter(|&&n| n != FALSE);
        match (terms.next(), terms.next()) {
            (None, _) => FALSE,
            (Some(&n), None) => n,
            _ => {
                let s = s.clone();
                let t = self.total(&s);
                if self.sound() {
                    let mut slots = vec![FALSE; s.0.len()];
                    slots[s.0.len() - 1] = t;
                    self.val[q as usize] = Sum(slots);
                }
                t
            }
        }
    }

    /// Whether everything computed since the last collection can be used.
    fn sound(&self) -> bool {
        !self.b.overflowed && !self.spent
    }

    /// Drops every node that no wire, bit, condition, sign or logged op names.
    fn collect(&mut self) {
        self.counts.nodes_peak = self.counts.nodes_peak.max(self.b.len() as u64);
        self.work = self
            .work
            .saturating_add((self.b.len() / COLLECT_PER_TICK) as u64);
        self.spent |= self.work > self.limit;
        let roots = self
            .val
            .iter_mut()
            .flat_map(|s| s.0.iter_mut())
            .chain(self.cl.iter_mut())
            .chain(self.stack.iter_mut())
            .chain(self.phi.values_mut().flat_map(|s| s.0.iter_mut()))
            .chain(self.log.iter_mut().flat_map(|(_, s)| s.nodes_mut()))
            .chain(self.forms.iter_mut().flat_map(|f| {
                let list = f.list.iter_mut().flat_map(|(_, s)| s.nodes_mut());
                list.chain(f.frame.iter_mut())
                    .chain(std::iter::once(&mut f.sign))
            }));
        self.b.collect(roots);
        self.anf.clear();
        self.b.overflowed = false;
        self.counts.collections += 1;
        // Next when the store has doubled, or sooner if that is more than half the room left.
        let (live, limit) = (self.b.len(), self.b.limit());
        self.collect_at = (2 * live)
            .min(live + limit.saturating_sub(live) / 2)
            .max(COLLECT_FLOOR);
    }

    fn budget(&self, at: Option<(usize, &Op)>) -> Obstruction {
        let why = if self.spent {
            "the work budget is exhausted".to_string()
        } else {
            format!(
                "the decision diagrams do not fit in {} nodes",
                self.b.limit()
            )
        };
        obstruct(at, "budget", format!("{why}; nothing is certified"))
    }

    /// Runs `attempt` (which commits nothing unless it returns `Done`); if the store fills,
    /// collects it and runs it once more.
    fn retry(
        &mut self,
        at: Option<(usize, &Op)>,
        mut attempt: impl FnMut(&mut Self) -> Result<Step, Obstruction>,
    ) -> Result<(), Obstruction> {
        for _ in 0..2 {
            if self.spent {
                break;
            }
            if let Step::Done = attempt(self)? {
                return Ok(());
            }
            self.collect();
        }
        Err(self.budget(at))
    }

    /// The lane operator `L(g, a, b)`, then `L(g, b, a)` (the same stream with the inner
    /// register's two values exchanged), then the comparison of the first's adjoint with the
    /// second.
    fn run(&mut self, ops: &[Op]) -> Result<(), Obstruction> {
        self.pass(ops, false)?;
        let first = self.counts;
        self.pass(ops, true)?;
        self.counts = Counts {
            nodes_peak: self.counts.nodes_peak,
            collections: self.counts.collections,
            ..first
        };
        // The comparison is stated in the first pass's variables.
        self.swapped = false;
        self.compare()?;
        self.counts.nodes_peak = self.counts.nodes_peak.max(self.b.len() as u64);
        Ok(())
    }

    /// Collects the store if it has grown enough since the last collection.
    fn room(&mut self) {
        if self.b.len() > self.collect_at {
            self.collect();
        }
    }

    /// One symbolic run of the stream, stages 1 and 2 on it, and its normal form, which is
    /// left as the newest of `forms`.
    fn pass(&mut self, ops: &[Op], swapped: bool) -> Result<(), Obstruction> {
        self.swapped = swapped;
        self.val.clear();
        self.cl.clear();
        self.stack.clear();
        self.phi.clear();
        self.outcomes.clear();
        self.log.clear();
        self.reflected = false;
        self.retry(None, |e| {
            Ok(if e.start() { Step::Done } else { Step::Full })
        })?;
        // Progress on stderr, for a run that takes minutes. It reads nothing back.
        let progress = std::env::var_os("FEMOCO_REFLECTION_PROGRESS").is_some();
        for (i, op) in ops.iter().enumerate() {
            if progress && i % 100_000 == 0 {
                eprintln!(
                    "  reflection  : pass {} of 2, op {i} of {}: {} nodes, work {}",
                    1 + u8::from(swapped),
                    ops.len(),
                    self.b.len(),
                    self.work
                );
            }
            self.admit(i, op)?;
            self.work += 1;
            self.spent |= self.work > self.limit;
            self.room();
            self.retry(Some((i, op)), |e| e.exec(i, op))?;
            self.counts.ops += 1;
        }
        self.collect();
        let no = |why: &str| Err(obstruct(None, "structure", why.to_string()));
        if !self.stack.is_empty() {
            return no("a condition block is left open");
        }
        if !self.reflected {
            return no("the stream has no Reflect");
        }
        // Stage 1: every register restored and every ancilla 0.
        for q in 0..self.val.len() as u32 {
            self.retry(None, |e| e.restored(q))?;
        }
        self.registers_restored = true;
        // Stage 2: nothing the lane operator is made of depends on an outcome.
        self.counts.system_ops = self.log.len() as u64;
        let k = self.forms.len();
        self.forms.push(Normal::new(self.l.system));
        for j in 0..self.log.len() {
            self.room();
            self.retry(None, |e| e.fold(k, Source::Log(j)))?;
        }
        self.independent(k, false)?;
        self.retry(None, |e| e.fold_sign(k))?;
        self.independent(k, true)?;
        self.outcome_independent = true;
        self.counts.outcomes_proved = self.outcomes.len() as u64;
        self.val.clear();
        self.cl.clear();
        self.log.clear();
        self.collect();
        Ok(())
    }

    /// What a variable stands for, for messages.
    fn describe(&self, level: u32, value: bool, out: &mut [Vec<String>; 3]) {
        let v = u8::from(value);
        if level >= self.outcome_base {
            let op = self.outcomes[(level - self.outcome_base) as usize];
            out[2].push(format!("op {op}={v}"));
        } else if let Some(k) = self.second.iter().position(|&x| x == level) {
            // In the exchanged pass these are the values the inner wires start with.
            out[usize::from(!self.swapped)].push(format!("q{}={v}", self.inner[k]));
        } else if let Some(q) = self.rank.iter().position(|&x| x == level) {
            let inner = self.inner.contains(&(q as u32));
            out[usize::from(inner && self.swapped)].push(format!("q{q}={v}"));
        }
    }

    /// For a function that is not 0: one assignment on which it is 1, enough to replay the lane
    /// on the lane engine, and the fraction of all assignments on which it is 1.
    fn counterexample(&self, f: Node) -> String {
        let mut parts: [Vec<String>; 3] = Default::default();
        for (level, value) in self.b.witness(f) {
            self.describe(level, value, &mut parts);
        }
        format!(
            "it is 1 for some lane or outcome record: on a fraction {:e} of the assignments of \
             its bits; for example wires at the start [{}], inner wires after the Reflect [{}], \
             HMR outcomes [{}], every other bit free",
            self.b.density(f),
            parts[0].join(" "),
            parts[1].join(" "),
            parts[2].join(" ")
        )
    }

    /// The algebraic normal form of `f`, which depends on outcomes alone: the products of
    /// outcomes whose sum it is, sorted.
    fn normal_form(&mut self, f: Node) -> Rc<Vec<Mono>> {
        let mut todo = vec![f];
        while let Some(&x) = todo.last() {
            if self.anf.contains_key(&x) {
                todo.pop();
                continue;
            }
            let Some((level, lo, hi)) = self.b.node(x) else {
                let form = if x == TRUE {
                    vec![Mono::from(Vec::new())]
                } else {
                    Vec::new()
                };
                self.anf.insert(x, Rc::new(form));
                todo.pop();
                continue;
            };
            let (Some(a), Some(b)) = (self.anf.get(&lo).cloned(), self.anf.get(&hi).cloned())
            else {
                todo.extend([lo, hi]);
                continue;
            };
            // lo + m (lo + hi): `m` is above every outcome of `lo` and `hi`.
            self.work = self.work.saturating_add((a.len() + b.len()) as u64);
            let mut form: Vec<Mono> = a.iter().cloned().collect();
            let mut i = 0;
            let mut j = 0;
            let with = |s: &Mono| {
                let mut v = Vec::with_capacity(s.len() + 1);
                v.push(level);
                v.extend_from_slice(s);
                Mono::from(v)
            };
            while i < a.len() || j < b.len() {
                match (a.get(i), b.get(j)) {
                    (Some(x), Some(y)) if x == y => {
                        i += 1;
                        j += 1;
                    }
                    (Some(x), Some(y)) if x < y => {
                        form.push(with(x));
                        i += 1;
                    }
                    (Some(x), None) => {
                        form.push(with(x));
                        i += 1;
                    }
                    (_, Some(y)) => {
                        form.push(with(y));
                        j += 1;
                    }
                    (None, None) => {}
                }
            }
            form.sort_unstable();
            self.anf.insert(x, Rc::new(form));
            todo.pop();
        }
        self.spent |= self.work > self.limit;
        self.anf.get(&f).cloned().unwrap_or_default()
    }

    /// `f` as [`Parts`]. The outcomes are below every lane bit in the variable order, so the
    /// diagram of `f` is a diagram over lane bits whose leaves are functions of outcomes
    /// alone; each leaf is put in normal form and the lane bits are put back on top.
    fn phase(&mut self, f: Node) -> Signs {
        let mut memo: HashMap<Node, Rc<Parts>> = HashMap::new();
        let mut todo = vec![f];
        while let Some(&x) = todo.last() {
            if memo.contains_key(&x) {
                todo.pop();
                continue;
            }
            let lane = self
                .b
                .node(x)
                .filter(|&(level, _, _)| level < self.outcome_base);
            let Some((level, lo, hi)) = lane else {
                let form = self.normal_form(x);
                memo.insert(x, Rc::new(form.iter().map(|m| (m.clone(), TRUE)).collect()));
                todo.pop();
                continue;
            };
            let (Some(a), Some(b)) = (memo.get(&lo).cloned(), memo.get(&hi).cloned()) else {
                todo.extend([lo, hi]);
                continue;
            };
            self.work = self.work.saturating_add((a.len() + b.len()) as u64);
            let mut parts = Parts::with_capacity(a.len().max(b.len()));
            let (mut i, mut j) = (0, 0);
            while i < a.len() || j < b.len() {
                let (key, l, h) = match (a.get(i), b.get(j)) {
                    (Some(x), Some(y)) if x.0 == y.0 => {
                        i += 1;
                        j += 1;
                        (&x.0, x.1, y.1)
                    }
                    (Some(x), Some(y)) if x.0 < y.0 => {
                        i += 1;
                        (&x.0, x.1, FALSE)
                    }
                    (Some(x), None) => {
                        i += 1;
                        (&x.0, x.1, FALSE)
                    }
                    (_, Some(y)) => {
                        j += 1;
                        (&y.0, FALSE, y.1)
                    }
                    (None, None) => break,
                };
                let n = self.b.pick(level, l, h);
                if n != FALSE {
                    parts.push((key.clone(), n));
                }
            }
            memo.insert(x, Rc::new(parts));
            todo.pop();
        }
        self.spent |= self.work > self.limit;
        let parts: Parts = memo.get(&f).map(|p| (**p).clone()).unwrap_or_default();
        // The new coefficients of the sign.
        parts
            .into_iter()
            .map(|(m, n)| {
                let old = self.phi.remove(&m).unwrap_or_default();
                let new = self.plus(&old, n);
                if !old.is_zero() {
                    self.phi.insert(m.clone(), old);
                }
                (m, new)
            })
            .collect()
    }

    /// Commits the coefficients [`Engine::phase`] computed.
    fn sign(&mut self, parts: Signs) {
        for (m, n) in parts {
            if n.is_zero() {
                self.phi.remove(&m);
            } else {
                self.phi.insert(m, n);
            }
        }
    }

    /// The condition an op with `c_condition = cc` executes under: the enclosing blocks'
    /// product, times the bit `cc` if given.
    fn cond(&mut self, cc: u32) -> Node {
        let base = self.stack.last().copied().unwrap_or(TRUE);
        if cc == NONE {
            return base;
        }
        let bit = self.cl[cc as usize];
        self.and(base, bit)
    }

    /// Bounds every id the op names and makes room for it.
    fn admit(&mut self, i: usize, op: &Op) -> Result<(), Obstruction> {
        let at = Some((i, op));
        if let Err(e) = op.validate() {
            return Err(obstruct(at, "unsupported-op", e));
        }
        for q in op.qubits() {
            if q >= MAX_QUBITS {
                return Err(obstruct(
                    at,
                    "unsupported-op",
                    "qubit id above the cap".into(),
                ));
            }
            self.wire(q);
        }
        for b in [op.c_target, op.c_condition] {
            if b == NONE {
                continue;
            }
            if b >= MAX_BITS {
                return Err(obstruct(
                    at,
                    "unsupported-op",
                    "bit id above the cap".into(),
                ));
            }
            if self.cl.len() <= b as usize {
                self.cl.resize(b as usize + 1, FALSE);
            }
        }
        Ok(())
    }

    /// Makes room for the wire `q`. A lane wire starts as its `Initial` variable, which is
    /// made when the wire is first read ([`Engine::start`]); every other wire starts 0.
    fn wire(&mut self, q: u32) {
        if self.val.len() <= q as usize {
            self.val.resize(q as usize + 1, Sum::default());
        }
    }

    /// Gives every lane wire its `Initial` variable. `false` if the store is too small.
    fn start(&mut self) -> bool {
        self.wire(self.first.saturating_sub(1));
        for q in 0..self.first {
            let level = match self.inner.iter().position(|&x| x == q) {
                Some(k) => self.inner_level(k, false),
                None => self.rank[q as usize],
            };
            if level != NO_LEVEL {
                self.val[q as usize] = Sum::of(self.b.var(level));
            }
        }
        !self.b.overflowed
    }

    /// One op. Everything is computed first; state changes only once the results are known to
    /// be sound, so an attempt that returns `Full` has changed nothing.
    #[allow(clippy::too_many_lines)]
    fn exec(&mut self, i: usize, op: &Op) -> Result<Step, Obstruction> {
        let at = Some((i, op));
        let no = |kind: &'static str, why: &str| Err(obstruct(at, kind, why.to_string()));
        let l = self.l;
        let sys = move |q: u32| l.sys(q).map(|q| q as usize);
        let (t, c1, c2) = (op.q_target, op.q_control1, op.q_control2);
        macro_rules! sound {
            () => {
                if !self.sound() {
                    return Ok(Step::Full);
                }
            };
        }
        match op.kind {
            // Bookkeeping: registers are taken from the compiled stream; hints have no effect.
            K::Register | K::AppendToRegister | K::Segment | K::DebugPrint => {}
            K::S | K::Sdg => {
                return no(
                    "unsupported-op",
                    "S and Sdg are outside the rule set (phases are tracked as signs only)",
                )
            }
            K::PushCondition => {
                let c = self.cond(op.c_condition);
                sound!();
                self.stack.push(c);
            }
            K::PopCondition => {
                if self.stack.pop().is_none() {
                    return no("structure", "pop on an empty condition stack");
                }
            }
            K::BitInvert | K::BitStore0 | K::BitStore1 => {
                let c = self.cond(op.c_condition);
                let mut v = self.cl[op.c_target as usize];
                if op.kind != K::BitInvert {
                    // Store 0: v + c v. Store 1: v + c v + c.
                    let d = self.and(c, v);
                    v = self.xor(v, d);
                }
                if op.kind != K::BitStore0 {
                    v = self.xor(v, c);
                }
                sound!();
                self.cl[op.c_target as usize] = v;
            }
            K::Neg => {
                let c = self.cond(op.c_condition);
                let p = self.phase(c);
                sound!();
                self.sign(p);
            }
            K::X | K::CX | K::CCX => {
                if [c1, c2].iter().any(|&c| c != NONE && sys(c).is_some()) {
                    return no("unsupported-op", "a system qubit controls a gate");
                }
                let mut p = self.cond(op.c_condition);
                for c in [c1, c2] {
                    if c != NONE {
                        let v = self.rd(c);
                        p = self.and(p, v);
                    }
                }
                match sys(t) {
                    Some(q) => {
                        sound!();
                        self.log.push((i, Sys::X { q, c: p }));
                    }
                    None => {
                        let old = std::mem::take(&mut self.val[t as usize]);
                        let v = self.plus(&old, p);
                        self.val[t as usize] = old;
                        sound!();
                        self.val[t as usize] = v;
                    }
                }
            }
            K::Z | K::CZ | K::CCZ => {
                let qs: Vec<u32> = op.qubits().collect();
                let on: Vec<usize> = qs.iter().filter_map(|&q| sys(q)).collect();
                if on.len() > 1 {
                    return no("unsupported-op", "more than one system qubit in a gate");
                }
                let mut w = self.cond(op.c_condition);
                for &q in qs.iter().filter(|&&q| sys(q).is_none()) {
                    let v = self.rd(q);
                    w = self.and(w, v);
                }
                if let [q] = on[..] {
                    sound!();
                    self.log.push((i, Sys::Z { q, c: w }));
                } else {
                    let p = self.phase(w);
                    sound!();
                    self.sign(p);
                }
            }
            K::Swap => {
                if sys(t).is_some() || sys(c1).is_some() {
                    return no("unsupported-op", "Swap on a system qubit");
                }
                let c = self.cond(op.c_condition);
                let (x, y) = (self.rd(t), self.rd(c1));
                let d = self.xor(x, y);
                let d = self.and(c, d);
                let (x, y) = (self.xor(x, d), self.xor(y, d));
                sound!();
                (self.val[t as usize], self.val[c1 as usize]) = (Sum::of(x), Sum::of(y));
            }
            K::Hmr => {
                if t < self.first {
                    return no("unsupported-op", "Hmr on a qubit that is not an ancilla");
                }
                let Some(level) = u32::try_from(self.outcomes.len())
                    .ok()
                    .and_then(|n| self.outcome_base.checked_add(n))
                    .filter(|&x| x < NO_LEVEL - 1)
                else {
                    return no("unsupported-op", "too many measurements");
                };
                let c = self.cond(op.c_condition);
                // On the lanes where `c` holds: sign (-1)^(m v), the qubit becomes 0, the bit m.
                let m = self.b.var(level);
                let v = self.rd(t);
                let cv = self.and(c, v);
                let s = self.and(cv, m);
                let p = self.phase(s);
                let w = self.xor(v, cv);
                let b = op.c_target as usize;
                let old = self.cl[b];
                let e = self.xor(old, m);
                let e = self.and(c, e);
                let bit = self.xor(old, e);
                sound!();
                self.sign(p);
                self.val[t as usize] = Sum::of(w);
                self.cl[b] = bit;
                self.outcomes.push(i);
                self.counts.measurements += 1;
            }
            K::R => {
                if t < self.first {
                    return no("unsupported-op", "R on a qubit that is not an ancilla");
                }
                // Stricter than the simulator inside a condition block: 0 on every lane.
                let v = self.rd(t);
                sound!();
                if v != FALSE {
                    return Err(obstruct(
                        at,
                        "reset-not-proven",
                        format!(
                            "ancilla q{t} is not 0 when freed: {}",
                            self.counterexample(v)
                        ),
                    ));
                }
                self.val[t as usize] = Sum::default();
                self.counts.resets_proved += 1;
            }
            K::Givens => {
                let c = self.cond(op.c_condition);
                sound!();
                if c != TRUE {
                    return no(
                        "conditioned-rotation",
                        "a Givens under a classical condition is outside the rule set",
                    );
                }
                let (Some(p), Some(q)) = (sys(c1), sys(t)) else {
                    return no("unsupported-op", "Givens acts on two system qubits");
                };
                let registers = self.registers;
                let Some(reg) = registers.get(op.r_target as usize) else {
                    return no("unsupported-op", "the angle register is not declared");
                };
                if p >= q || reg.iter().any(|&x| sys(x).is_some() || x >= MAX_QUBITS) {
                    return no("unsupported-op", "Givens operands outside the rule set");
                }
                let mut angle = Vec::with_capacity(reg.len());
                for &x in reg {
                    self.wire(x);
                    angle.push(self.rd(x));
                }
                sound!();
                let sign = FALSE;
                self.counts.givens += 1;
                self.log.push((i, Sys::G { p, q, angle, sign }));
            }
            K::SpinSwap | K::SpinSwapDg => {
                if sys(t).is_some() {
                    return no("unsupported-op", "SpinSwap controlled by a system qubit");
                }
                let c = self.cond(op.c_condition);
                let v = self.rd(t);
                let c = self.and(c, v);
                sound!();
                let dagger = op.kind == K::SpinSwapDg;
                self.counts.spin_swaps += 1;
                self.log.push((i, Sys::F { dagger, c }));
            }
            K::Reflect => return self.reflect(i, op),
        }
        Ok(Step::Done)
    }

    /// `Reflect`: the inner register must hold its initial value; it then holds new variables.
    fn reflect(&mut self, i: usize, op: &Op) -> Result<Step, Obstruction> {
        let at = Some((i, op));
        let no = |why: &str| Err(obstruct(at, "structure", why.to_string()));
        if self.reflected {
            return no("a second Reflect");
        }
        if !self.stack.is_empty() {
            return no("Reflect inside a condition block");
        }
        let mut reg = self
            .registers
            .get(op.r_target as usize)
            .cloned()
            .unwrap_or_default();
        reg.sort_unstable();
        if reg != self.inner {
            return no("Reflect is not on exactly the inner uniform register");
        }
        let mut after = Vec::with_capacity(self.inner.len());
        let mut wrong = None;
        for k in 0..self.inner.len() {
            let q = self.inner[k];
            let want = self.b.var(self.inner_level(k, false));
            let v = self.rd(q);
            let d = self.xor(v, want);
            if d != FALSE && wrong.is_none() {
                wrong = Some((q, d));
            }
            after.push(self.b.var(self.inner_level(k, true)));
        }
        if !self.sound() {
            return Ok(Step::Full);
        }
        if let Some((q, d)) = wrong {
            return Err(obstruct(
                at,
                "reflect-not-restored",
                format!(
                    "inner uniform qubit q{q} does not hold its initial value at the Reflect: {}",
                    self.counterexample(d)
                ),
            ));
        }
        for (k, v) in after.into_iter().enumerate() {
            self.val[self.inner[k] as usize] = Sum::of(v);
        }
        self.reflected = true;
        Ok(Step::Done)
    }

    /// The level of inner wire `k`'s variable before (`after = false`) or after the `Reflect`
    /// in the pass that is running.
    fn inner_level(&self, k: usize, after: bool) -> u32 {
        if after == self.swapped {
            self.rank[self.inner[k] as usize]
        } else {
            self.second[k]
        }
    }

    /// End of the stream, wire `q`: a lane wire holds the value the lane must end with, any
    /// other wire 0.
    fn restored(&mut self, q: u32) -> Result<Step, Obstruction> {
        if self.l.sys(q).is_some() {
            return Ok(Step::Done);
        }
        let (want, kind) = if let Some(k) = self.inner.iter().position(|&x| x == q) {
            let level = self.inner_level(k, true);
            (self.b.var(level), "registers-not-restored")
        } else if q < self.first {
            (self.b.var(self.rank[q as usize]), "registers-not-restored")
        } else {
            (FALSE, "ancilla-not-clean")
        };
        let v = self.rd(q);
        let d = self.xor(v, want);
        if !self.sound() {
            return Ok(Step::Full);
        }
        if d != FALSE {
            let what = if q < self.first {
                "does not hold the value the lane must end with"
            } else {
                "is not 0 at the end"
            };
            return Err(obstruct(
                None,
                kind,
                format!("qubit q{q} {what}: {}", self.counterexample(d)),
            ));
        }
        Ok(Step::Done)
    }

    /// Appends one system op to the normal form `forms[k]`: a `Z` joins the frame, any other
    /// op is moved past the frame and joins the list.
    fn fold(&mut self, k: usize, from: Source) -> Result<Step, Obstruction> {
        let (at, op) = match from {
            Source::Log(j) => self.log[j].clone(),
            // The adjoint of `forms[0]`, `Z^frame * (last of list) ... (first of list)`, as a
            // time-ordered list: the frame first, then the list reversed with each op
            // inverted (`X` and `Z` are their own inverses; a Givens changes its angle's sign;
            // `F` and `F^dagger` exchange).
            Source::Adjoint(j) => {
                let n = self.forms[0].frame.len();
                if j < n {
                    let c = self.forms[0].frame[j];
                    (usize::MAX, Sys::Z { q: j, c })
                } else {
                    let m = self.forms[0].list.len();
                    let (at, op) = self.forms[0].list[m - 1 - (j - n)].clone();
                    let inv = match op {
                        Sys::G { p, q, angle, sign } => Sys::G {
                            p,
                            q,
                            angle,
                            sign: self.xor(sign, TRUE),
                        },
                        Sys::F { dagger, c } => Sys::F { dagger: !dagger, c },
                        other => other,
                    };
                    (at, inv)
                }
            }
        };
        match op {
            Sys::Z { q, c } => {
                let f = self.xor(self.forms[k].frame[q], c);
                if !self.sound() {
                    return Ok(Step::Full);
                }
                self.forms[k].frame[q] = f;
            }
            // X^c Z^f = (-1)^(c f) Z^f X^c.
            Sys::X { q, c } => {
                let d = self.and(c, self.forms[k].frame[q]);
                let s = self.xor(self.forms[k].sign, d);
                if !self.sound() {
                    return Ok(Step::Full);
                }
                self.forms[k].sign = s;
                self.forms[k].list.push((at, Sys::X { q, c }));
            }
            // G_pq(t) Z_p = Z_p G_pq(-t), the same for Z_q; other Z commute with G_pq.
            Sys::G { p, q, angle, sign } => {
                let sign = self.xor(sign, self.forms[k].frame[p]);
                let sign = self.xor(sign, self.forms[k].frame[q]);
                if !self.sound() {
                    return Ok(Step::Full);
                }
                self.forms[k].list.push((at, Sys::G { p, q, angle, sign }));
            }
            // F exchanges Z_2j and Z_2j+1 (F^dagger too): where c is 1 the pair's frame bits
            // are exchanged.
            Sys::F { dagger, c } => {
                let pairs = self.forms[k].frame.len() / 2;
                let mut frame = Vec::with_capacity(2 * pairs);
                for j in 0..pairs {
                    let (x, y) = (self.forms[k].frame[2 * j], self.forms[k].frame[2 * j + 1]);
                    let d = self.xor(x, y);
                    let d = self.and(c, d);
                    frame.push(self.xor(x, d));
                    frame.push(self.xor(y, d));
                }
                if !self.sound() {
                    return Ok(Step::Full);
                }
                self.forms[k].frame[..2 * pairs].copy_from_slice(&frame);
                self.forms[k].list.push((at, Sys::F { dagger, c }));
            }
        }
        Ok(Step::Done)
    }

    /// Adds the lane's sign to `forms[k]`. Every coefficient of a product of outcomes must be
    /// 0; the part with no outcome is the sign.
    fn fold_sign(&mut self, k: usize) -> Result<Step, Obstruction> {
        let phi = std::mem::take(&mut self.phi);
        let totals: Vec<(Mono, Node)> = phi
            .iter()
            .map(|(m, s)| (m.clone(), self.total(s)))
            .collect();
        self.phi = phi;
        let plain = totals
            .iter()
            .find(|t| t.0.is_empty())
            .map_or(FALSE, |t| t.1);
        let s = self.xor(self.forms[k].sign, plain);
        if !self.sound() {
            return Ok(Step::Full);
        }
        if let Some((m, c)) = totals.iter().find(|t| !t.0.is_empty() && t.1 != FALSE) {
            let op = |x: u32| self.outcomes[(x - self.outcome_base) as usize];
            let ops: Vec<String> = m.iter().map(|&x| op(x).to_string()).collect();
            return Err(Obstruction {
                op: Some(op(m[0])),
                name: Some(K::Hmr.name()),
                kind: "outcome-phase",
                reason: format!(
                    "the lane's sign depends on the outcome of the HMR at op {}: the coefficient \
                     of the product of the outcomes at ops [{}] is not 0: {}",
                    op(m[0]),
                    ops.join(" "),
                    self.counterexample(*c)
                ),
            });
        }
        self.forms[k].sign = s;
        self.phi.clear();
        Ok(Step::Done)
    }

    /// The op of an outcome some function of `roots` depends on, if any.
    fn outcome_in(&self, roots: &[Node]) -> Option<usize> {
        self.b
            .reads_from(roots, self.outcome_base)
            .map(|level| self.outcomes[(level - self.outcome_base) as usize])
    }

    /// Refuses if `forms[k]` depends on an outcome: its list and frame, or (`sign`) its sign.
    fn independent(&self, k: usize, sign: bool) -> Result<(), Obstruction> {
        let f = &self.forms[k];
        let depends = |at: Option<usize>, kind: &'static str, what: &str, i: usize| Obstruction {
            op: at,
            name: None,
            kind,
            reason: format!("{what} depends on the outcome of the HMR at op {i}"),
        };
        if sign {
            return match self.outcome_in(&[f.sign]) {
                None => Ok(()),
                Some(i) => {
                    let mut o = depends(Some(i), "outcome-phase", "the lane's sign", i);
                    o.name = Some(K::Hmr.name());
                    Err(o)
                }
            };
        }
        let mut all: Vec<Node> = f.list.iter().flat_map(|(_, s)| s.nodes()).collect();
        all.extend_from_slice(&f.frame);
        if self.outcome_in(&all).is_none() {
            return Ok(());
        }
        let steers = "outcome-steers-system";
        for (at, s) in &f.list {
            if let Some(i) = self.outcome_in(&s.nodes()) {
                return Err(depends(Some(*at), steers, s.what(), i));
            }
        }
        let i = self.outcome_in(&f.frame).unwrap_or(usize::MAX);
        Err(depends(None, steers, "a system Z's control", i))
    }

    /// Stage 3: the normal form of the adjoint of `forms[0]`, the lane operator `L(g, a, b)`,
    /// is `forms[1]`, the normal form of `L(g, b, a)`, element by element.
    #[allow(clippy::too_many_lines)]
    fn compare(&mut self) -> Result<(), Obstruction> {
        let n = self.forms[0].frame.len() + self.forms[0].list.len();
        self.forms.push(Normal::new(self.l.system));
        for j in 0..n {
            self.room();
            self.retry(None, |e| e.fold(2, Source::Adjoint(j)))?;
        }
        // The adjoint's sign: the lane's, and what moving the frame back through cost.
        self.retry(None, |e| {
            let s = e.xor(e.forms[0].sign, e.forms[2].sign);
            if !e.sound() {
                return Ok(Step::Full);
            }
            e.forms[2].sign = s;
            Ok(Step::Done)
        })?;
        let differ = |at: Option<usize>, kind: &'static str, reason: String| Obstruction {
            op: at,
            name: None,
            kind,
            reason,
        };
        let mut wrong: Option<Mismatch> = None;
        let (back, twin) = (&self.forms[2], &self.forms[1]);
        if back.list.len() != twin.list.len() {
            return Err(differ(
                None,
                "shape-mismatch",
                "the adjoint's normal form has a different length".into(),
            ));
        }
        let mut compared = 0;
        for (k, ((_, got), (at, want))) in back.list.iter().zip(&twin.list).enumerate() {
            // Named by the op the lane operator has at this position.
            let at = Some(*at);
            let here = format!("element {k} of {} of the normal form", twin.list.len());
            let found = match (got, want) {
                (Sys::X { q, c }, Sys::X { q: q2, c: c2 }) if q == q2 => (c != c2).then(|| {
                    (
                        "control-mismatch",
                        format!(
                            "{here}: the X on system qubit {q} has a different control in the \
                             adjoint"
                        ),
                        (*c, *c2),
                    )
                }),
                (Sys::F { dagger, c }, Sys::F { dagger: d2, c: c2 }) if dagger == d2 => (c != c2)
                    .then(|| {
                        (
                            "control-mismatch",
                            format!("{here}: the SpinSwap has a different control in the adjoint"),
                            (*c, *c2),
                        )
                    }),
                (
                    Sys::G { p, q, angle, sign },
                    Sys::G {
                        p: p2,
                        q: q2,
                        angle: a2,
                        sign: s2,
                    },
                ) if p == p2 && q == q2 && angle.len() == a2.len() => angle
                    .iter()
                    .zip(a2)
                    .enumerate()
                    .find(|(_, (x, y))| x != y)
                    .map(|(j, (x, y))| {
                        (
                            "angle-mismatch",
                            format!(
                                "{here}: bit {j} of the angle of the Givens on modes ({p}, {q}) \
                                 differs in the adjoint"
                            ),
                            (*x, *y),
                        )
                    })
                    .or_else(|| {
                        (sign != s2).then(|| {
                            (
                                "angle-mismatch",
                                format!(
                                    "{here}: the sign of the angle of the Givens on modes ({p}, \
                                     {q}) differs in the adjoint"
                                ),
                                (*sign, *s2),
                            )
                        })
                    }),
                _ => {
                    return Err(differ(
                        at,
                        "shape-mismatch",
                        format!(
                            "{here}: the adjoint has a different system op here (kind, qubits, \
                             angle width or SpinSwap direction)"
                        ),
                    ))
                }
            };
            if let Some((kind, what, pair)) = found {
                wrong = Some((kind, what, pair, at));
                break;
            }
            compared += 1;
        }
        if wrong.is_none() {
            wrong = back
                .frame
                .iter()
                .zip(&twin.frame)
                .enumerate()
                .find(|(_, (x, y))| x != y)
                .map(|(q, (x, y))| {
                    (
                        "frame-mismatch",
                        format!("the Z left on system qubit {q} differs in the adjoint"),
                        (*x, *y),
                        None,
                    )
                });
        }
        if wrong.is_none() && back.sign != twin.sign {
            wrong = Some((
                "phase-mismatch",
                "the lane's sign differs in the adjoint".into(),
                (back.sign, twin.sign),
                None,
            ));
        }
        if let Some((kind, what, (x, y), at)) = wrong {
            let d = self.xor(x, y);
            let why = if self.sound() {
                self.counterexample(d)
            } else {
                "the two are different functions".into()
            };
            return Err(differ(at, kind, format!("{what}: {why}")));
        }
        self.counts.compared = compared;
        self.hermitian = true;
        Ok(())
    }
}
