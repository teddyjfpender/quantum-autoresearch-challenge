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
    // Two orbitals, one square: outer weights 4,4,8 and equal inner weights.
    // Both alias tables can be rounded EXACTLY, so correctness tests need no tolerance
    // in the coefficient rule. Rotation words remain the declared exact 8-bit words.
    let mut payload = b"FEMOSAS1".to_vec();
    for v in [1u32, 2, 1, 1, 1, 8, 2] {
        payload.extend(v.to_le_bytes());
    }
    for v in [0.0f64, 2.0, 2.0] {
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
        outer: (2, 2),
        inner: (1, 1),
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
