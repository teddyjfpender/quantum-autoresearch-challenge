//! Deterministic input coverage, supplementary to the sampled resource run.
//!
//! `exhaustive` enumerates the entire finite control/uniform/second-pass domain when it
//! fits the cap. `terms` enumerates a representative of EVERY reachable selected-term pair,
//! both controls and all semantically relevant spin bits, plus every alias bucket's boundaries on
//! both inner passes. Neither mode exhausts measurement outcomes or proves a large circuit
//! constant within an alias cell. Gaussian operator comparisons still use the reference
//! tracker's numerical tolerances. These limitations are part of the report.
use crate::fiat_shamir::Lane;
use crate::lanemap::df_nested::Table;
use crate::lanemap::sa_nested::SaNestedMap;
use crate::lanemap::{self, LaneMap};
use crate::score::{self, Inputs};
use crate::sim::{self, validate, Inner, Layout};
use crate::spec::sa::SaSpec;
use serde_json::{json, Value};

pub const MAX_LANES: usize = 1 << 24;

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
        // Square generators ignore the outer spin. One-body generators use both spins.
        for outer_spin in 0..if o < spec.n { 2u64 } else { 1u64 } {
            let out = rep | outer_spin << (lo - 1);
            for a in &inner {
                for b in &inner {
                    let spins = |v: u64| {
                        if o < spec.n || t.item(v) == spec.b {
                            1u64
                        } else {
                            2u64
                        }
                    };
                    for spin_a in 0..spins(*a) {
                        for spin_b in 0..spins(*b) {
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
                let v = out | a << lo;
                p.push(v, v, cap)?;
                p.boundary_cases += 1;
            }
        }
    }
    Ok(p)
}

/// Revalidates the exact rounding rule and static circuit before any coverage/export.
pub fn check(
    inp: &Inputs<'_>,
    engine: validate::Engine,
    mode: Option<Mode>,
    export: Option<&std::path::Path>,
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
    if let Some(path) = export {
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
    }
    let Some(mode) = mode else { return Ok(None) };
    let p = plan(map, spec, mode, MAX_LANES)?;
    let reference = |s| lm.reference_op(inp.spec, s);
    let reference_nested = |s, after| lm.reference_nested(inp.spec, s, after);
    let uniform_after = |s| lm.uniform_after(s);
    let ctx = validate::Context {
        compiled: &compiled,
        layout,
        hmr_key: inp.ops.sha256,
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
    Ok(Some(json!({
        "protocol":"femoco-deterministic-v1", "status":"passed", "mode":format!("{mode:?}"),
        "lanes":p.lanes.len(), "term_pair_cases":p.term_pairs, "boundary_cases":p.boundary_cases,
        "input_coverage":if mode==Mode::Exhaustive {"all finite control/uniform/second-pass inputs"} else {"all reachable term-pair representatives and alias comparator boundaries"},
        "measurement_outcomes":"sampled, not exhaustive",
        "system_equivalence":"reference Gaussian tolerances, not exact symbolic quantum equivalence",
        "certified":false,
        "resource_counts":"independent sampled run; deterministic coverage does not change the score"
    })))
}

use sha2::Digest;
