//! TRUSTED stage: re-reads `ops.bin`, `lanemap.bin` and `family.out.json`, validates the
//! submission on Fiat-Shamir-sampled lanes, counts, writes `score.json` and appends a row to
//! `results.tsv` (a failed run appends a `FAIL` row with its reason). Built with
//! `--no-default-features`, so the contestant's `walk` module is not compiled into it.
//!
//! Flags: `--samples K` (default `score::DEFAULT_SAMPLES`), `--note TEXT`, `--engine NAME`
//! (`reference`, the default, or `sliced`: the fast engine of spec/FAST-EVALUATOR.md, tooling not
//! yet adopted; it is printed on stdout and not recorded in `score.json`), `--root DIR` (repo
//! root holding `specs/`, `taxonomy/` and the three input files; default `.`), and
//! `--fail REASON` (used by benchmark.sh when the untrusted stage failed: record a `FAIL` row
//! with that reason without evaluating anything).
//!
//! Fresh-seed audit (spec/DESIGN.md section 14), all four together: `--freeze FILE` (a committed
//! freeze file; no freeze-file writer ships here), `--audit-round N`, `--audit-seed HEX` (that
//! drand quicknet round's 32-byte randomness) and `--audit-signature HEX` (its signature). The
//! evaluator stays offline: it checks that the freeze file is committed and unmodified, that
//! its last commit predates the round, that the round is the one the freeze file named and
//! that `randomness = SHA-256(signature)`, and that the circuit's four digests match one frozen
//! circuit; otherwise it refuses (exit 2, no `score.json`, no `results.tsv` row). It then draws
//! the lanes from the audit seed stream (`fiat_shamir::with_audit`) at the freeze file's sample
//! count (`--samples` may only repeat it) and adds `metrics.audit` to `score.json`. Without
//! these flags nothing about a run changes.
//!
//! The local artifact server can instead pass `--server-seed HEX` (32 bytes selected after
//! upload). This uses the audit seed domain with round 0 and records the seed in `score.json`;
//! it is distinct from the public-beacon audit and does not alter ordinary runs.
//!
//! Lane engine (spec/FAST-EVALUATOR.md): `--engine NAME` selects the engine that runs the sampled
//! lanes (`femoco_walk::equiv::ENGINES`; default `reference`, the oracle). Everything else is the
//! same code, and `score.json` does not record the engine. `--dump-verdicts FILE` writes one byte
//! per sampled lane, in sample order (`validate::VERDICT_PASS`, a failure category index, or
//! `validate::VERDICT_UNCHECKED`), for the equivalence harness (tools/fastsim/equivalence.sh).
//!
//! Deterministic input coverage: `--coverage terms|exhaustive` adds a separate lane set after
//! the sampled run, without changing resource tallies. `--export-symbolic FILE` writes the
//! trusted lowered controller and alias tables for independent SMT audit. Neither option
//! certifies exact quantum equivalence; see tools/verification/README.md for the domains.
use femoco_walk::circuit::read_ops;
use femoco_walk::fiat_shamir::{self, quicknet, AuditBeacon, Digests};
use femoco_walk::score::{self, append_results_row, score_json, Inputs, ResultsRow};
use femoco_walk::sim::validate::{self, Engine};
use femoco_walk::{equiv, sim, spec, taxonomy};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Format tag a freeze file must carry.
const FREEZE_FORMAT: &str = "femoco-audit-freeze-v1";
/// Name of the audit protocol recorded in `score.json`.
const AUDIT_PROTOCOL: &str = "femoco-fresh-seed-v1";

struct Args {
    coverage: Option<femoco_walk::coverage::Mode>,
    export_symbolic: Option<PathBuf>,
    samples: usize,
    samples_given: bool,
    note: String,
    root: PathBuf,
    fail: Option<String>,
    freeze: Option<PathBuf>,
    audit_seed: Option<String>,
    audit_round: Option<u64>,
    audit_signature: Option<String>,
    server_seed: Option<[u8; 32]>,
    engine: Option<(String, Engine)>,
    dump_verdicts: Option<PathBuf>,
}

fn args() -> Result<Args, String> {
    let mut a = Args {
        coverage: None,
        export_symbolic: None,
        samples: score::DEFAULT_SAMPLES,
        samples_given: false,
        note: String::new(),
        root: ".".into(),
        fail: None,
        freeze: None,
        audit_seed: None,
        audit_round: None,
        audit_signature: None,
        server_seed: None,
        engine: None,
        dump_verdicts: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let (key, inline) = match k.split_once('=') {
            Some((x, y)) => (x.to_string(), Some(y.to_string())),
            None => (k.clone(), None),
        };
        let mut val = || {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or(format!("{key} needs a value"))
        };
        match key.as_str() {
            "--coverage" => {
                a.coverage = Some(match val()?.as_str() {
                    "terms" => femoco_walk::coverage::Mode::Terms,
                    "exhaustive" => femoco_walk::coverage::Mode::Exhaustive,
                    other => return Err(format!("--coverage: unknown mode {other}")),
                })
            }
            "--export-symbolic" => a.export_symbolic = Some(val()?.into()),
            "--samples" => {
                a.samples = val()?.parse().map_err(|e| format!("--samples: {e}"))?;
                a.samples_given = true;
            }
            "--note" => a.note = val()?,
            "--root" => a.root = val()?.into(),
            "--fail" => a.fail = Some(val()?),
            "--freeze" => a.freeze = Some(val()?.into()),
            "--audit-seed" => a.audit_seed = Some(val()?),
            "--audit-round" => {
                a.audit_round = Some(val()?.parse().map_err(|e| format!("--audit-round: {e}"))?);
            }
            "--audit-signature" => a.audit_signature = Some(val()?),
            "--server-seed" => {
                let bytes = hex::decode(val()?).map_err(|e| format!("--server-seed: {e}"))?;
                a.server_seed = Some(
                    bytes
                        .try_into()
                        .map_err(|_| "--server-seed needs 32 bytes of hex")?,
                );
            }
            "--engine" => {
                let name = val()?;
                let e = equiv::engine(&name).ok_or_else(|| {
                    format!(
                        "--engine {name}: not in this build (engines: {})",
                        equiv::ENGINES.join(", ")
                    )
                })?;
                a.engine = Some((name, e));
            }
            "--dump-verdicts" => a.dump_verdicts = Some(val()?.into()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.samples == 0 {
        return Err("--samples must be positive".into());
    }
    let given = [
        a.freeze.is_some(),
        a.audit_seed.is_some(),
        a.audit_round.is_some(),
        a.audit_signature.is_some(),
    ];
    if given.iter().any(|&g| g) && !given.iter().all(|&g| g) {
        return Err(
            "an audit needs all of --freeze, --audit-round, --audit-seed and --audit-signature"
                .into(),
        );
    }
    if a.server_seed.is_some() && given.iter().any(|&g| g) {
        return Err("--server-seed cannot be combined with a beacon audit".into());
    }
    Ok(a)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn commit(root: &Path) -> String {
    git(root, &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|_| "nogit".into())
}

fn read(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    std::fs::read(root.join(name)).map_err(|e| format!("{name}: {e}"))
}

fn hex32(s: &str, what: &str) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    hex::decode_to_slice(s.trim(), &mut out)
        .map_err(|e| format!("{what}: not 32 bytes of hex ({e})"))?;
    Ok(out)
}

/// The audit's preconditions, checked before anything is evaluated.
struct Audit {
    beacon: AuditBeacon,
    signature: String,
    round_time: u64,
    samples: usize,
    freeze_path: PathBuf,
    freeze_sha256: String,
    freeze_commit: String,
    freeze_commit_time: u64,
    freeze: Value,
}

/// Everything about the audit that does not need the circuit: the freeze file is committed and
/// clean, it names this beacon round, the round comes after its last commit, and the recorded
/// randomness is the SHA-256 of the recorded signature.
fn prepare_audit(a: &Args) -> Result<Option<Audit>, String> {
    let (Some(path), Some(seed), Some(round), Some(sig)) =
        (&a.freeze, &a.audit_seed, a.audit_round, &a.audit_signature)
    else {
        return Ok(None);
    };
    let randomness = hex32(seed, "--audit-seed")?;
    let signature = hex::decode(sig.trim()).map_err(|e| format!("--audit-signature: {e}"))?;
    if !quicknet::randomness_matches(&signature, &randomness) {
        return Err("--audit-seed is not SHA-256(--audit-signature)".into());
    }
    let path = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let freeze: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if freeze["format"] != FREEZE_FORMAT {
        return Err(format!("freeze file format is not {FREEZE_FORMAT}"));
    }
    let b = &freeze["beacon"];
    if b["chain_hash"] != quicknet::CHAIN_HASH {
        return Err("freeze file does not name the drand quicknet chain".into());
    }
    let frozen_round = b["round"]
        .as_u64()
        .ok_or("freeze file: beacon.round missing")?;
    if frozen_round != round {
        return Err(format!(
            "--audit-round {round} is not the round the freeze file fixed ({frozen_round})"
        ));
    }
    let round_time = quicknet::round_time(round);
    if b["round_time_unix"].as_u64() != Some(round_time) {
        return Err(format!(
            "freeze file: beacon.round_time_unix is not quicknet's time for round {round} ({round_time})"
        ));
    }
    let samples = freeze["samples"]
        .as_u64()
        .and_then(|k| usize::try_from(k).ok())
        .filter(|&k| k > 0)
        .ok_or("freeze file: samples missing")?;
    if a.samples_given && a.samples != samples {
        return Err(format!(
            "--samples {} differs from the freeze file's {samples}; an audit's K is fixed at freeze",
            a.samples
        ));
    }
    let dir = path.parent().ok_or("freeze file has no directory")?;
    let file = path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or("freeze file name")?;
    git(dir, &["ls-files", "--error-unmatch", "--", file])
        .map_err(|_| "freeze file is not committed".to_string())?;
    if !git(dir, &["status", "--porcelain", "--", file])?.is_empty() {
        return Err("freeze file differs from its last commit".into());
    }
    let log = git(dir, &["log", "-1", "--format=%H %ct", "--", file])?;
    let (hash, time) = log.split_once(' ').ok_or("git log: no freeze commit")?;
    let time: u64 = time.parse().map_err(|e| format!("git log time: {e}"))?;
    if time >= round_time {
        return Err(format!(
            "the freeze file's last commit ({hash}, time {time}) is not before round {round} (time {round_time})"
        ));
    }
    Ok(Some(Audit {
        beacon: AuditBeacon { round, randomness },
        signature: hex::encode(signature),
        round_time,
        samples,
        freeze_sha256: hex::encode(Sha256::digest(&bytes)),
        freeze_path: path,
        freeze_commit: hash.to_string(),
        freeze_commit_time: time,
        freeze,
    }))
}

/// The frozen circuit whose four digests are `d`.
fn frozen_circuit(audit: &Audit, d: &Digests) -> Result<String, String> {
    let want = [
        ("spec", d.spec),
        ("lanemap", d.lanemap),
        ("family", d.family),
        ("ops", d.ops),
    ];
    let circuits = audit.freeze["circuits"]
        .as_array()
        .ok_or("freeze file: circuits missing")?;
    circuits
        .iter()
        .find(|c| {
            want.iter()
                .all(|(k, v)| c["digests"][k].as_str() == Some(hex::encode(v).as_str()))
        })
        .and_then(|c| c["name"].as_str().map(str::to_string))
        .ok_or_else(|| {
            format!(
                "no frozen circuit has these digests (ops {}, lanemap {}, family {}, spec {})",
                hex::encode(d.ops),
                hex::encode(d.lanemap),
                hex::encode(d.family),
                hex::encode(d.spec)
            )
        })
}

enum Failure {
    /// An audit precondition failed: nothing was evaluated and nothing is recorded.
    Refused(String),
    /// The submission was evaluated (or could not be read) and is rejected.
    Failed(String),
}

impl From<String> for Failure {
    fn from(s: String) -> Self {
        Self::Failed(s)
    }
}

fn run(
    a: &Args,
    audit: Option<&Audit>,
    row: &mut ResultsRow,
) -> Result<(score::Evaluation, Option<String>), Failure> {
    let family = read(&a.root, "family.out.json")?;
    let fam: score::FamilyOut =
        serde_json::from_slice(&family).map_err(|e| format!("family.out.json: {e}"))?;
    row.spec.clone_from(&fam.spec);
    row.family_name.clone_from(&fam.family.name);
    let tax = taxonomy::load_taxonomy(&a.root.join("taxonomy/taxonomy.json"))?;
    let spec = spec::load(&a.root, &fam.spec)?;
    let lanemap = read(&a.root, "lanemap.bin")?;
    let ops = read_ops(&a.root.join("ops.bin"))?;
    let samples = audit.map_or(a.samples, |x| x.samples);
    let circuit = match audit {
        None => None,
        Some(x) => {
            let d = Digests {
                spec: spec.payload_sha256(),
                lanemap: Sha256::digest(&lanemap).into(),
                family: Sha256::digest(&family).into(),
                ops: ops.sha256,
            };
            Some(frozen_circuit(x, &d).map_err(Failure::Refused)?)
        }
    };
    println!(
        "  spec        : {} ({} system qubits)",
        spec.id(),
        spec.system_qubits()
    );
    println!("  ops         : {}", ops.ops.len());
    println!("  samples     : {samples}");
    if let (Some(x), Some(name)) = (audit, &circuit) {
        println!(
            "  audit       : {name}, quicknet round {}, freeze commit {}",
            x.beacon.round, x.freeze_commit
        );
    }
    if let Some((name, _)) = &a.engine {
        if name == "reference" {
            println!("  engine      : {name}");
        } else {
            println!("  engine      : {name} (not the reference engine)");
        }
    }
    let check = |f: &taxonomy::Family, facts: &femoco_walk::facts::CircuitFacts| {
        taxonomy::check(&tax, f, facts)
    };
    let inputs = Inputs {
        spec: spec.as_ref(),
        lanemap: &lanemap,
        family: &family,
        ops: &ops,
        samples,
        tracker: sim::givens_tracker(spec.as_ref()),
        check: &check,
    };
    let engine = a
        .engine
        .as_ref()
        .map_or(validate::reference as Engine, |e| e.1);
    let mut probe = score::Probe::default();
    let probe_ref = a.dump_verdicts.is_some().then_some(&mut probe);
    let eval = || score::evaluate_with(&inputs, engine, probe_ref);
    let ev = match (audit, a.server_seed) {
        (None, None) => eval(),
        (None, Some(randomness)) => fiat_shamir::with_audit(
            AuditBeacon {
                round: 0,
                randomness,
            },
            eval,
        ),
        (Some(x), _) => fiat_shamir::with_audit(x.beacon, eval),
    };
    if let Some((name, _)) = &a.engine {
        // The engine's own counters (batches re-run on the reference, guard-band fallbacks...):
        // stdout only, never score.json.
        let counters = equiv::engine_counters(name);
        if !counters.is_empty() {
            let line: Vec<String> = counters.iter().map(|(k, v)| format!("{k}={v}")).collect();
            println!("  counters    : {}", line.join(" "));
        }
    }
    if let Some(path) = &a.dump_verdicts {
        std::fs::write(path, &probe.verdicts)
            .map_err(|e| format!("--dump-verdicts {}: {e}", path.display()))?;
    }
    let mut ev = ev?;
    if a.coverage.is_some() || a.export_symbolic.is_some() {
        ev.validation = femoco_walk::coverage::check(
            &inputs,
            engine,
            a.coverage,
            a.export_symbolic.as_deref(),
            a.server_seed.as_ref(),
        )?;
    }
    Ok((ev, circuit))
}

/// `metrics.audit` of an audited run's `score.json`.
fn audit_json(x: &Audit, circuit: &str, ev: &score::Evaluation) -> Value {
    let mut first = [0u8; 32];
    {
        use sha3::digest::XofReader;
        fiat_shamir::audit_seed(&ev.digests, &x.beacon).read(&mut first);
    }
    json!({
        "protocol": AUDIT_PROTOCOL,
        "circuit": circuit,
        "freeze_file": x.freeze_path.display().to_string(),
        "freeze_file_sha256": x.freeze_sha256,
        "freeze_commit": x.freeze_commit,
        "freeze_commit_time_unix": x.freeze_commit_time,
        "beacon": {
            "network": "drand quicknet",
            "chain_hash": quicknet::CHAIN_HASH,
            "scheme": quicknet::SCHEME,
            "round": x.beacon.round,
            "round_time_unix": x.round_time,
            "randomness": hex::encode(x.beacon.randomness),
            "signature": x.signature,
            "checked_here": "randomness = SHA-256(signature); round fixed by the freeze file; round time after the freeze commit time. The BLS signature is not checked by eval_circuit.",
        },
        "seed_domain": String::from_utf8_lossy(fiat_shamir::AUDIT_DOMAIN),
        "seed_xof_first32": hex::encode(first),
        "samples": ev.samples,
        "control_one_samples": ev.control_one_samples,
        "f_bound": ev.f_bound(),
        "implied_error_bound": ev.implied_error_bound(),
        "bound": "single draw: every sampled lane passed, so with confidence 1 - 1e-6 at most f_bound of lanes are wrong. The seed was fixed by a beacon round published after the freeze, so it could not be re-drawn; the grinding figures elsewhere in metrics do not apply to this run.",
    })
}

fn main() {
    println!("=== femoco_walk: eval_circuit (trusted stage) ===");
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("!! {e}");
            std::process::exit(2);
        }
    };
    // A stale score.json must never survive a failed run.
    let _ = std::fs::remove_file(a.root.join("score.json"));
    if let Some(path) = &a.export_symbolic {
        let _ = std::fs::remove_file(path);
    }
    let audit = if a.fail.is_some() {
        None
    } else {
        match prepare_audit(&a) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("!! audit refused: {e}");
                std::process::exit(2);
            }
        }
    };
    let mut row = ResultsRow::default();
    let outcome = match &a.fail {
        Some(reason) => Err(Failure::Failed(format!("untrusted stage: {reason}"))),
        None => run(&a, audit.as_ref(), &mut row),
    };
    let unix_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let tsv = a.root.join("results.tsv");
    let note = match (&audit, &outcome) {
        (Some(x), Ok((_, Some(name)))) => {
            let tag = format!("audit {name} quicknet#{}", x.beacon.round);
            if a.note.is_empty() {
                tag
            } else {
                format!("{tag} | {}", a.note)
            }
        }
        _ => a.note.clone(),
    };
    match outcome {
        Ok((ev, circuit)) => {
            let mut ok = ResultsRow::from_eval(&ev);
            // Results under the estimated rounding class carry its label (spec/SPEC-SA.md section 14).
            let note = if ev.rounding_estimate.is_some() {
                let label = femoco_walk::spec::rounding::ESTIMATED_LABEL;
                if note.is_empty() {
                    label.to_string()
                } else {
                    format!("{label} | {note}")
                }
            } else {
                note
            };
            (ok.unix_time, ok.commit, ok.note) = (unix_time, commit(&a.root), note);
            let mut body = score_json(&ev);
            if let (Some(x), Some(name)) = (&audit, &circuit) {
                body["metrics"]["audit"] = audit_json(x, name, &ev);
            }
            if let Some(seed) = a.server_seed {
                body["metrics"]["server_seed"] = json!({
                    "protocol": "server-post-upload-v1",
                    "seed": hex::encode(seed),
                    "note": "32 random bytes fixed by the server after upload; not a public beacon or a formal proof",
                });
            }
            let body = serde_json::to_string_pretty(&body).unwrap_or_default();
            if let Err(e) = std::fs::write(a.root.join("score.json"), body + "\n") {
                eprintln!("!! score.json: {e}");
                std::process::exit(1);
            }
            if let Err(e) = append_results_row(&tsv, &ok) {
                eprintln!("warning: results.tsv: {e}");
            }
            println!(
                "  lambda      : {:.6} (rounding error {:.3e})",
                ev.lambda.to_f64(),
                ev.rounding_error.to_f64()
            );
            println!("  toffoli     : {:.3} per step", ev.toffoli);
            println!("  qubits      : {}", ev.qubits);
            println!("  score       : {:.6e}", ev.score());
            println!("  eval time   : {:.2} s", ev.seconds);
            if audit.is_some() {
                println!(
                    "  audit bound : f_bound {:.3e} at K = {} (single draw)",
                    ev.f_bound(),
                    ev.samples
                );
            }
            println!("=== eval OK ===");
        }
        Err(Failure::Refused(reason)) => {
            eprintln!("!! audit refused: {reason}");
            std::process::exit(2);
        }
        Err(Failure::Failed(reason)) => {
            eprintln!("!! eval FAILED: {reason}");
            row.unix_time = unix_time;
            row.commit = commit(&a.root);
            row.samples = audit.as_ref().map_or(a.samples, |x| x.samples);
            row.status = "FAIL".into();
            let tag = audit
                .as_ref()
                .map(|x| format!("audit quicknet#{}", x.beacon.round));
            row.note = [
                tag,
                (!a.note.is_empty()).then(|| a.note.clone()),
                Some(reason),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" | ");
            if let Err(e) = append_results_row(&tsv, &row) {
                eprintln!("warning: results.tsv: {e}");
            }
            std::process::exit(1);
        }
    }
}
