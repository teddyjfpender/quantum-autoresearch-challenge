"""The submission manifest: `submissions/<track>/<id>/submission.json` with a `NOTES.md` beside it.

The manifest names the track, the declared architecture, the build knobs and who (and what) made
the circuit. The judge reads it as untrusted data.
"""
from __future__ import annotations

import pathlib
import re

from .common import LOGIN, SLUG, Challenge, ContractError, load_json

SCHEMA = "qac-submission-v1"
REQUIRED = ("schema", "challenge", "track", "architecture", "title", "build", "authors", "model", "harness")
OPTIONAL = ("claimed", "parents", "discussion", "pin")
TEXT_MAX = 200
NOTES_MIN_BYTES = 1024
NOTES_MAX_BYTES = 100 * 1024
ENV_VALUE = re.compile(r"^[A-Za-z0-9_+.,:=~-]{0,256}$")


def _text(value, name: str, limit: int = TEXT_MAX) -> str:
    if not isinstance(value, str) or not value.strip() or len(value) > limit:
        raise ContractError(f"{name}: a non-empty string of at most {limit} characters is required")
    if any(c in value for c in "\t\r\n"):
        raise ContractError(f"{name}: tabs and line breaks are not allowed")
    return value.strip()


def validate(challenge: Challenge, track: str, submission_id: str, data: dict, architectures: dict[str, dict]) -> dict:
    """Return the manifest in checked, normalised form or raise ContractError."""
    if not isinstance(data, dict):
        raise ContractError("submission.json: a JSON object is required")
    unknown = sorted(set(data) - set(REQUIRED) - set(OPTIONAL))
    if unknown:
        raise ContractError(f"submission.json: unknown keys {unknown}")
    missing = [k for k in REQUIRED if k not in data]
    if missing:
        raise ContractError(f"submission.json: missing keys {missing}")
    if data["schema"] != SCHEMA:
        raise ContractError(f"submission.json: schema must be {SCHEMA}")
    if data["challenge"] != challenge.id:
        raise ContractError(f"submission.json: challenge must be {challenge.id}")
    if data["track"] != track or track not in challenge.tracks:
        raise ContractError("submission.json: track must match the directory it is in and exist in benchmark.json")
    if not SLUG.match(submission_id):
        raise ContractError("submission id: 3-64 characters of a-z, 0-9 and '-', not starting or ending with '-'")
    architecture = data["architecture"]
    if architecture not in architectures:
        raise ContractError(
            f"submission.json: architecture '{architecture}' is not in the registry; add it to architectures.json "
            "in the same pull request to propose a new one"
        )
    build = data["build"]
    allowed = re.compile(challenge.contract["build"]["envPattern"])
    reserved = set(challenge.contract["build"].get("reservedEnv", []))
    if not isinstance(build, dict):
        raise ContractError("submission.json: build must be an object of environment knobs")
    for key, value in build.items():
        if not isinstance(key, str) or not allowed.match(key) or key in reserved:
            raise ContractError(f"submission.json: build knob '{key}' is not allowed")
        if not isinstance(value, str) or not ENV_VALUE.match(value):
            raise ContractError(f"submission.json: build knob '{key}' has a value outside [A-Za-z0-9_+.,:=~-]")
    authors = data["authors"]
    if not isinstance(authors, list) or not 1 <= len(authors) <= 8 or not all(isinstance(a, str) and LOGIN.match(a) for a in authors):
        raise ContractError("submission.json: authors must be 1-8 GitHub logins")
    claimed = data.get("claimed") or {}
    if not isinstance(claimed, dict) or any(k not in ("toffoli", "qubits") or not isinstance(v, (int, float)) or v <= 0 for k, v in claimed.items()):
        raise ContractError("submission.json: claimed may hold positive 'toffoli' and 'qubits' only")
    parents = data.get("parents") or []
    if not isinstance(parents, list) or len(parents) > 8 or not all(isinstance(p, str) and SLUG.match(p) for p in parents):
        raise ContractError("submission.json: parents must be up to 8 submission ids")
    discussion = data.get("discussion")
    if discussion is not None and not (isinstance(discussion, str) and re.match(r"^https://github\.com/[\w.-]+/[\w.-]+/discussions/\d+$", discussion)):
        raise ContractError("submission.json: discussion must be a GitHub Discussion URL")
    pin = data.get("pin")
    if pin is not None and not (isinstance(pin, str) and re.match(r"^[a-z0-9_]{1,64}$", pin)):
        raise ContractError("submission.json: pin must name a byte-identity pin of the challenge's test suite")
    return {
        "schema": SCHEMA,
        "challenge": challenge.id,
        "track": track,
        "id": submission_id,
        "architecture": architecture,
        "title": _text(data["title"], "title"),
        "build": dict(sorted(build.items())),
        "authors": authors,
        "model": _text(data["model"], "model"),
        "harness": _text(data["harness"], "harness"),
        "claimed": claimed,
        "parents": parents,
        "discussion": discussion,
        "pin": pin,
    }


def load(challenge: Challenge, directory: pathlib.Path, architectures: dict[str, dict] | None = None) -> dict:
    """Load and check the manifest and notes in `submissions/<track>/<id>/`."""
    track, submission_id = directory.parent.name, directory.name
    manifest = validate(
        challenge, track, submission_id, load_json(directory / "submission.json"),
        architectures if architectures is not None else challenge.architectures(),
    )
    notes = directory / "NOTES.md"
    if not notes.is_file():
        raise ContractError(f"{directory.name}: NOTES.md is required")
    size = notes.stat().st_size
    if not NOTES_MIN_BYTES <= size <= NOTES_MAX_BYTES:
        raise ContractError(f"{directory.name}/NOTES.md: {size} bytes; between {NOTES_MIN_BYTES} and {NOTES_MAX_BYTES} are required")
    return manifest
