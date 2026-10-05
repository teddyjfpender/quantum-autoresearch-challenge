//! Taxonomy 1.1.0's nested DF values over synthetic `CircuitFacts` (spec/DESIGN.md section
//! 16): `givens-nested` and `df-nested-alias-v1` verify on a nested circuit's facts, and each
//! cross-claim between the flat and nested architectures is contradicted.
use femoco_walk::facts::CircuitFacts;
use femoco_walk::taxonomy::{check, load_taxonomy, AxisStatus, Family, Taxonomy};
use std::collections::BTreeMap;

fn taxonomy() -> Taxonomy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    load_taxonomy(&path).unwrap()
}

fn family(select: &str, lane_map: &str) -> Family {
    let axes = [
        ("encoding", "df"),
        ("lane_map", lane_map),
        ("lookup", "qroam-clean"),
        ("select", select),
        ("uncompute", "measurement-based"),
        ("reuse", "serial"),
        ("rotation", "phase-gradient-givens"),
    ];
    Family {
        taxonomy_version: "1.1.0".into(),
        name: "t".into(),
        parent: None,
        axes: axes
            .iter()
            .map(|(a, v)| (a.to_string(), v.to_string()))
            .collect(),
    }
}

fn facts(nested: bool) -> CircuitFacts {
    let mut op_counts: BTreeMap<String, u64> = [("CCX", 500), ("GIVENS", 400), ("HMR", 10)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    if nested {
        op_counts.insert("REFLECT".into(), 1);
    }
    CircuitFacts {
        spec_id: "reiher-df-nested-v1".into(),
        encoding: "df".into(),
        lane_map_family: if nested {
            "df-nested-alias-v1"
        } else {
            "df-pair-alias-v1"
        }
        .into(),
        op_counts,
        executed_hmr: 10.0,
        condition_bits: 10,
        measurement_uncompute: true,
        nested_validated: nested,
        ..CircuitFacts::default()
    }
}

fn status(fam: &Family, f: &CircuitFacts, axis: &str) -> AxisStatus {
    check(&taxonomy(), fam, f)
        .into_iter()
        .find(|v| v.axis == axis)
        .unwrap()
        .status
}

#[test]
fn nested_values_verify_on_a_nested_circuit() {
    let (fam, f) = (family("givens-nested", "df-nested-alias-v1"), facts(true));
    for axis in ["encoding", "lane_map", "select", "rotation"] {
        assert_eq!(status(&fam, &f, axis), AxisStatus::Verified, "{axis}");
    }
}

#[test]
fn givens_network_cannot_be_declared_with_a_nested_lane_map() {
    let s = status(
        &family("givens-network", "df-nested-alias-v1"),
        &facts(true),
        "select",
    );
    let AxisStatus::Contradicted(r) = s else {
        panic!("{s:?}")
    };
    assert!(r.contains("df-nested-select"), "{r}");
}

#[test]
fn givens_nested_cannot_be_declared_on_a_flat_circuit() {
    let s = status(
        &family("givens-nested", "df-pair-alias-v1"),
        &facts(false),
        "select",
    );
    let AxisStatus::Contradicted(r) = s else {
        panic!("{s:?}")
    };
    assert!(r.contains("df-pair-select"), "{r}");
    // Even without the rule, the signature needs the harness's nested_validated fact.
    let mut f = facts(true);
    f.nested_validated = false;
    let s = status(&family("givens-nested", "df-nested-alias-v1"), &f, "select");
    let AxisStatus::Contradicted(r) = s else {
        panic!("{s:?}")
    };
    assert!(r.contains("nested_validated"), "{r}");
}
