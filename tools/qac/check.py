"""Repository contract check: what must hold on every commit of `main` and every pull request."""
from __future__ import annotations

import re

from . import ledger, manifest, site
from .common import ROOT, SLUG, ContractError, challenges, load_json, rel

BENCHMARK_KEYS = {
    "schema", "name", "title", "description", "status", "direction", "metric", "tracks", "editablePaths",
    "submissionPaths", "trustedPaths", "architectures", "targets", "ledger", "build", "validation",
    "acceptance", "setupCommand", "benchmarkCommand", "scorePath", "runner",
}
TRACK = re.compile(r"^[a-z0-9][a-z0-9-]{0,30}$")
LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)\)")


def check_markdown_links() -> list[str]:
    errors = []
    skip = {"target", "node_modules", ".git"}
    for path in ROOT.rglob("*.md"):
        if skip & set(path.relative_to(ROOT).parts):
            continue
        for target in LINK.findall(path.read_text(encoding="utf-8")):
            if re.match(r"^[a-z]+:", target) or target.startswith("#"):
                continue
            resolved = (path.parent / target.split("#")[0]).resolve()
            if not resolved.exists():
                errors.append(f"{rel(path)}: broken link {target}")
    return errors


def run(signed: bool = False) -> list[str]:
    errors: list[str] = []
    try:
        all_challenges = challenges()
    except ContractError as exc:
        return [str(exc)]
    for challenge in all_challenges:
        where = challenge.path
        contract = challenge.contract
        missing = BENCHMARK_KEYS - set(contract)
        if missing:
            errors.append(f"{where}/benchmark.json: missing keys {sorted(missing)}")
            continue
        if contract["name"] != challenge.id or contract["direction"] != "-":
            errors.append(f"{where}/benchmark.json: name must be the challenge id and direction '-'")
        for track in contract["tracks"]:
            if not TRACK.match(str(track.get("name", ""))) or not {"title", "spec", "description"} <= set(track):
                errors.append(f"{where}/benchmark.json: track {track.get('name')} needs name, title, spec and description")
        try:
            architectures = challenge.architectures()
            rows = ledger.read(challenge.ledger)
        except ContractError as exc:
            errors.append(str(exc))
            continue
        for arch_id, arch in architectures.items():
            if not SLUG.match(arch_id) or arch.get("parent") not in (None, *architectures):
                errors.append(f"{where}: architecture '{arch_id}' has a bad id or parent")
        errors += [f"{where}/{contract['ledger']}: {e}" for e in ledger.check_rows(rows, challenge.tracks, architectures, signed)]
        recorded = {r["submission"] for r in rows if r["kind"] == "submission"}
        for base in contract["submissionPaths"]:
            for track_dir in sorted((challenge.dir / base).glob("*")):
                if not track_dir.is_dir():
                    continue
                for directory in sorted(p for p in track_dir.iterdir() if p.is_dir()):
                    try:
                        manifest.load(challenge, directory, architectures)
                    except ContractError as exc:
                        errors.append(f"{rel(directory)}: {exc}")
                    recorded.discard(f"{track_dir.name}/{directory.name}")
        errors += [f"{where}: ledger row names submission '{s}', which has no directory" for s in sorted(recorded)]
        for track in contract["tracks"]:
            wanted = track.get("baseline", {}).get("submission")
            if wanted is None or not any(r["submission"] == wanted and r["track"] == track["name"] for r in rows):
                errors.append(f"{where}: track {track['name']} needs a baseline that is a ledger row of the track")
        for target in challenge.targets():
            if target.get("track") not in challenge.tracks:
                errors.append(f"{where}: target '{target.get('id')}' names an unknown track")
    try:
        activation = load_json(site.SITE / "activation.json")
        if activation.get("status") not in ("staging", "live", "closed"):
            errors.append("data/site/activation.json: status must be staging, live or closed")
        errors += [f"{p}: stale; run python3 challenge.py site" for p in site.export(check=True)]
    except ContractError as exc:
        errors.append(str(exc))
    errors += check_markdown_links()
    return errors
