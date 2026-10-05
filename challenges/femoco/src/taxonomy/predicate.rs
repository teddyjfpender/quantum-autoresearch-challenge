//! Signature predicates over the serialized `CircuitFacts`.
//!
//! Evaluation is three-valued: `Some(true)`, `Some(false)` or `None` (a fact the predicate needs
//! is missing or null). `None` never contradicts a claim and never verifies one, so a predicate
//! that names a fact the harness does not yet emit is inert rather than wrong.
use serde_json::Value;

/// A comparison operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl Op {
    fn parse(s: &str) -> Result<Self, String> {
        Ok(match s {
            "==" => Self::Eq,
            "!=" => Self::Ne,
            ">" => Self::Gt,
            ">=" => Self::Ge,
            "<" => Self::Lt,
            "<=" => Self::Le,
            _ => return Err(format!("unknown operator '{s}'")),
        })
    }

    fn symbol(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
        }
    }
}

/// The left side of a comparison: one field, or a sum of numeric fields.
#[derive(Clone, Debug)]
pub enum Lhs {
    Field(String),
    Sum(Vec<String>),
}

/// A parsed signature predicate.
#[derive(Clone, Debug)]
pub enum Pred {
    All(Vec<Pred>),
    Any(Vec<Pred>),
    Not(Box<Pred>),
    Cmp { lhs: Lhs, op: Op, value: Value },
}

/// Parses the JSON form documented in `taxonomy.json` (`predicate_language`).
///
/// # Errors
/// Any shape other than exactly one of `all`, `any`, `not`, `cmp`, or a malformed path.
pub fn parse(v: &Value) -> Result<Pred, String> {
    let obj = v.as_object().ok_or("predicate must be an object")?;
    if obj.len() != 1 {
        return Err(format!(
            "predicate must have exactly one key, got {}",
            obj.len()
        ));
    }
    let (key, body) = obj.iter().next().expect("one key");
    match key.as_str() {
        "all" | "any" => {
            let items = body.as_array().ok_or("all/any takes an array")?;
            if items.is_empty() {
                return Err("all/any needs at least one predicate".into());
            }
            let ps = items.iter().map(parse).collect::<Result<Vec<_>, _>>()?;
            Ok(if key == "all" {
                Pred::All(ps)
            } else {
                Pred::Any(ps)
            })
        }
        "not" => Ok(Pred::Not(Box::new(parse(body)?))),
        "cmp" => parse_cmp(body),
        other => Err(format!("unknown predicate key '{other}'")),
    }
}

fn parse_cmp(body: &Value) -> Result<Pred, String> {
    let obj = body.as_object().ok_or("cmp takes an object")?;
    for k in obj.keys() {
        if !matches!(k.as_str(), "field" | "sum" | "op" | "value") {
            return Err(format!("unknown cmp key '{k}'"));
        }
    }
    let op = Op::parse(
        obj.get("op")
            .and_then(Value::as_str)
            .ok_or("cmp needs 'op'")?,
    )?;
    let value = obj.get("value").cloned().ok_or("cmp needs 'value'")?;
    let lhs = match (obj.get("field"), obj.get("sum")) {
        (Some(f), None) => Lhs::Field(check_path(f.as_str().ok_or("field must be a string")?)?),
        (None, Some(s)) => {
            let arr = s.as_array().ok_or("sum takes an array of paths")?;
            if arr.is_empty() {
                return Err("sum needs at least one path".into());
            }
            let paths = arr
                .iter()
                .map(|p| check_path(p.as_str().ok_or("sum paths must be strings")?))
                .collect::<Result<Vec<_>, _>>()?;
            if !value.is_number() {
                return Err("a sum compares against a number".into());
            }
            Lhs::Sum(paths)
        }
        _ => return Err("cmp needs exactly one of 'field' or 'sum'".into()),
    };
    if matches!(lhs, Lhs::Field(_)) && !matches!(op, Op::Eq | Op::Ne) && !value.is_number() {
        return Err("ordering comparisons need a number".into());
    }
    Ok(Pred::Cmp { lhs, op, value })
}

fn check_path(p: &str) -> Result<String, String> {
    let segs: Vec<&str> = p.split('.').collect();
    if segs.iter().any(|s| s.is_empty()) {
        return Err(format!("malformed path '{p}'"));
    }
    if segs[..segs.len() - 1].contains(&"*") || segs[0] == "*" {
        return Err(format!(
            "'*' may only end a path of two or more parts: '{p}'"
        ));
    }
    Ok(p.to_string())
}

/// Every path a predicate reads.
pub fn paths(p: &Pred) -> Vec<String> {
    match p {
        Pred::All(ps) | Pred::Any(ps) => ps.iter().flat_map(paths).collect(),
        Pred::Not(q) => paths(q),
        Pred::Cmp {
            lhs: Lhs::Field(f), ..
        } => vec![f.clone()],
        Pred::Cmp {
            lhs: Lhs::Sum(fs), ..
        } => fs.clone(),
    }
}

/// Key comparison ignores case, `_` and `-`, so `HMR`, `Hmr` and `h_m_r` agree.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}

fn is_count_map(m: &serde_json::Map<String, Value>) -> bool {
    m.values().all(Value::is_number)
}

/// Resolves a dotted path. A final key missing from a nested map of counts reads 0; a trailing `*`
/// sums a map of counts; anything else missing is `None`.
pub fn resolve(root: &Value, path: &str) -> Option<Value> {
    let segs: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        let m = cur.as_object()?;
        if *seg == "*" {
            let total: f64 = m.values().map(Value::as_f64).sum::<Option<f64>>()?;
            return serde_json::Number::from_f64(total).map(Value::Number);
        }
        let key = norm(seg);
        match m.iter().find(|(k, _)| norm(k) == key) {
            Some((_, v)) => cur = v,
            None if last && i > 0 && is_count_map(m) => return Some(Value::from(0u64)),
            None => return None,
        }
    }
    if cur.is_null() {
        None
    } else {
        Some(cur.clone())
    }
}

fn lhs_value(facts: &Value, lhs: &Lhs) -> Option<Value> {
    match lhs {
        Lhs::Field(f) => resolve(facts, f),
        Lhs::Sum(fs) => {
            let total = fs
                .iter()
                .map(|f| resolve(facts, f).and_then(|v| v.as_f64()))
                .sum::<Option<f64>>()?;
            serde_json::Number::from_f64(total).map(Value::Number)
        }
    }
}

fn compare(a: &Value, op: Op, b: &Value) -> Option<bool> {
    if let (Some(x), Some(y)) = (a.as_f64(), b.as_f64()) {
        return Some(match op {
            Op::Eq => x == y,
            Op::Ne => x != y,
            Op::Gt => x > y,
            Op::Ge => x >= y,
            Op::Lt => x < y,
            Op::Le => x <= y,
        });
    }
    match op {
        Op::Eq => Some(a == b),
        Op::Ne => Some(a != b),
        _ => None,
    }
}

/// Three-valued evaluation against serialized facts.
pub fn eval(p: &Pred, facts: &Value) -> Option<bool> {
    match p {
        Pred::All(ps) => {
            let vs: Vec<Option<bool>> = ps.iter().map(|q| eval(q, facts)).collect();
            if vs.contains(&Some(false)) {
                Some(false)
            } else if vs.iter().all(|v| *v == Some(true)) {
                Some(true)
            } else {
                None
            }
        }
        Pred::Any(ps) => {
            let vs: Vec<Option<bool>> = ps.iter().map(|q| eval(q, facts)).collect();
            if vs.contains(&Some(true)) {
                Some(true)
            } else if vs.iter().all(|v| *v == Some(false)) {
                Some(false)
            } else {
                None
            }
        }
        Pred::Not(q) => eval(q, facts).map(|b| !b),
        Pred::Cmp { lhs, op, value } => compare(&lhs_value(facts, lhs)?, *op, value),
    }
}

/// A readable form of the predicate, for rejection messages.
pub fn describe(p: &Pred) -> String {
    let join = |ps: &[Pred], sep: &str| ps.iter().map(describe).collect::<Vec<_>>().join(sep);
    match p {
        Pred::All(ps) => format!("({})", join(ps, " and ")),
        Pred::Any(ps) => format!("({})", join(ps, " or ")),
        Pred::Not(q) => format!("not {}", describe(q)),
        Pred::Cmp {
            lhs: Lhs::Field(f),
            op,
            value,
        } => format!("{f} {} {value}", op.symbol()),
        Pred::Cmp {
            lhs: Lhs::Sum(fs),
            op,
            value,
        } => {
            format!("{} {} {value}", fs.join(" + "), op.symbol())
        }
    }
}

/// The observed value of every path the predicate reads, for rejection messages.
pub fn observed(p: &Pred, facts: &Value) -> String {
    paths(p)
        .iter()
        .map(|f| match resolve(facts, f) {
            Some(v) => format!("{f} = {v}"),
            None => format!("{f} = unknown"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
