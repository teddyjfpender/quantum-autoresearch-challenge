"""Command line for participants and for the judge workflows. Run it as `python3 challenge.py`."""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
import time

from . import check as repo_check
from . import ledger, manifest, policy, report, site, verifier
from .common import ROOT, Challenge, ContractError, challenges, load_json, sha256_file

KEY_ENV = "QAC_LEDGER_KEY"
CIRCUIT_FILES = ("ops.bin", "lanemap.bin", "family.out.json")


def ledger_key(required: bool) -> bytes | None:
    value = os.environ.get(KEY_ENV, "").strip()
    if not value:
        if required:
            raise ContractError(f"{KEY_ENV} is not set")
        return None
    try:
        key = bytes.fromhex(value)
    except ValueError as exc:
        raise ContractError(f"{KEY_ENV} must be hex") from exc
    if len(key) < 32:
        raise ContractError(f"{KEY_ENV} must be at least 32 bytes")
    return key


def cmd_list(_args) -> int:
    for challenge in challenges():
        print(f"{challenge.id}\t{challenge.contract['status']}\t{challenge.contract['title']}")
        for name, track in challenge.tracks.items():
            print(f"  {name}\t{track['spec']}\t{track['title']}")
    return 0


def cmd_check(args) -> int:
    errors = repo_check.run(signed=args.signed)
    for error in errors:
        print(f"FAIL: {error}", file=sys.stderr)
    if not errors:
        print(f"Contract OK: {len(challenges())} challenge(s)")
    return 1 if errors else 0


def cmd_verifier(args) -> int:
    print(verifier.digest(Challenge(args.challenge)))
    return 0


def cmd_policy(args) -> int:
    verdict = policy.evaluate(policy.diff(args.repo, args.base, args.head))
    if verdict.is_submission and verdict.ok and verdict.proposes_architecture:
        challenge = Challenge(verdict.challenge)
        path = f"{challenge.path}/{challenge.contract['architectures']}"
        show = lambda ref: json.loads(subprocess.run(  # noqa: E731
            ["git", "-C", args.repo, "show", f"{ref}:{path}"], check=True, capture_output=True).stdout)
        try:
            verdict.errors += policy.registry_is_append_only(show(args.base), show(args.head))
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            verdict.errors.append("architectures.json: unreadable at the base or the head")
    print(verdict.to_json())
    return 0 if verdict.ok else 1


def _show(repo: str, ref: str, path: str) -> bytes:
    return subprocess.run(["git", "-C", repo, "show", f"{ref}:{path}"], check=True, capture_output=True).stdout


def cmd_intake(args) -> int:
    """Path policy and manifest check of a pull request, read from git objects only.

    Writes policy.json and manifest.json to --out. Nothing from the pull request is executed and
    no file of it is checked out; the two submission files are read with `git show`.
    """
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    verdict = policy.evaluate(policy.diff(args.repo, args.base, args.head))
    checked = None
    if verdict.is_submission and verdict.ok:
        challenge = Challenge(verdict.challenge)
        architectures = None
        try:
            if verdict.proposes_architecture:
                path = f"{challenge.path}/{challenge.contract['architectures']}"
                old, new = json.loads(_show(args.repo, args.base, path)), json.loads(_show(args.repo, args.head, path))
                verdict.errors += policy.registry_is_append_only(old, new)
                architectures = {a["id"]: a for a in new.get("architectures", []) if isinstance(a, dict) and "id" in a}
            directory = out / "submission" / verdict.track / verdict.submission_id
            directory.mkdir(parents=True, exist_ok=True)
            for name in sorted(policy.SUBMISSION_FILES):
                data = _show(args.repo, args.head, f"{verdict.submission_dir}/{name}")
                if len(data) > manifest.NOTES_MAX_BYTES:
                    raise ContractError(f"{name}: larger than {manifest.NOTES_MAX_BYTES} bytes")
                (directory / name).write_bytes(data)
            if verdict.ok:
                checked = manifest.load(challenge, directory, architectures)
        except (ContractError, subprocess.CalledProcessError, json.JSONDecodeError, UnicodeDecodeError) as exc:
            verdict.errors.append(str(exc) if isinstance(exc, ContractError) else "submission files or registry are unreadable")
    (out / "policy.json").write_text(verdict.to_json() + "\n", encoding="utf-8")
    if checked is not None:
        (out / "manifest.json").write_text(json.dumps(checked, indent=1) + "\n", encoding="utf-8")
    print(verdict.to_json())
    return 0


def cmd_overlay(args) -> int:
    """Replace the challenge's editable paths in the working tree with the pull request's."""
    challenge = Challenge(args.challenge)
    for path in challenge.repo_paths("editablePaths"):
        subprocess.run(["git", "-C", str(ROOT), "rm", "-r", "-q", "--ignore-unmatch", "--", path], check=True)
        present = subprocess.run(["git", "-C", str(ROOT), "cat-file", "-e", f"{args.head}:{path}"], capture_output=True)
        if present.returncode == 0:
            subprocess.run(["git", "-C", str(ROOT), "checkout", args.head, "--", path], check=True)
        print(f"overlaid {path}")
    return 0


def cmd_build(args) -> int:
    """[judge] Run only the untrusted build stage for a checked manifest."""
    challenge = Challenge(args.challenge)
    checked = load_json(pathlib.Path(args.manifest))
    build = challenge.contract["build"]
    env = {**os.environ, **checked["build"], build["specEnv"]: challenge.tracks[checked["track"]]["spec"], build["stageEnv"]: "build"}
    return subprocess.run(challenge.contract["benchmarkCommand"], cwd=challenge.dir, env=env, check=False).returncode


def cmd_manifest(args) -> int:
    """Check a submission directory; `--architectures` reads the registry the pull request proposes."""
    challenge = Challenge(args.challenge)
    architectures = None
    if args.architectures:
        architectures = {a["id"]: a for a in load_json(pathlib.Path(args.architectures))["architectures"]}
    checked = manifest.load(challenge, pathlib.Path(args.directory), architectures)
    print(json.dumps(checked, indent=1))
    return 0


def circuit_digests(root: pathlib.Path) -> tuple[str, str, str]:
    for name in CIRCUIT_FILES:
        if not (root / name).is_file() or (root / name).stat().st_size == 0:
            raise ContractError(f"{name} is missing or empty")
    return tuple(sha256_file(root / name) for name in CIRCUIT_FILES)


def cmd_seed(args) -> int:
    print(ledger.derive_seed(ledger_key(required=True), *circuit_digests(pathlib.Path(args.root))))
    return 0


def cmd_row(args) -> int:
    challenge = Challenge(args.challenge)
    checked = json.loads(pathlib.Path(args.manifest).read_text(encoding="utf-8"))
    score = load_json(pathlib.Path(args.score))
    row = ledger.row_from_score(
        score, track=challenge.tracks[checked["track"]], architecture=checked["architecture"], engine=args.engine,
        seed=args.seed, verifier=verifier.digest(challenge), unix_time=int(args.unix_time or time.time()),
        commit=args.commit, pr=args.pr, author=args.author or checked["authors"][0], model=checked["model"],
        harness=checked["harness"], submission=f"{checked['track']}/{checked['id']}", kind="submission",
        note=checked["title"],
    )
    pathlib.Path(args.out).write_text(json.dumps(row, indent=1) + "\n", encoding="utf-8")
    print(json.dumps(row, indent=1))
    return 0


def decide(challenge: Challenge, row: dict) -> dict:
    rows = ledger.read(challenge.ledger)
    standings, reason = ledger.standing(rows, row, challenge.contract["acceptance"]["minImprovementBips"])
    return {"accepted": bool(standings), "standing": standings, "reason": reason}


def cmd_decide(args) -> int:
    challenge = Challenge(args.challenge)
    print(json.dumps(decide(challenge, load_json(pathlib.Path(args.row))), indent=1))
    return 0


def cmd_record(args) -> int:
    """Append a validated row to the ledger, sign it and rebuild the site feed."""
    challenge = Challenge(args.challenge)
    key = ledger_key(required=True)
    row = load_json(pathlib.Path(args.row))
    rows = ledger.read(challenge.ledger)
    errors = ledger.verify_chain(key, rows)
    if errors:
        raise ContractError("the ledger does not verify under the key: " + "; ".join(errors[:3]))
    verdict = decide(challenge, row)
    if not verdict["accepted"]:
        raise ContractError(verdict["reason"])
    row["standing"] = ",".join(verdict["standing"])
    if rows and int(row["unix_time"]) < int(rows[-1]["unix_time"]):
        row["unix_time"] = rows[-1]["unix_time"]
    row["mac"] = ledger.mac(key, rows[-1]["mac"] if rows else ledger.GENESIS, row)
    rows.append(row)
    ledger.write(challenge.ledger, rows)
    site.export()
    print(f"recorded {row['submission']}: {row['toffoli']} Toffolis x {row['qubits']} qubits ({row['standing']})")
    return 0


def cmd_compare_row(args) -> int:
    """[audit] A fresh validation of a recorded circuit must reproduce its ledger row."""
    challenge = Challenge(args.challenge)
    row = next((r for r in ledger.read(challenge.ledger) if r["submission"] == args.submission), None)
    if row is None:
        raise ContractError(f"no ledger row for {args.submission}")
    score = load_json(pathlib.Path(args.score))
    metrics = score["metrics"]
    checks = {
        "seed": row["seed"] == args.seed,
        "ops_sha256": row["ops_sha256"] == metrics["digests"]["ops"],
        "lanemap_sha256": row["lanemap_sha256"] == metrics["digests"]["lanemap"],
        "qubits": int(row["qubits"]) == int(metrics["qubits"]),
        "toffoli": abs(float(row["toffoli"]) - float(metrics["toffoli"])) < 5e-4,
        "samples": int(row["samples"]) == int(metrics["samples"]),
        "verifier_sha256": row["verifier_sha256"] == verifier.digest(challenge),
    }
    for name, ok in checks.items():
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
    print(f"{args.submission}: {metrics['toffoli']:.3f} Toffolis x {metrics['qubits']} qubits; ledger {row['toffoli']} x {row['qubits']}")
    return 0 if all(checks.values()) else 1


def cmd_verify_ledger(_args) -> int:
    key = ledger_key(required=True)
    failed = False
    for challenge in challenges():
        rows = ledger.read(challenge.ledger)
        errors = ledger.verify_chain(key, rows)
        failed |= bool(errors)
        for error in errors:
            print(f"FAIL: {challenge.path}/{challenge.contract['ledger']}: {error}", file=sys.stderr)
        if not errors:
            print(f"{challenge.id}: {len(rows)} rows verify")
    return 1 if failed else 0


def cmd_report(args) -> int:
    """Compose the judge's comment and verdict from the jobs' JSON outputs."""
    result = report.build(
        pathlib.Path(args.dir), {"build": args.build, "evaluate": args.evaluate},
        approved=args.approved == "true", run_url=args.run_url,
    )
    pathlib.Path(args.out).write_text(json.dumps(result, indent=1) + "\n", encoding="utf-8")
    print(result["body"])
    return 0


def cmd_unchanged(args) -> int:
    """Exit 0 if no trusted or editable path of the challenge differs between two commits.

    A submission validated against one base may be merged onto a later `main` only if the code
    that builds and judges circuits is the same; ledger, site and submission commits do not count.
    """
    challenge = Challenge(args.challenge)
    names = subprocess.run(
        ["git", "-C", args.repo, "diff", "--name-only", "--no-renames", "-z", args.base, args.head],
        check=True, capture_output=True,
    ).stdout.decode("utf-8").split("\0")
    watched = challenge.repo_paths("trustedPaths") + challenge.repo_paths("editablePaths") + ["tools/qac", "challenge.py"]
    changed = [n for n in names if n and (n in watched or any(n.startswith(w + "/") for w in watched))]
    for name in changed[:20]:
        print(f"changed: {name}")
    return 1 if changed else 0


def cmd_site(args) -> int:
    stale = site.export(check=args.check)
    for path in stale:
        print(("stale: " if args.check else "wrote: ") + path)
    return 1 if args.check and stale else 0


def cmd_run(args) -> int:
    """Build and evaluate a submission locally, exactly as the judge does but on public lanes."""
    challenge = Challenge(args.challenge)
    directory = pathlib.Path(args.submission).resolve()
    checked = manifest.load(challenge, directory)
    env = {**os.environ, **checked["build"], challenge.contract["build"]["specEnv"]: challenge.tracks[checked["track"]]["spec"]}
    extra = args.extra[1:] if args.extra[:1] == ["--"] else args.extra
    command = [*challenge.contract["benchmarkCommand"], *extra]
    print(f"{checked['track']}/{checked['id']} ({checked['architecture']}), knobs {checked['build']}", flush=True)
    return subprocess.run(command, cwd=challenge.dir, env=env, check=False).returncode


NOTES_TEMPLATE = """# {title}

<!-- Public research notes for this submission: at least 1 KiB, at most 100 KiB. -->

## Goal and starting point

Which recorded circuit or submission this builds on (its `ops_sha256` or submission id) and
what you set out to change.

## Mechanism

What is structurally different, which stage it changes, and why it is correct.

## Architecture

Why this belongs to the declared architecture, or, for a new one, the property that separates
it from every architecture in the registry.

## Experiments

Commands, counts (Toffolis per step, peak qubits, lanes, engine), and what did not work.

## Result and next steps

The measured circuit, its caveats and what to try next.

## Attribution

Model and harness, human direction, and the discussions and submissions this used.
"""


def cmd_new(args) -> int:
    """Create a submission directory with a manifest skeleton and a notes template."""
    challenge = Challenge(args.challenge)
    if args.track not in challenge.tracks:
        raise ContractError(f"unknown track '{args.track}'; tracks: {', '.join(challenge.tracks)}")
    if args.architecture not in challenge.architectures():
        raise ContractError(f"'{args.architecture}' is not in {challenge.contract['architectures']}; add it there to propose it")
    directory = challenge.dir / challenge.contract["submissionPaths"][0] / args.track / args.id
    if directory.exists():
        raise ContractError(f"{directory} already exists")
    title = args.title or args.id.replace("-", " ")
    skeleton = {
        "schema": manifest.SCHEMA, "challenge": challenge.id, "track": args.track, "architecture": args.architecture,
        "title": title, "build": {}, "claimed": {}, "authors": ["your-github-login"],
        "model": "exact model name", "harness": "exact harness name", "parents": [],
    }
    manifest.validate(challenge, args.track, args.id, skeleton, challenge.architectures())
    directory.mkdir(parents=True)
    (directory / "submission.json").write_text(json.dumps(skeleton, indent=1) + "\n", encoding="utf-8")
    (directory / "NOTES.md").write_text(NOTES_TEMPLATE.format(title=title), encoding="utf-8")
    print(f"created {directory.relative_to(ROOT)}: set the build knobs, authors, model and harness, then write NOTES.md")
    return 0


def cmd_setup(args) -> int:
    challenge = Challenge(args.challenge)
    return subprocess.run(challenge.contract["setupCommand"], cwd=challenge.dir, check=False).returncode


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="challenge.py", description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("list", help="challenges and their tracks").set_defaults(run=cmd_list)
    p = sub.add_parser("check", help="repository contract check")
    p.add_argument("--signed", action="store_true", help="refuse unsigned ledger rows")
    p.set_defaults(run=cmd_check)
    p = sub.add_parser("setup", help="install a challenge's toolchain and build it")
    p.add_argument("challenge")
    p.set_defaults(run=cmd_setup)
    p = sub.add_parser("run", help="build and evaluate a submission directory locally")
    p.add_argument("challenge")
    p.add_argument("submission", help="path to submissions/<track>/<id>")
    p.add_argument("extra", nargs=argparse.REMAINDER, help="arguments passed to the benchmark command")
    p.set_defaults(run=cmd_run)
    p = sub.add_parser("new", help="create a submission directory from the template")
    p.add_argument("challenge")
    p.add_argument("track")
    p.add_argument("id", help="lowercase slug, e.g. li-split5-folded-read")
    p.add_argument("--architecture", required=True)
    p.add_argument("--title", default="")
    p.set_defaults(run=cmd_new)
    p = sub.add_parser("site", help="rebuild data/site from the ledgers")
    p.add_argument("--check", action="store_true")
    p.set_defaults(run=cmd_site)
    p = sub.add_parser("verifier-digest", help="SHA-256 over a challenge's trusted files")
    p.add_argument("challenge")
    p.set_defaults(run=cmd_verifier)

    p = sub.add_parser("policy", help="[judge] path policy of a pull request")
    p.add_argument("--repo", default=str(ROOT))
    p.add_argument("--base", required=True)
    p.add_argument("--head", required=True)
    p.set_defaults(run=cmd_policy)
    p = sub.add_parser("intake", help="[judge] policy and manifest of a pull request, from git objects")
    p.add_argument("--repo", default=str(ROOT))
    for name in ("base", "head", "out"):
        p.add_argument(f"--{name}", required=True)
    p.set_defaults(run=cmd_intake)
    p = sub.add_parser("overlay", help="[judge] take the editable paths from a pull request head")
    p.add_argument("challenge")
    p.add_argument("--head", required=True)
    p.set_defaults(run=cmd_overlay)
    p = sub.add_parser("build", help="[judge] run the untrusted build stage for a checked manifest")
    p.add_argument("challenge")
    p.add_argument("--manifest", required=True)
    p.set_defaults(run=cmd_build)
    p = sub.add_parser("manifest", help="[judge] check a submission directory")
    p.add_argument("challenge")
    p.add_argument("directory")
    p.add_argument("--architectures")
    p.set_defaults(run=cmd_manifest)
    p = sub.add_parser("seed", help="[judge] lane seed of the circuit files in a directory")
    p.add_argument("--root", required=True)
    p.set_defaults(run=cmd_seed)
    p = sub.add_parser("row", help="[judge] ledger row from a checked manifest and score.json")
    p.add_argument("challenge")
    for name in ("manifest", "score", "engine", "seed", "commit", "pr", "out"):
        p.add_argument(f"--{name}", required=True)
    p.add_argument("--author", default="")
    p.add_argument("--unix-time", default="")
    p.set_defaults(run=cmd_row)
    p = sub.add_parser("decide", help="[judge] standing of a row against the ledger")
    p.add_argument("challenge")
    p.add_argument("--row", required=True)
    p.set_defaults(run=cmd_decide)
    p = sub.add_parser("record", help="[judge] append and sign a row, rebuild the site feed")
    p.add_argument("challenge")
    p.add_argument("--row", required=True)
    p.set_defaults(run=cmd_record)
    sub.add_parser("verify-ledger", help="[judge] verify every ledger's MAC chain").set_defaults(run=cmd_verify_ledger)
    p = sub.add_parser("compare-row", help="[audit] check a fresh validation against a ledger row")
    p.add_argument("challenge")
    for name in ("submission", "score", "seed"):
        p.add_argument(f"--{name}", required=True)
    p.set_defaults(run=cmd_compare_row)
    p = sub.add_parser("report", help="[judge] compose the pull-request comment and verdict")
    for name in ("dir", "build", "evaluate", "approved", "run-url", "out"):
        p.add_argument(f"--{name}", required=True)
    p.set_defaults(run=cmd_report)
    p = sub.add_parser("unchanged", help="[judge] no trusted or editable path differs between two commits")
    p.add_argument("challenge")
    p.add_argument("--repo", default=str(ROOT))
    p.add_argument("--base", required=True)
    p.add_argument("--head", required=True)
    p.set_defaults(run=cmd_unchanged)

    args = parser.parse_args(argv)
    try:
        return args.run(args)
    except ContractError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
