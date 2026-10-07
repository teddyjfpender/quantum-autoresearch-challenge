# Roadmap

The repository is built to hold several challenges. Each one is a directory under
`challenges/` with the same parts, and the shared tooling in `tools/qac` and the workflows read
them through `challenges.json`.

## What a challenge provides

| Part | File |
| --- | --- |
| Contract: tracks, editable and trusted paths, validation, acceptance | `benchmark.json` |
| Task for an optimisation agent | `TASK.md` |
| Trusted evaluator and editable circuit code | challenge-specific |
| Architecture registry | `architectures.json` |
| Published points to compare against | `targets.json` |
| Ledger of validated circuits | `results.tsv` |
| Technical specification | `spec/` |

Adding a challenge means adding that directory and one entry in `challenges.json`. The path
policy, the submission manifest, the ledger, the site feed and the judge workflow are shared.

## Planned challenges

These are being prepared and will open here once each has one pinned acceptance
standard, a trusted evaluator and a starting board. None is open yet.

| Challenge | Circuit |
| --- | --- |
| Modular multiplication | Reversible modular multiplication at cryptographic widths. |
| QROM | Table lookup under clean and dirty ancilla budgets. |
| AES oracle | A reversible AES round function for Grover-type search. |
| Fixed-point functions | Reversible evaluation of elementary functions to a stated precision. |
| Variable rotation | Rotation by a register-held angle. |

## Order of work

1. FeMoco: move from `staging` to `live` once the gates in `data/site/activation.json` are met.
2. Reference-engine re-validation of the headline rows of each track.
3. A complete symbolic certificate, with its statement of what is proved, for the final
   candidate of each track ([VALIDATION.md](VALIDATION.md#final-candidates)).
4. The first of the planned challenges, chosen by how soon its evaluator can be made the
   arbiter.
