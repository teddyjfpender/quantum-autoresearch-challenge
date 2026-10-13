#![cfg(feature = "walk")]
use femoco_walk::circuit::{Builder, Op, OperationType, OpsFile};
use femoco_walk::coverage::{self, Mode};
use femoco_walk::lanemap::df_nested::Table;
use femoco_walk::lanemap::sa_nested::SaNestedMap;
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::{FamilyOut, Inputs};
use femoco_walk::sim;
use femoco_walk::spec::{self, sa::SaSpec, EncodingSpec};
use femoco_walk::walk::sa_low::{self, Params, Tweaks};
use std::collections::BTreeSet;
use std::path::Path;

fn tiny() -> SaSpec {
    tiny_with([2.0, 2.0])
}

/// The tiny spec with the given one-body energies. Unequal energies give the outer alias
/// table non-zero keep values, so its comparator boundaries lie inside the sigma range.
fn tiny_with(e: [f64; 2]) -> SaSpec {
    // Two orbitals, one square: outer weights 4,4,8 and equal inner weights.
    // Both alias tables can be rounded EXACTLY, so correctness tests need no tolerance
    // in the coefficient rule. Rotation words remain the declared exact 8-bit words.
    let mut payload = b"FEMOSAS1".to_vec();
    for v in [1u32, 2, 1, 1, 1, 8, 2] {
        payload.extend(v.to_le_bytes());
    }
    for v in [0.0f64, e[0], e[1]] {
        payload.extend(v.to_le_bytes());
    }
    for v in [17u32, 43] {
        payload.extend(v.to_le_bytes());
    }
    for v in [2.0f64, 2.0] {
        payload.extend(v.to_le_bytes());
    }
    payload.extend(67u32.to_le_bytes());
    spec::sa::parse_payload("test-coverage-sa", &payload).unwrap()
}

fn params(s: &SaSpec) -> Params {
    Params {
        outer: (2, if s.b > 1 { 4 } else { 2 }),
        inner: if s.b > 1 { (2, 2) } else { (1, 1) },
        outer_a: 0,
        inner_a: 0,
        tw: Tweaks::OFF,
        ..Params::for_spec(s)
    }
}

fn map(s: &SaSpec) -> SaNestedMap {
    sa_low::lane_map(s, params(s)).unwrap()
}

#[test]
fn boundaries_include_both_sides_of_every_comparison() {
    for mu in 0..5u32 {
        let cap = 1u64 << mu;
        let table = Table {
            k: 2,
            mu,
            keep: vec![0, (cap - 1) as u32, (cap / 2) as u32, 0],
            alt: vec![0, 0, 1, 2],
        };
        let got: BTreeSet<_> = coverage::boundaries(&table).into_iter().collect();
        for i in 0..4 {
            for d in 0..cap {
                if d == 0
                    || d == cap - 1
                    || d == u64::from(table.keep[i])
                    || d + 1 == u64::from(table.keep[i])
                {
                    assert!(got.contains(&(i as u64 | d << table.k)));
                }
            }
        }
        assert!(got.iter().all(|s| *s < 1 << (table.k + mu)));
    }
}

#[test]
fn representatives_cover_exactly_the_nonzero_counts() {
    let table = Table::from_counts(3, 2, &[0, 4, 0, 7, 12, 9]).unwrap();
    let reps = coverage::representatives(&table, 6);
    for (i, (rep, count)) in reps.iter().zip(table.counts(6)).enumerate() {
        assert_eq!(rep.is_some(), count != 0);
        if let Some(s) = rep {
            assert_eq!(table.item(*s), i);
        }
    }
}

#[test]
fn exhaustive_plan_contains_every_input_pair_and_both_controls() {
    let s = tiny();
    let map = map(&s);
    let p = coverage::plan(&map, &s, Mode::Exhaustive, 4096).unwrap();
    let got: BTreeSet<_> = p
        .lanes
        .iter()
        .zip(p.after)
        .map(|(l, a)| (l.c, l.s, a))
        .collect();
    assert_eq!(got.len(), 4096);
    for v in 0..1u64 << map.uniform_bits() {
        for after in 0..1u64 << map.inner_width() {
            let a = (v & ((1 << map.outer_bits()) - 1)) | after << map.outer_bits();
            for c in [false, true] {
                assert!(got.contains(&(c, v, a)));
            }
        }
    }
    assert!(coverage::plan(&map, &s, Mode::Exhaustive, 4095)
        .unwrap_err()
        .contains("above cap"));
    assert!(coverage::plan(&map, &s, Mode::Terms, 2)
        .unwrap_err()
        .contains("no partial pass"));
}

#[test]
fn exhaustive_correctness_and_a_phase_mutant() {
    let spec = tiny();
    let mut b = Builder::new(spec.system_qubits());
    let lm = sa_low::build_with(&spec, &mut b, params(&spec));
    let ops = b.finish();
    let fam = serde_json::to_vec(&FamilyOut {
        family: sa_low::family(true),
        spec: spec.id().into(),
    })
    .unwrap();
    let check = |_: &_, _: &_| Vec::new();
    for mutated in [false, true] {
        let mut list = ops.clone();
        if mutated {
            let mut op = Op::new(OperationType::CCZ);
            (op.q_target, op.q_control1, op.q_control2) = (
                0,
                1 + spec.system_qubits() as u32,
                2 + spec.system_qubits() as u32,
            );
            list.push(op);
        }
        let file = OpsFile {
            ops: list,
            sha256: [0; 32],
        };
        let input = Inputs {
            spec: &spec,
            lanemap: &lm.to_bytes(),
            family: &fam,
            ops: &file,
            samples: 1,
            tracker: sim::givens_tracker(&spec),
            check: &check,
        };
        let result = coverage::check(
            &input,
            sim::validate::reference,
            Some(Mode::Exhaustive),
            None,
            None,
        );
        if mutated {
            assert!(result.unwrap_err().contains("deterministic coverage"));
        } else {
            let v = result.unwrap().unwrap();
            assert_eq!(v["lanes"], 4096);
            assert_eq!(v["certified"], false);
        }
    }
}

#[test]
fn promoted_precision_meets_exact_budget_and_estimated_precision_does_not() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (id, mu) in [("reiher-sa-v1", (19, 19)), ("li-sa-v1", (20, 21))] {
        let boxed = spec::load(root, id).unwrap();
        let s = boxed.as_any().downcast_ref::<SaSpec>().unwrap();
        let p = Params::for_spec(s);
        let rigorous = sa_low::lane_map(
            s,
            Params {
                outer: (p.outer.0, mu.0),
                inner: (p.inner.0, mu.1),
                ..p
            },
        )
        .unwrap();
        assert!(rigorous.rounding_error(s).unwrap() <= femoco_walk::score::max_rounding_error());
        let estimated = sa_low::lane_map(
            s,
            Params {
                outer: (p.outer.0, 8),
                inner: (p.inner.0, 8),
                ..p
            },
        )
        .unwrap();
        assert!(estimated.rounding_error(s).unwrap() > femoco_walk::score::max_rounding_error());
    }
}

/// The circuit and lane map `sa_low` builds for `spec`, as evaluator inputs.
fn built(spec: &SaSpec) -> (Vec<Op>, Vec<u8>, Vec<u8>) {
    let mut b = Builder::new(spec.system_qubits());
    let lm = sa_low::build_with(spec, &mut b, params(spec));
    let fam = serde_json::to_vec(&FamilyOut {
        family: sa_low::family(true),
        spec: spec.id().into(),
    })
    .unwrap();
    (b.finish(), lm.to_bytes(), fam)
}

fn run(
    spec: &SaSpec,
    ops: Vec<Op>,
    lanemap: &[u8],
    family: &[u8],
    mode: Mode,
) -> Result<serde_json::Value, String> {
    let check = |_: &_, _: &_| Vec::new();
    let file = OpsFile {
        ops,
        sha256: [0; 32],
    };
    let input = Inputs {
        spec,
        lanemap,
        family,
        ops: &file,
        samples: 1,
        tracker: sim::givens_tracker(spec),
        check: &check,
    };
    coverage::check(&input, sim::validate::reference, Some(mode), None, None)
        .map(|v| v.expect("a coverage report"))
}

#[test]
fn terms_plan_enumerates_every_spin_value_of_every_term_pair() {
    let s = tiny();
    let map = map(&s);
    let (lo, w) = (map.outer_bits(), map.inner_width());
    let p = coverage::plan(&map, &s, Mode::Terms, 1 << 20).unwrap();
    let got: BTreeSet<_> = p
        .lanes
        .iter()
        .zip(&p.after)
        .map(|(l, a)| (l.c, l.s, *a))
        .collect();
    let outer = coverage::representatives(&map.outer, s.outer_items());
    let mut pairs = 0;
    for (o, rep) in outer.into_iter().enumerate() {
        let rep = rep.expect("every outer item of the tiny spec is reachable");
        let inner: Vec<u64> = coverage::representatives(&map.inner[o], s.inner_items(o))
            .into_iter()
            .flatten()
            .collect();
        for outer_spin in 0..2u64 {
            let out = rep | outer_spin << (lo - 1);
            for a in &inner {
                for b in &inner {
                    for (sa, sb) in [(0u64, 0u64), (0, 1), (1, 0), (1, 1)] {
                        let first = out | (a | sa << (w - 1)) << lo;
                        let second = out | (b | sb << (w - 1)) << lo;
                        for c in [false, true] {
                            assert!(
                                got.contains(&(c, first, second)),
                                "item {o}, outer spin {outer_spin}, inner spins {sa}{sb}, c={c}"
                            );
                        }
                        pairs += 1;
                    }
                }
            }
        }
    }
    assert_eq!(p.term_pairs, pairs);
}

/// A sign error that appears only when the outer spin bit is 1 on an off-diagonal term pair
/// of a square, where the reference operator does not depend on that bit. Coverage that ran
/// squares with outer spin 0 only would accept it.
fn spin_keyed_mutant(spec: &SaSpec, ops: &[Op], map: &SaNestedMap) -> Vec<Op> {
    let system = spec.system_qubits() as u32;
    let uniform = |bit: u32| system + 1 + bit;
    let lo = map.outer_bits();
    let scratch = ops
        .iter()
        .flat_map(|op| [op.q_target, op.q_control1, op.q_control2])
        .filter(|&q| q != femoco_walk::circuit::NONE)
        .max()
        .unwrap()
        + 1;
    let gate = |kind, t, c1, c2| {
        let mut op = Op::new(kind);
        (op.q_target, op.q_control1, op.q_control2) = (t, c1, c2);
        op
    };
    // Wire 0 is the control and the uniform register follows the system qubits.
    // scratch = (outer index bit 1) AND (outer spin); then a controlled sign on an inner index bit.
    let flip = [
        gate(OperationType::CCX, scratch, uniform(1), uniform(lo - 1)),
        gate(OperationType::CCZ, 0, scratch, uniform(lo)),
        gate(OperationType::CCX, scratch, uniform(1), uniform(lo - 1)),
    ];
    let mut out = Vec::new();
    let mut inserted = 0;
    for op in ops {
        if op.kind == OperationType::Segment && op.r_target == femoco_walk::circuit::SEG_INNER_END {
            out.extend(flip);
            inserted += 1;
        }
        out.push(*op);
    }
    assert_eq!(inserted, 2, "one insertion per inner pass");
    out
}

#[test]
fn terms_accepts_the_circuit_and_rejects_a_spin_keyed_sign_error() {
    let spec = tiny();
    let (ops, lanemap, family) = built(&spec);
    let report = run(&spec, ops.clone(), &lanemap, &family, Mode::Terms).unwrap();
    assert_eq!(report["mode"], "Terms");
    assert_eq!(report["protocol"], "femoco-deterministic-v2");
    assert_eq!(report["certified"], false);
    let mutant = spin_keyed_mutant(&spec, &ops, &map(&spec));
    // The mutant is a real error: exhaustive enumeration rejects it.
    assert!(
        run(&spec, mutant.clone(), &lanemap, &family, Mode::Exhaustive)
            .unwrap_err()
            .contains("deterministic coverage")
    );
    assert!(run(&spec, mutant, &lanemap, &family, Mode::Terms)
        .unwrap_err()
        .contains("deterministic coverage"));
}

#[test]
fn terms_crosses_alias_boundaries_that_lie_inside_the_sigma_range() {
    let spec = tiny_with([1.0, 3.0]);
    let m = map(&spec);
    assert!(
        m.outer.keep.iter().any(|&k| k != 0),
        "the outer table needs a non-trivial comparator for this test"
    );
    let (ops, lanemap, family) = built(&spec);
    let report = run(&spec, ops, &lanemap, &family, Mode::Terms).unwrap();
    assert!(report["boundary_cases"].as_u64().unwrap() > 0);
    // The two lanes either side of the comparator select different outer items, and the plan
    // holds both under each control.
    let i = m.outer.keep.iter().position(|&k| k != 0).unwrap();
    let keep = u64::from(m.outer.keep[i]);
    let below = i as u64 | (keep - 1) << m.outer.k;
    let at = i as u64 | keep << m.outer.k;
    assert_ne!(m.outer_item(below), m.outer_item(at));
    let p = coverage::plan(&m, &spec, Mode::Terms, 1 << 20).unwrap();
    let outer_mask = (1u64 << (m.outer_bits() - 1)) - 1;
    for side in [below, at] {
        for c in [false, true] {
            assert!(p.lanes.iter().any(|l| l.c == c && l.s & outer_mask == side));
        }
    }
}

#[test]
fn a_rejected_run_leaves_no_export_and_a_passing_run_writes_one() {
    let spec = tiny();
    let (ops, lanemap, family) = built(&spec);
    let dir = std::env::temp_dir().join(format!("femoco-coverage-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let check = |_: &_, _: &_| Vec::new();
    for (name, list, ok) in [
        ("pass.json", ops.clone(), true),
        (
            "fail.json",
            spin_keyed_mutant(&spec, &ops, &map(&spec)),
            false,
        ),
    ] {
        let path = dir.join(name);
        let file = OpsFile {
            ops: list,
            sha256: [0; 32],
        };
        let input = Inputs {
            spec: &spec,
            lanemap: &lanemap,
            family: &family,
            ops: &file,
            samples: 1,
            tracker: sim::givens_tracker(&spec),
            check: &check,
        };
        let result = coverage::check(
            &input,
            sim::validate::reference,
            Some(Mode::Terms),
            Some(&path),
            Some(&[7; 32]),
        );
        assert_eq!(result.is_ok(), ok);
        assert_eq!(path.exists(), ok);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `sa.bin` bytes of a three-orbital spec whose networks have two rotations and whose square
/// has two items, so the exact checker's fixtures reach the branches a two-orbital one cannot.
fn wide_payload() -> Vec<u8> {
    let mut payload = b"FEMOSAS1".to_vec();
    for v in [1u32, 3, 1, 2, 1, 8, 2] {
        payload.extend(v.to_le_bytes());
    }
    for v in [0.0f64, 4.0, -4.0, 8.0] {
        payload.extend(v.to_le_bytes());
    }
    for v in [17u32, 43, 200, 5, 129, 77] {
        payload.extend(v.to_le_bytes());
    }
    for v in [-4.0f64, 2.0, -2.0] {
        payload.extend(v.to_le_bytes());
    }
    for v in [67u32, 250, 131, 9] {
        payload.extend(v.to_le_bytes());
    }
    payload
}

/// The exact checker (tools/verification/exact.py) is tested on real evaluator exports of
/// small circuits. The exports are committed under tests/fixtures/exact/ and this test
/// regenerates them, so they cannot drift from what the evaluator produces. Run with
/// `FEMOCO_WRITE_FIXTURES=1` to rewrite them after an intended change.
#[test]
fn exact_checker_fixtures_are_the_evaluator_exports() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exact");
    let scratch =
        std::env::temp_dir().join(format!("femoco-exact-fixtures-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let write = std::env::var_os("FEMOCO_WRITE_FIXTURES").is_some();
    let check = |_: &_, _: &_| Vec::new();
    let mut tiny_payload = b"FEMOSAS1".to_vec();
    for v in [1u32, 2, 1, 1, 1, 8, 2] {
        tiny_payload.extend(v.to_le_bytes());
    }
    for v in [0.0f64, 2.0, 2.0] {
        tiny_payload.extend(v.to_le_bytes());
    }
    for v in [17u32, 43] {
        tiny_payload.extend(v.to_le_bytes());
    }
    for v in [2.0f64, 2.0] {
        tiny_payload.extend(v.to_le_bytes());
    }
    tiny_payload.extend(67u32.to_le_bytes());
    for (name, payload, mutate) in [
        ("tiny", tiny_payload.clone(), false),
        ("tiny-spin-mutant", tiny_payload, true),
        ("wide", wide_payload(), false),
    ] {
        let spec = spec::sa::parse_payload("test-coverage-sa", &payload).unwrap();
        let (mut ops, lanemap, family) = built(&spec);
        if mutate {
            ops = spin_keyed_mutant(&spec, &ops, &map(&spec));
        }
        let file = OpsFile {
            ops,
            sha256: [0; 32],
        };
        let input = Inputs {
            spec: &spec,
            lanemap: &lanemap,
            family: &family,
            ops: &file,
            samples: 1,
            tracker: sim::givens_tracker(&spec),
            check: &check,
        };
        let path = scratch.join(format!("{name}.json"));
        // Exports only: a mutant must be exportable so the checker can be shown to reject it.
        coverage::check(&input, sim::validate::reference, None, Some(&path), None).unwrap();
        let export = std::fs::read(&path).unwrap();
        for (file, bytes) in [
            (format!("{name}.json"), export),
            (format!("{name}.sa.bin"), payload.clone()),
        ] {
            let committed = dir.join(&file);
            if write {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(&committed, &bytes).unwrap();
            }
            assert!(
                std::fs::read(&committed).is_ok_and(|have| have == bytes),
                "tests/fixtures/exact/{file} is not the evaluator's export; \
                 rerun with FEMOCO_WRITE_FIXTURES=1 and review the change"
            );
        }
    }
    std::fs::remove_dir_all(&scratch).unwrap();
}
