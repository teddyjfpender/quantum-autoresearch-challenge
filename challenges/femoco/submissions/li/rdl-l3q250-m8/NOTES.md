# Rank-scheduled delivery: sa-toff imchxgrdky3zabCAXJBMnFsUq250_1, keep 8+8, blocks 8/3

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky3zabCAXJBMnFsUq250_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdl_l3q250_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky3zabCAXJBMnFsUq250_1", 8, 8, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 14521.362 | 797 | 11573525.514 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `7e4e3dc19a63d771da9491b687412fb610a3ed15e4a332c4adb7cba302d7901e`
- Lane seed: `20cab73a905374044a1e4d2e46b1066d45a33c20606a5331f974f6899bbd6394`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
