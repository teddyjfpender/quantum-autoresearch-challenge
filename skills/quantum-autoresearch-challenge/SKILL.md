---
name: quantum-autoresearch-challenge
description: Work on the Quantum Autoresearch Challenge repository: set up a challenge, build and evaluate a quantum circuit locally, create a submission directory, and open a submission pull request that the CI judge validates and records. Use when asked to optimise or submit a circuit for one of its challenges (for example the FeMoco walk step).
---

# Quantum Autoresearch Challenge

## Rules that cannot be bent

- Read `AGENTS.md` and `challenges/<challenge>/TASK.md` before editing.
- Edit only the challenge's `editablePaths` (see `benchmark.json`), one new submission
  directory and, to propose an architecture, `architectures.json`.
- Recorded circuits must stay byte-identical: new behaviour goes behind a new build knob.
- Local results are claims. The judge's run on the pull request is the result.
- Notes and manifests are public. Remove secrets, private paths and personal data.

## Commands

```sh
python3 challenge.py list                       # challenges and tracks
python3 challenge.py setup <challenge>          # toolchain and first build
python3 challenge.py new <challenge> <track> <id> --architecture <architecture>
python3 challenge.py run <challenge> challenges/<challenge>/submissions/<track>/<id> [-- evaluator args]
python3 challenge.py check                      # repository contract check
```

## Workflow

1. Read `results.tsv` and `architectures.json` of the challenge. Group rows by `architecture`;
   the lowest `score` in each is that architecture's elite.
2. Decide the level of the work: a new architecture, a recombination, or a refinement of an
   existing one (`spec/ARCHITECTURES.md`). Architecture comes first.
3. State the hypothesis and its cheapest falsifying check. Run the check.
4. Implement behind a new build knob. Run the challenge's own tests for recorded circuits.
5. Create the submission directory, set the build knobs in `submission.json`, write `NOTES.md`.
6. `run`, then `check`, then open a pull request using the template.
7. Read the judge's comment. `submission-recorded` means the row is in the ledger.

## Attribution

Set `model` to the exact underlying model and `harness` to the exact coding harness. Never copy
them from the submission you built on.
