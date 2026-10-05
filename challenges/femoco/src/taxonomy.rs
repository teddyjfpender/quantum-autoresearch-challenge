//! Architecture families and their checkable signatures. See spec/DESIGN.md
//! section 10; the axes, values and what each signature proves are in taxonomy/TAXONOMY.md.
//!
//! `check` returns, for every axis of the taxonomy and every extra axis a family declares:
//! - `Contradicted` when the axis is missing or unknown, the value is unknown or reserved, a
//!   compatibility rule forbids it, the family's MAJOR version differs, or the value's necessary
//!   predicate is false on the circuit's facts;
//! - `Verified` only when the value's sufficient predicate is true;
//! - `DeclaredOnly` otherwise (including when a needed fact is unknown).
use crate::facts::CircuitFacts;
use std::collections::{BTreeMap, BTreeSet};

mod model;
mod predicate;

/// A submission's declared family (`family.out.json`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Family {
    pub taxonomy_version: String,
    pub name: String,
    pub parent: Option<String>,
    pub axes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum AxisStatus {
    Verified,
    Contradicted(String),
    DeclaredOnly,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AxisVerdict {
    pub axis: String,
    pub value: String,
    pub status: AxisStatus,
}

/// `taxonomy/taxonomy.json`, parsed.
#[derive(Clone, Debug)]
pub struct Taxonomy {
    pub raw: serde_json::Value,
}

/// Top-level field names of serialized `CircuitFacts`, the roots a predicate may read.
fn fact_fields() -> BTreeSet<String> {
    match serde_json::to_value(CircuitFacts::default()) {
        Ok(serde_json::Value::Object(m)) => m.keys().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

fn typed(raw: &serde_json::Value) -> Result<model::File, String> {
    serde_json::from_value(raw.clone()).map_err(|e| format!("taxonomy schema: {e}"))
}

/// Loads and validates `taxonomy/taxonomy.json` (see `model::validate` for what is proved).
///
/// # Errors
/// A missing, malformed or internally inconsistent taxonomy file.
pub fn load_taxonomy(path: &std::path::Path) -> Result<Taxonomy, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let raw = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = typed(&raw).map_err(|e| format!("{}: {e}", path.display()))?;
    model::validate(&file, &fact_fields()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Taxonomy { raw })
}

impl Taxonomy {
    /// The taxonomy's own version string.
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.raw.get("taxonomy_version").and_then(|v| v.as_str())
    }
}

fn verdict(axis: &str, value: &str, status: AxisStatus) -> AxisVerdict {
    AxisVerdict {
        axis: axis.to_string(),
        value: value.to_string(),
        status,
    }
}

/// One verdict per taxonomy axis and per extra declared axis. A `Contradicted` axis rejects
/// the run.
#[must_use]
pub fn check(taxonomy: &Taxonomy, family: &Family, facts: &CircuitFacts) -> Vec<AxisVerdict> {
    let file = match typed(&taxonomy.raw) {
        Ok(f) => f,
        Err(e) => return contradict_all(family, &e),
    };
    if let Err(e) = same_major(&file.taxonomy_version, &family.taxonomy_version) {
        return contradict_all(family, &e);
    }
    let facts_json = serde_json::to_value(facts).unwrap_or(serde_json::Value::Null);
    let mut out: Vec<AxisVerdict> = file
        .axes
        .iter()
        .map(|axis| match family.axes.get(&axis.id) {
            None => verdict(
                &axis.id,
                "",
                AxisStatus::Contradicted("axis not declared".into()),
            ),
            Some(value) => verdict(
                &axis.id,
                value,
                axis_status(&file, axis, value, family, &facts_json),
            ),
        })
        .collect();
    for (axis, value) in &family.axes {
        if file.axis(axis).is_none() {
            let reason = format!("unknown axis '{axis}' (taxonomy {})", file.taxonomy_version);
            out.push(verdict(axis, value, AxisStatus::Contradicted(reason)));
        }
    }
    out
}

fn contradict_all(family: &Family, reason: &str) -> Vec<AxisVerdict> {
    let status = AxisStatus::Contradicted(reason.to_string());
    family
        .axes
        .iter()
        .map(|(a, v)| verdict(a, v, status.clone()))
        .collect()
}

fn same_major(ours: &str, theirs: &str) -> Result<(), String> {
    let (a, _, _) = model::semver(ours)?;
    let (b, _, _) = model::semver(theirs).map_err(|e| format!("family {e}"))?;
    if a == b {
        Ok(())
    } else {
        Err(format!(
            "family taxonomy_version {theirs} is not compatible with {ours}"
        ))
    }
}

fn axis_status(
    file: &model::File,
    axis: &model::Axis,
    value: &str,
    family: &Family,
    facts: &serde_json::Value,
) -> AxisStatus {
    let Some(v) = axis.value(value) else {
        let known: Vec<&str> = axis.values.iter().map(|v| v.id.as_str()).collect();
        let reason = format!(
            "unknown value '{value}' for axis '{}'; known: {}",
            axis.id,
            known.join(", ")
        );
        return AxisStatus::Contradicted(reason);
    };
    if v.status == "reserved" {
        let todo = v.todo.as_deref().unwrap_or("TODO");
        return AxisStatus::Contradicted(format!("'{}:{value}' is reserved: {todo}", axis.id));
    }
    if let Some(reason) = rule_violation(file, &axis.id, value, family) {
        return AxisStatus::Contradicted(reason);
    }
    signature_status(&axis.id, v, facts)
}

/// The first rule whose `if` matches the family and whose `then` forbids this axis's value.
fn rule_violation(file: &model::File, axis: &str, value: &str, family: &Family) -> Option<String> {
    file.rules.iter().find_map(|rule| {
        let applies = rule.when.iter().all(|(a, v)| family.axes.get(a) == Some(v));
        let allowed = rule.then.get(axis)?;
        (applies && !allowed.permits(value)).then(|| {
            format!(
                "rule '{}' forbids '{axis}:{value}': {}",
                rule.id, rule.reason
            )
        })
    })
}

fn signature_status(axis: &str, v: &model::AxisValue, facts: &serde_json::Value) -> AxisStatus {
    let parsed = match v.signature.parsed() {
        Ok(p) => p,
        Err(e) => return AxisStatus::Contradicted(format!("bad signature: {e}")),
    };
    if let Some(n) = &parsed.necessary {
        if predicate::eval(n, facts) == Some(false) {
            return AxisStatus::Contradicted(format!(
                "necessary condition of '{axis}:{}' fails: {} [observed {}]",
                v.id,
                predicate::describe(n),
                predicate::observed(n, facts)
            ));
        }
    }
    match &parsed.sufficient {
        Some(s) if predicate::eval(s, facts) == Some(true) => AxisStatus::Verified,
        _ => AxisStatus::DeclaredOnly,
    }
}

/// The canonical family key: `axis=value` pairs in axis order, joined by `;`. Two submissions
/// are in the same family exactly when their keys are equal (name, parent and parameters are
/// not part of it).
#[must_use]
pub fn family_key(family: &Family) -> String {
    family
        .axes
        .iter()
        .map(|(a, v)| format!("{a}={v}"))
        .collect::<Vec<_>>()
        .join(";")
}

/// Axes on which two families differ (an axis present in only one counts as different).
#[must_use]
pub fn distinguishing_axes(a: &Family, b: &Family) -> Vec<String> {
    let axes: BTreeSet<&String> = a.axes.keys().chain(b.axes.keys()).collect();
    axes.into_iter()
        .filter(|x| a.axes.get(*x) != b.axes.get(*x))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::predicate::{eval, parse, resolve};
    use serde_json::json;

    #[test]
    fn missing_count_reads_zero_but_missing_segment_is_unknown() {
        let facts = json!({"op_counts": {"CCX": 3}, "segment_op_counts": {}});
        assert_eq!(resolve(&facts, "op_counts.Givens"), Some(json!(0)));
        assert_eq!(resolve(&facts, "op_counts.ccx"), Some(json!(3)));
        assert_eq!(resolve(&facts, "segment_op_counts.1.CCX"), None);
        assert_eq!(resolve(&facts, "no_such_field"), None);
    }

    #[test]
    fn unknown_is_neither_true_nor_false() {
        let p = parse(&json!({"cmp": {"field": "pending", "op": "==", "value": true}})).unwrap();
        assert_eq!(eval(&p, &json!({})), None);
        let not = parse(&json!({"not": {"cmp": {"field": "x", "op": ">", "value": 1}}})).unwrap();
        assert_eq!(eval(&not, &json!({"x": 0})), Some(true));
    }

    #[test]
    fn malformed_predicates_are_rejected() {
        for bad in [
            json!({"cmp": {"field": "a", "op": "~", "value": 1}}),
            json!({"cmp": {"field": "a.*.b", "op": "==", "value": 1}}),
            json!({"all": []}),
            json!({"cmp": {"field": "a", "sum": ["b"], "op": "==", "value": 1}}),
            json!({"xor": []}),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }
}
