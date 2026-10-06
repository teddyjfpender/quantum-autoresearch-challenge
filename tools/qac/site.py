"""The website feed: derived JSON under data/site/, rebuilt from the ledgers.

`sources.json` and each `<challenge>/leaderboard.json` are derived and never edited by hand;
`activation.json` and each `<challenge>/challenge.json` are authored. `sources.json` is the single
entry point a site reads; it names every other file. All figures come from the ledger rows.
"""
from __future__ import annotations

import json
import pathlib

from . import ledger
from .common import ROOT, Challenge, challenges

SITE = ROOT / "data" / "site"


def _row(row: dict) -> dict:
    toffoli, qubits = float(row["toffoli"]), int(row["qubits"])
    return {
        "unixTime": int(row["unix_time"]), "track": row["track"], "architecture": row["architecture"],
        "toffoli": toffoli, "qubits": qubits, "toffoliTimesQubits": round(toffoli * qubits, 3),
        "score": float(row["score"]), "samples": int(row["samples"]), "engine": row["engine"],
        "opsSha256": row["ops_sha256"], "verifierSha256": row["verifier_sha256"], "commit": row["commit"],
        "pr": int(row["pr"]) if row["pr"].isdigit() else None, "author": row["author"] or None,
        "model": row["model"] or None, "harness": row["harness"] or None, "submission": row["submission"],
        "kind": row["kind"], "standing": [s for s in row["standing"].split(",") if s], "note": row["note"],
    }


def _front(rows: list[dict]) -> list[dict]:
    """The (qubits, Toffolis) Pareto front: fewest Toffolis at each qubit count, strictly improving."""
    front, best = [], float("inf")
    for row in sorted(rows, key=lambda r: (r["qubits"], r["toffoli"])):
        if row["toffoli"] < best:
            front.append(row)
            best = row["toffoli"]
    return front


def _running_best(rows: list[dict]) -> list[dict]:
    best, steps = float("inf"), []
    for row in rows:
        if row["score"] < best:
            best = row["score"]
            steps.append(row)
    return steps


def leaderboard(challenge: Challenge) -> dict:
    raw = ledger.read(challenge.ledger)
    rows = [_row(r) for r in raw]
    architectures = challenge.architectures()
    targets = challenge.targets()
    tracks = []
    for name, track in challenge.tracks.items():
        mine = [r for r in rows if r["track"] == name]
        by_arch = []
        for arch_id, arch in architectures.items():
            circuits = [r for r in mine if r["architecture"] == arch_id]
            if not circuits:
                continue
            by_arch.append({
                "id": arch_id, "name": arch["name"], "parent": arch.get("parent"),
                "circuits": len(circuits), "firstUnixTime": min(r["unixTime"] for r in circuits),
                "elite": min(circuits, key=lambda r: (r["score"], r["qubits"])),
                "fewestQubits": min(circuits, key=lambda r: (r["qubits"], r["toffoli"])),
                "fewestToffoli": min(circuits, key=lambda r: (r["toffoli"], r["qubits"])),
                "history": _running_best(circuits),
            })
        by_arch.sort(key=lambda a: a["elite"]["score"])
        best = min(mine, key=lambda r: (r["score"], r["qubits"])) if mine else None
        baseline = next((r for r in mine if r["submission"] == track.get("baseline", {}).get("submission")), None)
        tracks.append({
            "track": name, "title": track["title"], "spec": track["spec"],
            "circuits": len(mine), "best": best,
            # The construction the challenge starts from, as this evaluator counts it, and how far
            # below it the best circuit is (a share of the baseline score, 0..1).
            "baseline": baseline,
            "belowBaseline": round(1 - best["score"] / baseline["score"], 6) if best and baseline else None,
            "architectures": by_arch, "front": _front(mine), "history": _running_best(mine),
            "targets": [t for t in targets if t["track"] == name],
        })
    return {
        "schema": "qac-leaderboard-v1", "challenge": challenge.id,
        "metric": challenge.contract["metric"], "ledger": f"{challenge.path}/{challenge.contract['ledger']}",
        # The row count and the last row's MAC: a later ledger must extend this one.
        "ledgerRows": len(rows), "ledgerHead": raw[-1]["mac"] if raw else None, "tracks": tracks,
    }


def sources(all_challenges: list[Challenge]) -> dict:
    return {
        "schema": "qac-site-sources-v1",
        "registry": "challenges.json",
        "activation": "data/site/activation.json",
        "challenges": {
            c.id: {
                "benchmark": f"{c.path}/benchmark.json",
                "architectures": f"{c.path}/{c.contract['architectures']}",
                "targets": f"{c.path}/{c.contract.get('targets', 'targets.json')}",
                "ledger": f"{c.path}/{c.contract['ledger']}",
                "leaderboard": f"data/site/{c.id}/leaderboard.json",
                "content": f"data/site/{c.id}/challenge.json",
            }
            for c in all_challenges
        },
    }


def render(value) -> str:
    return json.dumps(value, indent=1, ensure_ascii=False) + "\n"


def outputs() -> dict[pathlib.Path, str]:
    all_challenges = challenges()
    files = {SITE / "sources.json": render(sources(all_challenges))}
    for challenge in all_challenges:
        files[SITE / challenge.id / "leaderboard.json"] = render(leaderboard(challenge))
    return files


def export(check: bool = False) -> list[str]:
    """Write the derived files, or with `check` return the ones that are stale."""
    stale = []
    for path, text in outputs().items():
        if not path.exists() or path.read_text(encoding="utf-8") != text:
            stale.append(str(path.relative_to(ROOT)))
            if not check:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
    return stale
