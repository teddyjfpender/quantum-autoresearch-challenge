# FeMoco walk step

**Goal.** Build the cheapest controlled qubitized walk step for the FeMoco Hamiltonian, scored
by the product of **Toffolis per step x peak logical qubits**.

FeMoco, the iron-molybdenum cofactor of nitrogenase, is the standard benchmark molecule for
fault-tolerant quantum chemistry. Phase estimation of its ground-state energy repeats one walk
step tens of thousands of times, so the cost of that step sets the cost of the computation.
This challenge fixes the Hamiltonian, its encoding and the error model, and asks for the best
circuit.

## Tracks

| Track | Active space | Spec |
| --- | --- | --- |
| `reiher` | 54 orbitals, 108 spin orbitals (Reiher et al. 2017) | `reiher-sa-est-v1` |
| `li` | 76 orbitals, 152 spin orbitals (Li et al. 2019) | `li-sa-est-v1` |
| `reiher-rigorous` (staging) | Same 54-orbital stored Hamiltonian; exact coefficient rule | `reiher-sa-v1` |
| `li-rigorous` (staging) | Same 76-orbital stored Hamiltonian; exact coefficient rule | `li-sa-v1` |

## Acceptance standards

The original `reiher` and `li` tracks use the spectrum-amplified sum-of-squares encoding of
Low et al. 2025 with their estimated rounding class. The rigorous tracks enforce the existing
exact 0.1 mHa coefficient 1-norm rule on the stored Hamiltonian and add deterministic
selected-term-pair and alias-boundary checks. Their scores and baselines are separate.
The encoding is defined in
[spec/SPEC-SA.md](spec/SPEC-SA.md).

A circuit is valid when, on every sampled lane, it applies exactly the term the lane map
declares, with the right sign, restores its control, selection and ancilla qubits, leaves no
phase behind, and is the identity when the control is off. Skipping uncomputation or leaking
phase makes a run fail; it never makes it cheaper.

The promotion catalogue and exhaustive/symbolic obligations are described in
[tools/verification/README.md](tools/verification/README.md). Controller proofs do not by
themselves prove the entire quantum operator. The new tracks have pending baselines until
fresh judge runs create their ledger rows.

## Reference numbers

The baseline of each original estimated track is the construction the challenge starts from: the walk step of Low
et al. 2025, the research frontier for FeMoco, built with its published parameters and validated
by this evaluator. Everything on the board is measured against it.

| Track | | Toffolis per step | Peak qubits | Score |
| --- | --- | ---: | ---: | ---: |
| `reiher` | Baseline: Low et al. 2025 construction | 9,600 | 1,040 | 9,984,000 |
| `reiher` | Best circuit when the challenge opened | 11,500 | 313 | 3,599,650 |
| `li` | Baseline: Low et al. 2025 construction | 13,906 | 1,347 | 18,731,382 |
| `li` | Best circuit when the challenge opened | 17,777 | 471 | 8,373,194 |

The opening board is 64% below the baseline on `reiher` and 55% below it on `li`. The paper's own
figures (10,203 Toffolis at 1,132 qubits and 14,629 at 1,454) use a different Givens charge and
a register-rule qubit count; [`targets.json`](targets.json) records them with their conventions.
Under this evaluator's Givens charge the paper's Toffoli counts correspond to 9,355 and 13,429.
The `reiher` baseline row is 245 Toffolis above that figure and the `li` row 477 above; the
difference is this repository's implementation of the construction, not the paper's.

## The board

- [`results.tsv`](results.tsv): every validated circuit ([format](../../spec/LEDGER.md)).
- [`architectures.json`](architectures.json): the architectures circuits are grouped by.
- [`submissions/`](submissions/): one directory per recorded circuit, with the build knobs that
  reproduce it and its notes, so any of them can be rebuilt and built upon.
- [`targets.json`](targets.json): the published points of Low et al. 2025, with conventions.
- [`data/site/femoco/leaderboard.json`](../../data/site/femoco/leaderboard.json): elites per
  architecture, the Toffoli-qubit front and the improvement history, derived from the ledger.

## Take part

Read [TASK.md](TASK.md). In short:

```sh
python3 challenge.py setup femoco
python3 challenge.py new femoco reiher my-circuit --architecture onehot-split
python3 challenge.py run femoco challenges/femoco/submissions/reiher/my-circuit -- --samples 4096
```

You may change only [`src/walk/`](src/walk/). The evaluator, the specs and the tests are the
contract.

## Layout

| Path | Trust | What it is |
| --- | --- | --- |
| [`src/walk/`](src/walk/) | **editable** | Circuit builders. [`sa_low/README.md`](src/walk/sa_low/README.md) is the lever reference. |
| `src/` (the rest) | trusted | Op format, simulators, lane maps, scoring, build/evaluation binaries. |
| [`specs/`](specs/) | trusted | The pinned Hamiltonian specs and their certificates. |
| [`tests/`](tests/) | trusted | Harness tests and the byte-identity pins of recorded circuits. |
| [`spec/`](spec/) | | Design contract, the encoding spec, conventions, the fast evaluator. |
| [`taxonomy/`](taxonomy/) | trusted | The evaluator's circuit taxonomy. |
| [`tools/`](tools/) | | Reproduction scripts, a local validation server, maintenance tools. |
| [`submissions/`](submissions/) | | One directory per submission. |

## Scope

These are logical resource counts for one walk step under a stated error model. They are not
a claim of quantum advantage, and they say nothing about physical qubits, runtime or the
classical tractability of FeMoco. Sampled validation bounds the fraction of failing lanes.
Deterministic coverage and symbolic obligations have the separate scopes described above
([validation](../../spec/VALIDATION.md)). References are in
[spec/REFERENCES.md](spec/REFERENCES.md).
