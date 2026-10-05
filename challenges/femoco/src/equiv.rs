//! Equivalence harness for lane engines (tooling; spec/FAST-EVALUATOR.md sections 5 and 10).
//!
//! A lane engine (`sim::validate::Engine`) replaces `validate::run` / `run_nested` inside
//! `score::evaluate_with`; everything before it (parse, compile, sampling) and after it (facts,
//! taxonomy, score arithmetic, `score.json`) is shared code. Two engines are equivalent on an
//! input when, at every thread count, they give:
//! - the same `evaluate` result: the same rejection string, or the same `score.json` with only
//!   the wall-clock `eval_seconds` removed (compared as serialized text, so floats are compared
//!   bit for bit, signed zeros included);
//! - the same lanes handed to the engine, in the same order;
//! - the same per-lane verdict vector (pass, failure category, or unchecked);
//! - the same `Outcome` (`Debug` text: every tally, every failure count and the first failure of
//!   each category with its message), observable or not.
//!
//! This module never changes a verdict: `evaluate_checked`, `run_checked` and
//! `run_nested_checked` return the reference's own result and only panic when a second engine,
//! named by `FEMOCO_EQUIV_ENGINE`, differs from it. Without that variable they are exactly
//! `score::evaluate`, `validate::run` and `validate::run_nested`.
//!
//! Self-test engines (`selftest-*`) are deliberately wrong in one observable each. They exist
//! so the harness can be shown to catch each kind of difference; `engine` (what
//! `eval_circuit --engine` accepts) never returns them.
use crate::fiat_shamir::{self, AuditBeacon, Lane};
use crate::score::{self, Evaluation, Inputs, Probe};
use crate::sim::validate::{self, Context, Engine, Nested, Outcome, VERDICT_PASS};
use serde_json::{json, Value};
use std::fmt::Write as _;

/// Engines `eval_circuit --engine` accepts, in order. The reference is the default.
pub const ENGINES: &[&str] = &["reference", "sliced"];

/// The production engine named `name` (`eval_circuit --engine`).
#[must_use]
pub fn engine(name: &str) -> Option<Engine> {
    match name {
        "reference" => Some(validate::reference),
        "sliced" => Some(crate::fastsim::run_probed),
        _ => None,
    }
}

/// The named engine's own counters from its most recent run in this process (batches re-run
/// on the reference, memo keys, guard-band fallbacks...), for reports. Empty for the reference
/// and the self-test engines.
#[must_use]
pub fn engine_counters(name: &str) -> Vec<(&'static str, u64)> {
    match name {
        "sliced" => crate::fastsim::last_stats(),
        _ => Vec::new(),
    }
}

/// Self-test engines: each is the reference with one deliberate difference.
pub const SELFTEST_ENGINES: &[&str] = &[
    "selftest-tally",
    "selftest-verdict",
    "selftest-accept",
    "selftest-hmrkey",
    "selftest-message",
    "selftest-threads",
    "selftest-unguarded",
];

/// A production engine or a self-test engine (the harness's own tests only).
#[must_use]
pub fn harness_engine(name: &str) -> Option<Engine> {
    engine(name).or(match name {
        "selftest-tally" => Some(st_tally),
        "selftest-verdict" => Some(st_verdict),
        "selftest-accept" => Some(st_accept),
        "selftest-hmrkey" => Some(st_hmrkey),
        "selftest-message" => Some(st_message),
        "selftest-threads" => Some(st_threads),
        "selftest-unguarded" => Some(st_unguarded),
        _ => None,
    })
}

/// One more executed Clifford in total: `score.json`'s `cliffords` moves (a passing run).
fn st_tally(c: &Context<'_>, n: Option<&Nested<'_>>, l: &[Lane], v: Option<&mut [u8]>) -> Outcome {
    let mut o = validate::reference(c, n, l, v);
    o.tally.cliffords += 1;
    o
}

/// Lane 0's recorded verdict is wrong; the `Outcome` is the reference's.
fn st_verdict(
    c: &Context<'_>,
    n: Option<&Nested<'_>>,
    l: &[Lane],
    mut v: Option<&mut [u8]>,
) -> Outcome {
    let o = validate::reference(c, n, l, v.as_deref_mut());
    if let Some(x) = v.and_then(|v| v.first_mut()) {
        *x = if *x == VERDICT_PASS { 8 } else { VERDICT_PASS };
    }
    o
}

/// Accepts every lane: a rejected circuit passes.
fn st_accept(
    c: &Context<'_>,
    n: Option<&Nested<'_>>,
    l: &[Lane],
    mut v: Option<&mut [u8]>,
) -> Outcome {
    let mut o = validate::reference(c, n, l, v.as_deref_mut());
    o.failed = [0; 10];
    o.first = Default::default();
    if let Some(v) = v {
        v.fill(VERDICT_PASS);
    }
    o
}

/// Reads the `Hmr` outcomes from a different key: every outcome-dependent count or verdict moves.
fn st_hmrkey(c: &Context<'_>, n: Option<&Nested<'_>>, l: &[Lane], v: Option<&mut [u8]>) -> Outcome {
    let mut key = c.hmr_key;
    key[0] ^= 1;
    let ctx = Context {
        compiled: c.compiled,
        layout: c.layout,
        hmr_key: key,
        reference: c.reference,
        uniform_after: c.uniform_after,
        tracker: c.tracker,
    };
    validate::reference(&ctx, n, l, v)
}

/// The first failure's message is reworded: the rejection string moves, the verdict does not.
fn st_message(
    c: &Context<'_>,
    n: Option<&Nested<'_>>,
    l: &[Lane],
    v: Option<&mut [u8]>,
) -> Outcome {
    let mut o = validate::reference(c, n, l, v);
    if let Some(f) = o.first.iter_mut().flatten().next() {
        f.msg.push('.');
    }
    o
}

/// Depends on the thread count: equal to the reference at one thread only.
fn st_threads(
    c: &Context<'_>,
    n: Option<&Nested<'_>>,
    l: &[Lane],
    v: Option<&mut [u8]>,
) -> Outcome {
    let mut o = validate::reference(c, n, l, v);
    o.tally.resets += rayon::current_num_threads() as u64 - 1;
    o
}

/// The tracker check evaluated with a small floating-point error and no guard band: the `Ad`
/// residual is used as `off - UNGUARDED_ERROR`, as a folded check whose rounding differs from the
/// reference's by that much would see it. Far from the threshold nothing changes; within
/// `UNGUARDED_ERROR` above `AD_TOL` a failing lane passes. The guard-band tests
/// (tests/equiv_guard.rs) must catch it.
fn st_unguarded(
    c: &Context<'_>,
    n: Option<&Nested<'_>>,
    l: &[Lane],
    v: Option<&mut [u8]>,
) -> Outcome {
    let Some(g) = c
        .tracker
        .and_then(crate::sim::TrackerFactory::as_gaussian)
        .filter(|g| g.widths().is_none())
    else {
        return validate::reference(c, n, l, v);
    };
    let f = Unguarded { inner: g };
    let ctx = Context {
        compiled: c.compiled,
        layout: c.layout,
        hmr_key: c.hmr_key,
        reference: c.reference,
        uniform_after: c.uniform_after,
        tracker: Some(&f),
    };
    validate::reference(&ctx, n, l, v)
}

/// The emulated folding error of `selftest-unguarded` (the prototype's measured
/// `|off_fold - off_ref|` reached 2.3e-14; spec/FAST-EVALUATOR.md section 6).
pub const UNGUARDED_ERROR: f64 = 4e-15;

struct Unguarded<'a> {
    inner: &'a crate::sim::gaussian::GaussianFactory,
}

struct UnguardedLane(crate::sim::gaussian::GaussianLane);

impl crate::sim::tracker::LaneTracker for UnguardedLane {
    fn givens(
        &mut self,
        before: &crate::sim::tracker::LaneFrame,
        p: usize,
        angle: u64,
    ) -> Result<(), String> {
        self.0.givens(before, p, angle)
    }
    fn givens_modes(
        &mut self,
        before: &crate::sim::tracker::LaneFrame,
        p: usize,
        q: usize,
        angle: u64,
    ) -> Result<(), String> {
        self.0.givens_modes(before, p, q, angle)
    }
    /// `GaussianLane::finish` with `off - UNGUARDED_ERROR` (same messages).
    fn finish(
        &mut self,
        after: &crate::sim::tracker::LaneFrame,
        reference: Option<&crate::spec::SystemOp>,
    ) -> Result<(), String> {
        use crate::sim::gaussian::{AD_TOL, PHASE_TOL};
        let (off, c) = self.0.measure(after, reference)?;
        let off = off - UNGUARDED_ERROR;
        if off > AD_TOL {
            return Err(format!(
                "the rotated operator differs from the reference: max |Ad(O_ref^-1 O) - I| = {off:.3e} (tolerance {AD_TOL:e})"
            ));
        }
        match c.nearest_w() {
            (0, d) if d <= PHASE_TOL => Ok(()),
            (4, d) if d <= PHASE_TOL => {
                Err("sign flipped: the operator is -1 times the reference".into())
            }
            (j, d) if d <= PHASE_TOL => Err(format!("phase off by w^{j} from the reference")),
            (_, d) => Err(format!("vacuum overlap {c:?} is {d:.3e} from any w^j")),
        }
    }
}

impl crate::sim::TrackerFactory for Unguarded<'_> {
    fn lane(&self, system_qubits: usize) -> Box<dyn crate::sim::tracker::LaneTracker> {
        Box::new(UnguardedLane(crate::sim::gaussian::GaussianLane::new(
            system_qubits,
            self.inner.beta(),
        )))
    }
    fn givens_toffoli_cost(&self) -> f64 {
        self.inner.givens_toffoli_cost()
    }
    fn phase_gradient_qubits(&self) -> u64 {
        self.inner.phase_gradient_qubits()
    }
    fn quarter_turn(&self) -> Option<u64> {
        self.inner.quarter_turn()
    }
    fn givens_charge(&self, p: usize, q: usize) -> Option<u64> {
        self.inner.givens_charge(p, q)
    }
}

/// Which seed stream draws the lanes.
#[derive(Clone, Copy, Debug)]
pub enum Seed {
    /// The ordinary Fiat-Shamir formula.
    Ordinary,
    /// `eval_circuit --server-seed`: the audit domain at round 0.
    Server([u8; 32]),
    /// A fresh-seed audit (`fiat_shamir::with_audit`) at this beacon.
    Audit(AuditBeacon),
}

impl Seed {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Ordinary => "ordinary".into(),
            Self::Server(r) => format!("server:{}", &hex::encode(r)[..8]),
            Self::Audit(b) => format!("audit:{}:{}", b.round, &hex::encode(b.randomness)[..8]),
        }
    }

    fn beacon(&self) -> Option<AuditBeacon> {
        match *self {
            Self::Ordinary => None,
            Self::Server(randomness) => Some(AuditBeacon {
                round: 0,
                randomness,
            }),
            Self::Audit(b) => Some(b),
        }
    }

    /// Runs `f` under this seed on the calling thread.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        match self.beacon() {
            None => f(),
            Some(b) => fiat_shamir::with_audit(b, f),
        }
    }
}

/// A deterministic 32-byte seed from a label (for test plans).
#[must_use]
pub fn seed_bytes(label: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("femoco-equiv-seed:{label}").as_bytes()).into()
}

/// One engine's evaluation of one input.
#[derive(Clone, Debug)]
pub struct Run {
    pub engine: String,
    /// `0`: the caller's thread pool.
    pub threads: usize,
    pub seed: String,
    /// `score.json` without `eval_seconds` (serialized), or the rejection.
    pub result: Result<String, String>,
    pub probe: Probe,
    /// Wall-clock seconds of the whole evaluation (parse to score), for reports only.
    pub seconds: f64,
    /// The engine's own counters after the run (`engine_counters`), for reports only.
    pub counters: Vec<(&'static str, u64)>,
}

/// `score.json` for `ev` with every `eval_seconds` (wall clock) removed, serialized.
#[must_use]
pub fn normalized_score(ev: &Evaluation) -> String {
    let mut v = score::score_json(ev);
    strip_seconds(&mut v);
    serde_json::to_string(&v).unwrap_or_default()
}

fn strip_seconds(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("eval_seconds");
            m.values_mut().for_each(strip_seconds);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_seconds),
        _ => {}
    }
}

/// Runs `f` on a fresh pool of `threads` threads (`0`: the caller's pool), carrying the
/// caller's active audit beacon into it (`with_audit` is thread-local).
pub fn on_threads<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    if threads == 0 {
        return f();
    }
    let beacon = fiat_shamir::active_audit();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("thread pool");
    pool.install(|| match beacon {
        None => f(),
        Some(b) => fiat_shamir::with_audit(b, f),
    })
}

thread_local! {
    /// The engine and thread count [`threaded`] runs (set by [`evaluate_on`]).
    static THREADED: std::cell::Cell<Option<(Engine, usize)>> = const { std::cell::Cell::new(None) };
}

/// The engine of the enclosing [`evaluate_on`], run on its own pool of the requested size. Only
/// the engine moves to that pool: parsing, sampling and scoring stay on the calling thread.
fn threaded(c: &Context<'_>, n: Option<&Nested<'_>>, l: &[Lane], v: Option<&mut [u8]>) -> Outcome {
    let (e, t) = THREADED
        .with(std::cell::Cell::get)
        .expect("threaded engine outside evaluate_on");
    on_threads(t, || e(c, n, l, v))
}

/// `score::evaluate_with(inp, engine, probe)` with the engine on a pool of `threads` threads
/// (`0`: the caller's pool).
///
/// # Errors
/// As `score::evaluate`.
pub fn evaluate_on(
    inp: &Inputs<'_>,
    engine: Engine,
    threads: usize,
    probe: Option<&mut Probe>,
) -> Result<Evaluation, String> {
    if threads == 0 {
        return score::evaluate_with(inp, engine, probe);
    }
    let prev = THREADED.with(|c| c.replace(Some((engine, threads))));
    let r = score::evaluate_with(inp, threaded, probe);
    THREADED.with(|c| c.set(prev));
    r
}

/// Evaluates `inp` with `engine` on `threads` threads under `seed`, recording the probe.
#[must_use]
pub fn run(inp: &Inputs<'_>, name: &str, engine: Engine, threads: usize, seed: Seed) -> Run {
    let mut probe = Probe::default();
    let t0 = std::time::Instant::now();
    let r = seed.scope(|| evaluate_on(inp, engine, threads, Some(&mut probe)));
    let seconds = t0.elapsed().as_secs_f64();
    Run {
        engine: name.to_string(),
        threads,
        seed: seed.label(),
        result: r.as_ref().map(normalized_score).map_err(Clone::clone),
        probe,
        seconds,
        counters: engine_counters(name),
    }
}

/// The paths at which two JSON values differ (at most `limit`).
fn json_diff(a: &Value, b: &Value, path: &str, out: &mut Vec<String>, limit: usize) {
    if out.len() >= limit || a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let p = format!("{path}.{k}");
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => json_diff(u, v, &p, out, limit),
                    (u, v) => out.push(format!("{p}: {u:?} vs {v:?}")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                json_diff(u, v, &format!("{path}[{i}]"), out, limit);
            }
        }
        _ => out.push(format!("{path}: {a} vs {b}")),
    }
}

/// Every difference between two runs of the same input; empty when they are equivalent.
#[must_use]
pub fn differences(a: &Run, b: &Run) -> Vec<String> {
    let mut d = Vec::new();
    match (&a.result, &b.result) {
        (Ok(x), Ok(y)) if x != y => {
            let (u, v) = (
                serde_json::from_str::<Value>(x).unwrap_or(Value::Null),
                serde_json::from_str::<Value>(y).unwrap_or(Value::Null),
            );
            let mut paths = Vec::new();
            json_diff(&u, &v, "score", &mut paths, 12);
            if paths.is_empty() {
                paths.push("score.json text differs".into());
            }
            d.extend(paths);
        }
        (Err(x), Err(y)) if x != y => d.push(format!("rejection: {x:?} vs {y:?}")),
        (Ok(_), Err(y)) => d.push(format!("verdict: accepted vs rejected ({y})")),
        (Err(x), Ok(_)) => d.push(format!("verdict: rejected ({x}) vs accepted")),
        _ => {}
    }
    d.extend(probe_differences(&a.probe, &b.probe));
    d
}

/// Differences between two probes (lanes, verdicts, `Outcome`).
#[must_use]
pub fn probe_differences(a: &Probe, b: &Probe) -> Vec<String> {
    let mut d = Vec::new();
    if a.ran != b.ran {
        d.push(format!("engine ran: {} vs {}", a.ran, b.ran));
    }
    if a.lanes != b.lanes {
        let i = a.lanes.iter().zip(&b.lanes).position(|(x, y)| x != y);
        d.push(format!(
            "lanes: {} vs {} lanes, first difference at {i:?}",
            a.lanes.len(),
            b.lanes.len()
        ));
    }
    d.extend(verdict_differences(&a.verdicts, &b.verdicts));
    if a.outcome != b.outcome {
        d.push(format!("outcome: {}", text_diff(&a.outcome, &b.outcome)));
    }
    d
}

/// The first point at which two `Debug` texts differ, with the field name before it.
fn text_diff(a: &str, b: &str) -> String {
    let mut i = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    while !a.is_char_boundary(i) || !b.is_char_boundary(i) {
        i -= 1;
    }
    let start = a[..i].rfind(['{', ',', '[']).map_or(0, |k| k + 1);
    let field = |s: &str| {
        let rest = &s[start..];
        let end = rest.find([',', '}', ']']).unwrap_or(rest.len());
        clip(rest[..end].trim(), 300)
    };
    format!("{} vs {} (at byte {i})", field(a), field(b))
}

/// Differences between two verdict vectors: their lengths and the first few differing lanes.
#[must_use]
pub fn verdict_differences(a: &[u8], b: &[u8]) -> Vec<String> {
    if a == b {
        return Vec::new();
    }
    let bad: Vec<usize> = (0..a.len().min(b.len()))
        .filter(|&i| a[i] != b[i])
        .collect();
    let mut s = format!(
        "verdicts: {} vs {} lanes, {} differ",
        a.len(),
        b.len(),
        bad.len()
    );
    for &i in bad.iter().take(5) {
        let _ = write!(
            s,
            "; lane #{i}: {} vs {}",
            verdict_name(a[i]),
            verdict_name(b[i])
        );
    }
    vec![s]
}

/// A verdict byte's name.
#[must_use]
pub fn verdict_name(v: u8) -> &'static str {
    match v {
        VERDICT_PASS => "pass",
        validate::VERDICT_UNCHECKED => "unchecked",
        c => validate::CATEGORIES
            .get(c as usize)
            .copied()
            .unwrap_or("invalid"),
    }
}

/// Counts of each verdict in `v`, by name.
#[must_use]
pub fn verdict_counts(v: &[u8]) -> Value {
    let mut m = std::collections::BTreeMap::<&str, u64>::new();
    for &x in v {
        *m.entry(verdict_name(x)).or_default() += 1;
    }
    json!(m)
}

fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

// ---- Drop-in checked entry points for the existing test suites. ----

/// The candidate engine named by `FEMOCO_EQUIV_ENGINE`, if any.
///
/// # Panics
/// If the variable names no engine (a misspelt name must not silently skip the comparison).
#[must_use]
pub fn env_engine() -> Option<(String, Engine)> {
    let name = std::env::var("FEMOCO_EQUIV_ENGINE").ok()?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let e = harness_engine(&name)
        .unwrap_or_else(|| panic!("FEMOCO_EQUIV_ENGINE={name}: no such engine"));
    Some((name, e))
}

/// The candidate's thread counts, `FEMOCO_EQUIV_THREADS` (comma separated; default `1,4`).
#[must_use]
pub fn env_threads() -> Vec<usize> {
    std::env::var("FEMOCO_EQUIV_THREADS")
        .ok()
        .map(|s| {
            s.split(',')
                .filter_map(|t| t.trim().parse().ok())
                .collect::<Vec<usize>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![1, 4])
}

/// Appends one JSON line to `FEMOCO_EQUIV_LOG`, if set.
pub fn log(line: &Value) {
    let Ok(path) = std::env::var("FEMOCO_EQUIV_LOG") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        // One write per line: concurrent tests append to the same file (O_APPEND).
        let _ = f.write_all(format!("{line}\n").as_bytes());
    }
}

fn report(kind: &str, what: Value, reference: &Probe, cand: &str, diffs: &[(usize, Vec<String>)]) {
    let bad: Vec<_> = diffs.iter().filter(|(_, d)| !d.is_empty()).collect();
    log(&json!({
        "kind": kind,
        "what": what,
        "engine": cand,
        "threads": diffs.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
        "lanes": reference.verdicts.len(),
        "verdicts": verdict_counts(&reference.verdicts),
        "equal": bad.is_empty(),
        "differences": bad.iter().map(|(t, d)| json!({"threads": t, "differences": d})).collect::<Vec<_>>(),
    }));
    if let Some((t, d)) = bad.first() {
        panic!(
            "equivalence: engine {cand} at {t} threads differs from the reference ({kind} {what}):\n  {}",
            d.join("\n  ")
        );
    }
}

/// `score::evaluate`, and, when `FEMOCO_EQUIV_ENGINE` names a second engine, that engine at
/// every `FEMOCO_EQUIV_THREADS` count, compared with the reference under the caller's seed.
///
/// # Errors
/// The reference's rejection.
///
/// # Panics
/// If the second engine differs from the reference in any respect.
pub fn evaluate_checked(inp: &Inputs<'_>) -> Result<Evaluation, String> {
    let Some((name, cand)) = env_engine() else {
        return score::evaluate(inp);
    };
    let mut probe = Probe::default();
    let r = score::evaluate_with(inp, validate::reference, Some(&mut probe));
    let reference = Run {
        engine: "reference".into(),
        threads: 0,
        seed: String::new(),
        result: r.as_ref().map(normalized_score).map_err(Clone::clone),
        probe,
        seconds: 0.0,
        counters: Vec::new(),
    };
    let diffs: Vec<(usize, Vec<String>)> = env_threads()
        .into_iter()
        .map(|t| {
            let mut probe = Probe::default();
            let c = evaluate_on(inp, cand, t, Some(&mut probe));
            let run = Run {
                engine: name.clone(),
                threads: t,
                seed: String::new(),
                result: c.as_ref().map(normalized_score).map_err(Clone::clone),
                probe,
                seconds: 0.0,
                counters: engine_counters(&name),
            };
            (t, differences(&reference, &run))
        })
        .collect();
    let what = json!({
        "spec": inp.spec.id(),
        "ops": inp.ops.ops.len(),
        "samples": inp.samples,
        "audit": fiat_shamir::active_audit().is_some(),
        "rejection": r.as_ref().err().map(|e| clip(e, 160)),
    });
    report("evaluate", what, &reference.probe, &name, &diffs);
    r
}

fn run_engine_probe(
    e: Engine,
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    lanes: &[Lane],
) -> (Outcome, Probe) {
    let mut v = vec![validate::VERDICT_UNCHECKED; lanes.len()];
    let o = e(ctx, nested, lanes, Some(&mut v));
    let p = Probe {
        verdicts: v,
        lanes: lanes.to_vec(),
        outcome: format!("{o:?}"),
        ran: true,
    };
    (o, p)
}

/// The lane-level comparison: the reference's `Outcome` and verdicts against `cand`'s at each
/// thread count. Returns the reference's `Outcome` and the differences per thread count.
pub fn compare_engines(
    cand: Engine,
    threads: &[usize],
    ctx: &Context<'_>,
    nested: Option<&Nested<'_>>,
    lanes: &[Lane],
) -> (Outcome, Probe, Vec<(usize, Vec<String>)>) {
    let (o, p) = run_engine_probe(validate::reference, ctx, nested, lanes);
    let diffs = threads
        .iter()
        .map(|&t| {
            let (_, q) = on_threads(t, || run_engine_probe(cand, ctx, nested, lanes));
            (t, probe_differences(&p, &q))
        })
        .collect();
    (o, p, diffs)
}

fn checked(ctx: &Context<'_>, nested: Option<&Nested<'_>>, lanes: &[Lane]) -> Outcome {
    let Some((name, cand)) = env_engine() else {
        return validate::reference(ctx, nested, lanes, None);
    };
    let (o, p, diffs) = compare_engines(cand, &env_threads(), ctx, nested, lanes);
    let what = json!({
        "ops": ctx.compiled.ops.len(),
        "nested": nested.is_some(),
        "rejection": o.rejection().map(|e| clip(&e, 160)),
    });
    report("lanes", what, &p, &name, &diffs);
    o
}

/// `validate::run`, checked against `FEMOCO_EQUIV_ENGINE` as [`evaluate_checked`].
///
/// # Panics
/// If the second engine differs from the reference.
#[must_use]
pub fn run_checked(ctx: &Context<'_>, lanes: &[Lane]) -> Outcome {
    checked(ctx, None, lanes)
}

/// `validate::run_nested`, checked against `FEMOCO_EQUIV_ENGINE` as [`evaluate_checked`].
///
/// # Panics
/// If the second engine differs from the reference.
#[must_use]
pub fn run_nested_checked(ctx: &Context<'_>, nested: &Nested<'_>, lanes: &[Lane]) -> Outcome {
    checked(ctx, Some(nested), lanes)
}
