#!/usr/bin/env python3
"""Re-validate pinned circuits with ledger-key seeds, exactly as the judge validates a submission.

Every circuit pinned in tests/sa_circuits/list.rs is rebuilt by the pin test (byte-identity is
asserted there), given the lane seed the ledger key derives from its digests, screened on the
reference engine and validated in full on the sliced engine. One JSON per pin is written to the
output directory; nothing is appended to the ledger here.

    QAC_LEDGER_KEY=<hex> tools/heavy.sh python3 tools/authenticate_pins.py --out DIR [--only NAME ...]

Circuits are exported in small batches and deleted after evaluation, so disk use stays bounded.
The trusted evaluator is built once, without the editable walk code, into target/trusted.
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE.parents[1] / "tools"))

from qac import ledger  # noqa: E402
from qac.cli import circuit_digests, ledger_key  # noqa: E402
from qac.common import load_json  # noqa: E402

BATCH = 12
CARGO_TEST = ["cargo", "test", "--release", "--locked", "--test", "sa_digests", "--"]


def run(command: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=HERE, text=True, **kwargs)


def pin_names() -> list[str]:
    out = run([*CARGO_TEST, "--list"], check=True, capture_output=True).stdout
    return [line.split(":")[0] for line in out.splitlines() if line.endswith(": test")]


def evaluate(evaluator: pathlib.Path, root: pathlib.Path, seed: str, engine: str, samples: int) -> tuple[dict, float, str]:
    score = root / "score.json"
    score.unlink(missing_ok=True)
    started = time.time()
    done = subprocess.run(
        [str(evaluator), "--root", str(root), "--samples", str(samples), "--engine", engine, "--server-seed", seed],
        cwd=root, text=True, capture_output=True,
    )
    if done.returncode != 0 or not score.is_file():
        raise RuntimeError(f"{root.name}: {engine} at {samples} lanes failed\n{done.stdout[-2000:]}\n{done.stderr[-2000:]}")
    return load_json(score), time.time() - started, done.stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--only", nargs="*", default=None, help="pin names; default is every pin without a result")
    args = parser.parse_args()
    key = ledger_key(required=True)
    contract = load_json(HERE / "benchmark.json")
    screen, full = contract["validation"]["screen"], contract["validation"]["full"]
    args.out.mkdir(parents=True, exist_ok=True)
    scratch = args.out / "_export"

    trusted = HERE / "target" / "trusted"
    run(["cargo", "build", "--release", "--locked", "--no-default-features", "--bin", "eval_circuit"],
        check=True, env={**os.environ, "CARGO_TARGET_DIR": str(trusted)})
    evaluator = trusted / "release" / "eval_circuit"

    names = args.only if args.only else pin_names()
    todo = [n for n in names if not (args.out / f"{n}.json").exists()]
    print(f"{len(todo)} of {len(names)} pins to validate", flush=True)
    for start in range(0, len(todo), BATCH):
        batch = todo[start:start + BATCH]
        shutil.rmtree(scratch, ignore_errors=True)
        scratch.mkdir(parents=True)
        build_env = {k: v for k, v in os.environ.items() if not k.startswith("FEMOCO_SA_") and not k.startswith("FEMOCO_WALK_")}
        run([*CARGO_TEST, "--exact", *batch, "--test-threads=2"], check=True, capture_output=True,
            env={**build_env, "FEMOCO_PIN_EXPORT_DIR": str(scratch)})
        for name in batch:
            root = scratch / name
            pin = load_json(root / "pin.json")
            digests = circuit_digests(root)
            if digests != (pin["ops_sha256"], pin["lanemap_sha256"], pin["family_sha256"]):
                raise RuntimeError(f"{name}: exported files do not match the pin")
            for link in ("specs", "taxonomy"):
                (root / link).symlink_to(HERE / link)
            seed = ledger.derive_seed(key, *digests)
            screened, screen_s, _ = evaluate(evaluator, root, seed, screen["engine"], screen["samples"])
            validated, full_s, log = evaluate(evaluator, root, seed, full["engine"], full["samples"])
            counters = [line.strip() for line in log.splitlines() if "fallback" in line or "guard" in line]
            result = {
                "pin": pin, "seed": seed,
                "engine": f"{screen['engine']}-{screen['samples']}+{full['engine']}-{full['samples']}",
                "screen": {"toffoli": screened["metrics"]["toffoli"], "qubits": screened["metrics"]["qubits"], "seconds": round(screen_s, 1)},
                "seconds": round(full_s, 1), "counters": counters, "score": validated,
            }
            (args.out / f"{name}.json").write_text(json.dumps(result, indent=1) + "\n", encoding="utf-8")
            metrics = validated["metrics"]
            print(f"{start + batch.index(name) + 1}/{len(todo)} {name}: {metrics['toffoli']:.3f} x {metrics['qubits']} "
                  f"({screen_s:.0f} s + {full_s:.0f} s)", flush=True)
            shutil.rmtree(root)
    shutil.rmtree(scratch, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
