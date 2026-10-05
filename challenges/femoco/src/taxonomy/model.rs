//! The typed form of `taxonomy/taxonomy.json` and its validation.
//!
//! `validate` proves the file is internally consistent: unique axes and values, a known status
//! and strength for every value, predicates that match their declared strength and parse, a
//! citation (or a stated reason for none) on every active value, a TODO on every reserved one,
//! rules that name real axes and values, and predicate paths that name a `CircuitFacts` field or
//! a declared pending fact. It does not prove a signature is scientifically right; that is
//! argued in `taxonomy/TAXONOMY.md`.
use super::predicate::{self, Pred};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub taxonomy_version: String,
    pub challenge: String,
    pub family_rule: String,
    pub versioning: String,
    pub strengths: BTreeMap<String, String>,
    pub predicate_language: String,
    pub pending_facts: Vec<PendingFact>,
    pub references: BTreeMap<String, Value>,
    pub axes: Vec<Axis>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingFact {
    pub field: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub definition: String,
    pub used_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Axis {
    pub id: String,
    pub definition: String,
    pub values: Vec<AxisValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxisValue {
    pub id: String,
    pub status: String,
    pub definition: String,
    #[serde(default)]
    pub todo: Option<String>,
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub citation_note: Option<String>,
    pub signature: Signature,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    #[serde(rename = "ref")]
    pub reference: String,
    pub location: String,
    pub supports: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub strength: String,
    #[serde(default)]
    pub necessary: Option<Value>,
    #[serde(default)]
    pub sufficient: Option<Value>,
    pub proves: String,
    pub cannot_show: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    #[serde(rename = "if")]
    pub when: BTreeMap<String, String>,
    pub then: BTreeMap<String, Allowed>,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowed {
    #[serde(rename = "in", default)]
    pub within: Option<Vec<String>>,
    #[serde(default)]
    pub not_in: Option<Vec<String>>,
}

impl Allowed {
    pub fn permits(&self, value: &str) -> bool {
        let inside = self
            .within
            .as_ref()
            .is_none_or(|v| v.iter().any(|x| x == value));
        let outside = self
            .not_in
            .as_ref()
            .is_none_or(|v| v.iter().all(|x| x != value));
        inside && outside
    }
}

/// A value's parsed predicates.
pub struct Parsed {
    pub necessary: Option<Pred>,
    pub sufficient: Option<Pred>,
}

impl File {
    pub fn axis(&self, id: &str) -> Option<&Axis> {
        self.axes.iter().find(|a| a.id == id)
    }
}

impl Axis {
    pub fn value(&self, id: &str) -> Option<&AxisValue> {
        self.values.iter().find(|v| v.id == id)
    }
}

impl Signature {
    /// # Errors
    /// A predicate that does not parse.
    pub fn parsed(&self) -> Result<Parsed, String> {
        Ok(Parsed {
            necessary: self.necessary.as_ref().map(predicate::parse).transpose()?,
            sufficient: self.sufficient.as_ref().map(predicate::parse).transpose()?,
        })
    }
}

/// Parses a `MAJOR.MINOR.PATCH` version.
///
/// # Errors
/// Anything else.
pub fn semver(v: &str) -> Result<(u64, u64, u64), String> {
    let parts: Vec<u64> = v
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| format!("version '{v}' is not MAJOR.MINOR.PATCH"))?;
    match parts[..] {
        [a, b, c] => Ok((a, b, c)),
        _ => Err(format!("version '{v}' is not MAJOR.MINOR.PATCH")),
    }
}

/// Validates the whole file. `fact_fields` are the top-level fields of serialized `CircuitFacts`.
///
/// # Errors
/// The first inconsistency found, naming where it is.
pub fn validate(f: &File, fact_fields: &BTreeSet<String>) -> Result<(), String> {
    semver(&f.taxonomy_version)?;
    let prose = [
        &f.challenge,
        &f.family_rule,
        &f.versioning,
        &f.predicate_language,
    ];
    if prose.iter().any(|t| t.trim().is_empty()) {
        return Err(
            "challenge, family_rule, versioning and predicate_language are required".into(),
        );
    }
    let mut known: BTreeSet<String> = fact_fields.clone();
    known.extend(f.pending_facts.iter().map(|p| p.field.clone()));
    let mut axis_ids = BTreeSet::new();
    for axis in &f.axes {
        if !axis_ids.insert(axis.id.as_str()) {
            return Err(format!("duplicate axis '{}'", axis.id));
        }
        if axis.definition.trim().is_empty() {
            return Err(format!("axis '{}' needs a definition", axis.id));
        }
        if !axis.values.iter().any(|v| v.status == "active") {
            return Err(format!("axis '{}' has no active value", axis.id));
        }
        let mut ids = BTreeSet::new();
        for v in &axis.values {
            if !ids.insert(v.id.as_str()) {
                return Err(format!("duplicate value '{}:{}'", axis.id, v.id));
            }
            validate_value(f, v, &known).map_err(|e| format!("{}:{}: {e}", axis.id, v.id))?;
        }
    }
    for rule in &f.rules {
        validate_rule(f, rule).map_err(|e| format!("rule '{}': {e}", rule.id))?;
    }
    for p in &f.pending_facts {
        validate_pending(f, p).map_err(|e| format!("pending fact '{}': {e}", p.field))?;
    }
    Ok(())
}

/// A pending fact has a type and definition, and every value that claims to use it reads it.
fn validate_pending(f: &File, p: &PendingFact) -> Result<(), String> {
    if !matches!(p.ty.as_str(), "bool" | "u64" | "f64") || p.definition.trim().is_empty() {
        return Err("needs type bool, u64 or f64 and a definition".into());
    }
    for user in &p.used_by {
        let (axis, value) = user
            .split_once(':')
            .ok_or("used_by entries are 'axis:value'")?;
        let v = f.axis(axis).and_then(|a| a.value(value));
        let v = v.ok_or(format!("used_by names unknown '{user}'"))?;
        let parsed = v.signature.parsed()?;
        let reads = parsed.necessary.iter().chain(parsed.sufficient.iter());
        let reads: Vec<String> = reads.flat_map(predicate::paths).collect();
        if !reads
            .iter()
            .any(|r| r.split('.').next() == Some(p.field.as_str()))
        {
            return Err(format!("'{user}' does not read it"));
        }
    }
    Ok(())
}

fn validate_value(f: &File, v: &AxisValue, known: &BTreeSet<String>) -> Result<(), String> {
    match v.status.as_str() {
        "active" if v.citations.is_empty() && v.citation_note.is_none() => {
            return Err("active value needs a citation or a citation_note".into())
        }
        "active" => {}
        "reserved" if v.todo.as_deref().is_none_or(|t| !t.contains("TODO")) => {
            return Err("reserved value needs a 'todo' containing TODO".into())
        }
        "reserved" => {}
        s => return Err(format!("unknown status '{s}'")),
    }
    if v.definition.trim().is_empty() {
        return Err("needs a definition".into());
    }
    for c in &v.citations {
        if !f.references.contains_key(&c.reference) {
            return Err(format!("citation of unknown reference '{}'", c.reference));
        }
        if c.location.trim().is_empty() || c.supports.trim().is_empty() {
            return Err("a citation needs a location and what it supports".into());
        }
    }
    let sig = &v.signature;
    if sig.proves.trim().is_empty() || sig.cannot_show.trim().is_empty() {
        return Err("a signature must say what it proves and what it cannot show".into());
    }
    if !f.strengths.contains_key(&sig.strength) {
        return Err(format!("unknown strength '{}'", sig.strength));
    }
    let parsed = sig.parsed()?;
    let (n, s) = (parsed.necessary.is_some(), parsed.sufficient.is_some());
    let shape_ok = match sig.strength.as_str() {
        "exact" => n && s && sig.necessary == sig.sufficient,
        "necessary" => n && !s,
        "sufficient" => !n && s,
        "necessary+sufficient" => n && s && sig.necessary != sig.sufficient,
        "declared" => !n && !s,
        _ => false,
    };
    if !shape_ok {
        return Err(format!(
            "predicates do not match strength '{}'",
            sig.strength
        ));
    }
    let preds = parsed.necessary.iter().chain(parsed.sufficient.iter());
    for path in preds.flat_map(predicate::paths) {
        let root = path.split('.').next().unwrap_or_default();
        if !known.contains(root) {
            return Err(format!(
                "path '{path}' names no CircuitFacts field or pending fact"
            ));
        }
    }
    Ok(())
}

fn validate_rule(f: &File, rule: &Rule) -> Result<(), String> {
    if rule.when.is_empty() || rule.then.is_empty() {
        return Err("needs non-empty 'if' and 'then'".into());
    }
    let check = |axis: &str, value: &str| -> Result<(), String> {
        let a = f.axis(axis).ok_or(format!("unknown axis '{axis}'"))?;
        a.value(value)
            .map(|_| ())
            .ok_or(format!("unknown value '{axis}:{value}'"))
    };
    for (axis, value) in &rule.when {
        check(axis, value)?;
    }
    for (axis, allowed) in &rule.then {
        let lists = allowed.within.iter().chain(allowed.not_in.iter());
        let mut any = false;
        for v in lists.flatten() {
            check(axis, v)?;
            any = true;
        }
        if !any {
            return Err(format!("'then.{axis}' needs a non-empty 'in' or 'not_in'"));
        }
    }
    Ok(())
}
