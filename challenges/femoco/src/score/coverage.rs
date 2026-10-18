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
//! **What a site is not.** A site is a classical condition. An outcome can also be copied into a
//! qubit by a conditioned write and then act as a quantum control, with no condition on the op
//! it steers; such an op is not a site, and a block reached that way on a rare outcome pattern
//! leaves every site well exercised. `sim::condition::qubit_outcomes` finds the ops that read a
//! qubit written under a classical condition. The mode ([`Scope::Full`]) refuses a stream that
//! has one; [`Scope::SitesOnly`] applies the per-site rule alone and reports them, and its
//! statement says that they are not covered.
//!
//! Directed lanes are used for rejection and for these counts only. Their tallies are dropped,
//! so every cost figure and every bound of `score.json` is that of the uniform run.
use crate::circuit::{Op, OperationType as K, NONE};
use crate::sim::compile::SimOp;
use crate::sim::condition::{self, Analysis, Class, QubitOutcomes, System};
pub use crate::sim::condition::{COVERAGE_MIN, MAX_STRATA, STRATUM_LANES};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// `metrics.outcome_coverage.mode`. `v2` added the refusal of outcomes read from qubits and the
/// `scope` and `qubit_outcomes` keys.
pub const MODE: &str = "femoco-outcome-coverage-v2";

/// How much of the mode runs (`eval_circuit --outcome-coverage[=sites]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// The mode does not run and no output changes.
    #[default]
    Off,
    /// The per-site rule only. Ops that read a qubit written under a classical condition are
    /// reported, not refused; the run's statement says they are not covered.
    SitesOnly,
    /// The per-site rule, and a refusal when any op reads a qubit written under a classical
    /// condition.
    Full,
}

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
    /// Ops that write a non-system qubit under a classical condition, and ops that read a qubit
    /// so written (`sim::condition::qubit_outcomes`). The second is 0 on every run with
    /// [`Scope::Full`].
    pub conditioned_qubit_writes: u64,
    pub qubit_outcome_reads: u64,
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
    scope: Scope,
    qubits: QubitOutcomes,
    /// The first reads of `qubits`: the reading op in the raw stream (`None`: the end of the
    /// stream), the qubit, and the raw op its mark goes back to.
    reads: Vec<(Option<usize>, u32, usize)>,
}

impl Plan {
    /// Runs the static passes over the lowered stream `ops` and ties each site to its op in the
    /// raw stream `raw` (for messages and the report only). `registers` are the compiled
    /// registers and `inner` the inner uniform qubits of a nested lane map (empty otherwise).
    pub(crate) fn new(
        raw: &[Op],
        ops: &[SimOp],
        registers: &[Vec<u32>],
        inner: &[u32],
        scope: Scope,
    ) -> Result<Self, String> {
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
        let qubits = condition::qubit_outcomes(ops, registers, inner);
        let reads = qubits
            .first
            .iter()
            .map(|r| {
                let op = (r.op != u32::MAX).then(|| kept[r.op as usize]);
                (op, r.qubit, kept[r.source as usize])
            })
            .collect();
        Ok(Self {
            analysis,
            raw: at,
            scope,
            qubits,
            reads,
        })
    }

    /// The refusal of [`Scope::Full`] when an op reads a qubit written under a classical
    /// condition; `None` when there is no such op or the scope is sites only.
    fn outcome_in_qubit(&self, raw: &[Op]) -> Option<String> {
        let &(op, qubit, source) = self.reads.first()?;
        if self.scope != Scope::Full {
            return None;
        }
        let reader = match op {
            Some(i) => format!("op {i} ({}) reads", raw[i].kind.name()),
            None => "the checks at the end of the stream read".to_string(),
        };
        Some(format!(
            "outcome-coverage: outcome-in-qubit: {reader} qubit q{qubit}, which may hold a \
             measurement outcome since op {source} ({}) wrote a qubit under a classical \
             condition; {} such read(s) in all. An op steered by an outcome through a qubit is \
             not a conditioned site, so the per-site counts cannot show that it was exercised; \
             refusing (--outcome-coverage=sites applies the per-site rule alone and reports \
             these reads as not covered)",
            raw[source].kind.name(),
            self.qubits.reads
        ))
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
    // Before any stratum: a stream the mode cannot cover is refused whatever its sites show.
    if let Some(e) = plan.outcome_in_qubit(raw) {
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
        condition::systems(ops, &short, MAX_STRATA).map_err(|why| match why {
            condition::Unplanned::TooMany => format!(
                "outcome-coverage: more than {MAX_STRATA} distinct rare outcome conditions need \
                 directed lanes (the cap is {MAX_STRATA} strata of {STRATUM_LANES} lanes); refusing"
            ),
            condition::Unplanned::TooLarge => "outcome-coverage: the rare outcome conditions \
                 that need directed lanes are together larger than the static pass's memory \
                 budget; refusing"
                .to_string(),
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
    let q = &plan.qubits;
    // `Scope::Full` returned above unless there is no read, so a run that reaches this point
    // with reads is sites only.
    let uncovered = q.reads > 0;
    let json = json!({
        "mode": MODE,
        "scope": if plan.scope == Scope::Full { "sites-and-qubit-outcomes" } else { "sites-only" },
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
            "memory_cap_reached": plan.analysis.memory_capped,
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
        "qubit_outcomes": {
            "conditioned_qubit_writes": q.conditioned_writes,
            "reads": q.reads,
            "first_reads": plan.reads.iter().map(|&(op, qubit, source)| json!({
                "op": op,
                "kind": op.map(|i| raw[i].kind.name()),
                "qubit": qubit,
                "written_under_condition_at_op": source,
            })).collect::<Vec<_>>(),
            "covered": !uncovered,
            "rule": "a non-system qubit written while a classical condition is in force (X, CX, CCX, Swap, Hmr) may hold a function of the outcomes; the mark spreads through the controls of X-type gates and moves with Swap, and only an unconditioned Hmr removes it. reads: ops with such a qubit as a control, a phase operand, an angle bit, a SpinSwap control or the qubit measured or freed, plus each qubit still marked at the end of the stream (op null). With no reads, every qubit an op reads is a function of the lane alone for every outcome record. A read is not a finding that the circuit is wrong",
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
        "statement": statement(q.reads),
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
        conditioned_qubit_writes: q.conditioned_writes,
        qubit_outcome_reads: q.reads,
        json,
        table: Arc::new(table),
    })
}

/// `metrics.outcome_coverage.statement` of a passing run in which `reads` ops read a qubit
/// written under a classical condition (0 on every run with [`Scope::Full`]).
fn statement(reads: u64) -> String {
    let sites = format!(
        "every classical-condition site (each PushCondition block and each op with a condition \
         of its own) that is not proved unreachable executed on at least {COVERAGE_MIN} \
         control-1 lanes that passed every per-lane check"
    );
    let (qubits, not_covered) = if reads == 0 {
        (
            "; and no op reads a qubit written under a classical condition, so on every lane \
             and for every outcome record each qubit an op reads is a function of the lane \
             alone, and an outcome acts on the step only through the conditions of the counted \
             sites and the signs of the measurements themselves"
                .to_string(),
            String::new(),
        )
    } else {
        (
            String::new(),
            format!(
                " anything about the {reads} op(s) that read a qubit written under a classical \
                 condition (qubit_outcomes): an outcome copied into a qubit steers them as a \
                 quantum control, they are not sites, and a block reached that way only on a \
                 rare outcome pattern is not exercised by this mode;"
            ),
        )
    };
    format!(
        "Establishes: {sites}{qubits}. Does not establish:{not_covered} anything about \
         combinations of branch outcomes across sites beyond what uniform sampling gives; a \
         smaller f_bound or implied error bound; anything about unsampled uniform-register \
         values; an energy error. Control-0 executions are reported, not required. Toffoli, \
         qubit and all other counts and bounds come from the uniform lanes only."
    )
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
        let plan = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
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
        let plan = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
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

    /// `one_block` followed by a `Neg` conditioned on a bit that is stored 0: a second site,
    /// proved unreachable.
    fn block_and_dead_site() -> (Vec<Op>, Vec<SimOp>) {
        let (mut raw, mut ops) = one_block();
        let mut store = Op::new(K::BitStore0);
        store.c_target = 1;
        let mut neg = Op::new(K::Neg);
        neg.c_condition = 1;
        raw.extend([store, neg]);
        ops.extend([
            SimOp::Bit {
                bit: 1,
                kind: 1,
                cond: NONE,
            },
            SimOp::Neg { cond: 1 },
        ]);
        (raw, ops)
    }

    /// The guard on directed lanes: a site classified unreachable that executes on a lane of
    /// another site's stratum refuses the run, as it does on the uniform lanes.
    #[test]
    fn an_unreachable_site_that_executes_in_a_stratum_is_refused() {
        let (raw, ops) = block_and_dead_site();
        let plan = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
        let classes: Vec<Class> = plan.analysis.sites.iter().map(|s| s.class).collect();
        assert_eq!(classes, [Class::Affine, Class::Dead]);
        let stratum = |dead: [u64; 2]| {
            move |_: &System| -> Result<Stratum, String> {
                Ok(Stratum {
                    lanes: 1024,
                    counts: vec![[500, 524], dead],
                })
            }
        };
        let uniform = vec![[10, 12], [0, 0]];
        let r = settle(
            &plan,
            &raw,
            &ops,
            uniform.clone(),
            (40, 20),
            &mut stratum([0, 0]),
        )
        .unwrap();
        assert_eq!((r.directed_strata, r.proved_unreachable), (1, 1));
        for dead in [[1, 0], [0, 3]] {
            let e = settle(
                &plan,
                &raw,
                &ops,
                uniform.clone(),
                (40, 20),
                &mut stratum(dead),
            )
            .unwrap_err();
            assert!(
                e.contains("internal error")
                    && e.contains("op 5 (NEG, depth 1, bit 1) is classified unreachable")
                    && e.contains("the directed lanes for op 1"),
                "{e}"
            );
        }
    }

    /// A stream in which an op reads a qubit written under a classical condition: refused in
    /// the full scope before any stratum runs; passed with the reads reported, and the
    /// statement narrowed, in the sites-only scope.
    #[test]
    fn an_outcome_read_from_a_qubit_is_refused_or_reported() {
        let (mut raw, mut ops) = one_block();
        let mut x = Op::new(K::X);
        (x.q_target, x.c_condition) = (20, 0);
        let mut cz = Op::new(K::CZ);
        (cz.q_target, cz.q_control1) = (20, 21);
        raw.extend([x, cz, x]);
        let x_if = SimOp::X { t: 20, cond: 0 };
        ops.extend([
            x_if,
            SimOp::Phase {
                q: [20, 21, NONE],
                k: 4,
                cond: NONE,
            },
            x_if,
        ]);
        let counts = vec![[300, 290]; 3];
        let full = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
        let e = settle(&full, &raw, &ops, counts.clone(), (1200, 600), &mut never).unwrap_err();
        assert!(
            e.starts_with(
                "outcome-coverage: outcome-in-qubit: op 5 (CZ) reads qubit q20, which may hold \
                 a measurement outcome since op 4 (X) wrote a qubit under a classical condition; \
                 2 such read(s) in all."
            ),
            "{e}"
        );
        // The refusal comes before a stratum is drawn.
        let short = vec![[10, 12], [300, 290], [300, 290]];
        let e = settle(&full, &raw, &ops, short, (1200, 600), &mut never).unwrap_err();
        assert!(e.contains("outcome-in-qubit"), "{e}");
        let sites = Plan::new(&raw, &ops, &[], &[], Scope::SitesOnly).unwrap();
        let r = settle(&sites, &raw, &ops, counts, (1200, 600), &mut never).unwrap();
        assert_eq!((r.conditioned_qubit_writes, r.qubit_outcome_reads), (2, 2));
        let j = &r.json;
        assert_eq!(j["scope"], "sites-only");
        assert_eq!(j["qubit_outcomes"]["covered"], false);
        assert_eq!(j["qubit_outcomes"]["first_reads"][0]["op"], 5);
        assert_eq!(
            j["qubit_outcomes"]["first_reads"][0]["written_under_condition_at_op"],
            4
        );
        // The qubit is still marked at the end: read by the end-of-stream checks.
        assert!(j["qubit_outcomes"]["first_reads"][1]["op"].is_null());
        let s = j["statement"].as_str().unwrap();
        assert!(
            s.contains("Does not establish: anything about the 2 op(s) that read a qubit")
                && !s.contains("no op reads a qubit"),
            "{s}"
        );
        // Without such a read the statement makes the stronger claim, in either scope.
        let (raw, ops) = one_block();
        for scope in [Scope::Full, Scope::SitesOnly] {
            let plan = Plan::new(&raw, &ops, &[], &[], scope).unwrap();
            let r = settle(&plan, &raw, &ops, vec![[300, 290]], (1200, 600), &mut never).unwrap();
            let s = r.json["statement"].as_str().unwrap();
            assert!(s.contains("no op reads a qubit written under a classical condition"));
            assert_eq!(r.json["qubit_outcomes"]["covered"], true);
            assert_eq!(r.json["mode"], "femoco-outcome-coverage-v2");
        }
    }

    /// The guards on the static pass: a wrong classification refuses, it never certifies.
    #[test]
    fn a_wrong_static_classification_is_refused() {
        let (raw, ops) = one_block();
        // Classified unreachable, but the engine saw it execute.
        let mut plan = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
        plan.analysis.sites[0].class = Class::Dead;
        let e = settle(&plan, &raw, &ops, vec![[3, 1]], (8, 4), &mut never).unwrap_err();
        assert!(
            e.contains("internal error") && e.contains("classified unreachable"),
            "{e}"
        );
        // Directed at a system the site does not execute on.
        let plan = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
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
        let mut plan = Plan::new(&raw, &ops, &[], &[], Scope::Full).unwrap();
        plan.analysis.sites[0].class = Class::Opaque;
        let e = settle(&plan, &raw, &ops, vec![[3, 1]], (8, 4), &mut never).unwrap_err();
        assert!(
            e.contains("unexercised") && e.contains("no directed lanes"),
            "{e}"
        );
    }
}
