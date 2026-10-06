# One-hot angle delivery: sa-toff imchxgrEC, keep 8+8, blocks 5/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `li`, architecture `onehot` (One-hot angle delivery).

A unary copy of the selected network exists for the length of SELECT, so delivering an angle costs Clifford gates only. The Toffoli cost of delivery moves into writing and erasing the one-hot register, and the qubit peak is set by the number of stored networks, not by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `8` |
| `FEMOCO_SA_MU_I` | `8` |
| `FEMOCO_SA_INNER_A` | `5` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrEC` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`pf_l1c_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrEC", 8, 5, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 13116.502 | 1206 | 15818501.412 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `a029404fe2f8d6e3485bc06e844223521d8fd37d2d812b06238c8909a4741d1a`
- Lane seed: `c02cb3d25c048bdaf6a6bea83717609f1bfbbae245a886b2afd0c03f8235309b`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
