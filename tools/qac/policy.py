"""Path policy for a submission pull request.

A submission may change only the challenge's editable circuit code, add exactly one new
submission directory, and (to propose a new architecture) append to the architecture registry.
Everything else in the repository is the judge and is refused.

Input is `git diff --raw --no-renames -z BASE HEAD`, read as untrusted data.
"""
from __future__ import annotations

import dataclasses
import json
import subprocess

from .common import SLUG, Challenge, ContractError, challenges, under

REGULAR = "100644"
ABSENT = "000000"
SUBMISSION_FILES = {"submission.json", "NOTES.md"}
MAX_CHANGED_PATHS = 400


@dataclasses.dataclass
class Change:
    status: str
    old_mode: str
    new_mode: str
    path: str


@dataclasses.dataclass
class Verdict:
    is_submission: bool
    challenge: str | None = None
    track: str | None = None
    submission_id: str | None = None
    submission_dir: str | None = None
    touches_code: bool = False
    proposes_architecture: bool = False
    errors: list[str] = dataclasses.field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.errors

    def to_json(self) -> str:
        return json.dumps({**dataclasses.asdict(self), "ok": self.ok}, indent=1)


def parse_raw(raw: bytes) -> list[Change]:
    """Parse `git diff --raw -z` output: ':old new oldsha newsha S\\0path\\0' records."""
    fields = raw.split(b"\0")
    changes = []
    i = 0
    while i + 1 < len(fields):
        meta = fields[i].decode("ascii", "replace")
        if not meta.startswith(":"):
            raise ContractError("unreadable diff record")
        parts = meta[1:].split()
        if len(parts) != 5:
            raise ContractError("unreadable diff record")
        path = fields[i + 1].decode("utf-8", "strict")
        changes.append(Change(parts[4][0], parts[0], parts[1], path))
        i += 2
    return changes


def diff(repo: str, base: str, head: str) -> list[Change]:
    raw = subprocess.run(
        ["git", "-C", repo, "diff", "--raw", "--no-renames", "--no-abbrev", "-z", base, head],
        check=True, capture_output=True,
    ).stdout
    return parse_raw(raw)


def _safe(path: str) -> bool:
    return bool(path) and not path.startswith("/") and ".." not in path.split("/") and "\\" not in path and all(32 <= ord(c) < 127 for c in path)


def evaluate(changes: list[Change], all_challenges: list[Challenge] | None = None) -> Verdict:
    all_challenges = all_challenges if all_challenges is not None else challenges()
    owners = [c for c in all_challenges if any(under(ch.path, c.repo_paths("submissionPaths")) for ch in changes)]
    if not owners:
        return Verdict(is_submission=False)
    verdict = Verdict(is_submission=True)
    if len(owners) > 1:
        verdict.errors.append("a pull request may submit to one challenge only")
        return verdict
    challenge = owners[0]
    verdict.challenge = challenge.id
    editable = challenge.repo_paths("editablePaths")
    submissions = challenge.repo_paths("submissionPaths")
    registry = f"{challenge.path}/{challenge.contract['architectures']}"
    if len(changes) > MAX_CHANGED_PATHS:
        verdict.errors.append(f"more than {MAX_CHANGED_PATHS} changed paths")
    dirs: set[tuple[str, str]] = set()
    for change in changes:
        path = change.path
        if not _safe(path):
            verdict.errors.append(f"unusual path: {path!r}")
            continue
        if change.status not in "AMD":
            verdict.errors.append(f"{path}: only additions, modifications and deletions are allowed")
            continue
        if change.status != "D" and change.new_mode != REGULAR:
            verdict.errors.append(f"{path}: only regular non-executable files are allowed (mode {change.new_mode})")
        if change.status != "A" and change.old_mode not in (REGULAR, ABSENT):
            verdict.errors.append(f"{path}: replaces a file that is not a regular file")
        if under(path, submissions):
            prefix = next(p for p in submissions if under(path, [p]))
            parts = path[len(prefix) + 1:].split("/")
            if len(parts) != 3 or parts[2] not in SUBMISSION_FILES:
                verdict.errors.append(f"{path}: a submission adds <track>/<id>/submission.json and <track>/<id>/NOTES.md only")
                continue
            if change.status != "A":
                verdict.errors.append(f"{path}: existing submissions are immutable; add a new submission directory")
            dirs.add((parts[0], parts[1]))
            if parts[0] not in challenge.tracks:
                verdict.errors.append(f"{path}: unknown track '{parts[0]}'")
            if not SLUG.match(parts[1]):
                verdict.errors.append(f"{path}: submission id must be 3-64 characters of a-z, 0-9 and '-'")
            if len(dirs) == 1:
                verdict.track, verdict.submission_id = parts[0], parts[1]
                verdict.submission_dir = f"{prefix}/{parts[0]}/{parts[1]}"
        elif path == registry:
            if change.status != "M":
                verdict.errors.append(f"{path}: the architecture registry may only be appended to")
            verdict.proposes_architecture = True
        elif under(path, editable):
            verdict.touches_code = True
        else:
            verdict.errors.append(f"{path}: outside the editable surface ({', '.join(editable)})")
    if len(dirs) != 1:
        verdict.errors.append(f"exactly one new submission directory is required; found {len(dirs)}")
    return verdict


def registry_is_append_only(old: dict, new: dict) -> list[str]:
    """New registry keeps every existing architecture unchanged, in order, and only adds entries."""
    errors = []
    if {k: v for k, v in old.items() if k != "architectures"} != {k: v for k, v in new.items() if k != "architectures"}:
        errors.append("architectures.json: only the architectures list may change")
    before, after = old.get("architectures", []), new.get("architectures", [])
    if after[: len(before)] != before:
        errors.append("architectures.json: existing architectures must stay unchanged and in order")
    added = after[len(before):]
    if not added:
        errors.append("architectures.json: changed without adding an architecture")
    ids = [a.get("id") for a in after]
    if len(ids) != len(set(ids)):
        errors.append("architectures.json: duplicate architecture id")
    for entry in added:
        required = {"id", "name", "mechanism", "distinguishing", "references"}
        if not isinstance(entry, dict) or set(entry) - required - {"parent"} or required - set(entry):
            errors.append(f"architectures.json: a new architecture needs exactly {sorted(required)} (and optionally 'parent')")
            continue
        if not SLUG.match(str(entry["id"])):
            errors.append("architectures.json: architecture id must be a lowercase slug")
        if len(str(entry["mechanism"])) < 200 or len(str(entry["distinguishing"])) < 100:
            errors.append("architectures.json: 'mechanism' (200+ characters) and 'distinguishing' (100+) must say what is structurally new")
        if entry.get("parent") is not None and entry["parent"] not in ids:
            errors.append("architectures.json: 'parent' must be an existing architecture id")
    return errors
