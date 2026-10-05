//! Shared machinery for the lane-engine equivalence tests (spec/FAST-EVALUATOR.md section 10):
//! artifact-level comparison of the reference engine and a candidate under several seed modes and
//! thread counts, op-stream mutation operators, and a generator of small random op streams.
//!
//! Environment (all optional):
//! - `FEMOCO_EQUIV_ENGINE`: the candidate engine (`femoco_walk::equiv::harness_engine`); default
//!   `reference`, which checks the harness and the reference's thread-count determinism;
//! - `FEMOCO_EQUIV_THREADS`: the candidate's thread counts (default `1,4`);
//! - `FEMOCO_EQUIV_REF_THREADS`: the reference's (default 4);
//! - `FEMOCO_EQUIV_K`: sampled lanes per artifact run (default 4096);
//! - `FEMOCO_EQUIV_SEEDS`: seed modes per artifact (default 3: ordinary, server, audit);
//! - `FEMOCO_EQUIV_LOG`: a JSONL file every comparison is appended to.
#![allow(dead_code)]
use femoco_walk::circuit::{read_ops, write_ops, Op, OperationType as K, OpsFile, NONE};
use femoco_walk::equiv::{self, Seed};
use femoco_walk::fiat_shamir::AuditBeacon;
use femoco_walk::score::Inputs;
use femoco_walk::sim::validate::Engine;
use femoco_walk::sim::{givens_tracker, monomial_to_frame, Frame};
use femoco_walk::spec::{EncodingSpec, Monomial};
use femoco_walk::taxonomy;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

pub fn samples() -> usize {
    env_usize("FEMOCO_EQUIV_K", 4096)
}

pub fn ref_threads() -> usize {
    env_usize("FEMOCO_EQUIV_REF_THREADS", 4)
}

/// The candidate engine: `FEMOCO_EQUIV_ENGINE`, or the reference itself.
pub fn candidate() -> (String, Engine) {
    equiv::env_engine().unwrap_or_else(|| {
        (
            "reference".into(),
            femoco_walk::sim::validate::reference as Engine,
        )
    })
}

/// The seed modes for a named case: the ordinary stream, a server seed and a fresh-seed audit
/// beacon, then alternately more server and audit seeds (`FEMOCO_EQUIV_SEEDS` in all).
pub fn seeds(name: &str) -> Vec<Seed> {
    let n = env_usize("FEMOCO_EQUIV_SEEDS", 3).max(1);
    (0..n)
        .map(|i| match i {
            0 => Seed::Ordinary,
            i if i % 2 == 1 => Seed::Server(equiv::seed_bytes(&format!("{name}/server/{i}"))),
            i => Seed::Audit(AuditBeacon {
                round: 1_000_000 + i as u64,
                randomness: equiv::seed_bytes(&format!("{name}/audit/{i}")),
            }),
        })
        .collect()
}

static TMP: AtomicU64 = AtomicU64::new(0);

/// `ops` through `ops.bin` on disk (the writer, the reader's validation and its digest).
pub fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-equiv-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path);
    std::fs::remove_dir_all(&dir).ok();
    f.unwrap()
}

/// The production taxonomy check.
pub fn shipped_check(
) -> impl Fn(&taxonomy::Family, &femoco_walk::facts::CircuitFacts) -> Vec<taxonomy::AxisVerdict> {
    let tax = taxonomy::load_taxonomy(&root().join("taxonomy/taxonomy.json")).unwrap();
    move |f, facts| taxonomy::check(&tax, f, facts)
}

/// What one artifact comparison found.
#[derive(Debug, Default)]
pub struct Summary {
    pub comparisons: usize,
    pub accepted: usize,
    pub rejected: usize,
    /// Per seed: `accepted` or the reference's rejection category.
    pub classes: Vec<String>,
    pub differences: Vec<String>,
}

/// Compares the candidate with the reference on one artifact under every seed mode and the
/// candidate's thread counts. Returns the summary; [`assert_equal`] fails the test on any
/// difference.
pub fn compare_artifact(
    name: &str,
    inp: &Inputs<'_>,
    seeds: &[Seed],
    cand: &(String, Engine),
    threads: &[usize],
) -> Summary {
    let mut s = Summary::default();
    for seed in seeds {
        let r = equiv::run(
            inp,
            "reference",
            femoco_walk::sim::validate::reference,
            ref_threads(),
            *seed,
        );
        let verdict = match &r.result {
            Ok(_) => {
                s.accepted += 1;
                s.classes.push("accepted".into());
                "accepted".to_string()
            }
            Err(e) => {
                s.rejected += 1;
                let head = e.split(':').next().unwrap_or("");
                let class = if femoco_walk::sim::validate::CATEGORIES.contains(&head) {
                    head
                } else {
                    "static (before the engine)"
                };
                s.classes.push(class.to_string());
                format!("rejected: {}", e.chars().take(160).collect::<String>())
            }
        };
        for &t in threads {
            let c = equiv::run(inp, &cand.0, cand.1, t, *seed);
            let d = equiv::differences(&r, &c);
            s.comparisons += 1;
            equiv::log(&json!({
                "kind": "artifact",
                "name": name,
                "spec": inp.spec.id(),
                "ops": inp.ops.ops.len(),
                "samples": inp.samples,
                "seed": seed.label(),
                "engine": cand.0,
                "threads": t,
                "reference": verdict,
                "verdicts": equiv::verdict_counts(&r.probe.verdicts),
                "equal": d.is_empty(),
                "differences": d,
            }));
            println!(
                "{name} [{}] {} @{t}: {} | reference {verdict} {}",
                seed.label(),
                cand.0,
                if d.is_empty() { "EQUAL" } else { "DIFF" },
                equiv::verdict_counts(&r.probe.verdicts)
            );
            for x in &d {
                s.differences
                    .push(format!("{name} [{}] @{t}: {x}", seed.label()));
            }
        }
    }
    s
}

pub fn assert_equal(s: &Summary) {
    assert!(
        s.differences.is_empty(),
        "{} differences:\n{}",
        s.differences.len(),
        s.differences.join("\n")
    );
}

/// [`compare_artifact`] with the environment's candidate, threads and seeds, asserting equality.
pub fn check_artifact(
    name: &str,
    spec: &dyn EncodingSpec,
    lanemap: &[u8],
    family: &[u8],
    ops: &OpsFile,
) -> Summary {
    let check = shipped_check();
    let inp = Inputs {
        spec,
        lanemap,
        family,
        ops,
        samples: samples(),
        tracker: givens_tracker(spec),
        check: &check,
    };
    let s = compare_artifact(
        name,
        &inp,
        &seeds(name),
        &candidate(),
        &equiv::env_threads(),
    );
    assert_equal(&s);
    s
}

// ---- Deterministic randomness. ----

/// SplitMix64.
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x6A09_E667_F3BC_C909)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn idx(&mut self, n: usize) -> usize {
        self.below(n as u64) as usize
    }
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.idx(v.len())]
    }
}

// ---- Op-stream mutation operators. ----

/// The fixed qubit layout of a circuit plus the ids its stream uses.
#[derive(Clone, Debug)]
pub struct Shape {
    pub system: u32,
    pub uniform: u32,
    /// Inner uniform qubits (nested lane maps): never touched outside the copies.
    pub inner: Vec<u32>,
    pub num_qubits: u32,
    pub num_bits: u32,
    pub num_registers: u32,
}

impl Shape {
    pub fn of(ops: &[Op], system: usize, uniform: u32, inner: Option<(u32, u32)>) -> Self {
        let system = system as u32;
        let first = 1 + system + uniform;
        let mut s = Self {
            system,
            uniform,
            inner: inner
                .map(|(lo, w)| (lo..lo + w).map(|j| 1 + system + j).collect())
                .unwrap_or_default(),
            num_qubits: first,
            num_bits: 0,
            num_registers: 0,
        };
        for op in ops {
            for q in op.qubits() {
                s.num_qubits = s.num_qubits.max(q + 1);
            }
            for b in [op.c_target, op.c_condition] {
                if b != NONE {
                    s.num_bits = s.num_bits.max(b + 1);
                }
            }
            if matches!(op.kind, K::Register) {
                s.num_registers = s.num_registers.max(op.r_target + 1);
            }
        }
        s
    }
    pub fn first_ancilla(&self) -> u32 {
        1 + self.system + self.uniform
    }
    pub fn is_sys(&self, q: u32) -> bool {
        q >= 1 && q <= self.system
    }
    pub fn sys(&self, j: u32) -> u32 {
        1 + j
    }
    pub fn uni(&self, i: u32) -> u32 {
        1 + self.system + i
    }
    /// The uniform bits outside the inner register.
    pub fn outer_uniform(&self) -> Vec<u32> {
        (0..self.uniform)
            .map(|i| self.uni(i))
            .filter(|q| !self.inner.contains(q))
            .collect()
    }
}

/// The op-stream mutations. Each returns `None` when the stream has nothing to apply it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mut {
    /// Remove one op of the given kind.
    Drop(K),
    /// Remove one conditioned op's own condition.
    Uncond(K),
    /// Point one op's `q_target` at another qubit the stream uses.
    Retarget(K),
    /// Point one op's first control at another qubit.
    Recontrol(K),
    /// Point one op's condition at another bit.
    Recond(K),
    /// Duplicate one op of the given kind in place.
    Dup(K),
    /// Insert `X` on a used ancilla at a random point (dirty ancilla).
    InsertAncillaX,
    /// Insert a Pauli `X` or `Z` on a random system qubit (wrong term, frame hand-off).
    InsertSys,
    /// Insert `S` on a random system qubit (not a Pauli; tracker error under a Givens).
    InsertSysS,
    /// Insert a `Neg` conditioned on a random bit (phase garbage on part of the lanes).
    InsertNeg,
    /// Swap two adjacent ops.
    SwapAdjacent,
    /// A `Givens` reads another register.
    GivensRegister,
    /// A `Givens` acts on other modes.
    GivensModes,
    /// `SpinSwap` and `SpinSwapDg` exchanged.
    SpinSwapFlip,
    /// An `Hmr` gets a condition bit that is never set: it never executes, so its qubit stays
    /// as it was (documented verifier difference 2: the reference fails such a lane only at a
    /// later `R` or at the end, never at the `Hmr`).
    HmrNeverExecutes,
    /// An `Hmr` gets an existing bit as its own condition (executes on part of the lanes, and
    /// liveness no longer ends there: documented verifier difference 3).
    HmrOwnCondition,
    /// An `R` and an `Hmr` on a never-used ancilla at depth 0 (liveness edge: documented verifier
    /// difference 3, an `R` or `Hmr` on a qubit not yet live).
    LivenessNotLive,
    /// A fresh ancilla is touched inside a condition block and reset there (peak liveness edge).
    LivenessInCondition,
    /// Rare lanes: an AND ladder over the control and `k` uniform bits fixes one pattern and
    /// applies a `Z` on a system qubit there (wrong term on about `2^-(k+1)` of the lanes).
    RareWrongTerm(u32),
    /// As `RareWrongTerm`, leaving a fresh ancilla set on those lanes (dirty at the end).
    RareDirtyEnd(u32),
    /// As `RareDirtyEnd`, then freeing it with `R` (an execution error on those lanes' batches).
    RareDirtyFree(u32),
    /// As `RareWrongTerm` without the control: a `-1` on matching control-0 and control-1 lanes.
    RarePhase(u32),
    /// `S` on a system qubit inside `k` nested conditions on fresh random `Hmr` outcomes: not a
    /// Pauli on about `2^-k` of the lanes, chosen by the outcome stream.
    RareRandomS(u32),
    /// As `RareRandomS` with a `Neg`: phase garbage or a wrong phase on those lanes.
    RareRandomNeg(u32),
    /// Nested circuits: an `X` on an inner uniform qubit at the same point of both inner copies
    /// (the copies stay identical; the `Reflect` sees the wrong inner value).
    TwinInnerX,
    /// Nested circuits: one op removed at the same point of both copies (mutate.py `--twin`).
    TwinDrop,
}

/// Every kind-generic mutation for each kind in `kinds`, then the kind-free ones.
pub fn all_mutations(kinds: &[K], rare_bits: &[u32]) -> Vec<Mut> {
    let mut v = Vec::new();
    for &k in kinds {
        v.extend([
            Mut::Drop(k),
            Mut::Uncond(k),
            Mut::Retarget(k),
            Mut::Recontrol(k),
            Mut::Recond(k),
            Mut::Dup(k),
        ]);
    }
    v.extend([
        Mut::InsertAncillaX,
        Mut::InsertSys,
        Mut::InsertSysS,
        Mut::InsertNeg,
        Mut::SwapAdjacent,
        Mut::GivensRegister,
        Mut::GivensModes,
        Mut::SpinSwapFlip,
        Mut::HmrNeverExecutes,
        Mut::HmrOwnCondition,
        Mut::LivenessNotLive,
        Mut::LivenessInCondition,
        Mut::TwinInnerX,
        Mut::TwinDrop,
    ]);
    for &k in rare_bits {
        v.extend([
            Mut::RareWrongTerm(k),
            Mut::RareDirtyEnd(k),
            Mut::RareDirtyFree(k),
            Mut::RarePhase(k),
            Mut::RareRandomS(k),
            Mut::RareRandomNeg(k),
        ]);
    }
    v
}

/// The kinds that occur in `ops`, in `OperationType::ALL` order.
pub fn kinds_in(ops: &[Op]) -> Vec<K> {
    let mut seen = [false; 64];
    for op in ops {
        seen[op.kind as usize] = true;
    }
    K::ALL.into_iter().filter(|k| seen[*k as usize]).collect()
}

fn op(kind: K, t: u32, c1: u32, c2: u32) -> Op {
    let mut o = Op::new(kind);
    (o.q_target, o.q_control1, o.q_control2) = (t, c1, c2);
    o
}

/// Condition depth before each op (and after the last).
fn depths(ops: &[Op]) -> Vec<i64> {
    let mut d = vec![0i64; ops.len() + 1];
    for (i, o) in ops.iter().enumerate() {
        d[i + 1] = d[i]
            + match o.kind {
                K::PushCondition => 1,
                K::PopCondition => -1,
                _ => 0,
            };
    }
    d
}

/// The inner copies `(b1, e1, b2, e2)` of a nested stream (Segment 3 / 4 indices).
fn copies(ops: &[Op]) -> Option<(usize, usize, usize, usize)> {
    let at = |code| {
        ops.iter()
            .enumerate()
            .filter(|(_, o)| o.kind == K::Segment && o.r_target == code)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    match (&at(3)[..], &at(4)[..]) {
        (&[b1, b2], &[e1, e2]) => Some((b1, e1, b2, e2)),
        _ => None,
    }
}

/// Indices outside the inner copies (every index when the stream is not nested).
fn outside(ops: &[Op]) -> Vec<usize> {
    let c = copies(ops);
    (0..ops.len())
        .filter(|&i| c.is_none_or(|(b1, _, _, e2)| i < b1 || i > e2))
        .collect()
}

/// A depth-0 insertion point outside the inner copies.
fn depth0_point(ops: &[Op], rng: &mut Rng) -> usize {
    let d = depths(ops);
    let c = copies(ops);
    let pts: Vec<usize> = (0..=ops.len())
        .filter(|&i| d[i] == 0 && c.is_none_or(|(b1, _, _, e2)| i <= b1 || i > e2))
        .collect();
    rng.pick(&pts)
}

fn insert(ops: &[Op], at: usize, new: &[Op]) -> Vec<Op> {
    let mut v = Vec::with_capacity(ops.len() + new.len());
    v.extend_from_slice(&ops[..at]);
    v.extend_from_slice(new);
    v.extend_from_slice(&ops[at..]);
    v
}

/// The rare-lane ladder: flips so that `pattern` on the chosen uniform bits reads all ones, an
/// AND chain from `start` over them into fresh ancillas, `action(top)`, and the exact inverse.
fn rare(
    sh: &Shape,
    rng: &mut Rng,
    k: u32,
    with_control: bool,
    action: &dyn Fn(u32, u32) -> Vec<Op>,
) -> Option<Vec<Op>> {
    let bits = sh.outer_uniform();
    let k = (k as usize).min(bits.len());
    if k == 0 || (!with_control && k < 2) {
        return None;
    }
    let chosen: Vec<u32> = bits[..k].to_vec();
    let pattern = rng.next();
    let flips: Vec<Op> = chosen
        .iter()
        .enumerate()
        .filter(|(i, _)| pattern >> i & 1 == 0)
        .map(|(_, &q)| op(K::X, q, NONE, NONE))
        .collect();
    let mut fresh = sh.num_qubits;
    let mut alloc = || {
        fresh += 1;
        fresh - 1
    };
    let mut up = Vec::new();
    let mut ladder = Vec::new();
    let (mut prev, rest) = if with_control {
        (0u32, &chosen[..])
    } else {
        (chosen[0], &chosen[1..])
    };
    for &u in rest {
        let a = alloc();
        up.push(op(K::CCX, a, prev, u));
        ladder.push(a);
        prev = a;
    }
    let spare = alloc();
    let mut v = flips.clone();
    v.extend(up.iter().copied());
    v.extend(action(prev, spare));
    v.extend(up.iter().rev().copied());
    v.extend(ladder.iter().rev().map(|&a| op(K::R, a, NONE, NONE)));
    v.extend(flips);
    Some(v)
}

/// Applies mutation `m` to `ops`, choosing its point with `rng`. `None`: nothing to mutate.
pub fn mutate(ops: &[Op], sh: &Shape, m: Mut, rng: &mut Rng) -> Option<(Vec<Op>, String)> {
    let outside_idx = outside(ops);
    let of_kind = |k: K, extra: &dyn Fn(&Op) -> bool| -> Vec<usize> {
        outside_idx
            .iter()
            .copied()
            .filter(|&i| ops[i].kind == k && extra(&ops[i]))
            .collect()
    };
    let used: Vec<u32> = {
        let mut u: Vec<u32> = ops.iter().flat_map(Op::qubits).collect();
        u.sort_unstable();
        u.dedup();
        u.retain(|q| !sh.inner.contains(q));
        u
    };
    let ancillas: Vec<u32> = used
        .iter()
        .copied()
        .filter(|&q| q >= sh.first_ancilla())
        .collect();
    let mut v = ops.to_vec();
    let desc;
    match m {
        Mut::Drop(k) => {
            let i = *pick_opt(
                rng,
                &of_kind(k, &|o| {
                    !matches!(o.kind, K::PushCondition | K::PopCondition)
                }),
            )?;
            v.remove(i);
            desc = format!("drop op {i} ({})", k.name());
        }
        Mut::Uncond(k) => {
            let i = *pick_opt(
                rng,
                &of_kind(k, &|o| o.c_condition != NONE && o.kind != K::PushCondition),
            )?;
            v[i].c_condition = NONE;
            desc = format!("uncondition op {i} ({})", k.name());
        }
        Mut::Retarget(k) => {
            let i = *pick_opt(rng, &of_kind(k, &|o| o.q_target != NONE))?;
            let q = *pick_opt(rng, &used)?;
            if v[i].qubits().any(|x| x == q) {
                return None;
            }
            v[i].q_target = q;
            desc = format!("retarget op {i} ({}) to q{q}", k.name());
        }
        Mut::Recontrol(k) => {
            let i = *pick_opt(rng, &of_kind(k, &|o| o.q_control1 != NONE))?;
            let q = *pick_opt(rng, &used)?;
            if v[i].qubits().any(|x| x == q) {
                return None;
            }
            v[i].q_control1 = q;
            desc = format!("recontrol op {i} ({}) to q{q}", k.name());
        }
        Mut::Recond(k) => {
            let i = *pick_opt(rng, &of_kind(k, &|o| o.c_condition != NONE))?;
            if sh.num_bits < 2 {
                return None;
            }
            let b = rng.below(u64::from(sh.num_bits)) as u32;
            v[i].c_condition = b;
            desc = format!("recondition op {i} ({}) on bit {b}", k.name());
        }
        Mut::Dup(k) => {
            let i = *pick_opt(
                rng,
                &of_kind(k, &|o| {
                    !matches!(o.kind, K::PushCondition | K::PopCondition | K::Register)
                }),
            )?;
            v.insert(i, ops[i]);
            desc = format!("duplicate op {i} ({})", k.name());
        }
        Mut::InsertAncillaX => {
            let q = *pick_opt(rng, &ancillas)?;
            let at = rng.idx(outside_idx.len() + 1);
            let at = outside_idx.get(at).copied().unwrap_or(ops.len());
            v = insert(ops, at, &[op(K::X, q, NONE, NONE)]);
            desc = format!("insert X q{q} at {at}");
        }
        Mut::InsertSys | Mut::InsertSysS => {
            let q = sh.sys(rng.below(u64::from(sh.system)) as u32);
            let kind = match m {
                Mut::InsertSysS => K::S,
                _ if rng.chance(0.5) => K::X,
                _ => K::Z,
            };
            let at = rng.idx(outside_idx.len() + 1);
            let at = outside_idx.get(at).copied().unwrap_or(ops.len());
            v = insert(ops, at, &[op(kind, q, NONE, NONE)]);
            desc = format!("insert {} on system q{q} at {at}", kind.name());
        }
        Mut::InsertNeg => {
            if sh.num_bits == 0 {
                return None;
            }
            let mut o = Op::new(K::Neg);
            o.c_condition = rng.below(u64::from(sh.num_bits)) as u32;
            // After the last write of that bit, so it is set on part of the lanes.
            let last = ops
                .iter()
                .rposition(|x| x.c_target == o.c_condition)
                .map_or(ops.len(), |i| i + 1);
            let at = outside_idx
                .iter()
                .copied()
                .find(|&i| i >= last)
                .unwrap_or(ops.len());
            v = insert(ops, at, &[o]);
            desc = format!("insert Neg on bit {} at {at}", o.c_condition);
        }
        Mut::SwapAdjacent => {
            let i = *pick_opt(rng, &outside_idx)?;
            if i + 1 >= v.len() || !outside_idx.contains(&(i + 1)) || v[i] == v[i + 1] {
                return None;
            }
            v.swap(i, i + 1);
            desc = format!("swap ops {i} and {}", i + 1);
        }
        Mut::GivensRegister => {
            let i = *pick_opt(rng, &of_kind(K::Givens, &|_| true))?;
            if sh.num_registers < 2 {
                return None;
            }
            let r = rng.below(u64::from(sh.num_registers)) as u32;
            if r == v[i].r_target {
                return None;
            }
            v[i].r_target = r;
            desc = format!("Givens op {i} reads register {r}");
        }
        Mut::GivensModes => {
            let i = *pick_opt(rng, &of_kind(K::Givens, &|_| true))?;
            let p = rng.below(u64::from(sh.system - 1)) as u32;
            let q = p + 1 + rng.below(u64::from(sh.system - 1 - p)) as u32;
            v[i].q_control1 = sh.sys(p);
            v[i].q_target = sh.sys(q);
            if v[i] == ops[i] {
                return None;
            }
            desc = format!("Givens op {i} on modes ({p}, {q})");
        }
        Mut::SpinSwapFlip => {
            let mut idx = of_kind(K::SpinSwap, &|_| true);
            idx.extend(of_kind(K::SpinSwapDg, &|_| true));
            let i = *pick_opt(rng, &idx)?;
            v[i].kind = if v[i].kind == K::SpinSwap {
                K::SpinSwapDg
            } else {
                K::SpinSwap
            };
            desc = format!("op {i} SpinSwap <-> SpinSwapDg");
        }
        Mut::HmrNeverExecutes => {
            let i = *pick_opt(rng, &of_kind(K::Hmr, &|_| true))?;
            // A fresh bit, never written: 0 on every lane.
            v[i].c_condition = sh.num_bits;
            desc = format!("Hmr op {i} conditioned on never-set bit {}", sh.num_bits);
        }
        Mut::HmrOwnCondition => {
            let i = *pick_opt(rng, &of_kind(K::Hmr, &|o| o.c_condition == NONE))?;
            if sh.num_bits == 0 {
                return None;
            }
            let b = rng.below(u64::from(sh.num_bits)) as u32;
            if b == v[i].c_target {
                return None;
            }
            v[i].c_condition = b;
            desc = format!("Hmr op {i} conditioned on bit {b}");
        }
        Mut::LivenessNotLive => {
            let q = sh.num_qubits;
            let at = depth0_point(ops, rng);
            let mut h = op(K::Hmr, q + 1, NONE, NONE);
            h.c_target = sh.num_bits;
            v = insert(ops, at, &[op(K::R, q, NONE, NONE), h]);
            desc = format!("R q{q} and Hmr q{} before first use at {at}", q + 1);
        }
        Mut::LivenessInCondition => {
            if sh.num_bits == 0 {
                return None;
            }
            let q = sh.num_qubits;
            let at = depth0_point(ops, rng);
            let mut push = Op::new(K::PushCondition);
            push.c_condition = rng.below(u64::from(sh.num_bits)) as u32;
            v = insert(
                ops,
                at,
                &[
                    push,
                    op(K::X, q, NONE, NONE),
                    op(K::X, q, NONE, NONE),
                    op(K::R, q, NONE, NONE),
                    Op::new(K::PopCondition),
                ],
            );
            desc = format!("fresh q{q} used and reset inside a condition block at {at}");
        }
        Mut::RareWrongTerm(k)
        | Mut::RareDirtyEnd(k)
        | Mut::RareDirtyFree(k)
        | Mut::RarePhase(k) => {
            let sysq = sh.sys(rng.below(u64::from(sh.system)) as u32);
            let action: Box<dyn Fn(u32, u32) -> Vec<Op>> = match m {
                Mut::RareWrongTerm(_) => Box::new(move |top, _| vec![op(K::CZ, sysq, top, NONE)]),
                Mut::RareDirtyEnd(_) => Box::new(|top, spare| vec![op(K::CX, spare, top, NONE)]),
                Mut::RareDirtyFree(_) => Box::new(|top, spare| {
                    vec![op(K::CX, spare, top, NONE), op(K::R, spare, NONE, NONE)]
                }),
                _ => Box::new(|top, _| vec![op(K::Z, top, NONE, NONE)]),
            };
            let with_control = !matches!(m, Mut::RarePhase(_));
            let block = rare(sh, rng, k, with_control, &*action)?;
            let at = depth0_point(ops, rng);
            v = insert(ops, at, &block);
            desc = format!("{m:?} at {at} (system q{sysq})");
        }
        Mut::RareRandomS(k) | Mut::RareRandomNeg(k) => {
            // `k` fresh ancillas in |0> measured: `k` independent random bits per lane (no phase,
            // the qubits are 0). Nested conditions on all of them select about 2^-k of the lanes.
            let (q0, b0) = (sh.num_qubits, sh.num_bits);
            let mut block = Vec::new();
            for j in 0..k {
                let mut h = op(K::Hmr, q0 + j, NONE, NONE);
                h.c_target = b0 + j;
                block.push(h);
            }
            for j in 0..k {
                let mut p = Op::new(K::PushCondition);
                p.c_condition = b0 + j;
                block.push(p);
            }
            let sysq = sh.sys(rng.below(u64::from(sh.system)) as u32);
            block.push(if m == Mut::RareRandomS(k) {
                op(K::S, sysq, NONE, NONE)
            } else {
                Op::new(K::Neg)
            });
            block.extend((0..k).map(|_| Op::new(K::PopCondition)));
            let at = depth0_point(ops, rng);
            v = insert(ops, at, &block);
            desc = format!("{m:?} at {at} (system q{sysq})");
        }
        Mut::TwinInnerX | Mut::TwinDrop => {
            let (b1, e1, b2, _) = copies(ops)?;
            let len = e1 - b1 - 1;
            if len == 0 {
                return None;
            }
            let d = rng.idx(len);
            let (i1, i2) = (b1 + 1 + d, b2 + 1 + d);
            if m == Mut::TwinDrop {
                if matches!(ops[i1].kind, K::PushCondition | K::PopCondition) {
                    return None;
                }
                v.remove(i2);
                v.remove(i1);
                desc = format!("drop op {d} ({}) of both inner copies", ops[i1].kind.name());
            } else {
                let q = *pick_opt(rng, &sh.inner)?;
                let x = op(K::X, q, NONE, NONE);
                v.insert(i2, x);
                v.insert(i1, x);
                desc = format!("insert X on inner q{q} at op {d} of both copies");
            }
        }
    }
    Some((v, desc))
}

fn pick_opt<'a, T>(rng: &mut Rng, v: &'a [T]) -> Option<&'a T> {
    (!v.is_empty()).then(|| &v[rng.idx(v.len())])
}

// ---- Random small op streams (the property fuzzer). ----

/// A Hermitian monomial on `majoranas` (`n` system qubits), and its frame.
pub fn hermitian(majoranas: &[u16], n: usize, negative: bool) -> (Monomial, Frame) {
    for p in 0..2u8 {
        let m = Monomial {
            phase: p + 2 * u8::from(negative),
            majoranas: majoranas.to_vec(),
        };
        if let Ok(f) = monomial_to_frame(&m, n) {
            return (m, f);
        }
    }
    unreachable!("some phase is Hermitian")
}

/// A random Hermitian monomial on `n` system qubits (`2n` Majoranas).
pub fn random_monomial(rng: &mut Rng, n: usize) -> (Monomial, Frame) {
    let mut ms: Vec<u16> = (0..2 * n as u16).filter(|_| rng.chance(0.3)).collect();
    if ms.is_empty() && rng.chance(0.5) {
        ms.push(rng.below(2 * n as u64) as u16);
    }
    hermitian(&ms, n, rng.chance(0.5))
}

// ---- Artifacts built in process (what build_circuit would write). ----

/// A built artifact: the spec, `lanemap.bin`, `family.out.json` and `ops.bin` (through disk).
pub struct Artifact {
    pub name: String,
    pub spec: Box<dyn EncodingSpec>,
    pub lanemap: Vec<u8>,
    pub family: Vec<u8>,
    pub ops: OpsFile,
    pub uniform: u32,
    pub inner: Option<(u32, u32)>,
}

/// Builds spec `id` with `build` and declares `family`, as `build_circuit` does.
pub fn artifact(
    name: &str,
    id: &str,
    family: femoco_walk::taxonomy::Family,
    build: impl FnOnce(
        &dyn EncodingSpec,
        &mut femoco_walk::circuit::Builder,
    ) -> Box<dyn femoco_walk::lanemap::LaneMap>,
) -> Artifact {
    let spec = femoco_walk::spec::load(root(), id).unwrap();
    let mut b = femoco_walk::circuit::Builder::new(spec.system_qubits());
    let lm = build(spec.as_ref(), &mut b);
    let ops = b.finish();
    let file = ops_file(&ops);
    drop(ops);
    let fam = serde_json::to_vec_pretty(&femoco_walk::score::FamilyOut {
        family,
        spec: id.to_string(),
    })
    .unwrap();
    Artifact {
        name: name.to_string(),
        uniform: lm.uniform_bits(),
        inner: lm.inner_bits(),
        lanemap: lm.to_bytes(),
        family: fam,
        ops: file,
        spec,
    }
}

impl Artifact {
    pub fn check(&self) -> Summary {
        check_artifact(
            &self.name,
            self.spec.as_ref(),
            &self.lanemap,
            &self.family,
            &self.ops,
        )
    }
}

/// Mutants of one artifact: every mutation of [`all_mutations`] for the op kinds it has, `per`
/// times each with different points, compared under `seeds`. Returns the totals.
#[allow(clippy::too_many_arguments)]
pub fn mutant_sweep(
    name: &str,
    spec: &dyn EncodingSpec,
    lanemap: &[u8],
    family: &[u8],
    ops: &[Op],
    uniform: u32,
    inner: Option<(u32, u32)>,
    check: &dyn Fn(
        &taxonomy::Family,
        &femoco_walk::facts::CircuitFacts,
    ) -> Vec<taxonomy::AxisVerdict>,
    rare_bits: &[u32],
    per: usize,
    seeds_per_mutant: usize,
    max_mutations: usize,
) -> Summary {
    let sh = Shape::of(ops, spec.system_qubits(), uniform, inner);
    let mut muts = all_mutations(&kinds_in(ops), rare_bits);
    if muts.len() > max_mutations {
        // A deterministic spread over the list (every kind-free mutation stays: they are last).
        let fixed = muts.split_off(muts.len() - (14 + 7 * rare_bits.len()).min(muts.len()));
        let keep = max_mutations.saturating_sub(fixed.len()).max(1);
        let step = muts.len().div_ceil(keep).max(1);
        muts = muts.into_iter().step_by(step).chain(fixed).collect();
    }
    let cand = candidate();
    let threads = equiv::env_threads();
    let mut total = Summary::default();
    let mut categories = std::collections::BTreeMap::<String, u64>::new();
    let mut applied = 0usize;
    for (mi, m) in muts.iter().enumerate() {
        for j in 0..per {
            let mut rng = Rng::new((mi as u64) << 16 ^ j as u64 ^ 0x5EED);
            let Some((mops, desc)) = mutate(ops, &sh, *m, &mut rng) else {
                continue;
            };
            applied += 1;
            let file = ops_file(&mops);
            let inp = Inputs {
                spec,
                lanemap,
                family,
                ops: &file,
                samples: samples(),
                tracker: givens_tracker(spec),
                check,
            };
            let label = format!("{name} {m:?}#{j}: {desc}");
            let all = seeds(&label);
            let sd: Vec<Seed> = all.into_iter().take(seeds_per_mutant.max(1)).collect();
            let s = compare_artifact(&label, &inp, &sd, &cand, &threads);
            total.comparisons += s.comparisons;
            total.accepted += s.accepted;
            total.rejected += s.rejected;
            total.differences.extend(s.differences);
            let class = s.classes.first().cloned().unwrap_or_default();
            *categories.entry(class).or_default() += 1;
        }
    }
    let line = json!({
        "kind": "mutant-sweep",
        "name": name,
        "engine": cand.0,
        "mutations": muts.len(),
        "mutants": applied,
        "comparisons": total.comparisons,
        "outcomes": categories,
        "differences": total.differences.len(),
    });
    equiv::log(&line);
    println!("{}", serde_json::to_string_pretty(&line).unwrap());
    total
}
