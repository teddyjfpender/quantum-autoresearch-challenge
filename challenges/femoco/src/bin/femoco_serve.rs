//! Local, artifact-only FeMoco challenge service. The trusted evaluator remains a separate binary.
use femoco_walk::score::FamilyOut;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Request, Response, Server};

const MAGIC: &[u8; 8] = b"FEMOSUB1";
const MAX_UPLOAD: u64 = 512 << 20;
const MAX_PARTS: [u64; 4] = [480 << 20, 32 << 20, 64 << 10, 4 << 10];
const PARTS: [&str; 4] = [
    "ops.bin",
    "lanemap.bin",
    "family.out.json",
    "architecture.json",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Architecture {
    delivery: String,
    erasure: String,
    outer_slot: String,
}

impl Architecture {
    fn cell(&self) -> Result<String, String> {
        if !["word", "onehot", "onehot-split", "streamed"].contains(&self.delivery.as_str())
            || !["plain", "gated", "ladder", "ladder+gated"].contains(&self.erasure.as_str())
            || !["held", "dropped"].contains(&self.outer_slot.as_str())
        {
            return Err(
                "architecture must use the registered delivery/erasure/outer_slot cells".into(),
            );
        }
        Ok(format!(
            "{}/{}/{}",
            self.delivery, self.erasure, self.outer_slot
        ))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    id: String,
    submitted_unix: u64,
    state: String,
    spec: String,
    declared_cell: String,
    architecture_verified: bool,
    artifact_sha256: String,
    #[serde(default)]
    circuit_sha256: String,
    verifier_sha256: String,
    server_seed: Option<String>,
    quick: Option<Value>,
    full: Option<Value>,
    error: Option<String>,
}

struct Config {
    repo: PathBuf,
    state: PathBuf,
    evaluator: PathBuf,
    verifier_sha256: String,
    quick_samples: usize,
    full_samples: usize,
    quick_timeout: Duration,
    full_timeout: Duration,
}

struct App {
    cfg: Config,
    records: Mutex<()>,
    quick_active: AtomicUsize,
    full_tx: mpsc::SyncSender<String>,
}

enum EvalFailure {
    Rejected(String),
    Failed(String),
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn sha_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 1 << 16];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn circuit_sha(dir: &Path, spec: &str) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update(spec.as_bytes());
    let mut buf = [0; 1 << 16];
    for name in [PARTS[0], PARTS[1]] {
        let mut file = File::open(dir.join(name)).map_err(|e| e.to_string())?;
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn pack(args: &[String]) -> Result<(), String> {
    if args.len() != 5 {
        return Err("usage: femoco_serve pack OPS LANEMAP FAMILY ARCHITECTURE OUT.fmc".into());
    }
    let mut lengths = [0u64; 4];
    for (i, path) in args[..4].iter().enumerate() {
        lengths[i] = fs::metadata(path).map_err(|e| e.to_string())?.len();
        if lengths[i] == 0 || lengths[i] > MAX_PARTS[i] {
            return Err(format!(
                "{} size outside 1..={} bytes",
                PARTS[i], MAX_PARTS[i]
            ));
        }
    }
    let mut out = File::create(&args[4]).map_err(|e| e.to_string())?;
    out.write_all(MAGIC).map_err(|e| e.to_string())?;
    for len in lengths {
        out.write_all(&len.to_le_bytes())
            .map_err(|e| e.to_string())?;
    }
    for path in &args[..4] {
        std::io::copy(&mut File::open(path).map_err(|e| e.to_string())?, &mut out)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn unpack(bundle: &Path, dir: &Path, repo: &Path) -> Result<(String, String), String> {
    let mut file = File::open(bundle).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let mut header = [0u8; 40];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..8] != MAGIC {
        return Err("bad bundle magic".into());
    }
    let mut lengths = [0u64; 4];
    for (i, len) in lengths.iter_mut().enumerate() {
        *len = u64::from_le_bytes(header[8 + i * 8..16 + i * 8].try_into().unwrap());
        if *len == 0 || *len > MAX_PARTS[i] {
            return Err(format!("{} exceeds its size cap", PARTS[i]));
        }
    }
    if size != 40 + lengths.iter().sum::<u64>() {
        return Err("bundle length mismatch".into());
    }
    for (i, name) in PARTS.iter().enumerate() {
        let mut out = File::create_new(dir.join(name)).map_err(|e| e.to_string())?;
        let copied = std::io::copy(
            &mut std::io::Read::by_ref(&mut file).take(lengths[i]),
            &mut out,
        )
        .map_err(|e| e.to_string())?;
        if copied != lengths[i] {
            return Err(format!("short {name}"));
        }
    }
    let fam: FamilyOut =
        serde_json::from_slice(&fs::read(dir.join(PARTS[2])).map_err(|e| e.to_string())?)
            .map_err(|e| format!("family.out.json: {e}"))?;
    let arch: Architecture =
        serde_json::from_slice(&fs::read(dir.join(PARTS[3])).map_err(|e| e.to_string())?)
            .map_err(|e| format!("architecture.json: {e}"))?;
    let cell = arch.cell()?;
    if fam.spec.is_empty()
        || fam.spec.len() > 64
        || !fam
            .spec
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || !repo
            .join("specs")
            .join(&fam.spec)
            .join("spec.json")
            .is_file()
    {
        return Err("unknown or unsafe spec id".into());
    }
    let meta: Value = serde_json::from_slice(
        &fs::read(repo.join("specs").join(&fam.spec).join("spec.json"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if meta["encoding"] != "sos-sa" {
        return Err("this architecture-cell pilot accepts sos-sa specs only".into());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(repo.join("specs"), dir.join("specs"))
            .map_err(|e| e.to_string())?;
        std::os::unix::fs::symlink(repo.join("taxonomy"), dir.join("taxonomy"))
            .map_err(|e| e.to_string())?;
    }
    Ok((fam.spec, cell))
}

impl App {
    fn record_path(&self, id: &str) -> PathBuf {
        self.cfg.state.join(id).join("record.json")
    }
    fn load(&self, id: &str) -> Option<Record> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let _guard = self.records.lock().ok()?;
        serde_json::from_slice(&fs::read(self.record_path(id)).ok()?).ok()
    }
    fn save(&self, record: &Record) -> Result<(), String> {
        let _guard = self.records.lock().map_err(|e| e.to_string())?;
        let path = self.record_path(&record.id);
        let tmp = path.with_extension("tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, path).map_err(|e| e.to_string())
    }
    fn change(&self, id: &str, state: &str, result: Option<Value>, error: Option<String>) {
        if let Some(mut rec) = self.load(id) {
            rec.state = state.into();
            if state == "screened" {
                rec.quick = result;
            } else if state == "accepted" {
                rec.full = result;
            }
            rec.error = error;
            let _ = self.save(&rec);
        }
    }
    fn evaluate(
        &self,
        id: &str,
        stage: &str,
        samples: usize,
        timeout: Duration,
        seed: Option<&str>,
    ) -> Result<Value, EvalFailure> {
        let dir = self.cfg.state.join(id);
        let log = File::create(dir.join(format!("{stage}.log")))
            .map_err(|e| EvalFailure::Failed(e.to_string()))?;
        let mut command = Command::new(&self.cfg.evaluator);
        command.args([
            "--root",
            dir.to_str()
                .ok_or_else(|| EvalFailure::Failed("non-UTF8 state path".into()))?,
            "--samples",
            &samples.to_string(),
        ]);
        command.env("RAYON_NUM_THREADS", "4");
        if let Some(seed) = seed {
            command.args(["--server-seed", seed]);
        }
        let mut child = command
            .stdout(Stdio::from(
                log.try_clone()
                    .map_err(|e| EvalFailure::Failed(e.to_string()))?,
            ))
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|e| EvalFailure::Failed(e.to_string()))?;
        let start = Instant::now();
        loop {
            if let Some(status) = child
                .try_wait()
                .map_err(|e| EvalFailure::Failed(e.to_string()))?
            {
                if !status.success() {
                    let message = format!(
                        "{stage} rejected (exit {}); see {stage}.log",
                        status.code().unwrap_or(-1)
                    );
                    return Err(if status.code() == Some(1) {
                        EvalFailure::Rejected(message)
                    } else {
                        EvalFailure::Failed(message)
                    });
                }
                break;
            }
            if start.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EvalFailure::Failed(format!(
                    "{stage} timed out after {} seconds",
                    timeout.as_secs()
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let value: Value = serde_json::from_slice(
            &fs::read(dir.join("score.json")).map_err(|e| EvalFailure::Failed(e.to_string()))?,
        )
        .map_err(|e| EvalFailure::Failed(e.to_string()))?;
        fs::rename(dir.join("score.json"), dir.join(format!("{stage}.json")))
            .map_err(|e| EvalFailure::Failed(e.to_string()))?;
        Ok(value)
    }
    fn screen(self: &Arc<Self>, id: String) {
        self.change(&id, "screening", None, None);
        let result = self.evaluate(
            &id,
            "quick",
            self.cfg.quick_samples,
            self.cfg.quick_timeout,
            None,
        );
        match result {
            Ok(value) => {
                self.change(&id, "screened", Some(value), None);
                if self.full_tx.try_send(id.clone()).is_err() {
                    self.change(
                        &id,
                        "failed",
                        None,
                        Some("confirmation queue is full".into()),
                    );
                }
            }
            Err(EvalFailure::Rejected(e)) => self.change(&id, "rejected", None, Some(e)),
            Err(EvalFailure::Failed(e)) => self.change(&id, "failed", None, Some(e)),
        }
        self.quick_active.fetch_sub(1, Ordering::SeqCst);
    }
    fn confirm(&self, id: &str) {
        self.change(id, "confirming", None, None);
        let Some(mut record) = self.load(id) else {
            return;
        };
        if record.server_seed.is_none() {
            let mut seed = [0u8; 32];
            if File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut seed))
                .is_err()
            {
                return self.change(id, "failed", None, Some("OS randomness unavailable".into()));
            }
            record.server_seed = Some(hex::encode(seed));
            if self.save(&record).is_err() {
                return self.change(
                    id,
                    "failed",
                    None,
                    Some("could not persist confirmation seed".into()),
                );
            }
        }
        match self.evaluate(
            id,
            "full",
            self.cfg.full_samples,
            self.cfg.full_timeout,
            record.server_seed.as_deref(),
        ) {
            Ok(value) => {
                if record.quick.as_ref().is_some_and(|quick| {
                    confirmation_matches(
                        quick,
                        &value,
                        record.server_seed.as_deref().unwrap_or(""),
                        self.cfg.full_samples,
                    )
                }) {
                    self.change(id, "accepted", Some(value), None);
                } else {
                    self.change(
                        id,
                        "failed",
                        None,
                        Some("confirmation provenance differs from screening".into()),
                    );
                }
            }
            Err(EvalFailure::Rejected(e)) => self.change(id, "rejected", None, Some(e)),
            Err(EvalFailure::Failed(e)) => self.change(id, "failed", None, Some(e)),
        }
    }
}

fn reply(request: Request, status: u16, body: Value) {
    let response = Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
    let _ = request.respond(response);
}

fn receive(request: &mut Request, incoming: &Path) -> Result<String, String> {
    if request.body_length().is_some_and(|n| n as u64 > MAX_UPLOAD) {
        return Err("upload exceeds 512 MiB".into());
    }
    let mut out = File::create_new(incoming).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0; 1 << 16];
    loop {
        let n = request
            .as_reader()
            .read(&mut buf)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_UPLOAD {
            return Err("upload exceeds 512 MiB".into());
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
    }
    if total == 0 {
        return Err("empty upload".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn metric(v: &Value, name: &str) -> Option<f64> {
    v.get("metrics")?.get(name)?.as_f64()
}

fn confirmation_matches(quick: &Value, full: &Value, seed: &str, samples: usize) -> bool {
    quick["metrics"]["digests"] == full["metrics"]["digests"]
        && full["metrics"]["server_seed"]["seed"] == seed
        && full["metrics"]["samples"] == samples
}

fn frontier(app: &App, spec: &str) -> Result<Value, String> {
    if !app
        .cfg
        .repo
        .join("specs")
        .join(spec)
        .join("spec.json")
        .is_file()
    {
        return Err("unknown spec".into());
    }
    let mut rows = Vec::new();
    for entry in fs::read_dir(&app.cfg.state).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let id = entry.file_name().to_string_lossy().to_string();
        if let Some(r) = app
            .load(&id)
            .filter(|r| r.state == "accepted" && r.spec == spec)
        {
            rows.push(r);
        }
    }
    let scores: Vec<(f64, f64, f64)> = rows
        .iter()
        .map(|r| {
            let v = r.full.as_ref().unwrap();
            let lambda = metric(v, "lambda").unwrap_or(f64::INFINITY);
            let toffoli = metric(v, "toffoli").unwrap_or(f64::INFINITY);
            let qubits = metric(v, "qubits").unwrap_or(f64::INFINITY);
            (
                lambda * toffoli,
                qubits,
                v["score"].as_f64().unwrap_or(f64::INFINITY),
            )
        })
        .collect();
    let best = scores.iter().map(|x| x.2).fold(f64::INFINITY, f64::min);
    let output: Vec<Value> = rows.iter().enumerate().map(|(i, r)| {
        let (cost, qubits, score) = scores[i];
        let dominated = |j: usize| {
            let (other_cost, other_qubits, _) = scores[j];
            j != i && other_cost <= cost && other_qubits <= qubits
                && (other_cost < cost || other_qubits < qubits)
        };
        json!({
            "id": r.id, "declared_cell": r.declared_cell,
            "architecture_verified": r.architecture_verified,
            "taxonomy_family": r.full.as_ref().unwrap()["metrics"]["family"]["axes"],
            "verified_axes": r.full.as_ref().unwrap()["metrics"]["verified_axes"],
            "score": score, "lambda_times_toffoli": cost, "qubits": qubits,
            "global_best": score == best,
            "global_pareto": !(0..rows.len()).any(dominated),
            "cell_pareto": !(0..rows.len()).any(|j| rows[j].declared_cell == r.declared_cell && dominated(j)),
        })
    }).collect();
    Ok(
        json!({"spec": spec, "comparison": ["lambda_times_toffoli", "qubits"], "submissions": output}),
    )
}

fn handle(app: &Arc<App>, mut request: Request) {
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("");
    match (request.method(), path) {
        (Method::Get, "/health") => reply(
            request,
            200,
            json!({"status": "ok", "scope": "local-artifact-pilot"}),
        ),
        (Method::Get, p) if p.starts_with("/submissions/") => {
            let id = &p["/submissions/".len()..];
            match app.load(id) {
                Some(record) => reply(request, 200, json!(record)),
                None => reply(request, 404, json!({"error": "unknown submission"})),
            }
        }
        (Method::Get, "/frontier") => {
            let spec = url
                .split_once("spec=")
                .map(|(_, s)| s.split('&').next().unwrap_or(""));
            match spec.and_then(|s| frontier(app, s).ok()) {
                Some(value) => reply(request, 200, value),
                None => reply(
                    request,
                    400,
                    json!({"error": "valid spec query parameter required"}),
                ),
            }
        }
        (Method::Post, "/submissions") => {
            if app.quick_active.load(Ordering::SeqCst) >= 2 {
                return reply(
                    request,
                    503,
                    json!({"error": "screening capacity full; retry"}),
                );
            }
            let incoming = app.cfg.state.join(format!(
                ".incoming-{}-{:?}",
                now(),
                std::thread::current().id()
            ));
            let digest = receive(&mut request, &incoming);
            let result = (|| -> Result<(Record, bool), String> {
                let id = digest?;
                if let Some(existing) = app.load(&id) {
                    return Ok((existing, false));
                }
                let dir = app.cfg.state.join(&id);
                if dir.exists() {
                    fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
                }
                fs::create_dir(&dir).map_err(|e| e.to_string())?;
                fs::rename(&incoming, dir.join("submission.fmc")).map_err(|e| e.to_string())?;
                let (spec, cell) = match unpack(&dir.join("submission.fmc"), &dir, &app.cfg.repo) {
                    Ok(x) => x,
                    Err(e) => {
                        let _ = fs::remove_dir_all(&dir);
                        return Err(e);
                    }
                };
                let circuit_sha256 = circuit_sha(&dir, &spec)?;
                for entry in fs::read_dir(&app.cfg.state).map_err(|e| e.to_string())? {
                    let other = entry
                        .map_err(|e| e.to_string())?
                        .file_name()
                        .to_string_lossy()
                        .to_string();
                    if let Some(existing) = app.load(&other) {
                        let existing_sha = if existing.circuit_sha256.is_empty() {
                            circuit_sha(&app.cfg.state.join(&other), &existing.spec)?
                        } else {
                            existing.circuit_sha256.clone()
                        };
                        if existing_sha == circuit_sha256 {
                            let _ = fs::remove_dir_all(&dir);
                            return Err(if existing.declared_cell == cell {
                                "same circuit bytes were already submitted".into()
                            } else {
                                "same circuit bytes were already submitted under a different architecture cell".into()
                            });
                        }
                    }
                }
                let record = Record {
                    id: id.clone(),
                    submitted_unix: now(),
                    state: "queued".into(),
                    spec,
                    declared_cell: cell,
                    architecture_verified: false,
                    artifact_sha256: id,
                    circuit_sha256,
                    verifier_sha256: app.cfg.verifier_sha256.clone(),
                    server_seed: None,
                    quick: None,
                    full: None,
                    error: None,
                };
                app.save(&record)?;
                Ok((record, true))
            })();
            let _ = fs::remove_file(incoming);
            match result {
                Ok((record, new)) => {
                    if new {
                        app.quick_active.fetch_add(1, Ordering::SeqCst);
                        let app = Arc::clone(app);
                        let id = record.id.clone();
                        std::thread::spawn(move || app.screen(id));
                    }
                    reply(request, 202, json!(record));
                }
                Err(e) => reply(request, 400, json!({"error": e})),
            }
        }
        _ => reply(request, 404, json!({"error": "not found"})),
    }
}

fn serve(args: &[String]) -> Result<(), String> {
    if args.len() != 4 {
        return Err(
            "usage: femoco_serve serve REPO STATE EVAL_CIRCUIT BIND (loopback only)".into(),
        );
    }
    let repo = fs::canonicalize(&args[0]).map_err(|e| e.to_string())?;
    let state = PathBuf::from(&args[1]);
    fs::create_dir_all(&state).map_err(|e| e.to_string())?;
    let state = fs::canonicalize(state).map_err(|e| e.to_string())?;
    if state.starts_with(&repo) {
        return Err("state must be outside the benchmark checkout".into());
    }
    let evaluator = fs::canonicalize(&args[2]).map_err(|e| e.to_string())?;
    let bind: SocketAddr = args[3].parse().map_err(|e| format!("bind: {e}"))?;
    if !matches!(bind.ip(), IpAddr::V4(x) if x.is_loopback())
        && !matches!(bind.ip(), IpAddr::V6(x) if x.is_loopback())
    {
        return Err("local pilot binds only to loopback".into());
    }
    let (tx, rx) = mpsc::sync_channel(32);
    let app = Arc::new(App {
        cfg: Config {
            repo,
            state,
            verifier_sha256: sha_file(&evaluator)?,
            evaluator,
            quick_samples: 1 << 12,
            full_samples: 1 << 19,
            quick_timeout: Duration::from_secs(120),
            full_timeout: Duration::from_secs(3600),
        },
        records: Mutex::new(()),
        quick_active: AtomicUsize::new(0),
        full_tx: tx,
    });
    let worker = Arc::clone(&app);
    std::thread::spawn(move || {
        for id in rx {
            worker.confirm(&id);
        }
    });
    for entry in fs::read_dir(&app.cfg.state).map_err(|e| e.to_string())? {
        let id = entry
            .map_err(|e| e.to_string())?
            .file_name()
            .to_string_lossy()
            .to_string();
        if let Some(record) = app.load(&id) {
            if record.verifier_sha256 != app.cfg.verifier_sha256 {
                if !matches!(record.state.as_str(), "accepted" | "rejected" | "failed") {
                    app.change(
                        &id,
                        "failed",
                        None,
                        Some("verifier binary changed before confirmation".into()),
                    );
                }
                continue;
            }
            match record.state.as_str() {
                "queued" | "screening" => {
                    app.quick_active.fetch_add(1, Ordering::SeqCst);
                    let next = Arc::clone(&app);
                    std::thread::spawn(move || next.screen(id));
                }
                "screened" | "confirming" => {
                    app.full_tx.send(id).map_err(|e| e.to_string())?;
                }
                _ => {}
            }
        }
    }
    let server = Server::http(bind).map_err(|e| e.to_string())?;
    eprintln!(
        "FeMoco local service on http://{}; state {}",
        server.server_addr(),
        app.cfg.state.display()
    );
    for request in server.incoming_requests() {
        handle(&app, request);
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((cmd, rest)) if cmd == "pack" => pack(rest),
        Some((cmd, rest)) if cmd == "serve" => serve(rest),
        _ => Err("usage: femoco_serve pack OPS LANEMAP FAMILY ARCHITECTURE OUT.fmc | serve REPO STATE EVAL_CIRCUIT BIND".into()),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_cells_are_bounded() {
        let a = Architecture {
            delivery: "onehot".into(),
            erasure: "gated".into(),
            outer_slot: "held".into(),
        };
        assert_eq!(a.cell().unwrap(), "onehot/gated/held");
        let bad = Architecture {
            delivery: "nonce-123".into(),
            ..a
        };
        assert!(bad.cell().is_err());
    }

    #[test]
    fn losing_architecture_remains_on_its_cell_frontier() {
        let root =
            std::env::temp_dir().join(format!("femoco-frontier-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("repo/specs/reiher-sa-v1")).unwrap();
        fs::write(root.join("repo/specs/reiher-sa-v1/spec.json"), b"{}").unwrap();
        fs::create_dir(root.join("state")).unwrap();
        let (tx, _rx) = mpsc::sync_channel(1);
        let app = App {
            cfg: Config {
                repo: root.join("repo"),
                state: root.join("state"),
                evaluator: root.join("eval"),
                verifier_sha256: String::new(),
                quick_samples: 1,
                full_samples: 1,
                quick_timeout: Duration::from_secs(1),
                full_timeout: Duration::from_secs(1),
            },
            records: Mutex::new(()),
            quick_active: AtomicUsize::new(0),
            full_tx: tx,
        };
        for (id, cell, cost, qubits) in [
            ('a', "word/plain/held", 10.0, 10.0),
            ('b', "onehot/gated/held", 20.0, 20.0),
        ] {
            let id = id.to_string().repeat(64);
            fs::create_dir(app.cfg.state.join(&id)).unwrap();
            app.save(&Record {
                id, submitted_unix: 0, state: "accepted".into(), spec: "reiher-sa-v1".into(),
                declared_cell: cell.into(), architecture_verified: false,
                artifact_sha256: String::new(), circuit_sha256: String::new(),
                verifier_sha256: String::new(), server_seed: None, quick: None,
                full: Some(json!({"score": cost * qubits, "metrics": {"lambda": 1.0,
                    "toffoli": cost, "qubits": qubits, "family": {"axes": {}}, "verified_axes": []}})),
                error: None,
            }).unwrap();
        }
        let rows = frontier(&app, "reiher-sa-v1").unwrap()["submissions"]
            .as_array()
            .unwrap()
            .clone();
        let loser = rows
            .iter()
            .find(|r| r["declared_cell"] == "onehot/gated/held")
            .unwrap();
        assert_eq!(loser["global_pareto"], false);
        assert_eq!(loser["cell_pareto"], true);
        assert_eq!(rows.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn confirmation_is_bound_to_screened_artifact_and_fresh_seed() {
        let quick = json!({"metrics": {"digests": {"ops": "a", "spec": "b"}}});
        let full = json!({"metrics": {"digests": {"ops": "a", "spec": "b"},
            "server_seed": {"seed": "random"}, "samples": 524288}});
        assert!(confirmation_matches(&quick, &full, "random", 524288));
        assert!(!confirmation_matches(&quick, &full, "changed", 524288));
        assert!(!confirmation_matches(&quick, &full, "random", 4096));
        assert!(!confirmation_matches(
            &quick,
            &json!({"metrics": {"digests": {"ops": "z"}}}),
            "random",
            524288
        ));
    }
}
