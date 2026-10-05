//! Shared fixtures for the harness tests: a small synthetic flat spec (six spin orbitals, five
//! Majorana terms with dyadic weights), a naive hand-written controlled SELECT over the alias
//! lane map, and mutations of it.
#![allow(dead_code)]
use femoco_walk::circuit::{read_ops, write_ops, Builder, Op, OpsFile, Qubit, SEG_SELECT};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::facts::CircuitFacts;
use femoco_walk::lanemap::alias::AliasMap;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{Evaluation, FamilyOut, Inputs};
use femoco_walk::sim::{monomial_to_frame, Frame, TrackerFactory};
use femoco_walk::spec::{EncodingSpec, Exact, Monomial, SystemOp};
use femoco_walk::taxonomy::{AxisStatus, AxisVerdict, Family};
use num_bigint::BigInt;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub const SPATIAL: usize = 3;
pub const N: usize = 2 * SPATIAL;

pub struct TestSpec {
    pub terms: Vec<(Exact, SystemOp)>,
}

/// The Hermitian monomial on `majoranas` (phase 0 or 1), negated when `negative`.
#[must_use]
pub fn herm(majoranas: &[u16], negative: bool) -> Monomial {
    let phase = (0..2u8)
        .find(|&p| {
            let m = Monomial {
                phase: p,
                majoranas: majoranas.to_vec(),
            };
            monomial_to_frame(&m, N).is_ok()
        })
        .expect("some phase is Hermitian");
    Monomial {
        phase: phase + 2 * u8::from(negative),
        majoranas: majoranas.to_vec(),
    }
}

fn dy(m: i64, e: u32) -> Exact {
    Exact::dyadic(BigInt::from(m), e)
}

impl TestSpec {
    /// Five terms, weights 1/2, 1/4, 1/8, 1/16, 1/16 (lambda = 1), one with a negative sign.
    #[must_use]
    pub fn new() -> Self {
        let t = |w: Exact, m: &[u16], neg: bool| (w, SystemOp::Monomial(herm(m, neg)));
        Self {
            terms: vec![
                t(dy(1, 1), &[0, 1], false),
                t(dy(1, 2), &[2, 5], true),
                t(dy(1, 3), &[0, 3, 6, 9], false),
                t(dy(1, 4), &[7], false),
                t(dy(1, 4), &[1, 4, 8, 10, 11], false),
            ],
        }
    }
}

impl EncodingSpec for TestSpec {
    fn id(&self) -> &str {
        "test-flat-v1"
    }
    fn encoding(&self) -> &str {
        "test"
    }
    fn spatial_orbitals(&self) -> usize {
        SPATIAL
    }
    fn lambda(&self) -> Exact {
        self.terms.iter().fold(Exact::zero(), |a, (c, _)| a.add(c))
    }
    fn identity(&self) -> Exact {
        Exact::zero()
    }
    fn payload_sha256(&self) -> [u8; 32] {
        Sha256::digest(format!("{:?}", self.terms).as_bytes()).into()
    }
    fn flat_terms(&self) -> Option<Vec<(Exact, SystemOp)>> {
        Some(self.terms.clone())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The exact alias table for the spec's counts (32, 16, 8, 4, 4 of 64 lanes).
#[must_use]
pub fn alias() -> AliasMap {
    AliasMap::from_counts(3, 3, dy(1, 0), &[32, 16, 8, 4, 4]).unwrap()
}

/// Ways to break the hand-written circuit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    None,
    /// Flip the sign of every lane that selects this term.
    FlipSign(usize),
    /// Apply an extra X on system qubit 5 for this term.
    WrongTerm(usize),
    /// Leave the ladder ancilla of uniform value `s` set and free it with `R`.
    DirtyFree(u64),
    /// Leave the ladder ancilla of uniform value `s` set and never free it.
    DirtyEnd(u64),
    /// Kick a -1 phase onto every lane with uniform bit 0 set, regardless of control.
    PhaseGarbage,
    /// Flip the sign on exactly one uniform value.
    SignAt(u64),
    /// Apply the terms without the walk control.
    NoControl,
}

fn frame_of(op: &SystemOp) -> Frame {
    monomial_to_frame(op.as_monomial().unwrap(), N).unwrap()
}

/// Applies `w^p X^x Z^z` to the system, controlled by `a` (Z part first, then X, then phase).
fn apply_frame(b: &mut Builder, a: Qubit, f: &Frame) {
    for q in (0..N).filter(|&q| f.z_bit(q)) {
        let t = b.system(q);
        b.cz(a, t);
    }
    for q in (0..N).filter(|&q| f.x_bit(q)) {
        let t = b.system(q);
        b.cx(a, t);
    }
    match f.phase {
        2 => b.s(a),
        4 => b.z(a),
        6 => b.sdg(a),
        _ => {}
    }
}

/// A naive controlled SELECT: for every uniform value `s`, an AND ladder of the control and the
/// uniform bits (matched to `s`) selects the lane, the term's Pauli is applied controlled on the
/// ladder's top, and the ladder is uncomputed (the top by measurement: `Hmr` + `cz_if`).
#[must_use]
pub fn build(spec: &TestSpec, map: &AliasMap, m: Mutation) -> Vec<Op> {
    let mut b = Builder::new(N);
    let u = map.uniform_bits();
    b.declare_uniform(u);
    b.segment(SEG_SELECT);
    for s in 0..1u64 << u {
        let t = (0..spec.terms.len())
            .find(|&t| spec.terms[t].1 == map.reference_op(spec, s))
            .unwrap();
        let mut f = frame_of(&spec.terms[t].1);
        if m == Mutation::FlipSign(t) || m == Mutation::SignAt(s) {
            f.phase = (f.phase + 4) % 8;
        }
        if m == Mutation::WrongTerm(t) {
            f.x[0] ^= 1 << 5;
        }
        select_one(&mut b, s, &f, m);
    }
    if m == Mutation::PhaseGarbage {
        let a = b.alloc();
        let u0 = b.uniform(0);
        b.cx(u0, a);
        b.z(a);
        b.cx(u0, a);
        b.free(a);
    }
    b.finish()
}

fn select_one(b: &mut Builder, s: u64, f: &Frame, m: Mutation) {
    let u = b.uniform_bits();
    let flips: Vec<Qubit> = (0..u)
        .filter(|&i| s >> i & 1 == 0)
        .map(|i| b.uniform(i))
        .collect();
    flips.iter().for_each(|&q| b.x(q));
    let mut ladder = Vec::new();
    let mut prev = b.control();
    if m == Mutation::NoControl {
        prev = b.uniform(0);
    }
    for i in 0..u {
        let ui = b.uniform(i);
        if ui == prev {
            continue;
        }
        let a = b.alloc();
        b.ccx(prev, ui, a);
        ladder.push((prev, ui, a));
        prev = a;
    }
    apply_frame(b, prev, f);
    let (c1, c2, top) = ladder.pop().unwrap();
    if m == Mutation::DirtyEnd(s) {
        // Leak the top without uncomputing it; the rest of the ladder unwinds normally.
    } else if m == Mutation::DirtyFree(s) {
        b.free(top);
    } else {
        let bit = b.hmr(top);
        b.cz_if(c1, c2, bit);
    }
    while let Some((c1, c2, a)) = ladder.pop() {
        b.ccx(c1, c2, a);
        b.free(a);
    }
    flips.iter().for_each(|&q| b.x(q));
}

static TMP: AtomicU64 = AtomicU64::new(0);

/// Round-trips `ops` through `ops.bin` on disk (exercising the writer and the reader).
#[must_use]
pub fn ops_file(ops: &[Op]) -> OpsFile {
    let n = TMP.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("femoco-harness-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(ops, &path).unwrap();
    let f = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    f
}

#[must_use]
pub fn family(spec: &str) -> Vec<u8> {
    let axes: BTreeMap<String, String> = [
        ("encoding", "test"),
        ("lane_map", "alias-v1"),
        ("uncompute", "measurement"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    let fam = FamilyOut {
        family: Family {
            taxonomy_version: "1.0.0".into(),
            name: "test-naive-select".into(),
            parent: None,
            axes,
        },
        spec: spec.to_string(),
    };
    serde_json::to_vec(&fam).unwrap()
}

/// The taxonomy stub's verdicts (every axis declared only).
pub fn declared_only(f: &Family, _facts: &CircuitFacts) -> Vec<AxisVerdict> {
    f.axes
        .iter()
        .map(|(axis, value)| AxisVerdict {
            axis: axis.clone(),
            value: value.clone(),
            status: AxisStatus::DeclaredOnly,
        })
        .collect()
}

/// Evaluates `ops` against `lanemap` for the test spec.
pub fn eval_with(
    ops: &[Op],
    lanemap: &dyn LaneMap,
    samples: usize,
    tracker: Option<&dyn TrackerFactory>,
) -> Result<Evaluation, String> {
    let spec = TestSpec::new();
    let file = ops_file(ops);
    let lm = lanemap.to_bytes();
    let fam = family(spec.id());
    evaluate(&Inputs {
        spec: &spec,
        lanemap: &lm,
        family: &fam,
        ops: &file,
        samples,
        tracker,
        check: &declared_only,
    })
}

/// Builds the circuit with mutation `m` for the exact alias map and evaluates it.
pub fn eval_mutant(m: Mutation, samples: usize) -> Result<Evaluation, String> {
    let spec = TestSpec::new();
    let map = alias();
    eval_with(&build(&spec, &map, m), &map, samples, None)
}
