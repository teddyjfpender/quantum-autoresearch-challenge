//! Op format, static rules, the Givens plug-in point, lane-map I/O and facts.
mod harness_common;

use femoco_walk::circuit::{
    read_ops, write_ops, Builder, Op, OperationType as K, SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE,
};
use femoco_walk::facts::static_facts;
use femoco_walk::lanemap::{self, alias::largest_remainder_counts, LaneMap};
use femoco_walk::sim::{compile, LaneFrame, LaneTracker, Layout, TrackerFactory};
use femoco_walk::spec::{EncodingSpec, SystemOp};
use harness_common::{alias, eval_with, TestSpec, N};
use std::sync::atomic::{AtomicU64, Ordering};

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("femoco-circuit-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join("ops.bin")
}

fn sample_ops() -> Vec<Op> {
    let mut b = Builder::new(4);
    b.declare_uniform(2);
    let a = b.alloc();
    let (c, u0) = (b.control(), b.uniform(0));
    b.ccx(c, u0, a);
    let bit = b.hmr(a);
    b.cz_if(c, u0, bit);
    b.finish()
}

#[test]
fn ops_bin_round_trips_and_rejects_corruption() {
    let ops = sample_ops();
    let p = tmp("rt");
    write_ops(&ops, &p).unwrap();
    let back = read_ops(&p).unwrap();
    assert_eq!(back.ops, ops);
    let bytes = std::fs::read(&p).unwrap();
    assert_eq!(bytes.len(), 16 + 56 * ops.len());
    assert_eq!(&bytes[..8], b"FEMOOPS1");
    let corrupt = |f: &dyn Fn(&mut Vec<u8>), want: &str| {
        let mut b = bytes.clone();
        f(&mut b);
        std::fs::write(&p, &b).unwrap();
        let e = read_ops(&p).err().unwrap();
        assert!(e.contains(want), "{e}");
    };
    corrupt(&|b| b[0] = b'X', "bad magic");
    corrupt(&|b| b.truncate(b.len() - 1), "length");
    corrupt(&|b| b[16] = 99, "unknown kind 99");
    corrupt(&|b| b[20] = 1, "padding");
    // CCX with its target equal to a control (aliasing).
    corrupt(
        &|b| {
            let t = b[16 + 24..16 + 32].to_vec();
            b[16 + 16..16 + 24].copy_from_slice(&t);
        },
        "appears twice",
    );
}

fn layout(u: u32) -> Layout {
    Layout {
        system: 4,
        uniform: u,
    }
}

fn one(kind: K, t: u32, c1: u32, c2: u32) -> Op {
    let mut op = Op::new(kind);
    (op.q_target, op.q_control1, op.q_control2) = (t, c1, c2);
    op
}

#[test]
fn system_wire_rules() {
    const NO: u32 = u32::MAX;
    let anc = 7; // 1 control + 4 system + 2 uniform
    let bad = [
        (
            one(K::CX, anc, 1, NO),
            "system qubit may only be the target",
        ),
        (one(K::CZ, 1, 2, NO), "system qubit may only be the target"),
        (
            one(K::Swap, 1, anc, NO),
            "system qubit may only be the target",
        ),
        (
            one(K::CCX, 1, 2, anc),
            "system qubit may only be the target",
        ),
        (one(K::R, 1, NO, NO), "cannot be measured or reset"),
        (one(K::R, 5, NO, NO), "control and uniform"),
        (
            one(K::Givens, 2, 1, NO),
            "Givens needs the df tracker (not in this build)",
        ),
    ];
    for (mut op, want) in bad {
        if op.kind == K::Givens {
            op.r_target = 0;
        }
        let e = compile(&[op], &layout(2), None).err().unwrap_or_default();
        assert!(e.contains(want), "{op:?}: {e}");
    }
    let good = [
        one(K::CX, 1, anc, NO),
        one(K::CZ, anc, 1, NO),
        one(K::CCZ, anc, 0, 2),
        one(K::S, 3, NO, NO),
    ];
    compile(&good, &layout(2), None).unwrap();
    let mut pop = Op::new(K::PopCondition);
    pop.c_condition = u32::MAX;
    assert!(compile(&[pop], &layout(2), None)
        .err()
        .unwrap()
        .contains("empty stack"));
}

#[test]
fn liveness_counts_peak_not_ids() {
    let mut b = Builder::new(4);
    b.declare_uniform(1);
    for _ in 0..5 {
        let a = b.alloc();
        let c = b.control();
        b.cx(c, a);
        b.cx(c, a);
        b.free(a);
    }
    let pair = b.alloc_n(2);
    b.cx(pair[0], pair[1]);
    let ops = b.finish();
    let c = compile(&ops, &layout(1), None).unwrap();
    assert_eq!(c.q_peak, 1 + 4 + 1 + 2);
}

/// A stand-in tracker: records the angles it sees and accepts a lane when the frame left after
/// the last Givens is the identity. Proves the plug-in point is reachable from the op stream.
struct Fake {
    seen: std::sync::Arc<AtomicU64>,
}
struct FakeLane {
    seen: std::sync::Arc<AtomicU64>,
    angles: Vec<u64>,
}
impl LaneTracker for FakeLane {
    fn givens(&mut self, before: &LaneFrame, p: usize, angle: u64) -> Result<(), String> {
        assert_eq!(p, 2);
        assert!(before.x.iter().all(|&w| w == 0));
        self.angles.push(angle);
        self.seen.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn finish(&mut self, after: &LaneFrame, _r: Option<&SystemOp>) -> Result<(), String> {
        if self.angles != [5] {
            return Err(format!("angles {:?}", self.angles));
        }
        (after.x.iter().chain(&after.z).all(|&w| w == 0) && after.phase == 0)
            .then_some(())
            .ok_or_else(|| "frame after the rotation is not the identity".into())
    }
}
impl TrackerFactory for Fake {
    fn lane(&self, _n: usize) -> Box<dyn LaneTracker> {
        Box::new(FakeLane {
            seen: self.seen.clone(),
            angles: Vec::new(),
        })
    }
    fn givens_toffoli_cost(&self) -> f64 {
        10.0
    }
    fn phase_gradient_qubits(&self) -> u64 {
        7
    }
}

#[test]
fn givens_reaches_the_tracker_plug_in() {
    let map = alias();
    let mut b = Builder::new(N);
    b.declare_uniform(map.uniform_bits());
    let reg = b.alloc_n(3);
    b.x(reg[0]);
    b.x(reg[2]);
    let r = b.register(&reg);
    b.givens(2, r);
    b.x(reg[0]);
    b.x(reg[2]);
    reg.iter().for_each(|&q| b.free(q));
    let ops = b.finish();
    let seen = std::sync::Arc::new(AtomicU64::new(0));
    let fake = Fake { seen: seen.clone() };
    // Every lane (c = 0 or 1) goes to the fake tracker, which accepts the identity after one
    // Givens with angle 0b101.
    let ev = eval_with(&ops, &map, 256, Some(&fake)).unwrap();
    // With a second lane engine hooked in (FEMOCO_EQUIV_ENGINE, spec/FAST-EVALUATOR.md section
    // 10) the tracker is also called by that engine, which may call it less often per lane
    // (a memoizing engine); the reference's own 256 calls are then a lower bound.
    if femoco_walk::equiv::env_engine().is_none() {
        assert_eq!(seen.load(Ordering::SeqCst), 256);
    } else {
        assert!(seen.load(Ordering::SeqCst) >= 256);
    }
    assert_eq!(ev.toffoli, 10.0 + 4.0, "Givens charge + reflection");
    assert_eq!(ev.qubits, 1 + 6 + 6 + 3 + 7, "+ phase-gradient register");
    let e = eval_with(&ops, &map, 256, None).unwrap_err();
    assert!(
        e.contains("Givens needs the df tracker (not in this build)"),
        "{e}"
    );
}

#[test]
fn lanemap_bytes_round_trip_and_checks() {
    let spec = TestSpec::new();
    let map = alias();
    let bytes = map.to_bytes();
    let back = lanemap::parse(&bytes, &spec).unwrap();
    assert_eq!(back.family(), "alias-v1");
    assert_eq!(back.to_bytes(), bytes);
    for s in 0..64 {
        assert_eq!(back.reference_op(&spec, s), map.reference_op(&spec, s));
    }
    let mut bad = bytes.clone();
    let n = bad.len();
    bad[n - 4] = 9; // alt[7] = 9 >= L
    assert!(lanemap::parse(&bad, &spec)
        .err()
        .unwrap()
        .contains("is not a term"));
    let c = largest_remainder_counts(
        &spec
            .flat_terms()
            .unwrap()
            .into_iter()
            .map(|t| t.0)
            .collect::<Vec<_>>(),
        6,
    )
    .unwrap();
    assert_eq!(c, vec![32, 16, 8, 4, 4]);
}

#[test]
fn facts_see_segments_inverse_and_system_ops() {
    let mut b = Builder::new(4);
    b.declare_uniform(2);
    let (u0, u1) = (b.uniform(0), b.uniform(1));
    let a = b.alloc();
    b.segment(SEG_PREPARE);
    b.ccx(u0, u1, a);
    b.s(a);
    b.segment(SEG_SELECT);
    let t = b.system(1);
    b.cx(a, t);
    b.segment(SEG_UNPREPARE);
    b.sdg(a);
    b.ccx(u1, u0, a);
    b.free(a);
    let ops = b.finish();
    let f = static_facts(&ops, &layout(2));
    assert!(!f.prepare_unprepare_inverse, "the R is inside unprepare");
    let trimmed: Vec<Op> = ops.iter().copied().filter(|o| o.kind != K::R).collect();
    let f = static_facts(&trimmed, &layout(2));
    assert!(f.prepare_unprepare_inverse);
    assert_eq!(f.segment_system_ops[&1]["CX"], 1);
    assert_eq!(f.op_counts["CCX"], 2);
    assert_eq!(f.toffoli_depth, 2);
}

#[test]
fn spec_ids_cannot_escape_specs_dir() {
    let root = std::path::Path::new(".");
    for id in ["../src/walk/x", "a/b", "", "Reiher", "x.y"] {
        let e = femoco_walk::spec::load(root, id).err().unwrap();
        assert!(e.contains("is not of the form"), "{id}: {e}");
    }
}
