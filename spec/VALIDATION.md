# Validation and trust

The judge is a set of GitHub Actions workflows plus the trusted code of each challenge. This
document says what is trusted, what is not, and what a recorded row does and does not show.

## Trust boundaries

| Part | Trusted | Why |
| --- | --- | --- |
| Workflows, `tools/qac`, `challenge.py` | Yes | Always taken from the base branch, never from a pull request. |
| A challenge's `trustedPaths` (evaluator, specs, tests) | Yes | Outside the editable surface; changes need maintainer review. |
| A challenge's `editablePaths` (circuit builders) | No | Submission code. Compiled into the builder only, never into the evaluator. |
| The circuit files a builder emits | No | Parsed defensively by the evaluator. |
| `submission.json`, `NOTES.md` | No | Read as data. Build knobs are restricted to a pattern and a safe character set. |

The judge workflow runs on `pull_request_target`, so its definition comes from `main` and every
script it calls comes from the pull request's base commit, never from the pull request. Its jobs are separated by what they may touch:

| Job | Runs submission code | Token | Secrets | Output |
| --- | --- | --- | --- | --- |
| `intake` | No. Reads the pull request with `git show`. | read | none | policy verdict, manifest |
| `build` | Yes, inside bubblewrap: read-only filesystem, no network, unprivileged uid. | read | none | the circuit files |
| `evaluate` | No. Base-branch code and the evaluator only. | read | ledger key | ledger row, standing |
| `record` | No. | write | ledger key | comment, merge, ledger commit |

`record` relies only on job outputs of `intake` and `evaluate`. The circuit is handed from
`build` to `evaluate` as a workflow artifact and is parsed as hostile data; no artifact is
trusted, so a compromised `build` job cannot forge a result.

## The evaluator binary

`evaluate` uses a prebuilt evaluator from a GitHub release, named in the challenge's
`evaluator.lock.json` with its SHA-256 and the verifier digest of the source it was built from.
It is used only if that digest equals the digest of the trusted files on the base branch and
the download's SHA-256 matches. Otherwise the evaluator is built from source in the job. The
release is cut by `evaluator-release.yml` whenever trusted files change on `main`.

## Circuit code on main

A merged submission's circuit code is on `main` because its circuit passed the judge, not because
a person read it. The intake job refuses source that names process, network, `unsafe`, `extern`
or file-inclusion constructs, and every job that runs circuit code does so in the sandbox with
no secret. Workflows that hold the ledger key take only circuit files from such a job and check
their digests. The same care applies to anyone running the code locally
([SECURITY.md](../SECURITY.md)).

## Lanes and seeds

A circuit is validated on sampled lanes (a challenge defines what a lane is). The seed is
derived from the circuit's digests under the ledger key ([LEDGER.md](LEDGER.md)). A submitter
cannot know it before the judge runs and gets a different one for every change to the circuit,
so searching for a circuit whose sampled lanes happen to miss a fault is not possible. The seed
is published in the row, so the run can be repeated exactly.

FeMoco runs two stages, set in its `benchmark.json`:

| Stage | Engine | Lanes |
| --- | --- | ---: |
| Screen | reference | 4,096 |
| Full | sliced | 524,288 |

The reference engine is the original, straightforward evaluator. The sliced engine is a faster
implementation of the same checks; its design, its equivalence evidence and its limits are in
`challenges/femoco/spec/FAST-EVALUATOR.md`. Both must pass. Every lane must satisfy every
check; there is no tolerance.

## What a row shows

- The named circuit passed every check on every sampled lane, under the named evaluator.
- Its Toffoli and qubit counts are the evaluator's, not the submitter's.

## What a row does not show

- **Not a proof.** Sampling bounds the fraction of failing lanes; it does not exclude them.
  The bound for a full run is stated in the challenge's design document.
- **Not an audit of the sliced engine.** A row's `engine` column says which engines passed it.
  Headline rows are re-validated on the reference engine at full size before the challenge
  leaves `staging` ([ROADMAP.md](ROADMAP.md)).
- **Not a statement about hardware cost.** Only the counted metrics are compared.
- **Not novelty.** The declared architecture is checked against the registry's rules by a
  maintainer when it is new, not against the literature.

## Changing the contract

A spec, an acceptance rule, the score or the evaluator's behaviour changes only through a
reviewed pull request that starts a new contract epoch (`contractEpoch` in `benchmark.json`).
Rows of an earlier epoch are kept and marked by their `verifier_sha256`; they are never edited.
A confirmed soundness bug is handled the same way, with affected rows re-validated.
