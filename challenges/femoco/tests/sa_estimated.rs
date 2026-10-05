//! The estimated rounding class (spec/SPEC-SA.md section 14, `spec::rounding`): an sos-sa-only spec addition.
//! `reiher-sa-est-v1` / `li-sa-est-v1` state the same operator as their rigorous twins and judge a
//! lane map's rounding error by Low et al. 2025's estimated procedure instead of the 0.1 mHa
//! 1-norm. These tests pin that:
//! - every other spec stays rigorous, and a rigorous spec still rejects the authors' 9 + 9 keep bits;
//! - the class cannot be declared anywhere else, or with other constants;
//! - under the class, the authors' keep bits pass, fewer bits fail, and a lane map that is not a
//!   floor-or-ceiling rounding of the spec's weights (or declares another `lambda`) fails;
//! - score.json and results.tsv label every run of the class, and rigorous runs gain nothing.
#![cfg(feature = "walk")]
use femoco_walk::circuit::{read_ops, write_ops, Builder, OpsFile};
use femoco_walk::equiv::evaluate_checked as evaluate;
use femoco_walk::lanemap::sa_nested::{self, SaNestedMap};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{score_json, Evaluation, FamilyOut, Inputs, ResultsRow};
use femoco_walk::spec::rounding::{
    self, EstimatedParams, RoundingClass, ESTIMATED_CLASS, ESTIMATED_LABEL,
};
use femoco_walk::spec::sa::SaSpec;
use femoco_walk::spec::{self, EncodingSpec, Exact};
use femoco_walk::walk::sa_low::{self, Params};
use femoco_walk::{sim, taxonomy};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static TMP: AtomicUsize = AtomicUsize::new(0);

fn tmp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "femoco-estbits-{tag}-{}-{}",
        std::process::id(),
        TMP.fetch_add(1, Ordering::SeqCst)
    ))
}

const EST: [&str; 2] = ["reiher-sa-est-v1", "li-sa-est-v1"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn load(id: &str) -> Box<dyn EncodingSpec> {
    spec::load(root(), id).unwrap()
}

fn sa(s: &dyn EncodingSpec) -> &SaSpec {
    s.as_any().downcast_ref().unwrap()
}

fn params(id: &str) -> EstimatedParams {
    rounding::pinned(id).unwrap()
}

/// The walk's lane map (`sa_low::lane_map`) at the given keep bits.
fn map_at(s: &SaSpec, mu_o: u32, mu_i: u32) -> SaNestedMap {
    let p = Params::for_spec(s);
    sa_low::lane_map(
        s,
        Params {
            outer: (p.outer.0, mu_o),
            inner: (p.inner.0, mu_i),
            ..p
        },
    )
    .unwrap()
}

/// Builds `sa-toff` (default levers) at the given keep bits and runs the trusted `evaluate` at `K`.
fn run(id: &str, mu: Option<(u32, u32)>, samples: usize) -> Result<Evaluation, String> {
    let boxed = load(id);
    let s = sa(boxed.as_ref());
    let base = Params::for_spec(s);
    let p = Params {
        tw: sa_low::Tweaks::parse("all"),
        outer: mu.map_or(base.outer, |m| (base.outer.0, m.0)),
        inner: mu.map_or(base.inner, |m| (base.inner.0, m.1)),
        ..base
    };
    let mut b = Builder::new(boxed.system_qubits());
    let lm = sa_low::build_with(s, &mut b, p);
    let ops = b.finish();
    let dir = tmp(id);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ops.bin");
    write_ops(&ops, &path).unwrap();
    let file: OpsFile = read_ops(&path).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let tax = taxonomy::load_taxonomy(&root().join("taxonomy/taxonomy.json")).unwrap();
    let check = |f: &_, facts: &_| taxonomy::check(&tax, f, facts);
    let fam = serde_json::to_vec(&FamilyOut {
        family: sa_low::family_toff(),
        spec: id.to_string(),
    })
    .unwrap();
    evaluate(&Inputs {
        spec: boxed.as_ref(),
        lanemap: &lm.to_bytes(),
        family: &fam,
        ops: &file,
        samples,
        tracker: sim::givens_tracker(boxed.as_ref()),
        check: &check,
    })
}

#[test]
fn only_the_two_pinned_specs_are_estimated() {
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(root().join("specs/INDEX.json")).unwrap())
            .unwrap();
    for entry in index["specs"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        let s = load(id);
        let want = rounding::pinned(id).map_or(RoundingClass::Rigorous, |p| {
            RoundingClass::EstimatedLow2025(p)
        });
        assert_eq!(s.rounding_class(), want, "{id}");
    }
    // The twins state the same operator.
    for (est, rig) in EST.iter().zip(["reiher-sa-v1", "li-sa-v1"]) {
        let (a, b) = (load(est), load(rig));
        assert_eq!(a.payload_sha256(), b.payload_sha256());
        let (a, b) = (sa(a.as_ref()), sa(b.as_ref()));
        assert_eq!(a.lambda, b.lambda);
        assert_eq!(a.identity, b.identity);
        assert_eq!(a.e_sos, b.e_sos);
        assert_eq!(a.lambda_flat, b.lambda_flat);
        assert_eq!(a.published_lambda_eff, b.published_lambda_eff);
    }
}

/// A scratch `specs/` holding a copy of `from`'s files as `id`, with `edit` applied to spec.json.
fn scratch(from: &str, id: &str, edit: impl Fn(&mut Value)) -> PathBuf {
    let dir = tmp(&format!("spec-{id}"));
    let spec_dir = dir.join("specs").join(id);
    std::fs::create_dir_all(&spec_dir).unwrap();
    let src = root().join("specs").join(from);
    for e in std::fs::read_dir(&src).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), spec_dir.join(e.file_name())).unwrap();
    }
    let mut meta: Value =
        serde_json::from_str(&std::fs::read_to_string(spec_dir.join("spec.json")).unwrap())
            .unwrap();
    meta["id"] = Value::from(id);
    edit(&mut meta);
    std::fs::write(spec_dir.join("spec.json"), meta.to_string()).unwrap();
    dir
}

/// A spec edit applied to a scratch copy of `spec.json`.
type Edit = Box<dyn Fn(&mut Value)>;

#[test]
fn the_class_is_refused_off_its_pin() {
    let block = |v: &Value| v["rounding_class"].clone();
    let est_meta: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("specs/reiher-sa-est-v1/spec.json")).unwrap(),
    )
    .unwrap();
    let cases: Vec<(&str, &str, Edit, &str)> = vec![
        (
            "reiher-sa-est-v1",
            "reiher-sa-est-v1",
            Box::new(|m: &mut Value| m["rounding_class"]["coeff_const_fits"][0] = 6.0.into()),
            "differ from the harness's pin",
        ),
        (
            "reiher-sa-est-v1",
            "reiher-sa-est-v1",
            Box::new(|m: &mut Value| m["rounding_class"]["sigma_trunc_budget_mHa"] = 1.6.into()),
            "differ from the harness's pin",
        ),
        (
            "reiher-sa-est-v1",
            "reiher-sa-est-v1",
            Box::new(|m: &mut Value| m["rounding_class"]["class"] = "estimated-other".into()),
            "unknown rounding_class",
        ),
        (
            "reiher-sa-v1",
            "reiher-sa-v1",
            Box::new(move |m: &mut Value| m["rounding_class"] = block(&est_meta)),
            "the estimated rounding class is pinned for",
        ),
        (
            "reiher-sa-v1",
            "reiher-sa-v2",
            Box::new(|m: &mut Value| {
                m["rounding_class"] = serde_json::json!({ "class": ESTIMATED_CLASS })
            }),
            "the estimated rounding class is pinned for",
        ),
    ];
    for (from, id, edit, want) in cases {
        let dir = scratch(from, id, edit);
        let err = spec::load(&dir, id).err().unwrap_or_default();
        std::fs::remove_dir_all(&dir).ok();
        assert!(err.contains(want), "{id}: {err}");
    }
    // A non-sos-sa spec under a pinned id with the class is refused too.
    let est_meta: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("specs/reiher-sa-est-v1/spec.json")).unwrap(),
    )
    .unwrap();
    let dir = scratch("reiher-sa-est-v1", "reiher-sa-est-v1", |m| {
        m["encoding"] = Value::from("thc");
        m["rounding_class"] = est_meta["rounding_class"].clone();
    });
    let err = spec::load(&dir, "reiher-sa-est-v1")
        .err()
        .unwrap_or_default();
    std::fs::remove_dir_all(&dir).ok();
    assert!(err.contains("sos-sa specs only"), "{err}");
}

#[test]
fn the_rigorous_twins_still_reject_the_authors_keep_bits() {
    for id in ["reiher-sa-v1", "li-sa-v1"] {
        let err = run(id, Some((9, 9)), 64).err().unwrap_or_default();
        assert!(err.contains("exceeds 0.1 mHa"), "{id}: {err}");
    }
}

#[test]
fn the_estimate_follows_their_bit_rule() {
    for id in EST {
        let boxed = load(id);
        let s = sa(boxed.as_ref());
        let p = params(id);
        let at = |mu_o: u32, mu_i: u32| {
            map_at(s, mu_o, mu_i)
                .rounding_estimate(boxed.as_ref(), &p)
                .unwrap()
        };
        // 9 + 9: our tables are finer than their M = L 2^(b-1) bins by one bit (b_equiv 10).
        let e = at(9, 9);
        assert_eq!((e.b_equiv_outer, e.b_equiv_inner), (10, 10), "{id}");
        assert!(e.accepted() && e.rejection().is_none(), "{id}");
        assert_eq!(e.tables, s.outer_items() + 1);
        assert_eq!(e.beta, u32::from(s.beta));
        // The boundary: b_equiv 9 passes, 8 fails, on either level.
        // (b_equiv of 324 / 361 outer items at k_o + mu_o = 17, and of 28 / 58 inner items at
        // k_i + mu_i = 13 / 14, is 9 on both instances.)
        let (outer_9, inner_9) = (8, 8);
        assert!(at(outer_9, inner_9).accepted(), "{id}");
        let low_outer = at(outer_9 - 1, 9);
        assert_eq!(low_outer.b_coeff(), 8, "{id}");
        assert!(
            low_outer.rejection().unwrap().contains("below the 9"),
            "{id}"
        );
        let low_inner = at(9, inner_9 - 1);
        assert_eq!(low_inner.b_equiv_inner, 8, "{id}");
        assert!(!low_inner.accepted(), "{id}");
        // sigma at 9 + 9 against their bits (9): half theirs.
        let their = EstimatedParams::sigma_mha(p.coeff_const(), 9);
        assert!((e.sigma_coeff_mha() - their / 2.0).abs() < 1e-12);
    }
}

#[test]
fn the_estimate_refuses_a_map_that_is_not_a_rounding() {
    let id = "reiher-sa-est-v1";
    let boxed = load(id);
    let s = sa(boxed.as_ref());
    let p = params(id);
    let good = map_at(s, 9, 9);
    // Move two lanes of one bucket from its own item to its alias: both items end two lanes from a
    // count that was already within one lane of its ideal.
    let mut bad = map_at(s, 9, 9);
    let t = &mut bad.inner[s.n];
    let j = (0..t.keep.len()).find(|&j| t.keep[j] >= 2).unwrap();
    t.keep[j] -= 2;
    let err = bad.rounding_estimate(boxed.as_ref(), &p).err().unwrap();
    assert!(err.contains("not the floor or the"), "{err}");
    // Same counts but lambda_decl off Lambda by one ulp of its dyadic.
    let lam = good.lambda_decl.add(&Exact::dyadic(1.into(), 200));
    let off = SaNestedMap::new(lam, good.outer.clone(), good.inner.clone(), s).unwrap();
    let err = off.rounding_estimate(boxed.as_ref(), &p).err().unwrap();
    assert!(err.contains("must equal the spec's Lambda"), "{err}");
    // Largest-remainder counts of sa_nested::build pass.
    let lr = sa_nested::build(s, (9, 9), (5, 9)).unwrap();
    assert!(lr.rounding_estimate(boxed.as_ref(), &p).is_ok());
}

#[test]
fn the_authors_keep_bits_pass_and_are_labelled() {
    for id in EST {
        let ev = run(id, None, 256).unwrap();
        let est = ev.rounding_estimate.clone().unwrap();
        assert!(est.accepted());
        assert_eq!(est.outer_bits.1, 9);
        assert_eq!(est.inner_bits.1, 9);
        // The exact 1-norm is still computed and is far above 0.1 mHa (7.97e-2 / 2.83e-1 Ha).
        let one = ev.rounding_error.to_f64();
        assert!(one > 0.05, "{id}: {one}");
        let j = score_json(&ev);
        let rc = &j["metrics"]["rounding_class"];
        assert_eq!(rc["label"], ESTIMATED_LABEL);
        assert_eq!(rc["class"], ESTIMATED_CLASS);
        assert_eq!(rc["kind"], "estimate, not a bound");
        assert_eq!(rc["rigorous_rule_met"], false);
        assert_eq!(rc["accepted"], true);
        assert_eq!(rc["b_coeff_required"], 9);
        assert_eq!(rc["rigorous_one_norm_Ha"].as_f64().unwrap(), one);
        assert_eq!(j["metrics"]["spec"], id);
        // The rigorous lambda_eff bound still carries twice the exact 1-norm.
        let sa_view = &j["metrics"]["spectral_amplification"];
        assert_eq!(sa_view["available"], true);
        let row = ResultsRow::from_eval(&ev);
        assert_eq!(row.spec, id);
    }
}

#[test]
fn rigorous_runs_gain_no_class_block() {
    let ev = run("reiher-sa-v1", None, 64).unwrap();
    assert!(ev.rounding_estimate.is_none());
    let j = score_json(&ev);
    assert!(j["metrics"].get("rounding_class").is_none());
}

#[test]
fn estimated_runs_need_enough_bits() {
    let err = run("reiher-sa-est-v1", Some((9, 7)), 64)
        .err()
        .unwrap_or_default();
    assert!(err.contains("estimated rounding class"), "{err}");
}
