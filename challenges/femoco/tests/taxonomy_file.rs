//! `load_taxonomy` accepts the shipped file and rejects each kind of inconsistency.
use femoco_walk::taxonomy::load_taxonomy;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn shipped_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("taxonomy/taxonomy.json")
}

fn shipped() -> Value {
    serde_json::from_str(&std::fs::read_to_string(shipped_path()).unwrap()).unwrap()
}

/// Writes `v` to a unique temporary file and loads it.
fn load_value(v: &Value, tag: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("femoco-taxonomy-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}.json"));
    std::fs::write(&path, serde_json::to_string(v).unwrap()).unwrap();
    load_taxonomy(&path).map(|_| ())
}

fn value_mut<'a>(t: &'a mut Value, axis: &str, value: &str) -> &'a mut Value {
    let axes = t["axes"].as_array_mut().unwrap();
    let a = axes.iter_mut().find(|a| a["id"] == axis).unwrap();
    let vals = a["values"].as_array_mut().unwrap();
    vals.iter_mut().find(|v| v["id"] == value).unwrap()
}

#[test]
fn shipped_taxonomy_covers_the_required_axes() {
    let t = load_taxonomy(&shipped_path()).expect("loads");
    assert_eq!(t.version(), Some("1.3.0"));
    let raw = shipped();
    let ids: Vec<&str> = raw["axes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "encoding",
            "lane_map",
            "lookup",
            "select",
            "uncompute",
            "reuse",
            "rotation"
        ]
    );
    for (axis, value) in [("encoding", "bliss"), ("lane_map", "symmetry-class-alias")] {
        let mut r = raw.clone();
        assert_eq!(value_mut(&mut r, axis, value)["status"], "reserved");
    }
}

#[test]
fn inconsistent_files_are_rejected() {
    type Mutation = fn(&mut Value);
    let cases: [(&str, Mutation, &str); 9] = [
        (
            "strength",
            |t| value_mut(t, "rotation", "none")["signature"]["strength"] = json!("strong"),
            "unknown strength",
        ),
        (
            "shape",
            |t| value_mut(t, "rotation", "none")["signature"]["strength"] = json!("necessary"),
            "do not match strength",
        ),
        (
            "path",
            |t| {
                let bad = json!({"cmp": {"field": "gates.GIVENS", "op": "==", "value": 0}});
                let sig = &mut value_mut(t, "rotation", "none")["signature"];
                sig["necessary"] = bad.clone();
                sig["sufficient"] = bad;
            },
            "names no CircuitFacts field",
        ),
        (
            "todo",
            |t| value_mut(t, "encoding", "bliss")["todo"] = json!("later"),
            "containing TODO",
        ),
        (
            "cite",
            |t| value_mut(t, "lookup", "select-swap")["citations"][0]["ref"] = json!("nobody2030"),
            "unknown reference",
        ),
        (
            "dup",
            |t| value_mut(t, "reuse", "serial")["id"] = json!("shared-workspace"),
            "duplicate value",
        ),
        (
            "rule",
            |t| t["rules"][0]["then"]["lane_map"]["in"][0] = json!("alias-v9"),
            "unknown value 'lane_map:alias-v9'",
        ),
        (
            "version",
            |t| t["taxonomy_version"] = json!("1.0"),
            "MAJOR.MINOR.PATCH",
        ),
        (
            "pending",
            |t| {
                t["pending_facts"] = json!([{
                    "field": "hypothetical",
                    "type": "bool",
                    "definition": "a fact that a value claims to use but does not read",
                    "used_by": ["rotation:none"]
                }]);
            },
            "does not read it",
        ),
    ];
    for (tag, mutate, expect) in cases {
        let mut t = shipped();
        mutate(&mut t);
        let err = load_value(&t, tag).expect_err(tag);
        assert!(err.contains(expect), "{tag}: {err}");
    }
}

#[test]
fn unknown_fields_are_rejected() {
    let mut t = shipped();
    t["axes"][0]["colour"] = json!("red");
    assert!(load_value(&t, "unknown-field")
        .unwrap_err()
        .contains("colour"));
}
