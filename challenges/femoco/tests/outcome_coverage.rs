//! The outcome-coverage mode (`eval_circuit --outcome-coverage`; spec/SAMPLING.md section 2.3,
//! spec/DESIGN.md section 9) on the harness's toy spec: a block conditioned on a rare outcome
//! pattern is accepted by uniform sampling and rejected by the mode; a block whose condition
//! the static pass cannot follow is refused, not passed; honest circuits pass with the uniform
//! run's metrics untouched; and nothing changes without the flag.
mod harness_common;

use femoco_walk::circuit::{Op, OperationType as K, NONE};
use femoco_walk::equiv;
use femoco_walk::fastsim;
use femoco_walk::fiat_shamir::{self, AuditBeacon, Lane};
use femoco_walk::lanemap::LaneMap;
use femoco_walk::score::coverage::{COVERAGE_MIN, MAX_STRATA, STRATUM_LANES};
use femoco_walk::score::{self, score_json, Evaluation, Inputs, Options};
use femoco_walk::sim::validate::{self, Context, Engine, Nested, Outcome};
use femoco_walk::spec::EncodingSpec;
use harness_common::{alias, build, declared_only, family, ops_file, Mutation, TestSpec, N};
use serde_json::Value;

/// Lanes of the quick tier.
const K_LANES: usize = 4096;
/// First qubit and bit ids the appended blocks use (above anything the toy circuit names).
const Q0: u32 = 4000;
const B0: u32 = 4000;

fn op(kind: K) -> Op {
    Op::new(kind)
}

fn hmr(q: u32, bit: u32) -> Op {
    let mut o = op(K::Hmr);
    (o.q_target, o.c_target) = (q, bit);
    o
}

fn push(bit: u32) -> Op {
    let mut o = op(K::PushCondition);
    o.c_condition = bit;
    o
}

fn pop() -> Op {
    op(K::PopCondition)
}

fn invert(bit: u32, cond: u32) -> Op {
    let mut o = op(K::BitInvert);
    (o.c_target, o.c_condition) = (bit, cond);
    o
}

/// What sits inside a conditioned block.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Body {
    /// Two `Neg`s: the identity.
    Honest,
    /// `Z` on the walk control: a sign error on every control-1 lane (`wrong-phase`).
    Sign,
    /// `CZ(control, system 0)`: a wrong operator on every control-1 lane (`wrong-term`).
    Term,
}

impl Body {
    /// The body's ops; `cond` goes on the wrong op itself (`NONE`: unconditioned).
    fn ops(self, cond: u32) -> Vec<Op> {
        let mut o = match self {
            Self::Honest => return vec![op(K::Neg), op(K::Neg)],
            Self::Sign => op(K::Z),
            Self::Term => op(K::CZ),
        };
        o.q_target = 0;
        if self == Self::Term {
            (o.q_target, o.q_control1) = (1, 0);
        }
        o.c_condition = cond;
        vec![o]
    }

    fn category(self) -> &'static str {
        match self {
            Self::Sign => "wrong-phase",
            _ => "wrong-term",
        }
    }
}

/// The toy circuit, correct.
fn toy() -> Vec<Op> {
    build(&TestSpec::new(), &alias(), Mutation::None)
}

/// `n` fresh ancillas measured into bits `B0 .. B0 + n` (each outcome uniform, no phase: the
/// ancillas are 0). The first `inverted` bits are then inverted unconditionally.
fn measured_bits(n: u32, inverted: u32) -> Vec<Op> {
    let mut ops: Vec<Op> = (0..n).map(|i| hmr(Q0 + i, B0 + i)).collect();
    ops.extend((0..inverted).map(|i| invert(B0 + i, NONE)));
    ops
}

/// The toy circuit followed by a block nested under `n` measured bits: reached on a fraction
/// `2^-n` of lanes under uniform outcomes. With `own`, the innermost bit is the wrong op's own
/// condition instead of a `Push`.
fn conjunction(n: u32, inverted: u32, own: bool, body: Body) -> Vec<Op> {
    let mut ops = toy();
    ops.extend(measured_bits(n, inverted));
    let pushes = if own { n - 1 } else { n };
    ops.extend((0..pushes).map(|i| push(B0 + i)));
    ops.extend(body.ops(if own { B0 + n - 1 } else { NONE }));
    ops.extend((0..pushes).map(|_| pop()));
    ops
}

fn eval_on(
    ops: &[Op],
    samples: usize,
    engine: Engine,
    coverage: bool,
) -> Result<Evaluation, String> {
    let spec = TestSpec::new();
    let file = ops_file(ops);
    let lm = alias().to_bytes();
    let fam = family(spec.id());
    let inp = Inputs {
        spec: &spec,
        lanemap: &lm,
        family: &fam,
        ops: &file,
        samples,
        tracker: None,
        check: &declared_only,
    };
    let opts = Options {
        outcome_coverage: coverage,
    };
    score::evaluate_opts(&inp, engine, None, opts)
}

fn plain(ops: &[Op]) -> Result<Evaluation, String> {
    eval_on(ops, K_LANES, validate::reference, false)
}

fn covered(ops: &[Op]) -> Result<Evaluation, String> {
    eval_on(ops, K_LANES, validate::reference, true)
}

fn sliced() -> Engine {
    equiv::engine("sliced").unwrap()
}

/// The index in `ops` of the last op of `kind`.
fn last(ops: &[Op], kind: K) -> usize {
    ops.iter().rposition(|o| o.kind == kind).unwrap()
}

/// `score.json` without the wall clock; with `strip`, also without the mode's own block and its
/// pointer sentence, which is all the mode may add.
fn body(ev: &Evaluation, strip: bool) -> Value {
    let mut v = score_json(ev);
    let m = v["metrics"].as_object_mut().unwrap();
    m.remove("eval_seconds");
    if strip {
        assert!(m.remove("outcome_coverage").is_some());
        let s = m["sampling_limits"].as_object_mut().unwrap();
        let pointer = s.insert(
            "measurement_outcomes".into(),
            "drawn uniformly per lane: a block conditioned on a rare outcome pattern is tested on few or no lanes".into(),
        );
        assert!(pointer
            .unwrap()
            .as_str()
            .unwrap()
            .contains("metrics.outcome_coverage"));
    }
    v
}

/// (a) A wrong block behind a 24-bit conjunction of outcomes: uniform sampling at the quick tier
/// never reaches it and accepts; the mode draws lanes that reach it and rejects, naming the op.
#[test]
fn rare_conjunction_mutant_is_accepted_by_uniform_sampling_and_rejected_by_the_mode() {
    for (inverted, own, wrong) in [
        (0, false, Body::Sign),
        (0, false, Body::Term),
        // Half the bits inverted: an all-ones (or all-zeros) outcome stratum would miss it.
        (12, false, Body::Term),
        // The innermost bit as the wrong op's own condition.
        (0, true, Body::Sign),
        (12, true, Body::Term),
    ] {
        let ops = conjunction(24, inverted, own, wrong);
        let what = format!("{wrong:?}, {inverted} inverted, own condition {own}");
        plain(&ops).unwrap_or_else(|e| panic!("{what}: uniform sampling rejected: {e}"));
        // The failing lane is found in the stratum of the wrong op's own site or of a block
        // that encloses it (a lane directed at depth d reaches depth 24 with probability
        // 2^-(24 - d)); the message names that stratum's site.
        let sites = last(&ops, K::PushCondition) - 22..ops.len();
        let mut seen = Vec::new();
        for engine in [validate::reference as Engine, sliced()] {
            let e = eval_on(&ops, K_LANES, engine, true).unwrap_err();
            let named = e
                .strip_prefix("outcome-coverage (directed lanes for op ")
                .and_then(|r| r.split_once("): "))
                .and_then(|(i, _)| i.parse::<usize>().ok());
            assert!(
                named.is_some_and(|i| sites.contains(&i)) && e.contains(wrong.category()),
                "{what}: {e}"
            );
            seen.push(e);
        }
        assert_eq!(seen[0], seen[1], "{what}: the engines differ");
    }
}

/// (b) The same block with correct content passes: the rare sites get directed strata, every
/// live site reaches the floor, and every figure of the uniform run is exactly the no-flag run's.
#[test]
fn honest_rare_block_passes_with_the_uniform_metrics_unchanged() {
    for (inverted, own) in [(0, false), (12, false), (5, true)] {
        let mut ops = conjunction(24, inverted, own, Body::Honest);
        if own {
            // `Honest` carries no condition of its own; add a conditioned identity pair.
            let at = last(&ops, K::Neg) + 1;
            let mut neg = op(K::Neg);
            neg.c_condition = B0 + 23;
            ops.splice(at..at, [neg, neg]);
        }
        let base = plain(&ops).unwrap();
        assert!(base.outcome_coverage.is_none());
        let ev = covered(&ops).unwrap();
        let c = ev.outcome_coverage.as_ref().unwrap();
        // Ranks 4 to 24 cannot reach 256 control-1 executions on 4,096 uniform lanes.
        assert!(c.directed_strata >= 21, "{} strata", c.directed_strata);
        assert_eq!(c.lanes_per_stratum, STRATUM_LANES);
        assert!(c.min_control_one.unwrap() >= COVERAGE_MIN);
        assert_eq!(c.proved_unreachable, 0);
        assert_eq!(c.table.len(), 26 * c.sites);
        assert_eq!(
            (
                ev.toffoli,
                ev.qubits,
                ev.cliffords,
                ev.measurements,
                ev.resets
            ),
            (
                base.toffoli,
                base.qubits,
                base.cliffords,
                base.measurements,
                base.resets
            )
        );
        assert_eq!(
            (ev.samples, ev.control_one_samples),
            (base.samples, base.control_one_samples)
        );
        assert_eq!(body(&ev, true), body(&base, false));
        let j = &score_json(&ev)["metrics"]["outcome_coverage"];
        assert_eq!(j["mode"], "femoco-outcome-coverage-v1");
        assert_eq!(j["required_control_one_executions"], COVERAGE_MIN);
        assert_eq!(j["directed"]["strata"], c.directed_strata);
        assert_eq!(j["directed"]["lanes_per_stratum"], STRATUM_LANES);
        assert_eq!(j["static"]["max_rank"], 24);
        assert_eq!(j["sites"], c.sites);
        assert_eq!(j["uniform"]["lanes"], K_LANES);
        assert!(j["statement"]
            .as_str()
            .unwrap()
            .contains("Does not establish"));
        assert_eq!(
            score_json(&ev)["metrics"]["sampling_limits"]["certifies_energy_error"],
            false
        );
    }
}

/// The toy circuit alone: its 64 `cz_if` sites run on about half the control-1 lanes, so the
/// uniform lanes suffice and no stratum is drawn.
#[test]
fn common_branches_need_no_directed_lanes() {
    let ev = covered(&toy()).unwrap();
    let c = ev.outcome_coverage.unwrap();
    assert_eq!(
        (c.sites, c.affine, c.opaque, c.proved_unreachable),
        (64, 64, 0, 0)
    );
    assert_eq!(c.directed_strata, 0);
    assert!(c.min_control_one.unwrap() > 800 && c.min_control_zero.unwrap() > 800);
    // An existing mutant is still rejected with its ordinary message.
    let bad = build(&TestSpec::new(), &alias(), Mutation::FlipSign(1));
    assert_eq!(covered(&bad).unwrap_err(), plain(&bad).unwrap_err());
}

/// `t ^= m1 & m2`, twice: `t` is 0 for every outcome, but a product of outcomes is not an affine
/// form, so the static pass does not follow it.
fn untracked_zero(t: u32) -> Vec<Op> {
    let mut ops = measured_bits(2, 0);
    for _ in 0..2 {
        ops.extend([push(B0), invert(t, B0 + 1), pop()]);
    }
    ops
}

/// (c) A block that never executes and is not proved unreachable is refused, not passed.
#[test]
fn unexercised_block_that_is_not_proved_unreachable_is_refused() {
    let t = B0 + 100;
    let mut ops = toy();
    ops.extend(untracked_zero(t));
    ops.push(push(t));
    ops.extend(Body::Sign.ops(NONE));
    ops.push(pop());
    plain(&ops).unwrap();
    let site = last(&ops, K::PushCondition);
    for engine in [validate::reference as Engine, sliced()] {
        let e = eval_on(&ops, K_LANES, engine, true).unwrap_err();
        assert!(
            e.starts_with(&format!(
                "outcome-coverage: unexercised: op {site} (PUSH_CONDITION, depth 1, bit {t}) \
                 executed on 0 validated control-1 lanes, 256 required"
            )) && e.contains("no directed lanes can be drawn"),
            "{e}"
        );
    }
}

/// (c) A deep conjunction of bits the static pass does not follow: too rare for the uniform
/// lanes, and no directed lanes can be drawn, so the run is refused whatever the block holds.
#[test]
fn rare_conjunction_of_untracked_bits_is_refused() {
    for wrong in [Body::Honest, Body::Term] {
        let mut ops = toy();
        // o_i = (m & m') ^ (m'' & m'''), 1 on 3/8 of outcomes.
        let n = 12;
        for i in 0..n {
            let (q, b, o) = (Q0 + 4 * i, B0 + 4 * i, B0 + 1000 + i);
            ops.extend((0..4).map(|j| hmr(q + j, b + j)));
            ops.extend([push(b), invert(o, b + 1), pop()]);
            ops.extend([push(b + 2), invert(o, b + 3), pop()]);
        }
        ops.extend((0..n).map(|i| push(B0 + 1000 + i)));
        ops.extend(wrong.ops(NONE));
        ops.extend((0..n).map(|_| pop()));
        plain(&ops).unwrap();
        let e = covered(&ops).unwrap_err();
        assert!(
            e.starts_with("outcome-coverage: unexercised: op ") && e.contains("PUSH_CONDITION"),
            "{e}"
        );
    }
}

/// (d) Blocks the static pass proves unreachable are exempt, counted, and never executed.
#[test]
fn proved_unreachable_blocks_are_exempt() {
    let (b, m) = (B0 + 50, B0);
    let mut ops = toy();
    // BitStore0(b), then a wrong op conditioned on b.
    let mut store = op(K::BitStore0);
    store.c_target = b;
    ops.push(store);
    ops.extend(Body::Sign.ops(b));
    let first = ops.len() - 1;
    // b ^= m; b ^= m: the two inversions cancel, so b is 0 again.
    ops.push(hmr(Q0, m));
    ops.extend([invert(b, m), invert(b, m)]);
    ops.extend(Body::Term.ops(b));
    let second = ops.len() - 1;
    for engine in [validate::reference as Engine, sliced()] {
        let ev = eval_on(&ops, K_LANES, engine, true).unwrap();
        let c = ev.outcome_coverage.as_ref().unwrap();
        assert_eq!((c.proved_unreachable, c.directed_strata), (2, 0));
        let j = &score_json(&ev)["metrics"]["outcome_coverage"];
        assert_eq!(
            j["proved_unreachable_ops"],
            serde_json::json!([first, second])
        );
        // Their rows of the site table: class 0, no executions.
        let dead: Vec<&[u8]> = c.table.chunks(26).filter(|r| r[8] == 0).collect();
        assert_eq!(dead.len(), 2);
        for (r, want) in dead.iter().zip([first, second]) {
            assert_eq!(u64::from_le_bytes(r[..8].try_into().unwrap()), want as u64);
            assert!(r[10..].iter().all(|&x| x == 0));
        }
    }
}

fn width<const W: usize>(
    c: &Context<'_>,
    n: Option<&Nested<'_>>,
    l: &[Lane],
    v: Option<&mut [u8]>,
) -> Outcome {
    fastsim::run_width(c, n, l, W, v)
}

/// (g) The mode's result does not depend on the engine, the thread count or the sliced engine's
/// pass width, and it follows the seed.
#[test]
fn coverage_is_deterministic_across_engines_threads_and_pass_widths() {
    let ops = conjunction(24, 7, false, Body::Honest);
    let run = |engine: Engine, threads: usize| {
        let ev = equiv::on_threads(threads, || eval_on(&ops, K_LANES, engine, true)).unwrap();
        let table = ev.outcome_coverage.as_ref().unwrap().table.clone();
        (body(&ev, false).to_string(), table)
    };
    let want = run(validate::reference, 4);
    let engines: [(&str, Engine); 6] = [
        ("reference", validate::reference),
        ("sliced", sliced()),
        ("sliced, 1 batch per pass", width::<1>),
        ("sliced, 2 batches per pass", width::<2>),
        ("sliced, 4 batches per pass", width::<4>),
        ("sliced, 8 batches per pass", width::<8>),
    ];
    for (name, engine) in engines {
        for threads in [1, 4] {
            let got = run(engine, threads);
            assert_eq!(got.1, want.1, "{name} at {threads} threads: site table");
            assert_eq!(got.0, want.0, "{name} at {threads} threads: score.json");
        }
    }
    // A server seed moves the directed lanes (and so the counts); the same seed repeats them.
    let seeded = |byte: u8| {
        let beacon = AuditBeacon {
            round: 0,
            randomness: [byte; 32],
        };
        fiat_shamir::with_audit(beacon, || run(sliced(), 0)).1
    };
    let (a, b) = (seeded(1), seeded(2));
    assert_ne!(a, b);
    assert_ne!(a, want.1);
    assert_eq!(a, seeded(1));
}

/// (g) The strata derive from the run's measurement key and the site's condition, nothing else.
#[test]
fn stratum_lanes_follow_the_measurement_key_and_the_condition() {
    let lanes =
        |key: u8, sid: u8| fiat_shamir::coverage_lanes(&[key; 32], &[sid; 32], 6, None, 64).0;
    assert_eq!(lanes(1, 1), lanes(1, 1));
    assert_ne!(lanes(1, 1), lanes(2, 1));
    assert_ne!(lanes(1, 1), lanes(1, 2));
    assert!(lanes(1, 1).iter().all(|l| l.s < 64));
    assert_ne!(
        fiat_shamir::coverage_key(&[1; 32], &[1; 32]),
        fiat_shamir::coverage_key(&[1; 32], &[2; 32])
    );
    // The nested form keeps the lanes and adds the second-pass values, as the main sample does.
    let (l, after) = fiat_shamir::coverage_lanes(&[1; 32], &[1; 32], 6, Some((2, 3)), 64);
    assert_eq!(l, lanes(1, 1));
    let after = after.unwrap();
    assert!(l
        .iter()
        .zip(&after)
        .all(|(l, a)| l.s & !0b11100 == a & !0b11100));
    assert!(l.iter().zip(&after).any(|(l, a)| l.s != *a));
    assert_ne!(
        fiat_shamir::coverage_stratum_id(b"a"),
        fiat_shamir::coverage_stratum_id(b"b")
    );
}

/// (h) Without the flag nothing changes: `evaluate_opts` with the default options is
/// `evaluate`, and the toy circuit's `score.json` is the one pinned here before the mode existed.
#[test]
fn flag_absent_output_is_unchanged() {
    let ops = toy();
    let a = harness_common::eval_with(&ops, &alias(), K_LANES, None).unwrap();
    let b = eval_on(&ops, K_LANES, validate::reference, false).unwrap();
    assert!(a.outcome_coverage.is_none() && b.outcome_coverage.is_none());
    assert_eq!(body(&a, false), body(&b, false));
    let text = serde_json::to_string(&body(&a, false)).unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(
        hex::encode(Sha256::digest(text.as_bytes())),
        GOLDEN,
        "{text}"
    );
    assert_eq!(N, 6);
}

/// SHA-256 of the toy circuit's `score.json` (wall clock removed) at 4,096 lanes, computed at
/// the commit before the outcome-coverage mode (6190b2e) with the same serialization.
const GOLDEN: &str = "2684f06b451c8efb1e21731536a9546954eab09b30040c6a71bc9f956ce06a87";

/// (i) More distinct rare conditions than the cap: refused, stating the cap.
#[test]
fn too_many_rare_conditions_are_refused() {
    let mut ops = toy();
    // 12 measured bits; every 4-subset as its own nested block: 495 distinct rank-4 systems and
    // every 5-subset: 792 distinct rank-5 systems, 1,287 in all.
    ops.extend(measured_bits(12, 0));
    for size in [4u32, 5] {
        for mask in (0u32..1 << 12).filter(|m| m.count_ones() == size) {
            let bits: Vec<u32> = (0..12).filter(|i| mask >> i & 1 == 1).collect();
            ops.extend(bits.iter().map(|&i| push(B0 + i)));
            ops.extend(bits.iter().map(|_| pop()));
        }
    }
    plain(&ops).unwrap();
    let e = covered(&ops).unwrap_err();
    assert!(
        e.contains(&format!(
            "more than {MAX_STRATA} distinct rare outcome conditions"
        )),
        "{e}"
    );
}

/// (k) The ledger's circuits. Ignored: heavy, and the artifacts are built outside the test. Set
/// `FEMOCO_COVERAGE_ROOTS` to `:`-separated directories, each holding one built circuit
/// (`ops.bin`, `lanemap.bin`, `family.out.json`), and run through tools/heavy.sh. Each is
/// evaluated at `2^19` lanes on the sliced engine in the mode; the circuits pinned below must
/// pass with no directed strata and exactly the stated number of proved-unreachable sites.
#[test]
#[ignore = "heavy: needs built circuits (FEMOCO_COVERAGE_ROOTS); run through tools/heavy.sh"]
fn ledger_circuits_pass_with_no_directed_strata() {
    use femoco_walk::{circuit::read_ops, sim, spec, taxonomy};
    // (ops.bin SHA-256, proved-unreachable sites).
    let pinned = [
        (
            "95e8fe8982dca129be65f4d11e4bdce0eb1fd4173f9294687ad0dba8646e2ea9",
            100,
        ),
        ("a939d204", 4),
        ("22ed4a76", 10),
    ];
    let roots = std::env::var("FEMOCO_COVERAGE_ROOTS").expect("FEMOCO_COVERAGE_ROOTS");
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for dir in roots.split(':').map(std::path::Path::new) {
        let fam = std::fs::read(dir.join("family.out.json")).unwrap();
        let out: score::FamilyOut = serde_json::from_slice(&fam).unwrap();
        let spec = spec::load(here, &out.spec).unwrap();
        let tax = taxonomy::load_taxonomy(&here.join("taxonomy/taxonomy.json")).unwrap();
        let check = |f: &taxonomy::Family, facts: &femoco_walk::facts::CircuitFacts| {
            taxonomy::check(&tax, f, facts)
        };
        let lanemap = std::fs::read(dir.join("lanemap.bin")).unwrap();
        let ops = read_ops(&dir.join("ops.bin")).unwrap();
        let inp = Inputs {
            spec: spec.as_ref(),
            lanemap: &lanemap,
            family: &fam,
            ops: &ops,
            samples: score::DEFAULT_SAMPLES,
            tracker: sim::givens_tracker(spec.as_ref()),
            check: &check,
        };
        let digest = hex::encode(ops.sha256);
        let opts = Options {
            outcome_coverage: true,
        };
        let ev = score::evaluate_opts(&inp, sliced(), None, opts)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let c = ev.outcome_coverage.unwrap();
        println!(
            "{}: ops {digest}: {} sites, {} proved unreachable, {} opaque, min control-1 executions {:?}, {} strata, {:.1} s",
            dir.display(), c.sites, c.proved_unreachable, c.opaque, c.min_control_one, c.directed_strata, ev.seconds
        );
        assert_eq!(c.directed_strata, 0);
        if let Some((_, dead)) = pinned.iter().find(|(d, _)| digest.starts_with(d)) {
            assert_eq!(c.proved_unreachable, *dead, "{}", dir.display());
        }
    }
}
