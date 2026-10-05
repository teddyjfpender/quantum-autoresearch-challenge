//! `taxonomy::check` against the shipped taxonomy over synthetic `CircuitFacts`: what verifies,
//! what stays declared, and what each kind of false claim is rejected with.
use femoco_walk::facts::CircuitFacts;
use femoco_walk::taxonomy::{
    check, distinguishing_axes, family_key, load_taxonomy, AxisStatus, Family, Taxonomy,
};
use std::collections::BTreeMap;

fn taxonomy() -> Taxonomy {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json");
    load_taxonomy(&path).expect("shipped taxonomy loads")
}

fn counts(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn family(pairs: &[(&str, &str)]) -> Family {
    Family {
        taxonomy_version: "1.0.0".into(),
        name: "test".into(),
        parent: None,
        axes: pairs
            .iter()
            .map(|(a, v)| (a.to_string(), v.to_string()))
            .collect(),
    }
}

/// A labelled sparse circuit: Toffolis in prepare and select, measurement-based erasure.
fn sparse_facts() -> CircuitFacts {
    let mut f = CircuitFacts {
        spec_id: "reiher-sparse-v1".into(),
        encoding: "sparse".into(),
        lane_map_family: "sparse-sym-alias-v1".into(),
        u: 30,
        op_counts: counts(&[
            ("CCX", 900),
            ("HMR", 400),
            ("CX", 5000),
            ("PUSH_CONDITION", 400),
        ]),
        executed_ccx: 850.0,
        executed_hmr: 380.0,
        measurement_uncompute: true,
        condition_bits: 400,
        max_condition_depth: 1,
        ..CircuitFacts::default()
    };
    f.segment_op_counts
        .insert(0, counts(&[("CCX", 500), ("CX", 2000)]));
    f.segment_op_counts
        .insert(1, counts(&[("CCX", 400), ("CX", 3000)]));
    f.segment_op_counts.insert(2, counts(&[("HMR", 400)]));
    f.segment_system_ops
        .insert(1, counts(&[("CX", 800), ("CZ", 600)]));
    f
}

fn df_facts() -> CircuitFacts {
    let mut f = sparse_facts();
    f.spec_id = "reiher-df-v1".into();
    f.encoding = "df".into();
    f.lane_map_family = "df-pair-alias-v1".into();
    f.op_counts.insert("GIVENS".into(), 216);
    f.executed_givens = 216.0;
    f
}

const SPARSE: [(&str, &str); 7] = [
    ("encoding", "sparse"),
    ("lane_map", "sparse-sym-alias-v1"),
    ("lookup", "qroam-clean"),
    ("select", "selected-majorana"),
    ("uncompute", "measurement-based"),
    ("reuse", "shared-workspace"),
    ("rotation", "none"),
];

const DF: [(&str, &str); 7] = [
    ("encoding", "df"),
    ("lane_map", "df-pair-alias-v1"),
    ("lookup", "qroam-clean"),
    ("select", "givens-network"),
    ("uncompute", "measurement-based"),
    ("reuse", "serial"),
    ("rotation", "phase-gradient-givens"),
];

const THC: [(&str, &str); 7] = [
    ("encoding", "thc"),
    ("lane_map", "thc-pair-alias-v1"),
    ("lookup", "unary-iteration"),
    ("select", "givens-network"),
    ("uncompute", "measurement-based"),
    ("reuse", "serial"),
    ("rotation", "nonorthogonal-thc"),
];

fn thc_facts() -> CircuitFacts {
    let mut f = df_facts();
    f.spec_id = "reiher-thc-v1".into();
    f.encoding = "thc".into();
    f.lane_map_family = "thc-pair-alias-v1".into();
    f
}

fn with(base: &[(&str, &str)], axis: &str, value: &str) -> Family {
    let mut f = family(base);
    f.axes.insert(axis.into(), value.into());
    f
}

fn status_of(t: &Taxonomy, fam: &Family, facts: &CircuitFacts, axis: &str) -> AxisStatus {
    let v = check(t, fam, facts);
    v.into_iter()
        .find(|v| v.axis == axis)
        .expect("verdict for axis")
        .status
}

fn reason_of(t: &Taxonomy, fam: &Family, facts: &CircuitFacts, axis: &str) -> String {
    match status_of(t, fam, facts, axis) {
        AxisStatus::Contradicted(r) => r,
        other => panic!("{axis}: expected Contradicted, got {other:?}"),
    }
}

#[test]
fn sparse_baseline_verifies_what_the_circuit_shows() {
    let t = taxonomy();
    let v = check(&t, &family(&SPARSE), &sparse_facts());
    let by: BTreeMap<_, _> = v
        .iter()
        .map(|v| (v.axis.as_str(), v.status.clone()))
        .collect();
    assert_eq!(v.len(), 7);
    for axis in ["encoding", "lane_map", "uncompute", "rotation"] {
        assert_eq!(by[axis], AxisStatus::Verified, "{axis}");
    }
    for axis in ["lookup", "select", "reuse"] {
        assert_eq!(by[axis], AxisStatus::DeclaredOnly, "{axis}");
    }
}

#[test]
fn df_baseline_verifies_givens_axes() {
    let t = taxonomy();
    let v = check(&t, &family(&DF), &df_facts());
    let verified: Vec<&str> = v
        .iter()
        .filter(|v| v.status == AxisStatus::Verified)
        .map(|v| v.axis.as_str())
        .collect();
    assert_eq!(
        verified,
        ["encoding", "lane_map", "select", "uncompute", "rotation"]
    );
}

#[test]
fn encoding_claim_contradicted_by_the_loaded_spec() {
    let t = taxonomy();
    let r = reason_of(&t, &family(&SPARSE), &df_facts(), "encoding");
    assert!(
        r.contains("encoding == \"sparse\"") && r.contains("\"df\""),
        "{r}"
    );
}

#[test]
fn reserved_values_are_rejected_with_their_todo() {
    let t = taxonomy();
    for (axis, value, todo) in [
        ("encoding", "bliss", "TODO(BLISS)"),
        (
            "lane_map",
            "symmetry-class-alias",
            "TODO(symmetry-class alias)",
        ),
    ] {
        let r = reason_of(&t, &with(&SPARSE, axis, value), &sparse_facts(), axis);
        assert!(r.contains("reserved") && r.contains(todo), "{r}");
    }
}

#[test]
fn unknown_missing_and_extra_axes_are_rejected() {
    let t = taxonomy();
    let r = reason_of(
        &t,
        &with(&SPARSE, "lookup", "qram"),
        &sparse_facts(),
        "lookup",
    );
    assert!(r.contains("unknown value 'qram'"), "{r}");
    let r = reason_of(&t, &with(&SPARSE, "depth", "low"), &sparse_facts(), "depth");
    assert!(r.contains("unknown axis 'depth'"), "{r}");
    let mut fam = family(&SPARSE);
    fam.axes.remove("reuse");
    assert_eq!(
        reason_of(&t, &fam, &sparse_facts(), "reuse"),
        "axis not declared"
    );
}

#[test]
fn rotation_none_contradicted_by_a_givens_op() {
    let t = taxonomy();
    let mut facts = sparse_facts();
    facts.op_counts.insert("Givens".into(), 1);
    let r = reason_of(&t, &family(&SPARSE), &facts, "rotation");
    assert!(
        r.contains("op_counts.GIVENS == 0") && r.contains("= 1"),
        "{r}"
    );
}

#[test]
fn uncompute_claims_are_checked_against_measurements() {
    let t = taxonomy();
    let r = reason_of(
        &t,
        &with(&SPARSE, "uncompute", "unitary"),
        &sparse_facts(),
        "uncompute",
    );
    assert!(r.contains("op_counts.HMR == 0"), "{r}");
    let mut unitary = sparse_facts();
    unitary.op_counts.remove("HMR");
    unitary.executed_hmr = 0.0;
    unitary.measurement_uncompute = false;
    let fam = with(&SPARSE, "uncompute", "unitary");
    assert_eq!(
        status_of(&t, &fam, &unitary, "uncompute"),
        AxisStatus::Verified
    );
    let r = reason_of(&t, &family(&SPARSE), &unitary, "uncompute");
    assert!(r.contains("executed_hmr > 0"), "{r}");
    let mut no_pattern = sparse_facts();
    no_pattern.measurement_uncompute = false;
    let s = status_of(&t, &family(&SPARSE), &no_pattern, "uncompute");
    assert_eq!(s, AxisStatus::DeclaredOnly);
}

#[test]
fn compatibility_rules_reject_mismatched_axes() {
    let t = taxonomy();
    let facts = sparse_facts();
    let r = reason_of(
        &t,
        &with(&SPARSE, "lane_map", "df-pair-alias-v1"),
        &facts,
        "lane_map",
    );
    assert!(r.contains("rule 'sparse-lane-maps'"), "{r}");
    let r = reason_of(
        &t,
        &with(&SPARSE, "select", "givens-network"),
        &facts,
        "select",
    );
    assert!(r.contains("rule 'sparse-select'"), "{r}");
    let r = reason_of(&t, &with(&SPARSE, "lookup", "none"), &facts, "lookup");
    assert!(r.contains("rule 'sparse-sym-needs-lookup'"), "{r}");
    let mut mono = with(&SPARSE, "lookup", "monomial-lookup");
    mono.axes.insert("uncompute".into(), "unitary".into());
    let r = reason_of(&t, &mono, &facts, "uncompute");
    assert!(r.contains("rule 'monomial-measures'"), "{r}");
    let r = reason_of(&t, &with(&DF, "rotation", "none"), &df_facts(), "rotation");
    assert!(r.contains("rule 'df-rotation'"), "{r}");
}

#[test]
fn clifford_mask_needs_a_toffoli_free_select_and_is_never_verified_yet() {
    let t = taxonomy();
    let fam = with(&SPARSE, "select", "clifford-mask");
    let r = reason_of(&t, &fam, &sparse_facts(), "select");
    assert!(
        r.contains("segment_op_counts.1.CCX") && r.contains("= 400"),
        "{r}"
    );
    let mut mask = sparse_facts();
    mask.segment_op_counts
        .insert(1, counts(&[("CX", 3000), ("S", 10)]));
    assert_eq!(
        status_of(&t, &fam, &mask, "select"),
        AxisStatus::DeclaredOnly
    );
    let r = reason_of(&t, &with(&SPARSE, "select", "unary-pauli"), &mask, "select");
    assert!(r.contains("segment_op_counts.1.CCX"), "{r}");
}

#[test]
fn unlabelled_circuits_are_neither_contradicted_nor_verified_on_segment_axes() {
    let t = taxonomy();
    let mut bare = sparse_facts();
    bare.segment_op_counts.clear();
    bare.segment_system_ops.clear();
    for select in ["unary-pauli", "selected-majorana", "clifford-mask"] {
        let fam = with(&SPARSE, "select", select);
        assert_eq!(
            status_of(&t, &fam, &bare, "select"),
            AxisStatus::DeclaredOnly
        );
    }
}

#[test]
fn taxonomy_version_must_share_the_major_number() {
    let t = taxonomy();
    let mut fam = family(&SPARSE);
    fam.taxonomy_version = "1.4.0".into();
    let v = check(&t, &fam, &sparse_facts());
    assert!(v
        .iter()
        .all(|v| !matches!(v.status, AxisStatus::Contradicted(_))));
    fam.taxonomy_version = "2.0.0".into();
    let v = check(&t, &fam, &sparse_facts());
    assert_eq!(v.len(), 7);
    assert!(v
        .iter()
        .all(|v| matches!(&v.status, AxisStatus::Contradicted(r) if r.contains("2.0.0"))));
}

#[test]
fn family_identity_ignores_name_parent_and_parameters() {
    let a = family(&SPARSE);
    let mut b = family(&SPARSE);
    b.name = "renamed".into();
    b.parent = Some("sparse-alias-unary".into());
    assert_eq!(family_key(&a), family_key(&b));
    assert!(distinguishing_axes(&a, &b).is_empty());
    let c = with(&SPARSE, "lookup", "monomial-lookup");
    assert_eq!(distinguishing_axes(&a, &c), ["lookup"]);
    assert!(family_key(&a).starts_with("encoding=sparse;"));
}

#[test]
fn thc_verifies_its_axes_and_its_rules_hold() {
    let t = taxonomy();
    let v = check(&t, &family(&THC), &thc_facts());
    let verified: Vec<&str> = v
        .iter()
        .filter(|v| v.status == AxisStatus::Verified)
        .map(|v| v.axis.as_str())
        .collect();
    assert_eq!(
        verified,
        ["encoding", "lane_map", "select", "uncompute", "rotation"]
    );
    // Rotation values are tied to the encoding in both directions.
    let r = reason_of(
        &t,
        &with(&THC, "rotation", "phase-gradient-givens"),
        &thc_facts(),
        "rotation",
    );
    assert!(r.contains("rule 'thc-rotation'"), "{r}");
    let r = reason_of(
        &t,
        &with(&DF, "rotation", "nonorthogonal-thc"),
        &df_facts(),
        "rotation",
    );
    assert!(r.contains("rule 'df-rotation'"), "{r}");
    let r = reason_of(
        &t,
        &with(&THC, "lane_map", "df-pair-alias-v1"),
        &thc_facts(),
        "lane_map",
    );
    assert!(r.contains("rule 'thc-lane-maps'"), "{r}");
    let r = reason_of(&t, &with(&THC, "lookup", "none"), &thc_facts(), "lookup");
    assert!(r.contains("rule 'thc-pair-needs-lookup'"), "{r}");
    // A THC family on a df circuit is refuted by the encoding the harness loaded.
    let r = reason_of(&t, &family(&THC), &df_facts(), "encoding");
    assert!(r.contains("encoding == \"thc\""), "{r}");
    // The select value still verifies for df, and not without a Givens op.
    let mut no_givens = thc_facts();
    no_givens.op_counts.remove("GIVENS");
    let r = reason_of(&t, &family(&THC), &no_givens, "select");
    assert!(r.contains("op_counts.GIVENS > 0"), "{r}");
}
