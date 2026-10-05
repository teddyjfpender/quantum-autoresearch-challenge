//! The nested DF composition end to end (spec/DESIGN.md section 16): a hand-built nested circuit
//! on a 3-orbital nested spec passes in both one-body layouts with exact counts and a verified
//! `givens-nested`, and every mutant the brief lists is rejected with a specific message.
mod nested_common;
use femoco_walk::circuit::{Op, OperationType as K, NONE, SEG_INNER_BEGIN, SEG_INNER_END};
use femoco_walk::lanemap::df_nested::{self, DfNestedMap, OneBody, OuterItem, Table};
use femoco_walk::lanemap::{self, LaneMap};
use femoco_walk::spec::df::DfSpec;
use femoco_walk::spec::{EncodingSpec, Exact, SystemOp};
use femoco_walk::taxonomy::AxisStatus;
use nested_common::{
    build, eval_mutant, eval_ops, exact_map, flat_spec, spec, Mutation, BETA, NESTED_AXES,
};

const K: usize = 4096;

fn rejects(one_body: OneBody, m: Mutation, want: &str) {
    let e = eval_mutant(one_body, m, K).expect_err(&format!("{m:?} must be rejected"));
    println!("{one_body:?} {m:?}: {e}");
    assert!(e.contains(want), "{m:?}: got {e}");
}

fn rejects_ops(ops: &[Op], want: &str) {
    let s = spec();
    let map = exact_map(&s, OneBody::Direct);
    let e = eval_ops(&s, &map.to_bytes(), ops, &NESTED_AXES, K).expect_err("must be rejected");
    println!("{want}: {e}");
    assert!(e.contains(want), "got {e}");
}

fn good_ops() -> Vec<Op> {
    let s = spec();
    build(&s, &exact_map(&s, OneBody::Direct), Mutation::None)
}

fn index_of(ops: &[Op], pred: impl Fn(&Op) -> bool, nth: usize) -> usize {
    ops.iter()
        .enumerate()
        .filter(|(_, op)| pred(op))
        .nth(nth)
        .unwrap()
        .0
}

fn is_seg(code: u32) -> impl Fn(&Op) -> bool {
    move |op: &Op| op.kind == K::Segment && op.r_target == code
}

#[test]
fn nested_spec_has_the_nested_lambda_and_identity() {
    let s = spec();
    // lambda_T = 8, S_l^2/4 = 4 + 4; identity = ecore + sum t + sum (S^2/4 - A^2/2).
    assert_eq!(s.lambda(), Exact::from_int(16));
    // 0.25 + (4 - 2 + 2) + (16/4 - 2^2/2) + (16/4 - 2^2/2).
    assert_eq!(s.identity(), Exact::from_f64(8.25).unwrap());
    let flat = flat_spec();
    // The flat form of the same operator: lambda_T + sum (S^2/2 - Q/4) = 8 + (8 - 1.5) + (8 - 2.5).
    assert_eq!(flat.lambda(), Exact::from_f64(20.0).unwrap());
    // I_nested - I_flat = sum_l (S_l^2 - Q_l) / 4 = (16 - 6)/4 + (16 - 10)/4 = 4.
    assert_eq!(
        s.identity().sub(&flat.identity()),
        Exact::from_int(4),
        "identity offset of the Chebyshev form"
    );
}

#[test]
fn hand_built_nested_circuit_passes_in_both_layouts() {
    for one_body in [OneBody::Direct, OneBody::Folded] {
        let ev = eval_mutant(one_body, Mutation::None, K).unwrap();
        assert_eq!(ev.rounding_error, Exact::zero(), "{one_body:?}");
        assert_eq!(ev.lambda, Exact::from_int(16));
        let n = ev.nested.expect("nested stats");
        assert_eq!(n.inner_width, 3);
        assert_eq!(n.paired_samples + n.diagonal_samples, K);
        // One Reflect per lane on w = 3 qubits: max(3 - 2, 0) = 1 Toffoli.
        assert_eq!(n.inner_reflect_toffoli, 1.0);
        assert!(ev.facts.nested_validated && ev.facts.segments_validated);
        assert_eq!(ev.facts.op_counts.get("REFLECT"), Some(&1));
        for axis in ["encoding", "lane_map", "select", "rotation"] {
            let v = ev.verdicts.iter().find(|v| v.axis == axis).unwrap();
            assert_eq!(v.status, AxisStatus::Verified, "{one_body:?} {axis}");
        }
        let json = femoco_walk::score::score_json(&ev);
        // Over the control-1 paired and diagonal lanes (spec/DESIGN.md section 16).
        let (fp, fd) = n.f_bounds_control_one_at(0);
        let lambda = 16.0;
        assert_eq!(
            json["metrics"]["implied_error_bound"],
            serde_json::json!(2.0 * lambda * (2.0 * fp + fd))
        );
        println!(
            "{one_body:?}: C_step {} Q_peak {} paired {} diagonal {} implied {}",
            ev.toffoli,
            ev.qubits,
            n.paired_samples,
            n.diagonal_samples,
            ev.implied_error_bound()
        );
    }
}

#[test]
fn cost_counts_the_inner_reflect_once_per_lane() {
    let ev = eval_mutant(OneBody::Direct, Mutation::None, K).unwrap();
    let charge = 2.0 * f64::from(BETA - 2);
    let n = ev.nested.unwrap();
    let want = ev.facts.executed_ccx
        + ev.facts.executed_ccz
        + charge * ev.facts.executed_givens
        + ev.reflection_toffoli as f64
        + n.inner_reflect_toffoli;
    assert!((ev.toffoli - want).abs() < 1e-9, "{} vs {want}", ev.toffoli);
    // u = 7: the walk reflection is still charged u - 2 on the whole uniform register.
    assert_eq!(ev.reflection_toffoli, 5);
}

// ---------------------------------------------------------------- structural mutants (b)

#[test]
fn inner_copies_that_differ_by_one_op_are_rejected() {
    let mut ops = good_ops();
    let second = index_of(&ops, is_seg(SEG_INNER_BEGIN), 1);
    let at = second + 1 + index_of(&ops[second + 1..], |op| op.kind == K::CCX, 3);
    // Swapping a Toffoli's controls leaves the gate the same but the op different: the check is
    // syntactic, as specified.
    let op = &mut ops[at];
    std::mem::swap(&mut op.q_control1, &mut op.q_control2);
    rejects_ops(&ops, "the second inner copy differs from the first");
}

#[test]
fn a_missing_reflect_is_rejected() {
    let mut ops = good_ops();
    let r = index_of(&ops, |op| op.kind == K::Reflect, 0);
    ops.remove(r);
    rejects_ops(&ops, "exactly one Reflect, found 0");
}

#[test]
fn an_extra_reflect_is_rejected() {
    let mut ops = good_ops();
    let r = index_of(&ops, |op| op.kind == K::Reflect, 0);
    let end = index_of(&ops, is_seg(SEG_INNER_END), 1);
    let extra = ops[r];
    ops.insert(end + 1, extra);
    rejects_ops(&ops, "exactly one Reflect, found 2");
}

#[test]
fn a_reflect_that_is_not_alone_between_the_copies_is_rejected() {
    let mut ops = good_ops();
    let r = index_of(&ops, |op| op.kind == K::Reflect, 0);
    let mut dbg = Op::new(K::DebugPrint);
    dbg.r_target = NONE;
    ops.insert(r + 1, dbg);
    rejects_ops(&ops, "must be the only op between the inner copies");
}

#[test]
fn a_reflect_on_the_wrong_register_is_rejected() {
    let mut ops = good_ops();
    // Point the Reflect at the angle register (register 0) instead of the inner one.
    let r = index_of(&ops, |op| op.kind == K::Reflect, 0);
    ops[r].r_target = 0;
    rejects_ops(&ops, "must act on exactly the inner uniform register");
    // A register of the right width over outer uniform bits.
    let mut ops = good_ops();
    let s = spec();
    let map = exact_map(&s, OneBody::Direct);
    let first_uniform = 1 + 2 * nested_common::SPATIAL as u32;
    let reg = 99;
    let mut decl = vec![{
        let mut op = Op::new(K::Register);
        op.r_target = reg;
        op
    }];
    for i in 0..map.inner_width() {
        let mut op = Op::new(K::AppendToRegister);
        op.q_target = first_uniform + i;
        op.r_target = reg;
        decl.push(op);
    }
    let r = index_of(&ops, |op| op.kind == K::Reflect, 0);
    ops[r].r_target = reg;
    ops.splice(0..0, decl);
    rejects_ops(&ops, "must act on exactly the inner uniform register");
}

#[test]
fn touching_the_inner_register_outside_the_copies_is_rejected() {
    let mut ops = good_ops();
    let s = spec();
    let map = exact_map(&s, OneBody::Direct);
    let inner0 = 1 + 2 * nested_common::SPATIAL as u32 + map.outer_bits();
    let end = index_of(&ops, is_seg(SEG_INNER_END), 1);
    let mut x = Op::new(K::X);
    x.q_target = inner0;
    ops.splice(end + 1..end + 1, [x, x]);
    rejects_ops(
        &ops,
        "touches the inner uniform register outside the inner copies",
    );
}

#[test]
fn a_reflect_without_a_nested_lane_map_is_rejected() {
    // The compile path every non-nested lane map takes (score::evaluate).
    let s = flat_spec();
    let layout = femoco_walk::sim::Layout {
        system: 2 * nested_common::SPATIAL,
        uniform: 7,
    };
    let e = femoco_walk::sim::compile(&good_ops(), &layout, femoco_walk::sim::givens_tracker(&s))
        .err()
        .unwrap();
    assert!(e.contains("Reflect needs a nested lane map"), "{e}");
}

// ---------------------------------------------------------------- lane mutants (a)

#[test]
fn a_flipped_sign_in_one_inner_term_is_rejected() {
    rejects(
        OneBody::Direct,
        Mutation::FlipInnerSign { l: 0, j: 2 },
        "sign flipped",
    );
    rejects(
        OneBody::Folded,
        Mutation::FlipInnerSign { l: 1, j: 0 },
        "sign flipped",
    );
}

#[test]
fn an_inner_copy_that_does_not_restore_the_inner_register_is_rejected() {
    rejects(
        OneBody::Direct,
        Mutation::InnerNotRestored,
        "inner-register-at-reflect",
    );
}

#[test]
fn a_folded_one_body_item_applied_in_both_passes_is_rejected() {
    // Identical copies, but the one-body item acts twice: M(b) M(a) instead of M(a).
    rejects(
        OneBody::Folded,
        Mutation::FoldedBothPasses,
        "differs from the reference",
    );
}

// ---------------------------------------------------------------- lane-map mutants (c)

fn with_outer(spec: &DfSpec, counts: &[u64]) -> DfNestedMap {
    let inner = vec![
        Table::from_counts(3, 0, &[2, 2, 1, 1, 1, 1]).unwrap(),
        Table::from_counts(3, 0, &[3, 3, 1, 1]).unwrap(),
    ];
    DfNestedMap::new(
        OneBody::Direct,
        Exact::from_int(16),
        Table::from_counts(3, 1, counts).unwrap(),
        inner,
        spec,
    )
    .unwrap()
}

#[test]
fn a_wrong_leaf_weight_is_rejected() {
    let s = spec();
    // Leaf 0 gets 5 of 16 outer lanes and leaf 1 gets 3 (4 and 4 are exact).
    let wrong = with_outer(&s, &[2, 2, 1, 1, 1, 1, 5, 3]);
    // A circuit that implements the wrong map agrees with it lane by lane, so the rounding
    // error rejects it: 16 * 1/16 on each leaf's 2 O^2 - 1, spread over the flat terms.
    let e = eval_ops(
        &s,
        &wrong.to_bytes(),
        &build(&s, &wrong, Mutation::None),
        &NESTED_AXES,
        K,
    )
    .err()
    .unwrap();
    println!("{e}");
    assert!(e.contains("exceeds 0.1 mHa"), "{e}");
    // A circuit that implements the wrong weights, declared with the exact map, disagrees with
    // it on sampled lanes.
    let good = exact_map(&s, OneBody::Direct);
    let e = eval_ops(
        &s,
        &good.to_bytes(),
        &build(&s, &wrong, Mutation::None),
        &NESTED_AXES,
        K,
    )
    .err()
    .unwrap();
    println!("{e}");
    assert!(e.contains("differs from the reference"), "{e}");
}

#[test]
fn a_lane_map_whose_inner_widths_differ_is_rejected() {
    let s = spec();
    let e = DfNestedMap::new(
        OneBody::Direct,
        Exact::from_int(16),
        Table::from_counts(3, 1, &[2, 2, 1, 1, 1, 1, 4, 4]).unwrap(),
        vec![
            Table::from_counts(3, 0, &[2, 2, 1, 1, 1, 1]).unwrap(),
            Table::from_counts(3, 1, &[6, 6, 2, 2]).unwrap(),
        ],
        &s,
    )
    .err()
    .unwrap();
    assert!(e.contains("every inner table must share one width"), "{e}");
    // On the wire there is one (k_i, mu_i): a table of another width breaks the length.
    let map = exact_map(&s, OneBody::Direct);
    let mut bytes = map.to_bytes();
    bytes.extend_from_slice(&[0u8; 8]);
    let e = lanemap::parse(&bytes, &s).err().unwrap();
    assert!(e.contains("payload length"), "{e}");
}

#[test]
fn nested_and_flat_lane_maps_refuse_each_other_s_specs() {
    let (s, flat) = (spec(), flat_spec());
    let bytes = exact_map(&s, OneBody::Direct).to_bytes();
    let e = lanemap::parse(&bytes, &flat).err().unwrap();
    assert!(e.contains("flat-form df spec"), "{e}");
    let pair = femoco_walk::lanemap::df_pair::build(&s, 5, 2)
        .err()
        .unwrap();
    assert!(pair.contains("nested-form df spec"), "{pair}");
}

#[test]
fn lane_map_round_trips() {
    let s = spec();
    for one_body in [OneBody::Direct, OneBody::Folded] {
        let map = exact_map(&s, one_body);
        let bytes = map.to_bytes();
        let back = lanemap::parse(&bytes, &s).unwrap();
        assert_eq!(back.to_bytes(), bytes);
        assert_eq!(back.family(), df_nested::FAMILY);
        assert_eq!(back.inner_bits(), Some((map.outer_bits(), 3)));
        assert_eq!(back.rounding_error(&s).unwrap(), Exact::zero());
    }
}

/// The encoded coefficients by brute force over every lane `s` and second-pass value `b`
/// (spec/DESIGN.md section 16: `A = lambda E_s [2 E_b R(s, b) - R(s, a)]`), with the
/// reference operators identified by their inner indices. Returns `sum_t |c_t - c^_t|`.
fn brute_force_error(s: &DfSpec, map: &DfNestedMap) -> Exact {
    use std::collections::BTreeMap;
    let (u_o, w) = (map.outer_bits(), map.inner_width());
    let u = u_o + w;
    let lam = map.lambda_decl.clone();
    let unit = lam.mul(&Exact::dyadic(1.into(), u));
    let mut coef: BTreeMap<(i64, usize, usize), Exact> = BTreeMap::new();
    let mut add = |key: (i64, usize, usize), v: Exact| {
        let e = coef.entry(key).or_insert_with(Exact::zero);
        *e = e.add(&v);
    };
    for sv in 0..1u64 << u {
        let a = sv >> u_o;
        let item = map.decode_outer(sv);
        match (item, map.table_of(item)) {
            (OuterItem::OneBody { k, spin }, _) => {
                add((-1, 2 * k + spin as usize, 0), unit.clone())
            }
            (OuterItem::Folded, Some(t)) => add((-1, map.inner_item(t, a), 0), unit.clone()),
            (OuterItem::Leaf(l), Some(t)) => {
                let ja = map.inner_item(t, a);
                let pair = unit.mul(&Exact::dyadic(2.into(), w));
                for bv in 0..1u64 << w {
                    add((l as i64, map.inner_item(t, bv), ja), pair.clone());
                }
                add((l as i64, ja, ja), unit.neg());
            }
            _ => unreachable!(),
        }
    }
    let mut err = Exact::zero();
    let get = |key| coef.get(&key).cloned().unwrap_or_else(Exact::zero);
    for (k, &t) in s.t.iter().enumerate() {
        for spin in 0..2 {
            let c = Exact::from_f64(t.abs() / 2.0).unwrap();
            err = err.add(&c.sub(&get((-1, 2 * k + spin, 0))).abs());
        }
    }
    for (l, leaf) in s.leaves.iter().enumerate() {
        let m = 2 * leaf.e.len();
        for j1 in 0..m {
            for j2 in 0..m {
                if j1 != j2 {
                    let c = Exact::from_f64(leaf.e[j1 >> 1] * leaf.e[j2 >> 1] / 8.0)
                        .unwrap()
                        .abs();
                    err = err.add(&c.sub(&get((l as i64, j1, j2))).abs());
                }
            }
        }
    }
    err
}

#[test]
fn rounding_error_matches_a_brute_force_count_of_every_lane() {
    let s = spec();
    for one_body in [OneBody::Direct, OneBody::Folded] {
        // Largest-remainder tables at widths too small for exactness, and the exact ones.
        for (ko, muo, ki, mui) in [(3, 1, 3, 0), (3, 2, 3, 1), (4, 0, 3, 2)] {
            let Ok(map) = df_nested::build(&s, one_body, (ko, muo), (ki, mui)) else {
                continue;
            };
            let got = map.rounding_error(&s).unwrap();
            assert_eq!(
                got,
                brute_force_error(&s, &map),
                "{one_body:?} {ko} {muo} {ki} {mui}"
            );
        }
        let exact = exact_map(&s, one_body);
        assert_eq!(brute_force_error(&s, &exact), Exact::zero());
    }
    // A perturbed table: nonzero, and still equal to the brute force.
    let wrong = with_outer(&s, &[2, 2, 1, 1, 1, 1, 5, 3]);
    let got = wrong.rounding_error(&s).unwrap();
    assert!(got > Exact::zero());
    assert_eq!(got, brute_force_error(&s, &wrong));
}

#[test]
fn reference_is_the_product_second_pass_first() {
    let s = spec();
    let map = exact_map(&s, OneBody::Direct);
    let u_o = map.outer_bits();
    // An outer value naming leaf 1, inner values a = 0 and b = 7.
    let sv = (0..1u64 << u_o)
        .find(|&v| map.decode_outer(v) == OuterItem::Leaf(1))
        .unwrap();
    let (a, b) = (0u64, 7u64);
    let r = map.reference_nested(&s, sv | a << u_o, sv | b << u_o);
    let t = map.table_of(OuterItem::Leaf(1)).unwrap();
    let (ja, jb) = (map.inner_item(t, a), map.inner_item(t, b));
    let SystemOp::Rotated(r) = r else { panic!() };
    let SystemOp::Rotated(mb) = s.inner_op(1, jb >> 1, (jb & 1) as u8) else {
        panic!()
    };
    let SystemOp::Rotated(ma) = s.inner_op(1, ja >> 1, (ja & 1) as u8) else {
        panic!()
    };
    assert_eq!(r.phase, (mb.phase + ma.phase) % 4);
    assert_eq!(r.parts, [mb.parts, ma.parts].concat());
}
