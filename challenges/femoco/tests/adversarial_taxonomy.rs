//! Adversarial regressions for the taxonomy's segment facts (high: a mislabelled family passes).
//!
//! `select = clifford-mask` is Verified when `segments_validated` holds, the ops labelled SELECT
//! (segment 1) hold no Toffoli or Givens, and SELECT acts on the system. Two holes let a circuit
//! whose SELECT is full of Toffolis claim it:
//!
//! 1. `segments_validated` accepts inner-block hints (`Segment 3` ... `Segment 4`) inside SELECT
//!    for every lane map, but the ops between them are counted under segments 3 and 4, not 1.
//!    Wrapping a unary-iteration SELECT in an inner block therefore empties segment 1 of
//!    Toffolis. Inner blocks mean something only for a nested lane map (spec/DESIGN.md section
//!    16), whose rules already forbid `clifford-mask`; `evaluate` now clears
//!    `segments_validated` when inner hints appear without one.
//! 2. `segments_validated` and `segment_system_ops` looked only at `q_target`. `CZ`/`CCZ` are
//!    symmetric, and the harness accepts a system qubit in any of their positions, so a system
//!    `CZ` written with the system qubit as a control could sit in PREPARE unseen. Both facts now
//!    count an op that names a system qubit in any operand.
mod harness_common;

use femoco_walk::circuit::{Builder, Op, OperationType, SEG_INNER_BEGIN, SEG_INNER_END};
use femoco_walk::circuit::{SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::facts::{static_facts, CircuitFacts};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{FamilyOut, Inputs};
use femoco_walk::sim::Layout;
use femoco_walk::spec::EncodingSpec;
use femoco_walk::taxonomy::{self, AxisStatus, AxisVerdict, Family};
use harness_common::{alias, build, ops_file, Mutation, TestSpec, N};
use std::collections::BTreeMap;

fn taxonomy() -> taxonomy::Taxonomy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    taxonomy::load_taxonomy(&path).unwrap()
}

const CLIFFORD_MASK: [(&str, &str); 7] = [
    ("encoding", "sparse"),
    ("lane_map", "alias-v1"),
    ("lookup", "unary-iteration"),
    ("select", "clifford-mask"),
    ("uncompute", "measurement-based"),
    ("reuse", "serial"),
    ("rotation", "none"),
];

fn family_bytes(spec: &str) -> Vec<u8> {
    let axes: BTreeMap<String, String> = CLIFFORD_MASK
        .iter()
        .map(|(a, v)| (a.to_string(), v.to_string()))
        .collect();
    serde_json::to_vec(&FamilyOut {
        family: Family {
            taxonomy_version: "1.2.0".into(),
            name: "unary-select-relabelled".into(),
            parent: None,
            axes,
        },
        spec: spec.to_string(),
    })
    .unwrap()
}

fn seg(code: u32) -> Op {
    let mut op = Op::new(OperationType::Segment);
    op.r_target = code;
    op
}

/// The harness's naive unary-iteration SELECT (a CCX ladder per uniform value), relabelled:
/// an empty PREPARE, then SELECT: one system `CZ` controlled by a qubit that is always 0 (so
/// SELECT "acts on the system" with a Clifford), then the whole ladder inside an inner block,
/// then UNPREPARE.
fn relabelled_select() -> Vec<Op> {
    let spec = TestSpec::new();
    let map = alias();
    let ops = build(&spec, &map, Mutation::None);
    let at = ops
        .iter()
        .position(|op| op.kind == OperationType::Segment && op.r_target == SEG_SELECT)
        .unwrap();
    let spare = ops.iter().flat_map(Op::qubits).max().unwrap() + 1;
    let mut cz = Op::new(OperationType::CZ);
    (cz.q_target, cz.q_control1) = (1, spare);
    let mut out = ops[..at].to_vec();
    out.extend([seg(SEG_PREPARE), seg(SEG_SELECT), cz, seg(SEG_INNER_BEGIN)]);
    out.extend_from_slice(&ops[at + 1..]);
    out.extend([seg(SEG_INNER_END), seg(SEG_UNPREPARE)]);
    out
}

#[test]
fn clifford_mask_is_not_verified_when_select_toffolis_hide_in_an_inner_block() {
    let spec = TestSpec::new();
    let map = alias();
    let ops = relabelled_select();
    let tax = taxonomy();
    // The synthetic spec's encoding is "test"; read the circuit's genuine facts as a sparse run.
    let seen = std::cell::RefCell::new(None);
    let check = |f: &Family, facts: &CircuitFacts| -> Vec<AxisVerdict> {
        let mut facts = facts.clone();
        facts.encoding = "sparse".into();
        let v = taxonomy::check(&tax, f, &facts);
        *seen.borrow_mut() = Some((facts, v.clone()));
        v
    };
    let file = ops_file(&ops);
    let lm = map.to_bytes();
    let fam = family_bytes(spec.id());
    let ev = evaluate(&Inputs {
        spec: &spec,
        lanemap: &lm,
        family: &fam,
        ops: &file,
        samples: 2048,
        tracker: None,
        check: &check,
    });
    let (facts, verdicts) = seen.into_inner().expect("the taxonomy ran");
    // The circuit is correct and SELECT does hundreds of Toffolis.
    assert!(ev.is_ok(), "{:?}", ev.err());
    let select_ccx = facts.segment_op_counts[&3]["CCX"];
    assert!(select_ccx > 500, "{select_ccx}");
    let select = verdicts.iter().find(|v| v.axis == "select").unwrap();
    assert_ne!(
        select.status,
        AxisStatus::Verified,
        "clifford-mask verified on a SELECT with {select_ccx} Toffolis"
    );
    assert!(!facts.segments_validated);
}

/// A step whose PREPARE holds a system `CZ` written with the system qubit as its control.
fn system_cz_in_prepare() -> Vec<Op> {
    let mut b = Builder::new(N);
    b.declare_uniform(1);
    let (u0, t) = (b.uniform(0), b.system(0));
    let a = b.alloc();
    b.segment(SEG_PREPARE);
    b.cx(u0, a);
    b.cz(t, a); // q_target = a (ancilla), q_control1 = the system qubit.
    b.cx(u0, a);
    b.segment(SEG_SELECT);
    b.cz(a, t); // A Clifford system op in SELECT (a is 0 here).
    b.segment(SEG_UNPREPARE);
    b.free(a);
    b.finish()
}

#[test]
fn a_system_cz_outside_select_is_seen_whatever_its_operand_order() {
    let layout = Layout {
        system: N,
        uniform: 1,
    };
    let f = static_facts(&system_cz_in_prepare(), &layout);
    assert!(
        !f.segments_validated,
        "a CZ acting on the system in PREPARE passed segments_validated"
    );
    assert_eq!(f.segment_system_ops[&0].get("CZ"), Some(&1));
    assert_eq!(f.segment_system_ops[&1].get("CZ"), Some(&1));
}

#[test]
fn the_honest_layouts_still_validate() {
    // A flat step without inner hints is unaffected by the fix.
    let mut b = Builder::new(N);
    b.declare_uniform(1);
    let (u0, t) = (b.uniform(0), b.system(0));
    b.segment(SEG_PREPARE);
    b.segment(SEG_SELECT);
    b.cz(u0, t);
    b.cz(t, u0);
    b.segment(SEG_UNPREPARE);
    let layout = Layout {
        system: N,
        uniform: 1,
    };
    let f = static_facts(&b.finish(), &layout);
    assert!(f.segments_validated);
    assert_eq!(f.segment_system_ops[&1].get("CZ"), Some(&2));
}
