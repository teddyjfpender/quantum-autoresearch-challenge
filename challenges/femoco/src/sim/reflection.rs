//! The reflection check: a static certificate that a nested walk step is Hermitian on every
//! lane and for every measurement-outcome record (spec/SAMPLING.md section 4, spec/DESIGN.md
//! section 16.6). Opt-in (`eval_circuit --reflection-check`); it never reads a sampled lane.
//!
//! **Method ("symbolic lanes").** The op stream is executed once under the lane semantics of
//! `super::lanes`, with every non-system wire a GF(2) polynomial (`poly`) over immutable
//! symbols: one per initial control and uniform bit, one per inner uniform bit after the
//! `Reflect`, one per `Hmr` outcome. Each rule below is an exact identity of those semantics, so
//! at every op a wire's polynomial is the wire's value on every lane and outcome record.
//!
//! **What a certificate establishes**, in this order (`Report`):
//! 1. *Registers restored.* Every `R` frees a qubit that is 0; at the `Reflect` the inner
//!    register holds its initial value; at the end the control and the outer uniform bits hold
//!    their initial values, the inner register its second-pass value, and every other
//!    non-system wire is 0.
//! 2. *Outcome independent.* The lane's sign is `(-1)^Phi` with `Phi` a polynomial; the
//!    coefficient of every product of outcomes in `Phi` is identically zero, and no system op's
//!    control, no rotation angle and no part of the remaining sign names an outcome. So the lane
//!    operator `L(g, a, b)` on the system register is a function of the lane alone (`g`: control
//!    and outer bits, `a`, `b`: the inner value before and after the `Reflect`).
//! 3. *Hermitian.* `L(g, a, b)^dagger = L(g, b, a)` as an identity in the symbols. With 1 this is
//!    Hermiticity of the step `C = sum_g sum_{a,b} (2 / 2^w - [a = b]) |g, b><g, a| (x)
//!    L(g, a, b)` (spec/DESIGN.md section 16.2) on the space where every ancilla is 0.
//!
//! The comparison of 3 writes the lane's system ops in a normal form: `Z`s are moved to the end
//! (a symbolic `Z` frame), leaving the `X`s, `Givens` and `SpinSwap`s in order. It uses three
//! operator identities and the `X`/`Z` anticommutation sign: `Z_p G_pq(t) Z_p = G_pq(-t)` (and
//! the same for `Z_q`); `Z_k` commutes with `G_pq` for `k` outside `{p, q}`; `F Z_2p F^dagger =
//! Z_{2p+1}` for the `SpinSwap` layer `F`. tests/reflection.rs checks them against dense
//! matrices. The adjoint's normal form must then equal, element by element, the normal form
//! with the first- and second-pass inner symbols exchanged.
//!
//! **Fail closed.** The result is `certified`, or `not certified` with the first obstruction.
//! Anything outside the rules is an obstruction: `S`/`Sdg`, a rotation under a classical
//! condition, an outcome that reaches a system op, a zero that is not proven, a normal-form
//! mismatch, an exhausted work budget, a lane map that is not nested. "Not certified" means not
//! proven; it is not a finding that the step is not Hermitian. The rules are incomplete on
//! purpose: for example two system ops that commute are not reordered, and an angle register
//! that holds `2^beta - a` is not recognised as `-a`.
//!
//! **Not established.** That the step encodes the spec's operator: a step can be Hermitian and
//! wrong, and that stays a sampled statement. The certificate also says nothing about executed
//! gate counts.
pub mod poly;

use super::compile::{Layout, MAX_BITS, MAX_QUBITS};
use super::nested::Inner;
use crate::circuit::{Op, OperationType as K, NONE};
use poly::{Arena, Kind, Limits, Mono, Poly, Sym, Verdict, ONE};
use serde_json::json;
use std::collections::HashMap;

/// The rule set's name and version. The sealing threshold, the zero test's rungs and their
/// limits decide which true identities are found, so they belong to the version: change any of
/// them and this string must change.
pub const VERSION: &str = "reflection-v1";
/// A wire read as an operand of a product is sealed when its polynomial has more terms.
const CAP: usize = 3;
const NO_SYM: Sym = u32::MAX;

/// Deterministic work limits (counts of monomial operations; never wall clock). The default
/// work limit is about 60 times what the Low et al. baseline on the Reiher instance needs
/// (1.7e7); memory is bounded besides by `poly`'s cap on interned monomials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Total work of the run (`poly::Arena::work`).
    pub work: u64,
    /// Most monomials the expansion rung of one zero test may hold.
    pub expand: usize,
    /// Truth tables are used for a zero test over at most this many primary symbols.
    pub table_bits: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            work: 1 << 30,
            expand: 1 << 19,
            table_bits: 20,
        }
    }
}

/// Why the check did not certify: the first rule the op stream is not proven to meet.
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
    /// Products of outcomes whose phase coefficient was proven zero at the end of the stream
    /// (those that cancelled term by term on the way are not counted).
    pub outcome_phases_proved: u64,
    /// System ops in the lane operator: `X`/`Z` (controlled or not), `Givens`, `SpinSwap`.
    pub system_ops: u64,
    pub givens: u64,
    pub spin_swaps: u64,
    /// Elements of the normal form compared with the adjoint's.
    pub compared: u64,
    /// Zero tests by the rung that decided them (`poly::Arena::is_zero`).
    pub zero_syntactic: u64,
    pub zero_truth_table: u64,
    pub zero_expansion: u64,
    pub zero_undecided: u64,
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
    pub symbols: u64,
    pub sealed: u64,
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
            sealed: 0,
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
            "rule_set": "symbolic lanes: every non-system wire as a GF(2) polynomial in the lane's bits and the measurement outcomes; the system ops in a Z-frame normal form compared with their adjoint under exchange of the first- and second-pass inner values",
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
                "outcome_phase_coefficients_proved_zero": c.outcome_phases_proved,
                "system_ops": c.system_ops,
                "givens": c.givens,
                "spin_swaps": c.spin_swaps,
                "normal_form_elements_compared": c.compared,
                "zero_tests": {
                    "syntactic": c.zero_syntactic,
                    "truth_table": c.zero_truth_table,
                    "expansion": c.zero_expansion,
                    "undecided": c.zero_undecided,
                },
            },
            "symbols": self.symbols,
            "sealed": self.sealed,
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
#[derive(Clone, Debug)]
enum Sys {
    /// `Z` on system qubit `q` where `c` is 1.
    Z { q: usize, c: Poly },
    /// `X` on system qubit `q` where `c` is 1.
    X { q: usize, c: Poly },
    /// `G_{p,q}((-1)^sign 2 pi angle / 2^beta)`, the angle register bit by bit.
    G {
        p: usize,
        q: usize,
        angle: Vec<Poly>,
        sign: Poly,
    },
    /// The `SpinSwap` layer `F` (`F^dagger` when `dagger`) where `c` is 1.
    F { dagger: bool, c: Poly },
}

struct Engine<'a> {
    a: Arena,
    lim: Limits,
    l: Layout,
    first: u32,
    inner: Vec<u32>,
    registers: &'a [Vec<u32>],
    /// Per non-system wire: its value.
    val: Vec<Poly>,
    /// Per classical bit: its value, a polynomial in the outcomes.
    cl: Vec<Poly>,
    /// Per wire below `first`: its `Initial` symbol (`NO_SYM` for a system qubit).
    initial: Vec<Sym>,
    /// Per inner wire, in `inner` order: its `Second` symbol, once the `Reflect` has run.
    second: Vec<Sym>,
    /// The condition stack: per open block the product of its bit (as it was at the push) and
    /// every enclosing block's.
    stack: Vec<Poly>,
    /// The lane's sign is `(-1)^Phi`, `Phi = sum_mu mu * phi[mu]` over products `mu` of
    /// outcomes; `phi[ONE]` is the part with no outcome.
    phi: HashMap<Mono, Poly>,
    log: Vec<(usize, Sys)>,
    reflected: bool,
    twin: Vec<Sym>,
    counts: Counts,
    registers_restored: bool,
    outcome_independent: bool,
    hermitian: bool,
}

/// The adjoint of `Z^frame * (last of list) ... (first of list)` as a time-ordered list: the
/// frame first, then the list reversed with each op inverted (`X` and `Z` are their own
/// inverses; a Givens changes its angle's sign; `F` and `F^dagger` exchange).
fn adjoint(list: &[(usize, Sys)], frame: &[Poly]) -> Vec<(usize, Sys)> {
    let mut adj: Vec<(usize, Sys)> = Vec::with_capacity(list.len() + frame.len());
    for (q, f) in frame.iter().enumerate() {
        if !f.is_zero() {
            adj.push((usize::MAX, Sys::Z { q, c: f.clone() }));
        }
    }
    for (at, op) in list.iter().rev() {
        let inv = match op {
            Sys::G { p, q, angle, sign } => {
                let mut s = sign.clone();
                s.add(&Poly::one());
                Sys::G {
                    p: *p,
                    q: *q,
                    angle: angle.clone(),
                    sign: s,
                }
            }
            Sys::F { dagger, c } => Sys::F {
                dagger: !dagger,
                c: c.clone(),
            },
            other => other.clone(),
        };
        adj.push((*at, inv));
    }
    adj
}

/// Test hook (tests/reflection.rs): the normal form and the adjoint on system ops whose
/// controls are constants, so that both can be compared with dense matrices. Not used by the
/// check itself; it runs the same `Engine::normal` and `adjoint`.
#[doc(hidden)]
pub mod probe {
    use super::{adjoint, Budget, Engine, Inner, Layout, Poly, Sys};

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
    }

    /// An operator as `(-1)^sign * Z^frame * (last of list) ... (first of list)`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Normal {
        pub list: Vec<Gate>,
        pub frame: Vec<bool>,
        pub sign: bool,
    }

    const BITS: usize = 16;

    fn constant(b: bool) -> Poly {
        if b {
            Poly::one()
        } else {
            Poly::zero()
        }
    }

    fn lift(g: &Gate) -> Sys {
        match *g {
            Gate::X(q) => Sys::X { q, c: Poly::one() },
            Gate::Z(q) => Sys::Z { q, c: Poly::one() },
            Gate::G { p, q, angle, minus } => Sys::G {
                p,
                q,
                angle: (0..BITS).map(|k| constant(angle >> k & 1 == 1)).collect(),
                sign: constant(minus),
            },
            Gate::F { dagger } => Sys::F {
                dagger,
                c: Poly::one(),
            },
        }
    }

    fn lower(s: &Sys) -> Gate {
        match s {
            Sys::X { q, .. } => Gate::X(*q),
            Sys::Z { q, .. } => Gate::Z(*q),
            Sys::G { p, q, angle, sign } => Gate::G {
                p: *p,
                q: *q,
                angle: (0..angle.len()).fold(0, |a, k| a | u64::from(angle[k].is_one()) << k),
                minus: sign.is_one(),
            },
            Sys::F { dagger, .. } => Gate::F { dagger: *dagger },
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
        let mut e = Engine::new(&l, Inner { lo: 0, width: 0 }, &[], Budget::default());
        let seq: Vec<(usize, Sys)> = seq.iter().map(|g| (0, lift(g))).collect();
        let (list, frame, sign) = e.normal(&seq);
        let adj = adjoint(&list, &frame);
        let (adj_list, adj_frame, crossing) = e.normal(&adj);
        let pack = |list: &[(usize, Sys)], frame: &[Poly], sign: bool| Normal {
            list: list.iter().map(|(_, s)| lower(s)).collect(),
            frame: frame.iter().map(Poly::is_one).collect(),
            sign,
        };
        (
            pack(&list, &frame, sign.is_one()),
            pack(&adj_list, &adj_frame, sign.is_one() != crossing.is_one()),
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
    if budget.table_bits > 24 || l.first_ancilla() >= MAX_QUBITS {
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
    let mut e = Engine::new(l, inner, registers, budget);
    let obstruction = e.run(ops).err();
    let r = e.a.rungs;
    Report {
        certified: obstruction.is_none() && e.hermitian,
        obstruction,
        registers_restored: e.registers_restored,
        outcome_independent: e.outcome_independent,
        hermitian: e.hermitian,
        counts: Counts {
            zero_syntactic: r.syntactic,
            zero_truth_table: r.truth_table,
            zero_expansion: r.expansion,
            zero_undecided: r.undecided,
            ..e.counts
        },
        symbols: e.a.symbols(),
        sealed: e.a.sealed(),
        work: e.a.work,
        budget,
    }
}

impl<'a> Engine<'a> {
    fn new(l: &Layout, inner: Inner, registers: &'a [Vec<u32>], budget: Budget) -> Self {
        let mut a = Arena::new(budget.work);
        let first = l.first_ancilla();
        let (mut val, mut initial) = (Vec::new(), Vec::new());
        for q in 0..first {
            if l.sys(q).is_some() {
                initial.push(NO_SYM);
                val.push(Poly::zero());
            } else {
                let s = a.primary(Kind::Initial(q));
                initial.push(s);
                val.push(a.sym_poly(s));
            }
        }
        Self {
            a,
            lim: Limits {
                table_bits: budget.table_bits,
                expand: budget.expand,
            },
            l: *l,
            first,
            inner: inner.qubits(l),
            registers,
            val,
            cl: Vec::new(),
            initial,
            second: Vec::new(),
            stack: Vec::new(),
            phi: HashMap::new(),
            log: Vec::new(),
            reflected: false,
            twin: Vec::new(),
            counts: Counts::default(),
            registers_restored: false,
            outcome_independent: false,
            hermitian: false,
        }
    }

    fn budget(&self, at: Option<(usize, &Op)>) -> Obstruction {
        obstruct(
            at,
            "budget",
            "the work budget is exhausted; nothing is certified".into(),
        )
    }

    fn run(&mut self, ops: &[Op]) -> Result<(), Obstruction> {
        for (i, op) in ops.iter().enumerate() {
            self.step(i, op)?;
            if self.a.exhausted {
                return Err(self.budget(Some((i, op))));
            }
            self.counts.ops += 1;
        }
        let checked = self
            .finish()
            .and_then(|()| self.phases())
            .and_then(|()| self.compare());
        // An exhausted arena returns zero products, so nothing computed after that is used.
        if self.a.exhausted {
            self.hermitian = false;
            return Err(self.budget(None));
        }
        checked
    }

    /// `Ok` when `p` is proven identically zero; otherwise why not.
    fn prove_zero(&mut self, p: &Poly) -> Result<(), String> {
        match self.a.is_zero(p, &self.lim) {
            Verdict::Zero => Ok(()),
            Verdict::Nonzero => Err("it is 1 for some lane or outcome record".into()),
            Verdict::Unknown => {
                // For the reader of the refusal: how wide the unproven identity is.
                let (primaries, sealed) = self.a.support(p);
                Err(format!(
                    "not proven within the work limits (the identity involves {primaries} lane \
                     and outcome bits through {sealed} sealed definitions)"
                ))
            }
        }
    }

    /// The wire `q` as an operand of a product: sealed first if it has more than `CAP` terms
    /// (the wire then holds the sealed symbol, which is equal to what it held).
    fn rd(&mut self, q: u32) -> Poly {
        let v = &self.val[q as usize];
        if v.len() > CAP {
            let s = self.a.seal(v);
            self.val[q as usize] = self.a.sym_poly(s);
        }
        self.val[q as usize].clone()
    }

    /// The condition an op with `c_condition = cc` executes under: the enclosing blocks'
    /// product, times the bit `cc` if given.
    fn cond(&mut self, cc: u32) -> Poly {
        let base = self.stack.last().cloned().unwrap_or_else(Poly::one);
        self.a.tick(base.len());
        if cc == NONE {
            return base;
        }
        let b = self.cl[cc as usize].clone();
        self.a.mul(&base, &b)
    }

    /// Multiplies the lane's sign by `(-1)^(cond * w)`. Each monomial of the product is filed
    /// under the outcomes it names, so `phi[mu]` holds outcome-free monomials (sealed symbols
    /// aside, which the later checks look into).
    fn add_phase(&mut self, cond: &Poly, w: &Poly) {
        if cond.is_zero() || w.is_zero() {
            return;
        }
        self.a.tick(cond.len().saturating_mul(w.len()));
        if self.a.exhausted {
            return;
        }
        let plain = w.terms().iter().all(|&m| !self.a.has_outcome(m));
        for &mu in cond.terms() {
            if plain {
                let e = self.phi.entry(mu).or_default();
                e.add(w);
                if e.is_zero() {
                    self.phi.remove(&mu);
                }
                continue;
            }
            for &m in w.terms() {
                let (k, rest) = self.a.split_outcome(m);
                let key = self.a.mono_mul(mu, k);
                let e = self.phi.entry(key).or_default();
                e.toggle(rest);
                if e.is_zero() {
                    self.phi.remove(&key);
                }
            }
        }
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
            if self.val.len() <= q as usize {
                self.val.resize(q as usize + 1, Poly::zero());
            }
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
                self.cl.resize(b as usize + 1, Poly::zero());
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn step(&mut self, i: usize, op: &Op) -> Result<(), Obstruction> {
        let at = Some((i, op));
        let no = |kind: &'static str, why: &str| Err(obstruct(at, kind, why.to_string()));
        self.admit(i, op)?;
        let l = self.l;
        let sys = move |q: u32| l.sys(q).map(|q| q as usize);
        let (t, c1, c2) = (op.q_target, op.q_control1, op.q_control2);
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
                self.stack.push(c);
            }
            K::PopCondition => {
                if self.stack.pop().is_none() {
                    return no("structure", "pop on an empty condition stack");
                }
            }
            K::BitInvert | K::BitStore0 | K::BitStore1 => {
                let c = self.cond(op.c_condition);
                let mut v = std::mem::take(&mut self.cl[op.c_target as usize]);
                if op.kind != K::BitInvert {
                    // Store 0: v + c v. Store 1: v + c v + c.
                    let d = self.a.mul(&c, &v);
                    self.a.add(&mut v, &d);
                }
                if op.kind != K::BitStore0 {
                    self.a.add(&mut v, &c);
                }
                self.cl[op.c_target as usize] = v;
            }
            K::Neg => {
                let c = self.cond(op.c_condition);
                self.add_phase(&c, &Poly::one());
            }
            K::X | K::CX | K::CCX => {
                if [c1, c2].iter().any(|&c| c != NONE && sys(c).is_some()) {
                    return no("unsupported-op", "a system qubit controls a gate");
                }
                let mut p = self.cond(op.c_condition);
                for c in [c1, c2] {
                    if c != NONE {
                        let v = self.rd(c);
                        p = self.a.mul(&p, &v);
                    }
                }
                match sys(t) {
                    Some(q) => self.log.push((i, Sys::X { q, c: p })),
                    None => {
                        let mut v = std::mem::take(&mut self.val[t as usize]);
                        self.a.add(&mut v, &p);
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
                let c = self.cond(op.c_condition);
                let mut w = Poly::one();
                for &q in qs.iter().filter(|&&q| sys(q).is_none()) {
                    let v = self.rd(q);
                    w = self.a.mul(&w, &v);
                }
                match on[..] {
                    [q] => {
                        let c = self.a.mul(&c, &w);
                        self.log.push((i, Sys::Z { q, c }));
                    }
                    _ => self.add_phase(&c, &w),
                }
            }
            K::Swap => {
                if sys(t).is_some() || sys(c1).is_some() {
                    return no("unsupported-op", "Swap on a system qubit");
                }
                let c = self.cond(op.c_condition);
                if c.is_one() {
                    self.val.swap(t as usize, c1 as usize);
                } else if !c.is_zero() {
                    let mut d = self.rd(t);
                    let other = self.rd(c1);
                    self.a.add(&mut d, &other);
                    let d = self.a.mul(&c, &d);
                    for q in [t, c1] {
                        let mut v = std::mem::take(&mut self.val[q as usize]);
                        self.a.add(&mut v, &d);
                        self.val[q as usize] = v;
                    }
                }
            }
            K::Hmr => {
                if t < self.first {
                    return no("unsupported-op", "Hmr on a qubit that is not an ancilla");
                }
                self.counts.measurements += 1;
                let c = self.cond(op.c_condition);
                // On the lanes where `c` holds: sign (-1)^(m v), the qubit becomes 0, the bit m.
                let m = self.a.primary(Kind::Outcome(i));
                let m = self.a.sym_poly(m);
                let v = self.rd(t);
                let cm = self.a.mul(&c, &m);
                self.add_phase(&cm, &v);
                let b = op.c_target as usize;
                if c.is_one() {
                    self.val[t as usize] = Poly::zero();
                    self.cl[b] = m;
                } else {
                    let d = self.a.mul(&c, &v);
                    let mut w = v;
                    self.a.add(&mut w, &d);
                    self.val[t as usize] = w;
                    let mut bit = std::mem::take(&mut self.cl[b]);
                    let mut e = bit.clone();
                    self.a.add(&mut e, &m);
                    let e = self.a.mul(&c, &e);
                    self.a.add(&mut bit, &e);
                    self.cl[b] = bit;
                }
            }
            K::R => {
                if t < self.first {
                    return no("unsupported-op", "R on a qubit that is not an ancilla");
                }
                // Stricter than the simulator inside a condition block: 0 on every lane.
                let v = std::mem::take(&mut self.val[t as usize]);
                if let Err(why) = self.prove_zero(&v) {
                    return Err(obstruct(
                        at,
                        "reset-not-proven",
                        format!("ancilla q{t} is not proven 0 when freed: {why}"),
                    ));
                }
                self.counts.resets_proved += 1;
            }
            K::Givens => {
                if !self.cond(op.c_condition).is_one() {
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
                    if self.val.len() <= x as usize {
                        self.val.resize(x as usize + 1, Poly::zero());
                    }
                    angle.push(self.rd(x));
                }
                let sign = Poly::zero();
                self.counts.givens += 1;
                self.log.push((i, Sys::G { p, q, angle, sign }));
            }
            K::SpinSwap | K::SpinSwapDg => {
                if sys(t).is_some() {
                    return no("unsupported-op", "SpinSwap controlled by a system qubit");
                }
                let c = self.cond(op.c_condition);
                let v = self.rd(t);
                let c = self.a.mul(&c, &v);
                let dagger = op.kind == K::SpinSwapDg;
                self.counts.spin_swaps += 1;
                self.log.push((i, Sys::F { dagger, c }));
            }
            K::Reflect => self.reflect(i, op)?,
        }
        Ok(())
    }

    /// `Reflect`: the inner register must hold its initial value; it then holds new symbols.
    fn reflect(&mut self, i: usize, op: &Op) -> Result<(), Obstruction> {
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
        for k in 0..self.inner.len() {
            let q = self.inner[k];
            let mut d = self.val[q as usize].clone();
            d.add(&self.a.sym_poly(self.initial[q as usize]));
            if let Err(why) = self.prove_zero(&d) {
                return Err(obstruct(
                    at,
                    "reflect-not-restored",
                    format!(
                        "inner uniform qubit q{q} is not proven to hold its initial value at the \
                         Reflect: {why}"
                    ),
                ));
            }
            let s = self.a.primary(Kind::Second(q));
            self.second.push(s);
            self.val[q as usize] = self.a.sym_poly(s);
        }
        self.reflected = true;
        Ok(())
    }

    /// End of stream: every register restored and every ancilla 0.
    fn finish(&mut self) -> Result<(), Obstruction> {
        let no = |kind: &'static str, why: String| Err(obstruct(None, kind, why));
        if !self.stack.is_empty() {
            return no("structure", "a condition block is left open".into());
        }
        if !self.reflected {
            return no("structure", "the stream has no Reflect".into());
        }
        for q in 0..self.val.len() as u32 {
            if self.l.sys(q).is_some() {
                continue;
            }
            let mut d = self.val[q as usize].clone();
            let kind = if let Some(k) = self.inner.iter().position(|&x| x == q) {
                d.add(&self.a.sym_poly(self.second[k]));
                "registers-not-restored"
            } else if q < self.first {
                d.add(&self.a.sym_poly(self.initial[q as usize]));
                "registers-not-restored"
            } else {
                "ancilla-not-clean"
            };
            if let Err(why) = self.prove_zero(&d) {
                let what = if q < self.first {
                    "is not proven to hold the value the lane must end with"
                } else {
                    "is not proven 0 at the end"
                };
                return no(kind, format!("qubit q{q} {what}: {why}"));
            }
        }
        if self.a.exhausted {
            return Err(self.budget(None));
        }
        self.registers_restored = true;
        Ok(())
    }

    /// Every coefficient of a product of outcomes in the lane's sign is identically zero.
    fn phases(&mut self) -> Result<(), Obstruction> {
        let mut keys: Vec<Mono> = self.phi.keys().copied().filter(|&k| k != ONE).collect();
        keys.sort_unstable();
        for k in keys {
            let p = self.phi.remove(&k).unwrap_or_default();
            if let Err(why) = self.prove_zero(&p) {
                let hmr: Vec<usize> = self
                    .a
                    .syms(k)
                    .iter()
                    .filter_map(|&s| match self.a.kind(s) {
                        Kind::Outcome(i) => Some(i),
                        _ => None,
                    })
                    .collect();
                return Err(Obstruction {
                    op: hmr.first().copied(),
                    name: Some(K::Hmr.name()),
                    kind: "outcome-phase",
                    reason: format!(
                        "the lane's sign depends on the outcomes of the HMR ops {hmr:?}: their \
                         phase coefficient is not cancelled: {why}"
                    ),
                });
            }
            self.counts.outcome_phases_proved += 1;
        }
        if self.a.exhausted {
            return Err(self.budget(None));
        }
        Ok(())
    }

    /// The symbol `s` with the first- and second-pass inner symbols exchanged. A sealed symbol
    /// maps to the sealed symbol of its mapped definition. `Err` carries an `Outcome` symbol's
    /// op when `s` depends on one.
    fn twin_sym(&mut self, s: Sym) -> Result<Sym, usize> {
        let mut todo = vec![s];
        while let Some(&x) = todo.last() {
            if self.twin.len() < self.a.symbols() as usize {
                self.twin.resize(self.a.symbols() as usize, NO_SYM);
            }
            if self.twin[x as usize] != NO_SYM {
                todo.pop();
                continue;
            }
            let y = match self.a.kind(x) {
                Kind::Outcome(i) => return Err(i),
                Kind::Initial(q) => match self.inner.iter().position(|&w| w == q) {
                    Some(k) => self.second[k],
                    None => x,
                },
                Kind::Second(q) => self.initial[q as usize],
                Kind::Sealed => {
                    let Some(def) = self.a.def(x) else {
                        return Err(usize::MAX);
                    };
                    self.a.tick(def.len());
                    let before = todo.len();
                    for &m in def.terms() {
                        for &c in self.a.syms(m).iter() {
                            if self.twin[c as usize] == NO_SYM {
                                todo.push(c);
                            }
                        }
                    }
                    if todo.len() > before {
                        continue;
                    }
                    let mapped = self.twin_mapped(&def);
                    if mapped.is_zero() {
                        // Cannot happen for a renaming of a nonzero polynomial; refuse anyway.
                        return Err(usize::MAX);
                    }
                    self.a.seal(&mapped)
                }
            };
            if self.twin.len() < self.a.symbols() as usize {
                self.twin.resize(self.a.symbols() as usize, NO_SYM);
            }
            self.twin[x as usize] = y;
            todo.pop();
        }
        Ok(self.twin[s as usize])
    }

    /// `p` with every symbol replaced by its twin; every symbol's twin is already known.
    fn twin_mapped(&mut self, p: &Poly) -> Poly {
        self.a.tick(p.len());
        let mut out = Vec::with_capacity(p.len());
        for &m in p.terms() {
            let mut syms: Vec<Sym> = self
                .a
                .syms(m)
                .iter()
                .map(|&s| self.twin[s as usize])
                .collect();
            syms.sort_unstable();
            syms.dedup();
            out.push(self.a.intern(&syms));
        }
        Poly::from_terms(out)
    }

    fn twin_poly(&mut self, p: &Poly) -> Result<Poly, usize> {
        for &m in p.terms() {
            for &s in self.a.syms(m).iter() {
                self.twin_sym(s)?;
            }
        }
        Ok(self.twin_mapped(p))
    }

    /// The normal form of a time-ordered list of system ops: every `Z` moved to the end.
    /// Returns the remaining ops, the final `Z` frame and the sign picked up on the way, so that
    /// the list's operator is `(-1)^sign * Z^frame * (last op) ... (first op)`.
    fn normal(&mut self, seq: &[(usize, Sys)]) -> (Vec<(usize, Sys)>, Vec<Poly>, Poly) {
        let n = self.l.system;
        let mut zf = vec![Poly::zero(); n];
        let mut out = Vec::new();
        let mut sign = Poly::zero();
        for (at, op) in seq {
            match op {
                Sys::Z { q, c } => self.a.add(&mut zf[*q], c),
                // X^c Z^f = (-1)^(c f) Z^f X^c.
                Sys::X { q, c } => {
                    let d = self.a.mul(c, &zf[*q]);
                    self.a.add(&mut sign, &d);
                    out.push((*at, op.clone()));
                }
                // G_pq(t) Z_p = Z_p G_pq(-t), the same for Z_q; other Z commute with G_pq.
                Sys::G {
                    p,
                    q,
                    angle,
                    sign: s,
                } => {
                    let mut s = s.clone();
                    self.a.add(&mut s, &zf[*p]);
                    self.a.add(&mut s, &zf[*q]);
                    let g = Sys::G {
                        p: *p,
                        q: *q,
                        angle: angle.clone(),
                        sign: s,
                    };
                    out.push((*at, g));
                }
                // F exchanges Z_2k and Z_2k+1 (F^dagger too): where c is 1 the pair's frame
                // bits are exchanged.
                Sys::F { c, .. } => {
                    for k in 0..n / 2 {
                        let mut d = zf[2 * k].clone();
                        self.a.add(&mut d, &zf[2 * k + 1]);
                        let d = self.a.mul(c, &d);
                        self.a.add(&mut zf[2 * k], &d);
                        self.a.add(&mut zf[2 * k + 1], &d);
                    }
                    out.push((*at, op.clone()));
                }
            }
        }
        (out, zf, sign)
    }

    /// Stage 2's last part and stage 3: nothing left names an outcome, and the adjoint's normal
    /// form equals the twin of the lane operator's.
    #[allow(clippy::too_many_lines)]
    fn compare(&mut self) -> Result<(), Obstruction> {
        self.counts.system_ops = self.log.len() as u64;
        let log = std::mem::take(&mut self.log);
        let (list, frame, crossing) = self.normal(&log);
        let mut sign = self.phi.get(&ONE).cloned().unwrap_or_default();
        self.a.add(&mut sign, &crossing);
        // The twin of everything the lane operator is made of. An outcome anywhere is refused.
        let steered = |at: Option<usize>, i: usize, what: &str| Obstruction {
            op: at,
            name: None,
            kind: "outcome-steers-system",
            reason: match i {
                usize::MAX => format!("{what} names a symbol the check cannot map"),
                _ => format!("{what} depends on the outcome of the HMR at op {i}"),
            },
        };
        let mut twins: Vec<Sys> = Vec::with_capacity(list.len());
        for (at, op) in &list {
            let at = Some(*at);
            let t = match op {
                Sys::X { q, c } => Sys::X {
                    q: *q,
                    c: self
                        .twin_poly(c)
                        .map_err(|i| steered(at, i, "a system X's control"))?,
                },
                Sys::F { dagger, c } => Sys::F {
                    dagger: *dagger,
                    c: self
                        .twin_poly(c)
                        .map_err(|i| steered(at, i, "a SpinSwap's control"))?,
                },
                Sys::G { p, q, angle, sign } => {
                    let mut bits = Vec::with_capacity(angle.len());
                    for b in angle {
                        bits.push(
                            self.twin_poly(b)
                                .map_err(|i| steered(at, i, "a Givens angle"))?,
                        );
                    }
                    Sys::G {
                        p: *p,
                        q: *q,
                        angle: bits,
                        sign: self
                            .twin_poly(sign)
                            .map_err(|i| steered(at, i, "a system Z's control"))?,
                    }
                }
                Sys::Z { .. } => return Err(steered(at, usize::MAX, "a normal-form element")),
            };
            twins.push(t);
        }
        let mut frame_twin = Vec::with_capacity(frame.len());
        for f in &frame {
            frame_twin.push(
                self.twin_poly(f)
                    .map_err(|i| steered(None, i, "a system Z's control"))?,
            );
        }
        let sign_twin = self.twin_poly(&sign).map_err(|i| Obstruction {
            op: (i != usize::MAX).then_some(i),
            name: Some(K::Hmr.name()),
            kind: "outcome-phase",
            reason: format!(
                "the lane's sign depends on the outcome of the HMR at op {i} through a value the \
                 check does not expand"
            ),
        })?;
        if self.a.exhausted {
            return Err(self.budget(None));
        }
        self.outcome_independent = true;

        let adj = adjoint(&list, &frame);
        let (adj_list, adj_frame, adj_crossing) = self.normal(&adj);
        let mut adj_sign = sign.clone();
        self.a.add(&mut adj_sign, &adj_crossing);

        let differ = |at: Option<usize>, kind: &'static str, reason: String| Obstruction {
            op: at,
            name: None,
            kind,
            reason,
        };
        debug_assert_eq!(adj_list.len(), twins.len());
        for (k, ((_, got), want)) in adj_list.iter().zip(&twins).enumerate() {
            // Named by the op the lane operator has at this position.
            let at = Some(list[k].0);
            let here = format!("element {k} of {} of the normal form", twins.len());
            match (got, want) {
                (Sys::X { q, c }, Sys::X { q: q2, c: c2 }) if q == q2 => {
                    self.same(c, c2).map_err(|why| {
                        differ(
                            at,
                            "control-mismatch",
                            format!(
                                "{here}: the X on system qubit {q} has a different control in \
                                 the adjoint: {why}"
                            ),
                        )
                    })?;
                }
                (Sys::F { dagger, c }, Sys::F { dagger: d2, c: c2 }) if dagger == d2 => {
                    self.same(c, c2).map_err(|why| {
                        differ(
                            at,
                            "control-mismatch",
                            format!(
                                "{here}: the SpinSwap has a different control in the adjoint: \
                                 {why}"
                            ),
                        )
                    })?;
                }
                (
                    Sys::G { p, q, angle, sign },
                    Sys::G {
                        p: p2,
                        q: q2,
                        angle: a2,
                        sign: s2,
                    },
                ) if p == p2 && q == q2 && angle.len() == a2.len() => {
                    for (j, (x, y)) in angle.iter().zip(a2).enumerate() {
                        self.same(x, y).map_err(|why| {
                            differ(
                                at,
                                "angle-mismatch",
                                format!(
                                    "{here}: bit {j} of the angle of the Givens on modes ({p}, \
                                     {q}) differs in the adjoint: {why}"
                                ),
                            )
                        })?;
                    }
                    self.same(sign, s2).map_err(|why| {
                        differ(
                            at,
                            "angle-mismatch",
                            format!(
                                "{here}: the sign of the angle of the Givens on modes ({p}, {q}) \
                                 differs in the adjoint: {why}"
                            ),
                        )
                    })?;
                }
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
            }
            self.counts.compared += 1;
        }
        for (q, (got, want)) in adj_frame.iter().zip(&frame_twin).enumerate() {
            self.same(got, want).map_err(|why| {
                differ(
                    None,
                    "frame-mismatch",
                    format!("the Z left on system qubit {q} differs in the adjoint: {why}"),
                )
            })?;
        }
        self.same(&adj_sign, &sign_twin).map_err(|why| {
            differ(
                None,
                "phase-mismatch",
                format!("the lane's sign differs in the adjoint: {why}"),
            )
        })?;
        if self.a.exhausted {
            return Err(self.budget(None));
        }
        self.hermitian = true;
        Ok(())
    }

    /// `Ok` when `p = q` is proven.
    fn same(&mut self, p: &Poly, q: &Poly) -> Result<(), String> {
        let mut d = p.clone();
        self.a.add(&mut d, q);
        self.prove_zero(&d)
    }
}
