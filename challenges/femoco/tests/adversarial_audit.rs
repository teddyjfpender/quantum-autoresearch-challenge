//! Soundness-audit pins (2026-10 evaluator audit): behaviours a submitter could lean on, each
//! fixed here so that a later change to the simulator, the liveness rule or the condition
//! semantics is deliberate. None of these is a defect; they are the edges of the rules of
//! spec/DESIGN.md sections 5, 6 and 8 that the other adversarial suites do not pin.
//!
//! Flat fixture (tests/harness_common): no tracker, six system qubits, `u = 6`.
//! Nested fixture (tests/nested_common): `Givens` at `beta = 8`, one `Reflect`.
mod harness_common;
mod nested_common;

use femoco_walk::circuit::{Op, OperationType as K, NONE};
use femoco_walk::lanemap::df_nested::OneBody;
use femoco_walk::lanemap::LaneMap;
use harness_common::{alias, build, eval_with, Mutation, TestSpec};
use nested_common::{eval_ops, exact_map, spec, NESTED_AXES};

const K_FLAT: usize = 4096;
const K_NESTED: usize = 2048;

fn flat_base() -> Vec<Op> {
    build(&TestSpec::new(), &alias(), Mutation::None)
}

fn fresh_qubit(ops: &[Op]) -> u32 {
    ops.iter().flat_map(Op::qubits).max().unwrap() + 1
}

fn fresh_bit(ops: &[Op]) -> u32 {
    ops.iter()
        .flat_map(|op| [op.c_target, op.c_condition])
        .filter(|&b| b != NONE)
        .max()
        .map_or(0, |b| b + 1)
}

fn op(kind: K, t: u32, c1: u32, c2: u32) -> Op {
    let mut o = Op::new(kind);
    (o.q_target, o.q_control1, o.q_control2) = (t, c1, c2);
    o
}

fn flat_eval(ops: &[Op]) -> Result<femoco_walk::score::Evaluation, String> {
    eval_with(ops, &alias(), K_FLAT, None)
}

fn flat_rejects(ops: &[Op], want: &str) -> String {
    let e = flat_eval(ops).expect_err("must be rejected");
    println!("{want}: {e}");
    assert!(e.starts_with(want), "wanted {want}, got {e}");
    e
}

// ------------------------------------------------------------------ measurement and liveness

/// An `Hmr` whose own condition is a bit no op ever sets never executes (spec/DESIGN.md
/// section 5): the qubit keeps its value and the lane fails at the end. A conditioned `Hmr`
/// is not a free reset.
#[test]
fn hmr_under_a_never_set_condition_does_not_free_the_qubit() {
    let mut ops = flat_base();
    let (a, bit) = (fresh_qubit(&ops), fresh_bit(&ops));
    ops.push(op(K::CX, a, 0, NONE)); // a = control
    let mut h = op(K::Hmr, a, NONE, NONE);
    h.c_target = bit + 1;
    h.c_condition = bit; // never written: 0 on every lane
    ops.push(h);
    let e = flat_rejects(&ops, "dirty-ancilla-at-end");
    assert!(e.contains("c=1"), "{e}");
}

/// Copying a uniform bit into an ancilla, clearing the uniform bit and measuring the ancilla
/// destroys the lane's identity: the uniform register cannot be restored. (In the real walk
/// that measurement's outcome would not be uniform; the lane check rejects it first.)
#[test]
fn moving_a_uniform_bit_into_an_ancilla_and_measuring_it_is_rejected() {
    let mut ops = flat_base();
    let (a, bit) = (fresh_qubit(&ops), fresh_bit(&ops));
    let u0 = 1 + harness_common::N as u32;
    ops.push(op(K::CX, a, u0, NONE));
    ops.push(op(K::CX, u0, a, NONE));
    let mut h = op(K::Hmr, a, NONE, NONE);
    h.c_target = bit;
    ops.push(h);
    flat_rejects(&ops, "registers-not-restored");
}

/// Swapping the control into an ancilla and measuring the ancilla is rejected the same way.
#[test]
fn measuring_a_copy_of_the_control_is_rejected() {
    let mut ops = flat_base();
    let (a, bit) = (fresh_qubit(&ops), fresh_bit(&ops));
    ops.push(op(K::Swap, a, 0, NONE));
    let mut h = op(K::Hmr, a, NONE, NONE);
    h.c_target = bit;
    ops.push(h);
    flat_rejects(&ops, "registers-not-restored");
}

// ------------------------------------------------------------------ classical conditions

/// A condition bit no op writes is 0 on every lane: a block under it never executes, costs
/// nothing and does nothing. The circuit must therefore be correct without it (here it is),
/// so a never-executing block cannot hide required work.
#[test]
fn a_block_under_an_unwritten_bit_never_executes_and_is_never_charged() {
    let base = flat_eval(&flat_base()).unwrap();
    let mut ops = flat_base();
    let (a, bit) = (fresh_qubit(&ops), fresh_bit(&ops));
    let u0 = 1 + harness_common::N as u32;
    let mut push = Op::new(K::PushCondition);
    push.c_condition = bit;
    ops.push(push);
    ops.push(op(K::CCX, a, 0, u0));
    ops.push(op(K::X, 1, NONE, NONE)); // would break every lane if it ran
    ops.push(Op::new(K::PopCondition));
    let ev = flat_eval(&ops).unwrap();
    assert_eq!(ev.toffoli, base.toffoli);
    assert_eq!(ev.facts.executed_ccx, base.facts.executed_ccx);
    // The qubit named inside the block is live all the same (liveness is static); it is the
    // only live ancilla at that point, so the peak (six ladder ancillas) does not move.
    assert_eq!(ev.qubits, base.qubits);
    assert_eq!(ev.facts.q_peak, base.facts.q_peak);
}

/// `BitStore1` makes a condition constant 1: a phase under it is applied on every lane and
/// caught like an unconditioned one.
#[test]
fn a_constant_true_block_cannot_hide_a_sign_flip() {
    let mut ops = flat_base();
    let bit = fresh_bit(&ops);
    let mut store = Op::new(K::BitStore1);
    store.c_target = bit;
    ops.push(store);
    let mut push = Op::new(K::PushCondition);
    push.c_condition = bit;
    ops.push(push);
    ops.push(op(K::Z, 0, NONE, NONE)); // -1 on control-1 lanes
    ops.push(Op::new(K::PopCondition));
    let e = flat_rejects(&ops, "wrong-phase");
    assert!(e.contains("sign flipped"), "{e}");
}

/// Toffolis under `j` fresh measurement outcomes are charged at their probability `2^-j`
/// (spec/DESIGN.md section 8: the lane average is an expectation over outcomes). The block
/// here is a no-op pair, so the circuit stays valid; only the count moves.
#[test]
fn toffolis_under_fresh_outcomes_are_charged_by_their_probability() {
    let base = flat_eval(&flat_base()).unwrap();
    for j in [1u32, 3] {
        let mut ops = flat_base();
        let (a, scratch, bit) = (fresh_qubit(&ops), fresh_qubit(&ops) + 1, fresh_bit(&ops));
        let u0 = 1 + harness_common::N as u32;
        for i in 0..j {
            let mut h = op(K::Hmr, scratch, NONE, NONE);
            h.c_target = bit + i;
            ops.push(h);
        }
        for i in 0..j {
            let mut push = Op::new(K::PushCondition);
            push.c_condition = bit + i;
            ops.push(push);
        }
        ops.push(op(K::CCX, a, 0, u0));
        ops.push(op(K::CCX, a, 0, u0));
        ops.extend((0..j).map(|_| Op::new(K::PopCondition)));
        let ev = flat_eval(&ops).unwrap();
        let expect = 2.0 * 0.5f64.powi(j as i32);
        let got = ev.toffoli - base.toffoli;
        // Binomial noise over K_FLAT lanes.
        let sd = (2.0 * 2.0 * 0.5f64.powi(j as i32) / K_FLAT as f64).sqrt();
        assert!(
            (got - expect).abs() < 5.0 * sd + 1e-9,
            "j = {j}: {got} vs {expect}"
        );
    }
}

/// A `Neg` under a fresh outcome puts `-1` on about half the lanes: phase garbage on control-0
/// lanes, caught.
#[test]
fn an_outcome_conditioned_global_phase_is_rejected() {
    let mut ops = flat_base();
    let (scratch, bit) = (fresh_qubit(&ops), fresh_bit(&ops));
    let mut h = op(K::Hmr, scratch, NONE, NONE);
    h.c_target = bit;
    ops.push(h);
    let mut neg = Op::new(K::Neg);
    neg.c_condition = bit;
    ops.push(neg);
    flat_rejects(&ops, "phase-garbage");
}

// ------------------------------------------------------------------ registers (nested fixture)

fn nested_base() -> (femoco_walk::spec::df::DfSpec, Vec<u8>, Vec<Op>) {
    let s = spec();
    let map = exact_map(&s, OneBody::Direct);
    let ops = nested_common::build(&s, &map, nested_common::Mutation::None);
    (s, map.to_bytes(), ops)
}

fn inner_qubits() -> Vec<u32> {
    let s = spec();
    let map = exact_map(&s, OneBody::Direct);
    let first_uniform = 1 + 2 * nested_common::SPATIAL as u32;
    (0..map.inner_width())
        .map(|i| first_uniform + map.outer_bits() + i)
        .collect()
}

fn register(id: u32, qubits: &[u32]) -> Vec<Op> {
    let mut out = vec![{
        let mut o = Op::new(K::Register);
        o.r_target = id;
        o
    }];
    for &q in qubits {
        let mut o = Op::new(K::AppendToRegister);
        o.q_target = q;
        o.r_target = id;
        out.push(o);
    }
    out
}

/// The `Reflect`'s register may list the inner qubits in any order (each once).
#[test]
fn reflect_register_in_reverse_order_is_accepted() {
    let (s, lm, mut ops) = nested_base();
    let mut qs = inner_qubits();
    qs.reverse();
    let r = ops.iter().position(|o| o.kind == K::Reflect).unwrap();
    ops[r].r_target = 99;
    ops.splice(0..0, register(99, &qs));
    eval_ops(&s, &lm, &ops, &NESTED_AXES, K_NESTED).unwrap();
}

/// A register that names an inner qubit twice (and so misses another) is not the inner
/// register.
#[test]
fn reflect_register_with_a_duplicate_qubit_is_rejected() {
    let (s, lm, mut ops) = nested_base();
    let mut qs = inner_qubits();
    qs[1] = qs[0];
    let r = ops.iter().position(|o| o.kind == K::Reflect).unwrap();
    ops[r].r_target = 99;
    ops.splice(0..0, register(99, &qs));
    let e = eval_ops(&s, &lm, &ops, &NESTED_AXES, K_NESTED).expect_err("must be rejected");
    assert!(
        e.contains("must act on exactly the inner uniform register"),
        "{e}"
    );
}

/// A register is read with its final contents: `AppendToRegister` ops after a `Givens` that
/// names the register still feed that `Givens` (spec/DESIGN.md section 5 declares registers
/// as bookkeeping). Pinned so that a change to this is deliberate; it is harmless, because a
/// qubit that is to carry information at the `Givens` must have been written by a gate, and
/// that gate makes it live.
#[test]
fn a_givens_register_is_read_with_its_final_contents() {
    let (s, lm, mut ops) = nested_base();
    // Register 0 of the fixture is the angle register; move every Givens to an empty register
    // 99 declared first, and append the angle qubits to 99 only at the very end.
    let bits: Vec<u32> = ops
        .iter()
        .filter(|o| o.kind == K::AppendToRegister && o.r_target == 0)
        .map(|o| o.q_target)
        .collect();
    assert_eq!(bits.len(), nested_common::BETA as usize);
    for o in ops.iter_mut().filter(|o| o.kind == K::Givens) {
        o.r_target = 99;
    }
    let tail: Vec<Op> = register(99, &bits)[1..].to_vec();
    ops.extend(tail);
    ops.splice(0..0, register(99, &[]));
    eval_ops(&s, &lm, &ops, &NESTED_AXES, K_NESTED).unwrap();
    // And with the angle qubits never appended the rotations read angle 0 and the lane fails.
    let (s, lm, mut ops) = nested_base();
    for o in ops.iter_mut().filter(|o| o.kind == K::Givens) {
        o.r_target = 99;
    }
    ops.splice(0..0, register(99, &[]));
    let e = eval_ops(&s, &lm, &ops, &NESTED_AXES, K_NESTED).expect_err("angle 0 everywhere");
    assert!(e.starts_with("tracker:"), "{e}");
}

/// The control and uniform registers cannot be measured, whatever the op.
#[test]
fn hmr_on_a_uniform_or_control_qubit_is_rejected_at_compile() {
    for q in [0u32, 1 + harness_common::N as u32] {
        let mut ops = flat_base();
        let bit = fresh_bit(&ops);
        let mut h = op(K::Hmr, q, NONE, NONE);
        h.c_target = bit;
        ops.push(h);
        let e = flat_eval(&ops).expect_err("must be rejected");
        assert!(e.contains("cannot be measured or reset"), "{e}");
    }
}

/// The lane map's `u` is the uniform width the circuit is simulated at: a lane map whose
/// declared width differs from the one the circuit was built for is rejected, never silently
/// re-laid out. Here the wider map turns the fixture's first ancilla into uniform bit 6, which
/// the static pass refuses to reset; a circuit that survives compilation fails on its lanes.
#[test]
fn a_lane_map_of_another_width_is_rejected() {
    use femoco_walk::lanemap::alias::AliasMap;
    use femoco_walk::spec::Exact;
    let wider = AliasMap::from_counts(3, 4, Exact::from_int(1), &[64, 32, 16, 8, 8]).unwrap();
    assert_eq!(wider.uniform_bits(), 7);
    let e = eval_with(&flat_base(), &wider, K_FLAT, None).expect_err("must be rejected");
    println!("{e}");
    assert!(
        e.contains("control and uniform registers cannot be measured or reset")
            || e.starts_with("wrong-term:")
            || e.starts_with("control-0-not-identity:")
            || e.starts_with("registers-not-restored:"),
        "{e}"
    );
}
