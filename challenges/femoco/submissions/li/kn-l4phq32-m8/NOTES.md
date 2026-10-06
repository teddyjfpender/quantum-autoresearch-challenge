# Rank-scheduled delivery: sa-toff imchxgrdky4zabCAXZJBMnFsYUt_q32_1, keep 8+8, blocks 8/3

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `li`, architecture `rank-scheduled` (Rank-scheduled delivery).

Computed products persist across rotations: delivery is a caching problem over a held subspace with an explicit qubit budget, instead of being recomputed for every rotation and bit.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `8` |
| `FEMOCO_SA_MU_I` | `8` |
| `FEMOCO_SA_INNER_A` | `8` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCAXZJBMnFsYUt_q32_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`kn_l4phq32_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zabCAXZJBMnFsYUt_q32_1", 8, 8, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17633.506 | 499 | 8799119.494 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `def5ceebc39f0af19a224a0a3c84d0b2b638abbc5503b148e6c11a93bc63d03b`
- Lane seed: `fd5516fdd004a5cbc2bba1f6ca3f741c1190e847ee3c662aae88a83eeaa80870`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
