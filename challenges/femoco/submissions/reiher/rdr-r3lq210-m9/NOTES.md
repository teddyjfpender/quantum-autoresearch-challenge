# Rank-scheduled delivery: sa-toff imchxlgrdky3zabCHKVIDXtNBWsq210, keep 9+9, blocks 3/3

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
| `FEMOCO_SA_TWEAKS` | `imchxlgrdky3zabCHKVIDXtNBWsq210` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdr_r3lq210_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxlgrdky3zabCHKVIDXtNBWsq210", 9, 3, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 8894.549 | 524 | 4660743.676 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `2ce695f251679c941d95bcb8723d57228f02b8284c73899e3d5fcc98a0b5f795`
- Lane seed: `0cc9355d72b1e9132866cb6d5e87aba72beda2426028546f1beaeacae22de488`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
