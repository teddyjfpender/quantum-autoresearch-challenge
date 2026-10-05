//! Lane-engine equivalence on op-stream mutants (tests/equiv_common `Mut`): every mutation for
//! every op kind present, plus the kind-free ones (dirty ancillas, system Paulis and `S`, phase,
//! conditions, `Givens` registers and modes, `SpinSwap` direction, rare-lane ladders, random
//! rare-lane conditions, the three documented verifier differences, twin mutations of nested
//! inner copies), each compared between the reference and the candidate (`FEMOCO_EQUIV_ENGINE`)
//! on the whole `score.json` or rejection, the lanes, the verdict vector and the `Outcome`.
//!
//! The fixture sweeps (the small specs of tests/harness_common, df_common, thc_common and
//! nested_common) run by default; the sweeps of full-size circuits are ignored and run by
//! tools/fastsim/equivalence.sh. The fixtures' own named mutants (`Mutation`) run through
//! `equiv::evaluate_checked` in their own test files.
#![cfg(feature = "walk")]
mod df_common;
mod equiv_common;
mod harness_common;
mod nested_common;
mod thc_common;

use equiv_common::{assert_equal, mutant_sweep, Summary};
use femoco_walk::lanemap::df_nested::OneBody;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::sim::givens_tracker;
use femoco_walk::spec::EncodingSpec;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A fixture's mutant sweep: `FEMOCO_EQUIV_MUTANT_PER` points per mutation (default 1) and
/// `FEMOCO_EQUIV_MUTANT_SEEDS` seed modes per mutant (default 1).
fn sweep_fixture(
    name: &str,
    spec: &dyn EncodingSpec,
    lm: &dyn LaneMap,
    family: &[u8],
    ops: &[femoco_walk::circuit::Op],
) -> Summary {
    let _ = givens_tracker(spec);
    mutant_sweep(
        name,
        spec,
        &lm.to_bytes(),
        family,
        ops,
        lm.uniform_bits(),
        lm.inner_bits(),
        &harness_common::declared_only,
        &[1, 2, 3],
        env("FEMOCO_EQUIV_MUTANT_PER", 1),
        env("FEMOCO_EQUIV_MUTANT_SEEDS", 1),
        usize::MAX,
    )
}

#[test]
fn fixture_flat_mutants() {
    let spec = harness_common::TestSpec::new();
    let map = harness_common::alias();
    let ops = harness_common::build(&spec, &map, harness_common::Mutation::None);
    let fam = harness_common::family(spec.id());
    assert_equal(&sweep_fixture("fixture-flat", &spec, &map, &fam, &ops));
}

#[test]
fn fixture_df_mutants() {
    let spec = df_common::spec();
    let map = df_common::lanemap(&spec);
    let ops = df_common::build(&spec, &map, df_common::Mutation::None);
    let fam = df_common::family(spec.id());
    assert_equal(&sweep_fixture("fixture-df", &spec, &map, &fam, &ops));
}

#[test]
fn fixture_thc_mutants() {
    let spec = thc_common::spec();
    let map = thc_common::lanemap(&spec);
    let ops = thc_common::build(&spec, &map, thc_common::Mutation::None);
    let fam = thc_common::family(spec.id());
    assert_equal(&sweep_fixture("fixture-thc", &spec, &map, &fam, &ops));
}

#[test]
fn fixture_nested_mutants() {
    let spec = nested_common::spec();
    let map = nested_common::exact_map(&spec, OneBody::Folded);
    let ops = nested_common::build(&spec, &map, nested_common::Mutation::None);
    let fam = nested_common::family(&spec.id, &nested_common::NESTED_AXES);
    assert_equal(&sweep_fixture("fixture-nested", &spec, &map, &fam, &ops));
}

/// The lanemap byte layout's documented verifier difference 1:
/// a `lambda_decl` sign byte other than 0 or 1 reads as non-negative here. Shared parse code,
/// before any engine; checked so that both engines see the same lane map and verdicts.
#[test]
fn fixture_lambda_sign_byte() {
    use femoco_walk::spec::Exact;
    let spec = harness_common::TestSpec::new();
    let map = harness_common::alias();
    let ops = harness_common::build(&spec, &map, harness_common::Mutation::None);
    let fam = harness_common::family(spec.id());
    let mut lm = map.to_bytes();
    let lambda = map.lambda_decl().to_bytes();
    let at = lm
        .windows(lambda.len())
        .position(|w| w == lambda.as_slice())
        .expect("lambda_decl bytes in the lane map");
    let _ = Exact::zero();
    let file = equiv_common::ops_file(&ops);
    for sign in [2u8, 0xff] {
        lm[at] = sign;
        let inp = femoco_walk::score::Inputs {
            spec: &spec,
            lanemap: &lm,
            family: &fam,
            ops: &file,
            samples: equiv_common::samples(),
            tracker: None,
            check: &harness_common::declared_only,
        };
        let s = equiv_common::compare_artifact(
            &format!("lambda-sign-{sign}"),
            &inp,
            &equiv_common::seeds("lambda-sign"),
            &equiv_common::candidate(),
            &femoco_walk::equiv::env_threads(),
        );
        assert_equal(&s);
    }
}

fn sweep_artifact(a: &equiv_common::Artifact, rare: &[u32], max: usize) {
    let check = equiv_common::shipped_check();
    let s = mutant_sweep(
        &a.name,
        a.spec.as_ref(),
        &a.lanemap,
        &a.family,
        &a.ops.ops,
        a.uniform,
        a.inner,
        &check,
        rare,
        env("FEMOCO_EQUIV_MUTANT_PER", 1),
        env("FEMOCO_EQUIV_MUTANT_SEEDS", 1),
        max,
    );
    assert_equal(&s);
}

/// The rare-lane ladder widths for `K` lanes: matched lanes about `K / 2^(k+1)` = 8, 2 and 1/2.
fn rare_bits() -> Vec<u32> {
    let lg = equiv_common::samples().max(2).ilog2();
    [lg.saturating_sub(4), lg.saturating_sub(2), lg]
        .into_iter()
        .filter(|&k| k > 0)
        .collect()
}

#[test]
#[ignore = "heavy: tools/fastsim/equivalence.sh"]
fn full_sa_mutants() {
    use femoco_walk::spec::sa::SaSpec;
    use femoco_walk::walk::sa_low::{self, Params, Tweaks};
    let a = equiv_common::artifact("c1-R-lgr", "reiher-sa-v1", sa_low::family_toff(), |s, b| {
        let sa: &SaSpec = s.as_any().downcast_ref().unwrap();
        let p = Params {
            tw: Tweaks::parse("imchxlgr"),
            ..Params::for_spec(sa)
        };
        sa_low::build_with(sa, b, p)
    });
    let max = std::env::var("FEMOCO_EQUIV_SA_MUTANTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    sweep_artifact(&a, &rare_bits(), max);
}
