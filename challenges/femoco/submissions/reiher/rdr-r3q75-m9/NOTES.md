# Rank-scheduled delivery: sa-toff imchxgrdky3zabCHKVIDXtNBWsq75, keep 9+9, blocks 2/2

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
| `FEMOCO_SA_INNER_A` | `2` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky3zabCHKVIDXtNBWsq75` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdr_r3q75_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky3zabCHKVIDXtNBWsq75", 9, 2, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 11514.528 | 376 | 4329462.528 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `31fad4c036553e9c2c2bacb7e7e5bc57a0c25038087663d4d969e4697f46a9e2`
- Lane seed: `603c1273f71b034da5cb81b2d6067469740859cb4fd0c171178e99c4d492a3e7`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
