# One-hot angle delivery: sa-toff imchxgrs, keep 9+9, blocks 5/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `li`, architecture `onehot` (One-hot angle delivery).

A unary copy of the selected network exists for the length of SELECT, so delivering an angle costs Clifford gates only. The Toffoli cost of delivery moves into writing and erasing the one-hot register, and the qubit peak is set by the number of stored networks, not by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `5` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrs` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`lr_l1s_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_mua("imchxgrs", (9, 9), 5, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 13341.000 | 1218 | 16249338.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `6d386519836dfee7bf5ad226a92d8e9b72b23289ffd0d3ab841ea7b8c01b4ebc`
- Lane seed: `548f2e8fe2929c6eca9dfdac89a71a3dfafe993be009063e61aca51f119fee35`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
