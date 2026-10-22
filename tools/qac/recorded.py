"""Recorded circuits that no pin test covers.

Every circuit in the ledger must still build byte for byte with the circuit code of a pull
request (spec/SUBMISSIONS.md, step 3). The starting board is pinned by the challenge's test
suite; a row whose manifest names no `pin` (every judged submission, and the one historical
circuit its manifest alone reproduces) is rebuilt from its `submission.json` through the same
sandboxed build stage the judge used, and the three digests must equal the row's. Build knobs
are read when the builder is compiled, so each circuit costs a rebuild; the judge runs this
only when a pull request changes circuit code.
"""
from __future__ import annotations

from collections.abc import Callable, Iterable

from . import ledger, manifest
from .common import Challenge, ContractError

DIGESTS = (("ops_sha256", "ops.bin"), ("lanemap_sha256", "lanemap.bin"), ("family_sha256", "family.out.json"))
Build = Callable[[dict], tuple[str, str, str]]


def unpinned(challenge: Challenge, only: Iterable[str] | None = None) -> list[tuple[dict, dict]]:
    """`(row, checked manifest)` of every ledger row whose manifest names no pin test."""
    wanted = None if only is None else set(only)
    found = []
    for row in ledger.read(challenge.ledger):
        if wanted is not None and row["submission"] not in wanted:
            continue
        directory = challenge.dir / challenge.contract["submissionPaths"][0] / row["submission"]
        checked = manifest.load(challenge, directory)
        if checked.get("pin"):
            continue
        if checked["track"] != row["track"]:
            raise ContractError(f"{row['submission']}: the manifest's track differs from its ledger row")
        found.append((row, checked))
    return found


def compare(row: dict, digests: tuple[str, str, str]) -> list[str]:
    """The circuit files whose digest differs from the row, as messages."""
    return [
        f"{row['submission']}: {name} rebuilt as {got[:16]}…, the ledger row has {row[column][:16]}…"
        for (column, name), got in zip(DIGESTS, digests)
        if got != row[column]
    ]


def rebuild(challenge: Challenge, build: Build, only: Iterable[str] | None = None, log: Callable[[str], None] = print) -> list[str]:
    """Rebuild every unpinned recorded circuit with `build(checked manifest) -> digests` and
    return the mismatches and build failures; an empty list means every row still reproduces."""
    targets = unpinned(challenge, only)
    log(f"{len(targets)} recorded circuit(s) without a pin to rebuild")
    errors: list[str] = []
    for index, (row, checked) in enumerate(targets, start=1):
        log(f"[{index}/{len(targets)}] {row['submission']}: {checked['build']}")
        try:
            digests = build(checked)
        except ContractError as exc:
            errors.append(f"{row['submission']}: {exc}")
            continue
        errors += compare(row, digests)
    return errors
