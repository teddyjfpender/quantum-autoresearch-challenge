#!/usr/bin/env python3
"""Rebuild submissions from their manifests and check them against the ledger.

For each ledger row, the circuit is built from nothing but `submission.json` (the track's spec
and the manifest's build knobs, through the sandboxed build stage) and the SHA-256 of its op
stream must equal the row's `ops_sha256`. This is what makes a manifest a complete recipe.

    tools/heavy.sh python3 tools/verify_submissions.py --out DIR [--only track/id ...]

One JSON line per submission is appended to DIR/verified.jsonl; finished submissions are
skipped on a re-run. Build knobs are read at compile time, so each one costs a rebuild.
"""
from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parents[1]
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from qac import ledger, manifest  # noqa: E402
from qac.common import Challenge, sha256_file  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--only", nargs="*", default=None)
    args = parser.parse_args()
    challenge = Challenge("femoco")
    args.out.mkdir(parents=True, exist_ok=True)
    log = args.out / "verified.jsonl"
    done = {json.loads(line)["submission"] for line in log.read_text().splitlines()} if log.exists() else set()
    rows = [r for r in ledger.read(challenge.ledger) if args.only is None or r["submission"] in args.only]
    todo = [r for r in rows if r["submission"] not in done]
    print(f"{len(todo)} of {len(rows)} submissions to rebuild", flush=True)
    failures = 0
    for index, row in enumerate(todo, start=1):
        directory = challenge.dir / challenge.contract["submissionPaths"][0] / row["submission"]
        checked = manifest.load(challenge, directory)
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as handle:
            json.dump(checked, handle)
        built = subprocess.run(
            [sys.executable, str(ROOT / "challenge.py"), "build", "femoco", "--manifest", handle.name],
            capture_output=True, text=True,
        )
        pathlib.Path(handle.name).unlink()
        ops = HERE / "ops.bin"
        got = sha256_file(ops) if built.returncode == 0 and ops.is_file() else None
        ok = got == row["ops_sha256"]
        failures += not ok
        with log.open("a") as out:
            out.write(json.dumps({"submission": row["submission"], "ok": ok, "ops_sha256": got}) + "\n")
        print(f"{index}/{len(todo)} {row['submission']}: {'ok' if ok else 'MISMATCH'}", flush=True)
        if not ok:
            print(built.stdout[-800:], built.stderr[-800:], sep="\n", flush=True)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
