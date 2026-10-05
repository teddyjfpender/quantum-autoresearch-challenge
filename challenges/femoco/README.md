# FeMoco walk step

**Goal.** Build the cheapest controlled qubitized walk step for the FeMoco Hamiltonian, scored
by **Toffolis per step x peak logical qubits**, weighted by the walk's effective 1-norm
([scoring](../../spec/SCORING.md)).

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

## One acceptance standard

Both tracks use the spectrum-amplified sum-of-squares encoding of Low et al. 2025 with their
estimated rounding class: the coefficient and rotation precision their paper assumes. There are
no other encodings or error rules here, so every circuit of a track answers the same question
and can be compared with every other. The standard is defined in
[spec/SPEC-SA.md](spec/SPEC-SA.md).

A circuit is valid when, on every sampled lane, it applies exactly the term the lane map
declares, with the right sign, restores its control, selection and ancilla qubits, leaves no
phase behind, and is the identity when the control is off. Skipping uncomputation or leaking
phase makes a run fail; it never makes it cheaper.

## The board

- [`results.tsv`](results.tsv): every validated circuit ([format](../../spec/LEDGER.md)).
- [`architectures.json`](architectures.json): the architectures circuits are grouped by.
- [`circuits.json`](circuits.json): the build knobs of every circuit recorded before the challenge
  opened, so any of them can be rebuilt and built upon.
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
| `src/` (the rest) | trusted | Op format, simulators, lane maps, scoring, the two binaries. |
| [`specs/`](specs/) | trusted | The pinned Hamiltonian specs and their certificates. |
| [`tests/`](tests/) | trusted | Harness tests and the byte-identity pins of recorded circuits. |
| [`spec/`](spec/) | | Design contract, the encoding spec, conventions, the fast evaluator. |
| [`taxonomy/`](taxonomy/) | trusted | The evaluator's circuit taxonomy. |
| [`tools/`](tools/) | | Reproduction scripts, a local validation server, maintenance tools. |
| [`submissions/`](submissions/) | | One directory per submission. |

## Scope

These are logical resource counts for one walk step under a stated error model. They are not
a claim of quantum advantage, and they say nothing about physical qubits, runtime or the
classical tractability of FeMoco. Validation is by sampling: it bounds the fraction of failing
lanes and is not a proof ([validation](../../spec/VALIDATION.md)). References are in
[spec/REFERENCES.md](spec/REFERENCES.md).
