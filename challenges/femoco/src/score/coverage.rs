//! The outcome-coverage mode's acceptance rule and report (`eval_circuit --outcome-coverage`;
//! spec/SAMPLING.md section 2.3, spec/DESIGN.md section 9).
//!
//! `Hmr` outcomes are drawn uniformly per lane, so a block conditioned on a rare outcome pattern
//! runs on few or no sampled lanes. In this mode the engines count, per conditioned site, the
//! lanes on which the site executed, and the rule here requires
//! [`COVERAGE_MIN`] control-1 executions of every site that is not proved unreachable. Sites
//! the uniform lanes leave short get directed lanes: outcomes uniform on the solutions of the
//! site's own condition (`sim::condition::Directed`), every per-lane check applied as usual.
//!
//! What may be relied on, and why:
//!
//! - Counts come from the engine that validated the lanes, from the masks that gate the ops, and
//!   are read only after every lane of that engine call passed. So "executed" means "executed on
//!   a lane that passed".
//! - The static pass (`sim::condition::analyze`) can only relax the rule for a site it proves
//!   unreachable. [`settle`] checks that claim against the counts (such a site must have no
//!   execution anywhere) and checks that a site executes on every lane of its own stratum. Any
//!   disagreement refuses the run.
//! - A site whose condition the static pass cannot follow gets no directed lanes; if the uniform
//!   lanes leave it short the run is refused.
//!
//! Directed lanes are used for rejection and for these counts only. Their tallies are dropped,
//! so every cost figure and every bound of `score.json` is that of the uniform run.
use crate::circuit::{Op, OperationType as K, NONE};
use crate::sim::compile::SimOp;
use crate::sim::condition::{self, Analysis, Class, System};
pub use crate::sim::condition::{COVERAGE_MIN, MAX_STRATA, STRATUM_LANES};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// `metrics.outcome_coverage.mode`.
pub const MODE: &str = "femoco-outcome-coverage-v1";

/// What a passing run in the mode adds to an `Evaluation`.
#[derive(Clone, Debug)]
pub struct CoverageReport {
    /// Conditioned sites: `PushCondition` blocks and ops with a condition of their own.
    pub sites: usize,
    pub proved_unreachable: usize,
    pub affine: usize,
    pub opaque: usize,
    /// Directed strata that ran, and the lanes of each.
    pub directed_strata: usize,
    pub lanes_per_stratum: usize,
    /// Fewest validated control-1 (control-0) executions of a site not proved unreachable;
    /// `None` when there is no such site.
    pub min_control_one: Option<u64>,
    pub min_control_zero: Option<u64>,
    /// `metrics.outcome_coverage`.
    pub json: Value,
    /// The whole site table (`eval_circuit --dump-coverage`): per site, in stream order, `u64`
    /// op index in `ops.bin`, `u8` class (0 proved unreachable, 1 affine, 2 opaque), `u8` rank,
    /// `u64` control-1 executions, `u64` control-0 executions, little-endian. `json` carries its
    /// SHA-256.
    pub table: Arc<Vec<u8>>,
}

/// The static pass's result with each site's place in `ops.bin`.
pub(crate) struct Plan {
    pub(crate) analysis: Analysis,
    /// Per site: the index of its op in the raw stream.
    raw: Vec<u32>,
}

impl Plan {
    /// Runs the static pass over the lowered stream `ops` and ties each site to its op in the
    /// raw stream `raw` (for messages and the report only).
    pub(crate) fn new(raw: &[Op], ops: &[SimOp]) -> Result<Self, String> {
        let internal = |what: &str| format!("outcome-coverage: internal error: {what}; refusing");
        // The ops `sim::compile` lowers to nothing.
        let kept: Vec<usize> = (0..raw.len())
            .filter(|&i| {
                !matches!(
                    raw[i].kind,
                    K::Register | K::AppendToRegister | K::DebugPrint | K::Segment
                )
            })
            .collect();
        if kept.len() != ops.len() {
            return Err(internal("the lowered stream does not line up with ops.bin"));
        }
        let analysis = condition::analyze(ops);
        let mut at = Vec::with_capacity(analysis.sites.len());
        for s in &analysis.sites {
            let i = kept[s.op as usize];
            if raw[i].kind != K::PushCondition && raw[i].c_condition == NONE {
                return Err(internal("a site's op in ops.bin carries no condition"));
            }
            at.push(u32::try_from(i).map_err(|_| internal("op index above 2^32"))?);
        }
        Ok(Self { analysis, raw: at })
    }

    pub(crate) fn sites(&self) -> usize {
        self.analysis.sites.len()
    }

    /// `op 123456 (PUSH_CONDITION, depth 2, bit 17)`.
    fn name(&self, raw: &[Op], site: usize) -> String {
        let i = self.raw[site] as usize;
        format!(
            "op {i} ({}, depth {}, bit {})",
            raw[i].kind.name(),
            self.analysis.sites[site].depth,
            raw[i].c_condition
        )
    }
}

/// One directed stratum's engine call, every lane of which passed.
pub(crate) struct Stratum {
    pub lanes: usize,
    /// Per site `[control-1, control-0]` executions.
    pub counts: Vec<[u64; 2]>,
}

fn class_name(c: Class) -> &'static str {
    match c {
        Class::Dead => "proved-unreachable",
        Class::Affine => "affine",
        Class::Opaque => "opaque",
    }
}

/// Applies the acceptance rule to the uniform run's counts `s0`, running a directed stratum
/// (`run`) for each distinct condition among the affine sites the uniform lanes leave short.
///
/// # Errors
/// The reason the run is refused: a site short of [`COVERAGE_MIN`] validated control-1
/// executions, a failing directed lane, too many strata, or a disagreement between the static
/// pass and the engine's counts.
pub(crate) fn settle(
    plan: &Plan,
    raw: &[Op],
    ops: &[SimOp],
    s0: Vec<[u64; 2]>,
    (lanes, control_one): (usize, usize),
    run: &mut dyn FnMut(&System) -> Result<Stratum, String>,
) -> Result<CoverageReport, String> {
    let sites = &plan.analysis.sites;
    let internal = |what: String| format!("outcome-coverage: internal error: {what}; refusing");
    if s0.len() != sites.len() {
        return Err(internal("site counts do not match the static pass".into()));
    }
    let dead_executed = |counts: &[[u64; 2]], whose: &str| {
        (0..sites.len())
            .find(|&i| sites[i].class == Class::Dead && counts[i] != [0, 0])
            .map(|i| {
                internal(format!(
                    "{} is classified unreachable but executed on {} lanes of {whose}",
                    plan.name(raw, i),
                    counts[i][0] + counts[i][1]
                ))
            })
    };
    if let Some(e) = dead_executed(&s0, "the uniform run") {
        return Err(e);
    }
    let live = |i: usize| sites[i].class != Class::Dead;
    let short: Vec<usize> = (0..sites.len())
        .filter(|&i| live(i) && s0[i][0] < COVERAGE_MIN)
        .collect();
    let uniform_min = (0..sites.len())
        .filter(|&i| live(i))
        .map(|i| s0[i][0])
        .min();
    if let Some(&i) = short.iter().find(|&&i| sites[i].class == Class::Opaque) {
        return Err(format!(
            "outcome-coverage: unexercised: {} executed on {} validated control-1 lanes, \
             {COVERAGE_MIN} required; its condition is not a conjunction of affine outcome \
             forms, so no directed lanes can be drawn",
            plan.name(raw, i),
            s0[i][0]
        ));
    }
    let groups = if short.is_empty() {
        Vec::new()
    } else {
        condition::systems(ops, &short, MAX_STRATA).map_err(|condition::TooMany| {
            format!(
                "outcome-coverage: more than {MAX_STRATA} distinct rare outcome conditions need \
                 directed lanes (the cap is {MAX_STRATA} strata of {STRATUM_LANES} lanes); refusing"
            )
        })?
    };
    if groups.iter().map(|g| g.1.len()).sum::<usize>() != short.len() {
        return Err(internal(
            "the static pass did not reproduce its own systems".into(),
        ));
    }
    let mut total = s0;
    let mut directed = vec![false; sites.len()];
    for (system, members) in &groups {
        let op = plan.raw[members[0]];
        let st = run(system)
            .map_err(|why| format!("outcome-coverage (directed lanes for op {op}): {why}"))?;
        if st.counts.len() != sites.len() {
            return Err(internal("site counts do not match the static pass".into()));
        }
        if let Some(e) = dead_executed(&st.counts, &format!("the directed lanes for op {op}")) {
            return Err(e);
        }
        for &i in members {
            let [one, zero] = st.counts[i];
            if one + zero != st.lanes as u64 {
                return Err(internal(format!(
                    "{} executed on {} of the {} lanes directed at its own condition",
                    plan.name(raw, i),
                    one + zero,
                    st.lanes
                )));
            }
            total[i][0] += one;
            total[i][1] += zero;
            directed[i] = true;
            if total[i][0] < COVERAGE_MIN {
                return Err(format!(
                    "outcome-coverage: unexercised: {} executed on {} validated control-1 lanes \
                     with its directed lanes, {COVERAGE_MIN} required",
                    plan.name(raw, i),
                    total[i][0]
                ));
            }
        }
    }
    // Every live site now has COVERAGE_MIN control-1 executions on lanes that passed.
    debug_assert!((0..sites.len()).all(|i| !live(i) || total[i][0] >= COVERAGE_MIN));

    let count = |c: Class| sites.iter().filter(|s| s.class == c).count();
    let mut ranks = vec![0u64; condition::MAX_RANK + 1];
    for s in sites.iter().filter(|s| s.class == Class::Affine) {
        ranks[usize::from(s.rank)] += 1;
    }
    while ranks.len() > 1 && ranks.last() == Some(&0) {
        ranks.pop();
    }
    let max_rank = ranks.len() - 1;
    let mut log2 = std::collections::BTreeMap::<u32, u64>::new();
    let mut table = Vec::with_capacity(sites.len() * 26);
    for (i, s) in sites.iter().enumerate() {
        table.extend_from_slice(&u64::from(plan.raw[i]).to_le_bytes());
        table.push(match s.class {
            Class::Dead => 0,
            Class::Affine => 1,
            Class::Opaque => 2,
        });
        table.push(s.rank);
        table.extend_from_slice(&total[i][0].to_le_bytes());
        table.extend_from_slice(&total[i][1].to_le_bytes());
        if live(i) {
            *log2.entry(total[i][0].max(1).ilog2()).or_default() += 1;
        }
    }
    let mut order: Vec<usize> = (0..sites.len()).filter(|&i| live(i)).collect();
    let min_one = order.iter().map(|&i| total[i][0]).min();
    let min_zero = order.iter().map(|&i| total[i][1]).min();
    order.sort_by_key(|&i| (total[i][0], i));
    let row = |i: usize| {
        let r = plan.raw[i] as usize;
        json!({
            "op": r,
            "kind": raw[r].kind.name(),
            "depth": sites[i].depth,
            "bit": raw[r].c_condition,
            "class": class_name(sites[i].class),
            "rank": (sites[i].class == Class::Affine).then_some(sites[i].rank),
            "control_one_executions": total[i][0],
            "control_zero_executions": total[i][1],
            "source": if directed[i] { "uniform lanes and its directed stratum" } else { "uniform lanes" },
        })
    };
    let unreachable: Vec<u32> = (0..sites.len())
        .filter(|&i| !live(i))
        .map(|i| plan.raw[i])
        .take(8)
        .collect();
    let delta = crate::score::DELTA;
    let json = json!({
        "mode": MODE,
        "document": "spec/SAMPLING.md section 2.3",
        "required_control_one_executions": COVERAGE_MIN,
        "sites": sites.len(),
        "push_blocks": plan.raw.iter().filter(|&&r| raw[r as usize].kind == K::PushCondition).count(),
        "conditioned_ops": plan.raw.iter().filter(|&&r| raw[r as usize].kind != K::PushCondition).count(),
        "hmr_ops": plan.analysis.hmr_ops,
        "static": {
            "affine": count(Class::Affine),
            "opaque": count(Class::Opaque),
            "proved_unreachable": count(Class::Dead),
            "affine_rank_histogram": ranks,
            "max_rank": max_rank,
            "work_cap_reached": plan.analysis.capped,
            "classes": "affine: the site executes exactly on the solutions of a GF(2)-linear system of that rank over the Hmr outcomes, so with probability 2^-rank under uniform outcomes; opaque: the condition is not followed by the static pass; proved_unreachable: the condition is 0 for every outcome vector, so the site never executes and is exempt",
        },
        "uniform": {
            "lanes": lanes,
            "control_one_lanes": control_one,
            "min_control_one_executions": uniform_min,
            "sites_below_requirement": short.len(),
        },
        "directed": {
            "strata": groups.len(),
            "lanes_per_stratum": STRATUM_LANES,
            "total_lanes": groups.len() * STRATUM_LANES,
            "sites_covered": directed.iter().filter(|&&d| d).count(),
            "max_strata": MAX_STRATA,
            "outcomes": "uniform on the solutions of one site condition per stratum; lanes and free outcome bits derive from the run's measurement key",
        },
        "min_control_one_executions": min_one,
        "min_control_zero_executions": min_zero,
        "control_one_executions_log2_histogram": log2
            .iter()
            .map(|(k, v)| json!({ "log2_floor": k, "sites": v }))
            .collect::<Vec<_>>(),
        "rarest": order.iter().take(8).map(|&i| row(i)).collect::<Vec<_>>(),
        "proved_unreachable_ops": unreachable,
        "per_site_bound": min_one.map(|n| (1.0 / delta).ln() / n.max(1) as f64),
        "per_site_bound_meaning": "ln(1/delta) / min_control_one_executions with delta = 1e-6, single draw: for any one site, with confidence 1 - delta, at most this fraction of the control-1 (lane, outcome) pairs that execute the site fail. Per site; no union bound over sites and no allowance for re-drawn seeds",
        "site_table_sha256": hex::encode(Sha256::digest(&table)),
        "site_table_format": "per site in stream order: u64 op index, u8 class (0 proved unreachable, 1 affine, 2 opaque), u8 rank, u64 control-1 executions, u64 control-0 executions, little-endian (eval_circuit --dump-coverage)",
        "statement": format!(
            "Establishes: every conditioned site (each PushCondition block and each op with a condition of its own) that is not proved unreachable executed on at least {COVERAGE_MIN} control-1 lanes that passed every per-lane check. Does not establish: anything about combinations of branch outcomes across sites beyond what uniform sampling gives; a smaller f_bound or implied error bound; anything about unsampled uniform-register values; an energy error. Control-0 executions are reported, not required. Toffoli, qubit and all other counts and bounds come from the uniform lanes only."
        ),
    });
    Ok(CoverageReport {
        sites: sites.len(),
        proved_unreachable: count(Class::Dead),
        affine: count(Class::Affine),
        opaque: count(Class::Opaque),
        directed_strata: groups.len(),
        lanes_per_stratum: STRATUM_LANES,
        min_control_one: min_one,
        min_control_zero: min_zero,
        json,
        table: Arc::new(table),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::Op;

    /// `Hmr` into bit 0, then a block pushed on it holding a `Neg`: one site, affine of rank 1.
    fn one_block() -> (Vec<Op>, Vec<SimOp>) {
        let mut hmr = Op::new(K::Hmr);
        (hmr.q_target, hmr.c_target) = (9, 0);
        let mut push = Op::new(K::PushCondition);
        push.c_condition = 0;
        let raw = vec![hmr, push, Op::new(K::Neg), Op::new(K::PopCondition)];
        let ops = vec![
            SimOp::Hmr {
                t: 9,
                bit: 0,
                cond: NONE,
            },
            SimOp::Push { bit: 0 },
            SimOp::Neg { cond: NONE },
            SimOp::Pop,
        ];
        (raw, ops)
    }

    fn never(_: &System) -> Result<Stratum, String> {
        panic!("no stratum expected")
    }

    #[test]
    fn enough_uniform_executions_pass_without_strata() {
        let (raw, ops) = one_block();
        let plan = Plan::new(&raw, &ops).unwrap();
        assert_eq!(plan.analysis.sites[0].class, Class::Affine);
        let r = settle(&plan, &raw, &ops, vec![[300, 290]], (1200, 600), &mut never).unwrap();
        assert_eq!((r.sites, r.directed_strata), (1, 0));
        assert_eq!(r.min_control_one, Some(300));
        assert_eq!(r.table.len(), 26);
        assert_eq!(r.json["rarest"][0]["op"], 1);
    }

    #[test]
    fn a_short_site_gets_its_own_stratum_and_must_reach_the_floor() {
        let (raw, ops) = one_block();
        let plan = Plan::new(&raw, &ops).unwrap();
        let mut calls = 0;
        let mut run = |s: &System| {
            calls += 1;
            assert_eq!(s.len(), 1);
            Ok(Stratum {
                lanes: 1024,
                counts: vec![[500, 524]],
            })
        };
        let r = settle(&plan, &raw, &ops, vec![[10, 12]], (40, 20), &mut run).unwrap();
        assert_eq!((calls, r.directed_strata), (1, 1));
        assert_eq!(r.min_control_one, Some(510));
        // Too few control-1 lanes even with the stratum.
        let mut thin = |_: &System| {
            Ok(Stratum {
                lanes: 1024,
                counts: vec![[100, 924]],
            })
        };
        let e = settle(&plan, &raw, &ops, vec![[10, 12]], (40, 20), &mut thin).unwrap_err();
        assert!(e.contains("unexercised") && e.contains("op 1 "), "{e}");
        // A failing directed lane names the block.
        let mut bad = |_: &System| Err("wrong-phase: lane #3".to_string());
        let e = settle(&plan, &raw, &ops, vec![[10, 12]], (40, 20), &mut bad).unwrap_err();
        assert!(
            e.starts_with("outcome-coverage (directed lanes for op 1): wrong-phase"),
            "{e}"
        );
    }

    /// The guards on the static pass: a wrong classification refuses, it never certifies.
    #[test]
    fn a_wrong_static_classification_is_refused() {
        let (raw, ops) = one_block();
        // Classified unreachable, but the engine saw it execute.
        let mut plan = Plan::new(&raw, &ops).unwrap();
        plan.analysis.sites[0].class = Class::Dead;
        let e = settle(&plan, &raw, &ops, vec![[3, 1]], (8, 4), &mut never).unwrap_err();
        assert!(
            e.contains("internal error") && e.contains("classified unreachable"),
            "{e}"
        );
        // Directed at a system the site does not execute on.
        let plan = Plan::new(&raw, &ops).unwrap();
        let mut off = |_: &System| {
            Ok(Stratum {
                lanes: 1024,
                counts: vec![[400, 300]],
            })
        };
        let e = settle(&plan, &raw, &ops, vec![[3, 1]], (8, 4), &mut off).unwrap_err();
        assert!(
            e.contains("internal error") && e.contains("700 of the 1024 lanes"),
            "{e}"
        );
        // Classified opaque and short: refused, with no stratum.
        let mut plan = Plan::new(&raw, &ops).unwrap();
        plan.analysis.sites[0].class = Class::Opaque;
        let e = settle(&plan, &raw, &ops, vec![[3, 1]], (8, 4), &mut never).unwrap_err();
        assert!(
            e.contains("unexercised") && e.contains("no directed lanes"),
            "{e}"
        );
    }
}
