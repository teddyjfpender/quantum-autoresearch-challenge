//! `CircuitFacts::segments_validated`: the Segment hints form one walk step, system ops sit in
//! SELECT, and nothing but register bookkeeping precedes the first hint.
use femoco_walk::circuit::{
    Builder, Op, SEG_INNER_BEGIN, SEG_INNER_END, SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE,
};
use femoco_walk::facts::static_facts;
use femoco_walk::sim::Layout;

const SPATIAL: usize = 2;

fn layout(u: u32) -> Layout {
    Layout {
        system: 2 * SPATIAL,
        uniform: u,
    }
}

/// Emits a walk step whose Segment codes are `codes`, with a system CX after `sys_after` (the
/// index into `codes` after which it is placed) and, if `early`, one gate before the first hint.
fn circuit(codes: &[u32], sys_after: usize, early: bool) -> Vec<Op> {
    let mut b = Builder::new(2 * SPATIAL);
    b.declare_uniform(2);
    let (u0, u1, t) = (b.uniform(0), b.uniform(1), b.system(0));
    let a = b.alloc();
    if early {
        b.x(a);
        b.x(a);
    }
    for (i, &code) in codes.iter().enumerate() {
        b.segment(code);
        if code == SEG_PREPARE {
            b.ccx(u0, u1, a);
        }
        if i == sys_after {
            b.cx(a, t);
        }
        if code == SEG_UNPREPARE {
            b.ccx(u0, u1, a);
        }
    }
    b.free(a);
    b.finish()
}

fn validated(codes: &[u32], sys_after: usize, early: bool) -> bool {
    static_facts(&circuit(codes, sys_after, early), &layout(2)).segments_validated
}

const STEP: [u32; 3] = [SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE];

#[test]
fn one_walk_step_with_system_ops_in_select_is_validated() {
    assert!(validated(&STEP, 1, false));
    let nested = [
        SEG_PREPARE,
        SEG_SELECT,
        SEG_INNER_BEGIN,
        SEG_INNER_END,
        SEG_UNPREPARE,
    ];
    assert!(validated(&nested, 2, false), "inside inner-begin");
    assert!(
        validated(&nested, 3, false),
        "after inner-end, still in SELECT"
    );
}

#[test]
fn a_system_op_outside_select_is_not_validated() {
    assert!(!validated(&STEP, 0, false), "system op in PREPARE");
    assert!(!validated(&STEP, 2, false), "system op in UNPREPARE");
}

#[test]
fn malformed_segment_sequences_are_not_validated() {
    for codes in [
        &[SEG_SELECT, SEG_PREPARE, SEG_UNPREPARE][..],
        &[SEG_PREPARE, SEG_SELECT][..],
        &[SEG_PREPARE, SEG_SELECT, SEG_UNPREPARE, SEG_SELECT][..],
        &[SEG_PREPARE, SEG_INNER_BEGIN, SEG_SELECT, SEG_UNPREPARE][..],
        &[SEG_PREPARE, SEG_SELECT, SEG_INNER_BEGIN, SEG_UNPREPARE][..],
        &[SEG_PREPARE, SEG_SELECT, SEG_INNER_END, SEG_UNPREPARE][..],
    ] {
        assert!(!validated(codes, 1, false), "{codes:?}");
    }
}

#[test]
fn a_gate_before_the_first_hint_is_not_validated() {
    assert!(!validated(&STEP, 1, true));
}

#[test]
fn every_segment_has_a_system_ops_entry() {
    let f = static_facts(&circuit(&STEP, 1, false), &layout(2));
    for code in f.segment_op_counts.keys() {
        assert!(f.segment_system_ops.contains_key(code), "segment {code}");
    }
    assert!(f.segment_system_ops[&0].is_empty());
}
