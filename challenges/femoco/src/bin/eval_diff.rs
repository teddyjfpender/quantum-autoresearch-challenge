//! TOOLING (spec/FAST-EVALUATOR.md section 10): the differential runner. Evaluates one artifact
//! directory (the three files `eval_circuit` reads) with the reference lane engine and with each
//! candidate engine, under each seed mode and thread count, and compares everything
//! (`femoco_walk::equiv::differences`): the rejection or the whole `score.json` (bar
//! `eval_seconds`), the sampled lanes, every lane's verdict and the engine's `Outcome`. It writes
//! no `score.json` and no `results.tsv` row. Built like `eval_circuit`, with
//! `--no-default-features`.
//!
//! Flags:
//! - `--root DIR`: as `eval_circuit` (`specs/`, `taxonomy/` and the three files; default `.`);
//! - `--samples K` (default 4096);
//! - `--candidate NAME[,NAME...]` (default `reference`): engines from `equiv::ENGINES` or the
//!   harness's self-test engines (`equiv::SELFTEST_ENGINES`);
//! - `--threads LIST` (default `1,4`): the candidates' thread counts;
//! - `--ref-threads N` (default 4): the reference's;
//! - `--seed MODE[,MODE...]` (default `ordinary`): `ordinary`, `server:HEX32`,
//!   `server-label:TEXT` (the seed is SHA-256 of the label), `audit:ROUND:HEX32` or
//!   `audit-label:ROUND:TEXT`;
//! - `--json FILE`: append one JSON line per comparison;
//! - `--quiet`: print only differences and the summary.
//!
//! Exit status: 0 when every candidate equals the reference everywhere, 1 when any differs, 2 on
//! a usage or input error.
use femoco_walk::circuit::read_ops;
use femoco_walk::equiv::{self, Seed};
use femoco_walk::fiat_shamir::AuditBeacon;
use femoco_walk::score::{FamilyOut, Inputs};
use femoco_walk::sim::validate;
use femoco_walk::{sim, spec, taxonomy};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;

struct Args {
    root: PathBuf,
    samples: usize,
    candidates: Vec<String>,
    threads: Vec<usize>,
    ref_threads: usize,
    seeds: Vec<Seed>,
    json: Option<PathBuf>,
    quiet: bool,
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    let mut out = [0u8; 32];
    hex::decode_to_slice(s.trim(), &mut out)
        .map_err(|e| format!("{s}: not 32 bytes of hex ({e})"))?;
    Ok(out)
}

fn parse_seed(s: &str) -> Result<Seed, String> {
    // A label may itself contain ':'.
    if let Some(t) = s.strip_prefix("server-label:") {
        return Ok(Seed::Server(equiv::seed_bytes(t)));
    }
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    let round = |r: &str| {
        r.parse::<u64>()
            .map_err(|e| format!("seed {s}: round: {e}"))
    };
    Ok(match parts[..] {
        ["ordinary"] => Seed::Ordinary,
        ["server", h] => Seed::Server(hex32(h)?),
        ["audit", r, h] => Seed::Audit(AuditBeacon {
            round: round(r)?,
            randomness: hex32(h)?,
        }),
        ["audit-label", r, t] => Seed::Audit(AuditBeacon {
            round: round(r)?,
            randomness: equiv::seed_bytes(t),
        }),
        _ => return Err(format!("unknown seed mode {s}")),
    })
}

fn list<T>(s: &str, f: impl Fn(&str) -> Result<T, String>) -> Result<Vec<T>, String> {
    s.split(',').filter(|x| !x.is_empty()).map(f).collect()
}

fn args() -> Result<Args, String> {
    let mut a = Args {
        root: ".".into(),
        samples: 4096,
        candidates: vec!["reference".into()],
        threads: vec![1, 4],
        ref_threads: 4,
        seeds: vec![Seed::Ordinary],
        json: None,
        quiet: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        if k == "--quiet" {
            a.quiet = true;
            continue;
        }
        let v = it.next().ok_or(format!("{k} needs a value"))?;
        let num = |v: &str| v.parse::<usize>().map_err(|e| format!("{k}: {e}"));
        match k.as_str() {
            "--root" => a.root = v.into(),
            "--samples" => a.samples = num(&v)?,
            "--candidate" => a.candidates = list(&v, |x| Ok(x.to_string()))?,
            "--threads" => a.threads = list(&v, num)?,
            "--ref-threads" => a.ref_threads = num(&v)?,
            "--seed" => a.seeds = list(&v, parse_seed)?,
            "--json" => a.json = Some(v.into()),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.samples == 0 || a.threads.is_empty() || a.seeds.is_empty() {
        return Err("--samples, --threads and --seed must be non-empty".into());
    }
    for c in &a.candidates {
        if equiv::harness_engine(c).is_none() {
            return Err(format!("no engine {c}"));
        }
    }
    Ok(a)
}

fn run(a: &Args) -> Result<bool, String> {
    let read = |n: &str| std::fs::read(a.root.join(n)).map_err(|e| format!("{n}: {e}"));
    let family = read("family.out.json")?;
    let fam: FamilyOut =
        serde_json::from_slice(&family).map_err(|e| format!("family.out.json: {e}"))?;
    let tax = taxonomy::load_taxonomy(&a.root.join("taxonomy/taxonomy.json"))?;
    let spec = spec::load(&a.root, &fam.spec)?;
    let lanemap = read("lanemap.bin")?;
    let ops = read_ops(&a.root.join("ops.bin"))?;
    let check = |f: &taxonomy::Family, facts: &femoco_walk::facts::CircuitFacts| {
        taxonomy::check(&tax, f, facts)
    };
    let inputs = Inputs {
        spec: spec.as_ref(),
        lanemap: &lanemap,
        family: &family,
        ops: &ops,
        samples: a.samples,
        tracker: sim::givens_tracker(spec.as_ref()),
        check: &check,
    };
    let mut log = match &a.json {
        Some(p) => Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| format!("{}: {e}", p.display()))?,
        ),
        None => None,
    };
    let mut all_equal = true;
    let (mut n, mut bad) = (0usize, 0usize);
    for seed in &a.seeds {
        let reference = equiv::run(
            &inputs,
            "reference",
            validate::reference,
            a.ref_threads,
            *seed,
        );
        let verdict = match &reference.result {
            Ok(_) => "accepted".to_string(),
            Err(e) => format!("rejected: {}", e.chars().take(140).collect::<String>()),
        };
        if !a.quiet {
            println!(
                "{} {} K={} seed {}: reference {verdict}; verdicts {}",
                a.root.display(),
                fam.spec,
                a.samples,
                seed.label(),
                equiv::verdict_counts(&reference.probe.verdicts)
            );
        }
        for c in &a.candidates {
            let e = equiv::harness_engine(c).ok_or(format!("no engine {c}"))?;
            for &t in &a.threads {
                let run = equiv::run(&inputs, c, e, t, *seed);
                let d = equiv::differences(&reference, &run);
                n += 1;
                if !d.is_empty() {
                    bad += 1;
                    all_equal = false;
                }
                if !a.quiet || !d.is_empty() {
                    println!(
                        "  {} {c} @ {t} threads ({:.2} s; reference {:.2} s @ {}){}: {}",
                        if d.is_empty() { "EQUAL" } else { "DIFF " },
                        run.seconds,
                        reference.seconds,
                        a.ref_threads,
                        if run.counters.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " [{}]",
                                run.counters
                                    .iter()
                                    .map(|(k, v)| format!("{k} {v}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        },
                        if d.is_empty() {
                            String::new()
                        } else {
                            format!("\n    {}", d.join("\n    "))
                        }
                    );
                }
                if let Some(f) = log.as_mut() {
                    let line = json!({
                        "root": a.root.display().to_string(),
                        "spec": fam.spec,
                        "ops": ops.ops.len(),
                        "samples": a.samples,
                        "seed": seed.label(),
                        "candidate": c,
                        "threads": t,
                        "ref_threads": a.ref_threads,
                        "reference": verdict,
                        "verdicts": equiv::verdict_counts(&reference.probe.verdicts),
                        "equal": d.is_empty(),
                        "differences": d,
                        "seconds": run.seconds,
                        "reference_seconds": reference.seconds,
                        "engine_counters": run.counters.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
                    });
                    f.write_all(format!("{line}\n").as_bytes())
                        .map_err(|e| format!("--json: {e}"))?;
                }
            }
        }
    }
    println!("eval_diff: {n} comparisons, {bad} with differences");
    Ok(all_equal)
}

fn main() {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("!! {e}");
            std::process::exit(2);
        }
    };
    match run(&a) {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("!! {e}");
            std::process::exit(2);
        }
    }
}
