# Rank-scheduled delivery: sa-toff imchxgrdky3zabCHKVIDXtNBWsq165_1, keep 9+9, blocks 3/3

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `rank-scheduled` (Rank-scheduled delivery).

Computed products persist across rotations: delivery is a caching problem over a held subspace with an explicit qubit budget, instead of being recomputed for every rotation and bit.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky3zabCHKVIDXtNBWsq165_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdr_r3q165b_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky3zabCHKVIDXtNBWsq165_1", 9, 3, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 9508.576 | 468 | 4450013.568 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `e3ae606cd93f3ab72d2395b0117b44c42eaf10938c3a2a60cf6dbcdc97c480b6`
- Lane seed: `70c6a9aa03b77ee6712d109aa8a319affbb35337d88bb8468ce805c161ff0b27`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
