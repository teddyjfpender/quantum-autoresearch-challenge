//! A correct hand-written circuit passes and each required mutant (spec/DESIGN.md section 9) is
//! rejected with its specific reason.
mod harness_common;

use femoco_walk::lanemap::alias::AliasMap;
use femoco_walk::lanemap::LaneMap;
use harness_common::{alias, build, eval_mutant, eval_with, Mutation, TestSpec};
use num_bigint::BigInt;

const K: usize = 4096;

fn rejected(m: Mutation) -> String {
    match eval_mutant(m, K) {
        Ok(_) => panic!("{m:?} was accepted"),
        Err(e) => {
            eprintln!("{m:?}: {e}");
            e
        }
    }
}

#[test]
fn correct_circuit_passes_and_is_counted() {
    let ev = eval_mutant(Mutation::None, K).unwrap();
    // u = 6 ladder CCXs per uniform value, the top uncomputed by measurement: 6 + 5 per value.
    assert_eq!(
        ev.toffoli,
        64.0 * 11.0 + 4.0,
        "C_step = executed CCX + reflection (u - 2)"
    );
    assert_eq!(ev.reflection_toffoli, 4);
    // control + 6 system + 6 uniform + 6 ladder ancillas.
    assert_eq!(ev.qubits, 1 + 6 + 6 + 6);
    assert!(ev.rounding_error.is_zero());
    assert_eq!(ev.lambda.to_f64(), 1.0);
    assert!(
        ev.facts.measurement_uncompute,
        "Hmr + cz_if on the former controls"
    );
    assert!((ev.facts.executed_hmr - 64.0).abs() < 1e-12);
    assert_eq!(ev.score(), 1.0 * ev.toffoli * ev.qubits as f64);
}

#[test]
fn wrong_sign_on_one_term() {
    let e = rejected(Mutation::FlipSign(2));
    assert!(
        e.starts_with("wrong-phase:") && e.contains("sign flipped"),
        "{e}"
    );
}

#[test]
fn one_wrong_term() {
    let e = rejected(Mutation::WrongTerm(1));
    assert!(
        e.starts_with("wrong-term:") && e.contains("requires"),
        "{e}"
    );
}

#[test]
fn dirty_ancilla_freed() {
    let e = rejected(Mutation::DirtyFree(37));
    assert!(
        e.starts_with("dirty-ancilla-freed:") && e.contains("s=37"),
        "{e}"
    );
}

#[test]
fn dirty_ancilla_at_end() {
    let e = rejected(Mutation::DirtyEnd(12));
    assert!(
        e.starts_with("dirty-ancilla-at-end:") && e.contains("s=12"),
        "{e}"
    );
}

#[test]
fn phase_garbage() {
    let e = rejected(Mutation::PhaseGarbage);
    assert!(e.starts_with("phase-garbage:") && e.contains("c=0"), "{e}");
}

#[test]
fn control_ignored() {
    let e = rejected(Mutation::NoControl);
    assert!(e.starts_with("control-0-not-identity:"), "{e}");
}

/// The circuit realizes one exact alias table; the lane map declares another with the same
/// counts (so its rounding error is also zero). The lanes where they disagree are caught.
#[test]
fn lane_map_disagrees_with_circuit() {
    let spec = TestSpec::new();
    let circuit_map = alias();
    let declared = AliasMap::new(
        3,
        3,
        femoco_walk::spec::Exact::dyadic(BigInt::from(1), 0),
        vec![0, 0, 0, 4, 0, 0, 0, 0],
        vec![0, 0, 0, 4, 1, 1, 2, 0],
    )
    .unwrap();
    assert_eq!(declared.counts(5), vec![32, 16, 8, 4, 4]);
    assert!(declared.rounding_error(&spec).unwrap().is_zero());
    let ops = build(&spec, &circuit_map, Mutation::None);
    let e = eval_with(&ops, &declared, K, None).unwrap_err();
    assert!(e.starts_with("wrong-term:"), "{e}");
}

#[test]
fn lane_map_rounding_error_above_threshold() {
    let spec = TestSpec::new();
    let off = AliasMap::from_counts(
        3,
        3,
        femoco_walk::spec::Exact::dyadic(BigInt::from(1), 0),
        &[33, 15, 8, 4, 4],
    )
    .unwrap();
    let ops = build(&spec, &off, Mutation::None);
    let e = eval_with(&ops, &off, K, None).unwrap_err();
    assert!(e.contains("rounding error") && e.contains("0.1 mHa"), "{e}");
}

/// A circuit wrong on exactly one of the 128 lane values. With few samples most Fiat-Shamir
/// seeds miss it, so it would pass for a lucky seed; but the seed is a hash of the circuit
/// itself, and at the harness's sample counts every re-roll (here: padding the op stream with
/// no-op segment hints to change its hash) is rejected.
#[test]
fn passing_only_for_a_lucky_seed_is_not_possible() {
    let spec = TestSpec::new();
    let map = alias();
    let base = build(&spec, &map, Mutation::SignAt(41));
    let variant = |pad: usize| {
        let mut ops = base.clone();
        let mut hint = femoco_walk::circuit::Op::new(femoco_walk::circuit::OperationType::Segment);
        hint.r_target = 1;
        ops.extend(std::iter::repeat_n(hint, pad));
        ops
    };
    let lucky = (0..40)
        .filter(|&p| eval_with(&variant(p), &map, 8, None).is_ok())
        .count();
    assert!(
        lucky > 0,
        "with 8 samples some re-roll should miss the bad lane"
    );
    for p in 0..40 {
        let e = eval_with(&variant(p), &map, K, None).unwrap_err();
        assert!(e.contains("s=41") && e.contains("sign flipped"), "{e}");
    }
    // The outcome is a function of the submission: the same bytes give the same verdict.
    let again = (0..40)
        .filter(|&p| eval_with(&variant(p), &map, 8, None).is_ok())
        .count();
    assert_eq!(lucky, again);
}

/// TODO(taxonomy): the taxonomy stub returns `DeclaredOnly` for every axis, so a contradicted
/// axis cannot be expressed yet. Once `taxonomy::check` verifies the rotation axis from
/// `CircuitFacts` (e.g. `executed_givens`), un-ignore this: a family declaring a Givens rotation
/// implementation for a circuit with no `Givens` must be rejected with "taxonomy: declared".
#[test]
fn declared_family_axis_contradicted() {
    use femoco_walk::equiv::evaluate_checked as evaluate;
    use femoco_walk::score::Inputs;
    use femoco_walk::spec::EncodingSpec;
    let spec = TestSpec::new();
    let map = alias();
    // Declare an axis value the circuit cannot have: a Givens rotation implementation in a
    // circuit with no Givens (value name from taxonomy/taxonomy.json).
    let ops = build(&spec, &map, Mutation::None);
    let tax = femoco_walk::taxonomy::load_taxonomy(std::path::Path::new("taxonomy/taxonomy.json"))
        .unwrap();
    let file = harness_common::ops_file(&ops);
    let mut fam: femoco_walk::score::FamilyOut =
        serde_json::from_slice(&harness_common::family(spec.id())).unwrap();
    fam.family
        .axes
        .insert("rotation".into(), "phase-gradient-givens".into());
    let fam = serde_json::to_vec(&fam).unwrap();
    let seen = std::cell::RefCell::new(Vec::new());
    let check = |f: &femoco_walk::taxonomy::Family, facts: &femoco_walk::facts::CircuitFacts| {
        let v = femoco_walk::taxonomy::check(&tax, f, facts);
        seen.borrow_mut().clone_from(&v);
        v
    };
    let e = evaluate(&Inputs {
        spec: &spec,
        lanemap: &map.to_bytes(),
        family: &fam,
        ops: &file,
        samples: K,
        tracker: None,
        check: &check,
    })
    .unwrap_err();
    // Any contradicted axis rejects the run.
    assert!(e.starts_with("taxonomy: declared"), "{e}");
    // And the declared Givens implementation is itself contradicted by a Givens-free circuit.
    let rotation = seen
        .borrow()
        .iter()
        .find(|v| v.axis == "rotation")
        .cloned()
        .expect("a rotation verdict");
    assert!(
        matches!(
            rotation.status,
            femoco_walk::taxonomy::AxisStatus::Contradicted(_)
        ),
        "{rotation:?}"
    );
}

/// The grinding bounds keep confidence `1 - delta` against `2^40` free re-draws of the seed:
/// the numerator grows from `ln(1/delta)` to `ln(2^40/delta)`, about 3.0x at `delta = 1e-6`.
#[test]
fn grinding_bounds_charge_the_redraw_budget() {
    use femoco_walk::score::{DELTA, GRINDING_LOG2};
    let ev = eval_mutant(Mutation::None, K).unwrap();
    let single = (1.0 / DELTA).ln() / K as f64;
    let ground =
        ((1.0 / DELTA).ln() + f64::from(GRINDING_LOG2) * std::f64::consts::LN_2) / K as f64;
    assert!((ev.f_bound() - single).abs() < 1e-15);
    assert!((ev.f_bound_at(GRINDING_LOG2) - ground).abs() < 1e-15);
    assert!((ground / single - 3.007).abs() < 1e-3);
    let lambda = ev.lambda.to_f64();
    // The implied bound counts the control-1 lanes only (spec/DESIGN.md section 9).
    let ground_1 = ground * K as f64 / ev.control_one_samples as f64;
    assert!((ev.implied_error_bound_at(GRINDING_LOG2) - 2.0 * lambda * ground_1).abs() < 1e-12);
    let json = femoco_walk::score::score_json(&ev);
    assert_eq!(json["metrics"]["grinding_draws_log2"], 40);
    assert!(
        json["metrics"]["f_bound_grinding"].as_f64().unwrap()
            > json["metrics"]["f_bound"].as_f64().unwrap()
    );
}
