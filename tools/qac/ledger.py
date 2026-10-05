"""The results ledger: one append-only TSV per challenge, one row per validated circuit.

A row is written only by the judge after the trusted evaluator has passed the circuit. Each row
names the exact circuit (SHA-256 of its op stream, lane map and family file), the evaluator that
passed it (`verifier_sha256`), the lane seed it was sampled with, and carries a chained
HMAC-SHA256 (`mac`) under the ledger key, so rows cannot be added, changed, reordered or removed
without the key. See spec/LEDGER.md.
"""
from __future__ import annotations

import hashlib
import hmac
import pathlib

from .common import HEX64, ContractError

COLUMNS = [
    "unix_time", "track", "spec", "architecture", "toffoli", "qubits", "score", "lambda_eff",
    "samples", "engine", "seed", "ops_sha256", "lanemap_sha256", "family_sha256", "family_name",
    "verifier_sha256", "commit", "pr", "author", "model", "harness", "submission", "kind",
    "standing", "status", "note", "mac",
]
KINDS = ("historical", "submission")
STANDINGS = ("new-architecture", "architecture-elite", "front")
UNSIGNED = "unsigned"
GENESIS = "0" * 64
SEED_DOMAIN = b"qac-lane-seed-v1"
MAC_DOMAIN = b"qac-ledger-row-v1"


def derive_seed(key: bytes, ops: str, lanemap: str, family: str) -> str:
    """The lane seed of a circuit: unpredictable without the key, reproducible with it, and
    published in the row so anyone can re-run the exact evaluation."""
    for digest in (ops, lanemap, family):
        if not HEX64.match(digest):
            raise ContractError("derive_seed: SHA-256 hex digests are required")
    message = SEED_DOMAIN + b"|" + "|".join((ops, lanemap, family)).encode()
    return hmac.new(key, message, hashlib.sha256).hexdigest()


def clean(text) -> str:
    return " ".join(str(text).replace("\t", " ").split())


def row_message(row: dict) -> bytes:
    return "\t".join(clean(row[c]) for c in COLUMNS if c != "mac").encode("utf-8")


def mac(key: bytes, previous: str, row: dict) -> str:
    return hmac.new(key, MAC_DOMAIN + b"|" + previous.encode() + b"|" + row_message(row), hashlib.sha256).hexdigest()


def read(path: pathlib.Path) -> list[dict]:
    if not path.exists():
        return []
    lines = path.read_text(encoding="utf-8").split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    if not lines or lines[0].split("\t") != COLUMNS:
        raise ContractError(f"{path.name}: the header must be exactly the ledger columns")
    rows = []
    for number, line in enumerate(lines[1:], start=2):
        cells = line.split("\t")
        if len(cells) != len(COLUMNS):
            raise ContractError(f"{path.name}:{number}: {len(cells)} cells, {len(COLUMNS)} expected")
        rows.append(dict(zip(COLUMNS, cells)))
    return rows


def write(path: pathlib.Path, rows: list[dict]) -> None:
    lines = ["\t".join(COLUMNS)] + ["\t".join(clean(r[c]) for c in COLUMNS) for r in rows]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def sign(key: bytes | None, rows: list[dict]) -> None:
    """Recompute the MAC chain in place (ledger generation; the bot signs appended rows)."""
    previous = GENESIS
    for row in rows:
        row["mac"] = mac(key, previous, row) if key else UNSIGNED
        previous = row["mac"]


def verify_chain(key: bytes, rows: list[dict]) -> list[str]:
    errors, previous = [], GENESIS
    for number, row in enumerate(rows, start=2):
        if not hmac.compare_digest(row["mac"], mac(key, previous, row)):
            errors.append(f"line {number}: MAC does not verify")
        previous = row["mac"]
    return errors


def _f(row: dict, column: str) -> float:
    return float(row[column])


def check_rows(rows: list[dict], tracks: dict[str, dict], architectures: dict[str, dict], signed: bool) -> list[str]:
    """Static integrity of a ledger: formats, track and architecture names, ordering, uniqueness."""
    errors: list[str] = []
    seen: set[tuple[str, str]] = set()
    last_time = 0
    for number, row in enumerate(rows, start=2):
        where = f"line {number}"
        try:
            when, toffoli, qubits, score = int(row["unix_time"]), _f(row, "toffoli"), int(row["qubits"]), _f(row, "score")
            lam, samples = _f(row, "lambda_eff"), int(row["samples"])
        except ValueError:
            errors.append(f"{where}: unreadable number")
            continue
        if when < last_time:
            errors.append(f"{where}: rows must be in time order")
        last_time = max(last_time, when)
        track = tracks.get(row["track"])
        if track is None or row["spec"] != track["spec"]:
            errors.append(f"{where}: unknown track or wrong spec")
        if row["architecture"] not in architectures:
            errors.append(f"{where}: architecture '{row['architecture']}' is not in the registry")
        if toffoli <= 0 or qubits <= 0 or samples <= 0 or lam <= 0:
            errors.append(f"{where}: counts must be positive")
        elif abs(score - lam * toffoli * qubits) > 1e-5 * score:
            errors.append(f"{where}: score is not lambda_eff x toffoli x qubits")
        for column in ("seed", "ops_sha256", "lanemap_sha256", "family_sha256", "verifier_sha256"):
            if not HEX64.match(row[column]):
                errors.append(f"{where}: {column} must be 64 hex characters")
        if row["kind"] not in KINDS or row["status"] != "OK":
            errors.append(f"{where}: kind must be one of {KINDS} and status OK")
        if any(s not in STANDINGS for s in row["standing"].split(",") if s):
            errors.append(f"{where}: unknown standing")
        key = (row["track"], row["ops_sha256"])
        if key in seen:
            errors.append(f"{where}: the same circuit is already in the ledger")
        seen.add(key)
        if signed and not HEX64.match(row["mac"]):
            errors.append(f"{where}: unsigned row")
    return errors


def standing(rows: list[dict], candidate: dict, min_improvement_bips: int) -> tuple[list[str], str]:
    """Where a validated circuit stands against the ledger. Returns (standings, reason).

    Architecture comes first: a circuit is recorded if it is the first of its architecture, or
    beats that architecture's best score, or advances the track's (Toffolis, qubits) front. The
    margin `min_improvement_bips` keeps gains inside sampling noise from counting.
    """
    eps = min_improvement_bips / 10_000
    track = [r for r in rows if r["track"] == candidate["track"]]
    if any(r["ops_sha256"] == candidate["ops_sha256"] for r in track):
        return [], "this exact circuit is already in the ledger"
    toffoli, qubits, score = _f(candidate, "toffoli"), int(candidate["qubits"]), _f(candidate, "score")
    result = []
    same = [r for r in track if r["architecture"] == candidate["architecture"]]
    if not same:
        result.append("new-architecture")
    elif score < min(_f(r, "score") for r in same) * (1 - eps):
        result.append("architecture-elite")
    if all(toffoli < _f(r, "toffoli") * (1 - eps) for r in track if int(r["qubits"]) <= qubits):
        result.append("front")
    if result:
        return result, ""
    best = min(same, key=lambda r: _f(r, "score"))
    return [], (
        f"valid, but it neither beats the best {candidate['architecture']} score "
        f"({_f(best, 'toffoli'):,.1f} Toffolis x {best['qubits']} qubits) by {min_improvement_bips / 100:g}% "
        f"nor has {min_improvement_bips / 100:g}% fewer Toffolis than every recorded circuit with at most {qubits} qubits"
    )


def row_from_score(score: dict, *, track: dict, architecture: str, engine: str, seed: str, verifier: str,
                   unix_time: int, commit: str, pr: str, author: str, model: str, harness: str,
                   submission: str, kind: str, note: str) -> dict:
    """A ledger row from the trusted evaluator's score.json."""
    metrics = score["metrics"]
    if metrics["spec"] != track["spec"]:
        raise ContractError(f"score.json is for spec {metrics['spec']}, the track needs {track['spec']}")
    digests = metrics["digests"]
    return {
        "unix_time": str(unix_time), "track": track["name"], "spec": track["spec"], "architecture": architecture,
        "toffoli": f"{float(metrics['toffoli']):.3f}", "qubits": str(int(metrics["qubits"])),
        "score": f"{float(score['score']):.6e}",
        # The normalisation the evaluator used in the score: score / (toffoli x qubits).
        "lambda_eff": f"{float(score['score']) / (float(metrics['toffoli']) * int(metrics['qubits'])):.6f}",
        "samples": str(int(metrics["samples"])), "engine": engine, "seed": seed,
        "ops_sha256": digests["ops"], "lanemap_sha256": digests["lanemap"], "family_sha256": digests["family"],
        "family_name": metrics["family"]["name"], "verifier_sha256": verifier, "commit": commit, "pr": pr,
        "author": author, "model": model, "harness": harness, "submission": submission, "kind": kind,
        "standing": "", "status": "OK", "note": clean(note), "mac": UNSIGNED,
    }
