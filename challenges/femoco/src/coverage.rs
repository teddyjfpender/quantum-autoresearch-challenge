//! Deterministic input coverage, supplementary to the sampled resource run.
//!
//! `exhaustive` enumerates the entire finite control/uniform/second-pass domain when it
//! fits the cap. `terms` enumerates one representative of EVERY reachable selected-term pair
//! with both controls and every value of the outer and both inner spin bits, plus every alias
//! bucket's boundaries on both inner passes. A spin bit is enumerated even where the reference
//! operator ignores it: whether the circuit ignores it is what is being checked.
//!
//! Neither mode exhausts measurement outcomes or proves a large circuit constant within an
//! alias cell, and `terms` takes one alias-cell representative per item. Measurement outcomes
//! come from one stream per batch keyed by the circuit digest and, when the run has one, the
//! server seed. Gaussian operator comparisons still use the reference tracker's numerical
//! tolerances. These limitations are part of the report.
use crate::fiat_shamir::Lane;
use crate::lanemap::df_nested::Table;
use crate::lanemap::sa_nested::SaNestedMap;
use crate::lanemap::{self, LaneMap};
use crate::score::{self, Inputs};
use crate::sim::{self, validate, Inner, Layout};
use crate::spec::sa::SaSpec;
use serde_json::{json, Value};
use sha2::Digest;

/// The most lanes one coverage run may hold. A plan above it is refused, never truncated.
pub const MAX_LANES: usize = 1 << 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Terms,
    Exhaustive,
}

#[derive(Debug)]
pub struct Plan {
    pub lanes: Vec<Lane>,
    pub after: Vec<u64>,
    pub term_pairs: usize,
    pub boundary_cases: usize,
}

impl Plan {
    fn new() -> Self {
        Self {
            lanes: Vec::new(),
            after: Vec::new(),
            term_pairs: 0,
            boundary_cases: 0,
        }
    }
    fn push(&mut self, s: u64, after: u64, cap: usize) -> Result<(), String> {
        if self.lanes.len().saturating_add(2) > cap {
            return Err(format!(
                "deterministic coverage exceeds lane cap {cap}; no partial pass"
            ));
        }
        for c in [false, true] {
            self.lanes.push(Lane { c, s });
            self.after.push(after);
        }
        Ok(())
    }
}

/// Every alias bucket at sigma = 0, keep-1, keep, max, with duplicates removed.
/// Values outside the finite sigma domain are never admitted (including keep=0).
pub fn boundaries(t: &Table) -> Vec<u64> {
    let max = (1u64 << t.mu) - 1;
    let mut out = Vec::new();
    for (i, &keep) in t.keep.iter().enumerate() {
        let mut draw = vec![0, max, u64::from(keep)];
        if keep != 0 {
            draw.push(u64::from(keep) - 1);
        }
        draw.sort_unstable();
        draw.dedup();
        for d in draw.into_iter().filter(|&d| d <= max) {
            out.push(i as u64 | d << t.k);
        }
    }
    out
}

/// One representative of each REACHABLE item, derived from the exact declared table.
/// Zero-count items have no input preimage and are not silently manufactured.
pub fn representatives(t: &Table, items: usize) -> Vec<Option<u64>> {
    let mut reps = vec![None; items];
    for s in boundaries(t) {
        reps[t.item(s)].get_or_insert(s);
    }
    reps
}

pub fn plan(map: &SaNestedMap, spec: &SaSpec, mode: Mode, cap: usize) -> Result<Plan, String> {
    let mut p = Plan::new();
    let (u, lo, w) = (map.uniform_bits(), map.outer_bits(), map.inner_width());
    if mode == Mode::Exhaustive {
        // Includes c, the first uniform value, and EVERY second-pass inner value.
        let bits = 1 + u + w;
        if bits >= usize::BITS || (1usize << bits) > cap {
            return Err(format!(
                "exhaustive domain needs 2^{bits} lanes, above cap {cap}"
            ));
        }
        for s in 0..1u64 << u {
            let outer = s & ((1u64 << lo) - 1);
            for b in 0..1u64 << w {
                p.push(s, outer | b << lo, cap)?;
            }
        }
        return Ok(p);
    }
    let outer = representatives(&map.outer, spec.outer_items());
    for (o, rep) in outer.into_iter().enumerate() {
        let Some(rep) = rep else { continue };
        let t = &map.inner[o];
        let inner: Vec<u64> = representatives(t, spec.inner_items(o))
            .into_iter()
            .flatten()
            .collect();
        let boundary = boundaries(t);
        // Every spin bit takes both values, including where the reference operator does not
        // depend on it (the outer spin of a square, the inner spin of a one-body or identity
        // item): a circuit can read a bit its reference ignores.
        for outer_spin in 0..2u64 {
            let out = rep | outer_spin << (lo - 1);
            for a in &inner {
                for b in &inner {
                    for spin_a in 0..2u64 {
                        for spin_b in 0..2u64 {
                            p.push(
                                out | (a | spin_a << (w - 1)) << lo,
                                out | (b | spin_b << (w - 1)) << lo,
                                cap,
                            )?;
                            p.term_pairs += 1;
                        }
                    }
                }
            }
            for a in boundary.iter().copied() {
                for spin in 0..2u64 {
                    let a = a | spin << (w - 1);
                    // Diagonal, and both orientations of each boundary against a representative.
                    p.push(out | a << lo, out | a << lo, cap)?;
                    p.boundary_cases += 1;
                    if let Some(b) = inner.first().copied() {
                        p.push(out | a << lo, out | b << lo, cap)?;
                        p.push(out | b << lo, out | a << lo, cap)?;
                        p.boundary_cases += 2;
                    }
                }
            }
        }
    }
    // Outer comparator boundaries may follow a different route to the SAME generator.
    for s in boundaries(&map.outer) {
        let o = map.outer_item(s);
        let t = &map.inner[o];
        let inner: Vec<u64> = representatives(t, spec.inner_items(o))
            .into_iter()
            .flatten()
            .collect();
        for spin in 0..2u64 {
            let out = s | spin << (lo - 1);
            for a in inner.iter().copied() {
                for inner_spin in 0..2u64 {
                    let v = out | (a | inner_spin << (w - 1)) << lo;
                    p.push(v, v, cap)?;
                    p.boundary_cases += 1;
                }
            }
        }
    }
    Ok(p)
}

/// The key of the coverage run's measurement-outcome streams: the circuit digest, and the
/// run's server seed when it has one, so a judged run's outcomes are not known in advance.
fn outcome_key(ops_sha256: &[u8; 32], seed: Option<&[u8; 32]>) -> [u8; 32] {
    let mut h = sha2::Sha256::new();
    h.update(b"femoco-deterministic-outcomes-v2");
    h.update(ops_sha256);
    if let Some(seed) = seed {
        h.update(seed);
    }
    h.finalize().into()
}

/// Revalidates the exact rounding rule and static circuit before any coverage/export.
/// `seed` is the run's server seed, if any. The export is written only after the requested
/// coverage has passed, so a rejected run leaves none behind.
pub fn check(
    inp: &Inputs<'_>,
    engine: validate::Engine,
    mode: Option<Mode>,
    export: Option<&std::path::Path>,
    seed: Option<&[u8; 32]>,
) -> Result<Option<Value>, String> {
    let lm = lanemap::parse(inp.lanemap, inp.spec)?;
    score::check_lanemap(lm.as_ref(), inp.spec)?;
    let map = lm
        .as_sa_nested()
        .ok_or("coverage requires sa-nested-alias-v1")?;
    let spec = inp
        .spec
        .as_any()
        .downcast_ref::<SaSpec>()
        .ok_or("coverage requires sos-sa")?;
    let layout = Layout {
        system: inp.spec.system_qubits(),
        uniform: lm.uniform_bits(),
    };
    let inner = Inner {
        lo: map.outer_bits(),
        width: map.inner_width(),
    };
    let compiled = sim::compile_sa(&inp.ops.ops, &layout, inp.tracker, Some(inner))?;
    let write_export = || -> Result<(), String> {
        let Some(path) = export else { return Ok(()) };
        // This is an INPUT to the independent SMT checker, never a certificate supplied by
        // the builder. The checker binds its result to these artifact digests.
        let table = |t: &Table| json!({"k":t.k,"mu":t.mu,"keep":t.keep,"alt":t.alt});
        let doc = json!({
            "schema":"femoco-symbolic-input-v1", "spec":inp.spec.id(),
            "digests":{"ops":hex::encode(inp.ops.sha256),
                "spec":hex::encode(inp.spec.payload_sha256()),
                "lanemap":hex::encode(sha2::Sha256::digest(inp.lanemap)),
                "family":hex::encode(sha2::Sha256::digest(inp.family))},
            "layout":{"system":layout.system,"uniform":layout.uniform,
                "num_qubits":compiled.num_qubits,"num_bits":compiled.num_bits},
            "inner":{"lo":inner.lo,"width":inner.width},
            "beta":spec.beta, "b":spec.b, "rotation_widths":spec.widths,
            "registers":compiled.registers, "ops":compiled.ops,
            "outer":table(&map.outer), "inner_tables":map.inner.iter().map(table).collect::<Vec<_>>(),
        });
        std::fs::write(path, serde_json::to_vec(&doc).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(())
    };
    let Some(mode) = mode else {
        write_export()?;
        return Ok(None);
    };
    let p = plan(map, spec, mode, MAX_LANES)?;
    if p.lanes.is_empty() {
        return Err("deterministic coverage: the plan is empty".into());
    }
    let reference = |s| lm.reference_op(inp.spec, s);
    let reference_nested = |s, after| lm.reference_nested(inp.spec, s, after);
    let uniform_after = |s| lm.uniform_after(s);
    let ctx = validate::Context {
        compiled: &compiled,
        layout,
        hmr_key: outcome_key(&inp.ops.sha256, seed),
        reference: &reference,
        uniform_after: &uniform_after,
        tracker: inp.tracker,
    };
    let nested = validate::Nested {
        lo: inner.lo,
        width: inner.width,
        after: &p.after,
        reference: &reference_nested,
    };
    let out = engine(&ctx, Some(&nested), &p.lanes, None);
    if let Some(why) = out.rejection() {
        return Err(format!("deterministic coverage: {why}"));
    }
    if out.lanes != p.lanes.len() {
        return Err(format!(
            "deterministic coverage: the engine ran {} of {} lanes",
            out.lanes,
            p.lanes.len()
        ));
    }
    write_export()?;
    Ok(Some(json!({
        "protocol":"femoco-deterministic-v2", "status":"passed", "mode":format!("{mode:?}"),
        "lanes":p.lanes.len(), "term_pair_cases":p.term_pairs, "boundary_cases":p.boundary_cases,
        "input_coverage":if mode==Mode::Exhaustive {"all finite control/uniform/second-pass inputs"} else {"one alias-cell representative of every reachable term pair under both controls and every outer and inner spin value, plus alias comparator boundaries"},
        "measurement_outcomes":if seed.is_some() {"one stream per batch keyed by the circuit digest and the server seed; not exhaustive"} else {"one fixed stream per batch keyed by the circuit digest; not exhaustive"},
        "system_equivalence":"reference Gaussian tolerances, not exact symbolic quantum equivalence",
        "certified":false,
        "resource_counts":"independent sampled run; deterministic coverage does not change the score"
    })))
}
