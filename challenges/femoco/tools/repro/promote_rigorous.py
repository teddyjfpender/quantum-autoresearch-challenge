#!/usr/bin/env python3
"""Rebuild and revalidate rigorous frontier candidates without writing the signed ledger.

This is local maintainer tooling. Run the ordinary sandboxed build_circuit/judge path for
an official submission; this bulk runner must never be used to authenticate a ledger row.

    tools/repro/promote_rigorous.py --out-root DIR --exact reiher-fewest-toffoli-rigorous \\
        --jobs 4 --write-evidence

rebuilds every candidate of rigorous/promotions.json, runs the reference screen and the full
sliced stage with deterministic coverage, runs the symbolic checker (tools/verification/exact.py)
on the candidates named by --exact, and with --write-evidence replaces
rigorous/validation-results.json and rigorous/exact-<candidate>.json. Every field of those files
is written here; nothing in them is entered by hand. Evidence is written only from a clean
checkout, so `source_commit` names the code that produced it.
"""
from __future__ import annotations
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXACT = ROOT / "tools/verification/exact.py"


def output(command):
    return subprocess.run(command, cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def environment(env):
    """Where and from what the evidence was produced."""
    dirty = output(["git", "status", "--porcelain", "--", "."])
    try:
        import dd
        dd_version = dd.__version__
    except ImportError:
        dd_version = None
    return {
        "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d"),
        "source_commit": output(["git", "rev-parse", "HEAD"]),
        "source_clean": not dirty,
        "rustc": output(["rustc", "--version"]),
        "python": platform.python_version(),
        "dd": dd_version,
        "platform": f"{platform.system()} {platform.machine()}",
        "rayon_threads": int(env["RAYON_NUM_THREADS"]),
    }


def exact_name(candidate):
    return "exact-" + candidate.removesuffix("-rigorous") + ".json"


def run(args):
    args.out_root.mkdir(parents=True, exist_ok=True)
    cat = json.loads((ROOT / "rigorous/promotions.json").read_text())
    selected = [c for c in cat["candidates"] if not args.id or c["id"] in args.id]
    if not selected or set(args.id or []) - {c["id"] for c in selected}:
        raise ValueError("unknown or empty candidate selection")
    env = dict(os.environ)
    # Compile-time environment from an earlier experiment must not contaminate a promotion.
    for key in list(env):
        if key.startswith("FEMOCO_"): del env[key]
    env.setdefault("RAYON_NUM_THREADS", "4")
    cargo = args.cargo
    where = environment(env)
    if args.write_evidence and not where["source_clean"]:
        raise ValueError("commit the code first: evidence is written only from a clean checkout")
    if args.write_evidence and (args.id or args.coverage != "terms"):
        raise ValueError("evidence covers every candidate with terms coverage")
    subprocess.run([cargo, "build", "--release", "--locked", "--no-default-features", "--bin", "eval_circuit"], cwd=ROOT, env=env, check=True)
    # Copy the trusted no-walk binary BEFORE the builder feature build.
    evaluator = args.out_root / "eval_circuit"
    shutil.copy2(ROOT / "target/release/eval_circuit", evaluator)
    subprocess.run([cargo, "build", "--release", "--locked", "--bin", "build_rigorous"], cwd=ROOT, env=env, check=True)
    results = []
    for c in selected:
        subprocess.run([str(ROOT / "target/release/build_rigorous"), str(args.out_root), c["id"]], cwd=ROOT, env=env, check=True)
        directory = args.out_root / c["track"] / c["id"]
        for name in ("specs", "taxonomy"):
            target = directory / name
            if not target.exists(): target.symlink_to(ROOT / name, target_is_directory=True)
        command = [str(evaluator), "--root", str(directory), "--samples", str(args.samples), "--engine", "sliced"]
        if args.coverage: command += ["--coverage", args.coverage]
        exact = c["id"] in args.exact
        if args.symbolic or exact: command += ["--export-symbolic", str(directory / "symbolic-input.json")]
        screen = [str(evaluator), "--root", str(directory), "--samples", str(args.screen_samples), "--engine", "reference"]
        with (directory / "screen.log").open("w") as log:
            status = subprocess.run(screen, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
        screen_status = status
        # Leave the diagnostics beside each candidate. A failed candidate is recorded as a
        # failure and the runner continues to expose the surviving rigorous frontier.
        if status == 0:
            (directory / "score.json").unlink(missing_ok=True)
            started = time.monotonic()
            with (directory / "evaluation.log").open("w") as log:
                status = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
            seconds = round(time.monotonic() - started, 1)
        result = {"id": c["id"], "track": c["track"], "source": c["source"],
                  "status": "passed" if status == 0 else "failed",
                  "reference_screen": {"status": "passed" if screen_status == 0 else "failed", "samples": args.screen_samples}}
        if status == 0:
            metrics = json.loads((directory / "score.json").read_text())["metrics"]
            for key in ("spec", "toffoli", "qubits", "samples", "rounding_error", "rounding_error_exact", "digests", "validation"):
                if key in metrics: result[key] = metrics[key]
            result["product"] = metrics["toffoli"] * metrics["qubits"]
            result["full_stage_seconds"] = seconds
            if exact:
                proof = directory / exact_name(c["id"])
                with (directory / "exact.log").open("w") as log:
                    code = subprocess.run(
                        [sys.executable, str(EXACT), str(directory / "symbolic-input.json"),
                         "--payload", str(ROOT / "specs" / c["spec"] / "sa.bin"),
                         "--circuit", str(directory), "--out", str(proof), "--jobs", str(args.jobs)],
                        stdout=log, stderr=subprocess.STDOUT, check=False).returncode
                report = json.loads(proof.read_text()) if proof.exists() else {}
                result["exact"] = {
                    "report": exact_name(c["id"]),
                    "certified": code == 0 and report.get("certified") is True,
                    "checker_sha256": hashlib.sha256(EXACT.read_bytes()).hexdigest(),
                }
                if not result["exact"]["certified"]:
                    status, result["status"] = 1, "failed"
                    result["reason"] = "the symbolic checker did not certify the circuit"
            if args.symbolic:
                proof = directory / "symbolic-report.json"
                proof.unlink(missing_ok=True)
                try:
                    with (directory / "symbolic.log").open("w") as log:
                        subprocess.run([sys.executable, str(ROOT / "tools/verification/symbolic.py"),
                                        str(directory / "symbolic-input.json"), "--out", str(proof),
                                        "--timeout-ms", str(args.solver_timeout_ms)],
                                       stdout=log, stderr=subprocess.STDOUT,
                                       timeout=args.symbolic_wall_seconds, check=False)
                    result["symbolic"] = json.loads(proof.read_text()) if proof.exists() else {"status": "inconclusive", "reason": "checker failed"}
                except subprocess.TimeoutExpired:
                    proof.unlink(missing_ok=True)
                    result["symbolic"] = {"status": "inconclusive", "reason": "checker wall timeout"}
        else:
            result["reason"] = (directory / ("screen.log" if screen_status else "evaluation.log")).read_text()[-2000:]
        results.append(result)
        report = {"schema": "femoco-rigorous-local-results-v1", "authenticated_ledger_result": False,
                  "catalogue_sha256": hashlib.sha256((ROOT / "rigorous/promotions.json").read_bytes()).hexdigest(),
                  "deterministic_coverage_requested": args.coverage, "environment": where,
                  "results": results}
        (args.out_root / "results.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({k: result[k] for k in ("id", "status", "toffoli", "qubits") if k in result}), flush=True)
    passed = all(r["status"] == "passed" for r in results)
    if args.write_evidence:
        if not passed or set(args.exact) - {r["id"] for r in results if "exact" in r}:
            raise ValueError("evidence is written only when every candidate and every requested proof passed")
        for stale in (ROOT / "rigorous").glob("exact-*.json"):
            stale.unlink()
        for r in results:
            if "exact" in r:
                shutil.copy2(args.out_root / r["track"] / r["id"] / r["exact"]["report"],
                             ROOT / "rigorous" / r["exact"]["report"])
        shutil.copy2(args.out_root / "results.json", ROOT / "rigorous/validation-results.json")
    return 0 if passed else 2


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out-root", type=lambda s: pathlib.Path(s).resolve(), required=True)
    p.add_argument("--id", action="append")
    p.add_argument("--cargo", default="cargo")
    p.add_argument("--samples", type=int, default=524288)
    p.add_argument("--screen-samples", type=int, default=4096)
    p.add_argument("--coverage", choices=("terms", "exhaustive"), default="terms")
    p.add_argument("--screen-only", action="store_true", help="skip deterministic coverage; result remains sampled only")
    p.add_argument("--symbolic", action="store_true", help="also run the SMT controller audit (symbolic.py)")
    p.add_argument("--exact", action="append", default=[], metavar="ID",
                   help="run the symbolic checker (exact.py) on this candidate; repeatable")
    p.add_argument("--jobs", type=int, default=4, help="worker processes of the symbolic checker")
    p.add_argument("--write-evidence", action="store_true",
                   help="replace rigorous/validation-results.json and rigorous/exact-*.json")
    p.add_argument("--solver-timeout-ms", type=int, default=120000)
    p.add_argument("--symbolic-wall-seconds", type=int, default=600)
    args = p.parse_args()
    if min(args.samples, args.screen_samples, args.solver_timeout_ms, args.symbolic_wall_seconds) <= 0: p.error("budgets must be positive")
    if args.screen_only: args.coverage = None
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
