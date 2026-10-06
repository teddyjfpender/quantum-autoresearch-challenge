"""Shared helpers: the challenge registry, per-challenge contracts and small file utilities.

Everything under tools/qac is trusted judge code. It uses the Python standard library only.
"""
from __future__ import annotations

import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[2]
REGISTRY = ROOT / "challenges.json"
SLUG = re.compile(r"\A[a-z0-9][a-z0-9-]{1,62}[a-z0-9]\Z")
HEX64 = re.compile(r"\A[0-9a-f]{64}\Z")
LOGIN = re.compile(r"\A[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?(?:\[bot\])?\Z")


class ContractError(Exception):
    """A file does not meet the contract it claims to follow."""


def load_json(path: pathlib.Path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ContractError(f"{rel(path)}: {exc}") from exc


def rel(path: pathlib.Path) -> str:
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def registry() -> list[dict]:
    data = load_json(REGISTRY)
    if data.get("schema") != "qac-registry-v1":
        raise ContractError("challenges.json: schema must be qac-registry-v1")
    return data["challenges"]


class Challenge:
    """One challenge directory and its contract (`benchmark.json`)."""

    def __init__(self, challenge_id: str):
        entry = next((c for c in registry() if c["id"] == challenge_id), None)
        if entry is None:
            raise ContractError(f"unknown challenge: {challenge_id}")
        self.id = challenge_id
        self.entry = entry
        self.dir = ROOT / entry["path"]
        self.path = entry["path"].rstrip("/")
        self.contract = load_json(self.dir / "benchmark.json")
        if self.contract.get("schema") != "qac-benchmark-v1":
            raise ContractError(f"{self.path}/benchmark.json: schema must be qac-benchmark-v1")
        self.tracks = {t["name"]: t for t in self.contract["tracks"]}

    @property
    def ledger(self) -> pathlib.Path:
        return self.dir / self.contract["ledger"]

    @property
    def architectures_path(self) -> pathlib.Path:
        return self.dir / self.contract["architectures"]

    def architectures(self) -> dict[str, dict]:
        data = load_json(self.architectures_path)
        if data.get("schema") != "qac-architectures-v1":
            raise ContractError(f"{rel(self.architectures_path)}: schema must be qac-architectures-v1")
        return {a["id"]: a for a in data["architectures"]}

    def targets(self) -> list[dict]:
        path = self.dir / self.contract.get("targets", "targets.json")
        return load_json(path)["targets"] if path.exists() else []

    def repo_paths(self, key: str) -> list[str]:
        """Contract paths (relative to the challenge) as repository-relative prefixes."""
        return [f"{self.path}/{p.strip('/')}" for p in self.contract.get(key, [])]


def challenges() -> list[Challenge]:
    return [Challenge(c["id"]) for c in registry()]


def under(path: str, prefixes: list[str]) -> bool:
    return any(path == p or path.startswith(p + "/") for p in prefixes)
