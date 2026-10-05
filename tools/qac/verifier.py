"""The verifier digest: one SHA-256 over every trusted file of a challenge.

A ledger row carries the digest of the evaluator that passed its circuit, so a later change to
the judge is visible in the data: rows validated by different judges have different digests.
"""
from __future__ import annotations

import hashlib
import subprocess

from .common import ROOT, Challenge, sha256_file, under


def tracked_files() -> list[str]:
    out = subprocess.run(["git", "-C", str(ROOT), "ls-files", "-z"], check=True, capture_output=True).stdout
    return sorted(p for p in out.decode("utf-8").split("\0") if p)


def trusted_files(challenge: Challenge, files: list[str] | None = None) -> list[str]:
    prefixes = challenge.repo_paths("trustedPaths")
    editable = challenge.repo_paths("editablePaths")
    files = tracked_files() if files is None else files
    return [f for f in files if under(f, prefixes) and not under(f, editable)]


def digest(challenge: Challenge, files: list[str] | None = None) -> str:
    total = hashlib.sha256(b"qac-verifier-v1\n")
    for path in trusted_files(challenge, files):
        total.update(f"{path}\0{sha256_file(ROOT / path)}\n".encode())
    return total.hexdigest()
