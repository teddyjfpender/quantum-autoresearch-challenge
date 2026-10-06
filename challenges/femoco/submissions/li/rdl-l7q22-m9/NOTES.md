# Rank-scheduled delivery: sa-toff imchxgrdky7zabCHVIXJsoq22_1, keep 9+9, blocks 3/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `li`, architecture `rank-scheduled` (Rank-scheduled delivery).

Computed products persist across rotations: delivery is a caching problem over a held subspace with an explicit qubit budget, instead of being recomputed for every rotation and bit.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky7zabCHVIXJsoq22_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdl_l7q22_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky7zabCHVIXJsoq22_1", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 24825.718 | 400 | 9930287.200 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `ac194c5bbeef0435b4dc8ab9417b7e3ed76c43ef30c5fcbfddbdcc607deb05ce`
- Lane seed: `2ad4f74ec43ff32400f91841808484abb54b1bc4fe4dc609035551f735a482cc`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
