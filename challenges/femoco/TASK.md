# Task for an optimisation agent: FeMoco walk step

Lower the score of a track, **Toffolis per walk step x peak logical qubits**, with a circuit
the trusted evaluator accepts. The original tracks' baselines are the construction of Low et al.
2025 as this evaluator counts it (`reiher` 9,600 x 1,040, `li` 13,906 x 1,347). Rigorous staging
tracks have separately rebuilt baseline candidates awaiting judge recording. General rules are in
[AGENTS.md](../../AGENTS.md); this file is
the FeMoco-specific part.

## Fixed

- **Tracks.** `reiher` (`reiher-sa-est-v1`) and `li` (`li-sa-est-v1`), plus the separate staging
  tracks `reiher-rigorous` (`reiher-sa-v1`) and `li-rigorous` (`li-sa-v1`).
- **Standard.** The spectrum-amplified encoding with Low et al. 2025's estimated rounding
  class ([spec/SPEC-SA.md](spec/SPEC-SA.md), section 14). The evaluator enforces it from the
  spec: the lane map's table resolutions and rotation bits must meet their precision, and every
  sampled lane must apply the declared operator exactly.
  The rigorous tracks replace the coefficient rule with the exact 0.1 mHa 1-norm rule on
  the stored Hamiltonian and require deterministic term-pair/boundary coverage. The payload
  and rotation words are unchanged. See [tools/verification/README.md](tools/verification/README.md)
  for the proof scopes and the reproducible catalogue of promoted frontier circuits.
- **What is counted.** Toffolis: executed `CCX` and `CCZ` plus the charge of each Givens
  rotation, averaged over lanes. Qubits: the high-water mark of live qubits
  ([spec/DESIGN.md](spec/DESIGN.md), section 8).
- **Trusted code.** Everything except `src/walk/`.

## Yours to change

`src/walk/`: the builders that emit the circuit. A builder is selected and parameterised by
build knobs, environment variables read when `build_circuit` is compiled. You set them in
`submission.json`:

| Knob | Meaning |
| --- | --- |
| `FEMOCO_WALK_ARCH` | The builder: `sa-low2025`, `sa-lowq`, `sa-toff`, `sa-pareto`, or one you add. |
| `FEMOCO_SA_TWEAKS` | The lever string of `sa-toff`. |
| `FEMOCO_SA_MU_O`, `FEMOCO_SA_MU_I` | Outer and inner alias keep bits. |
| `FEMOCO_SA_OUTER_A`, `FEMOCO_SA_INNER_A` | log2 of the outer and inner lookup block counts. |
| `FEMOCO_SA_*` | Any further knob a builder reads, including new ones you add. |

The lever reference, with every existing lever's mechanism and cost, is
[`src/walk/sa_low/README.md`](src/walk/sa_low/README.md).

## Architectures

Declare one in `submission.json`. The registry is [`architectures.json`](architectures.json):

| Architecture | Angle delivery | Shipped builders and knobs |
| --- | --- | --- |
| `qroam-word` | One lookup loads a network's whole angle word. | `sa-low2025`; `sa-toff` without one-hot levers; `sa-pareto` with one chunk |
| `streamed` | The angle word is loaded in chunks along the rotation chain. | `sa-lowq`; `sa-pareto` with `FEMOCO_SA_CHUNKS` above 1 |
| `onehot` | A one-hot copy of the network index; angles by CNOTs. | `sa-toff` with the one-hot levers and no group count |
| `onehot-split` | The one-hot register split into G groups with paired corrections. | `sa-toff` with a group count 2 to 9 in the lever string |
| `rank-scheduled` | Split one-hot plus a held subspace of angle directions under a qubit budget. | `sa-toff` with lever `q<b>` |

A circuit whose organisation fits none of these is a new architecture. Propose it as
[spec/ARCHITECTURES.md](../../spec/ARCHITECTURES.md) describes. A new lever, width, group count
or schedule within one of these is a refinement, not a new architecture.

## Where the cost is

Read the ledger before choosing a direction. Every recorded circuit has a directory under
[`submissions/`](submissions/) whose `submission.json` holds the build knobs that reproduce it;
copy the `build` object of the one you start from into your manifest and name it in `parents`. As a guide to the recorded circuits:

- The Givens rotations are close to their minimum; little is left there.
- **Angle delivery** dominates the low-qubit end: fewer qubits force more groups, and each
  group raises the Toffolis per delivered bit.
- **PREPARE** (the alias-sampling reads and their erasure) dominates the low-Toffoli end.
- The qubit peak is usually shared by several stages. Saving a qubit at one stage does nothing
  unless every stage at the peak gives it up. Print the stage peaks before optimising one.

## Commands

```sh
python3 challenge.py setup femoco

# Static counts of a lever bundle, no simulation (seconds):
cd challenges/femoco
SA_SPECS=li-sa-est-v1 SA_MU=9,9 SA_TWEAKS=<levers> SA_INNER_A=3 SA_OUTER_A=3 \
  cargo test --release --lib sa_low::tests_combo::combo_scan -- --ignored --nocapture

# What is live at the qubit peak:
SA_SPECS=li-sa-est-v1 SA_TWEAKS=<levers> SA_INNER_A=3 \
  cargo test --release --lib sa_low::tests_combo -- --ignored --nocapture

# Recorded circuits must still build byte for byte (minutes):
cargo test --release --locked --test sa_digests
cd ../..

# Build and evaluate a submission directory:
python3 challenge.py new femoco li <id> --architecture <architecture>
python3 challenge.py run femoco challenges/femoco/submissions/li/<id> -- --samples 4096
python3 challenge.py run femoco challenges/femoco/submissions/li/<id> -- --engine sliced
```

`run` without arguments uses the reference engine on 524,288 lanes, which takes tens of minutes;
`--samples 4096` is a quick screen and `--engine sliced` is the fast full run the judge uses.
Local lanes are derived from the circuit; the judge's are not.

## Invariants

- Every pinned circuit (`tests/sa_circuits/list.rs`) still builds byte for byte. Add a knob or
  a lever letter for new behaviour; never change an existing one.
- Each new gadget has a test on small exact specs and at least one mutant the evaluator must
  reject. The existing levers show the pattern (`src/walk/sa_low/tests_*.rs`).
- No new dependencies. `Cargo.toml` and `Cargo.lock` are trusted.

## Submit

One pull request: your `src/walk/` changes, `submissions/<track>/<id>/submission.json` and
`NOTES.md`, and the registry entry if you propose an architecture. The judge validates it on
4,096 lanes with the reference engine and 524,288 with the sliced engine, under a seed derived
from the ledger key, and a bot records it if it earns a standing
([spec/SUBMISSIONS.md](../../spec/SUBMISSIONS.md), [spec/SCORING.md](../../spec/SCORING.md)).
Rigorous tracks additionally run deterministic term-pair and alias-boundary coverage;
`challenge.py run` adds that track's coverage flag automatically.
