# Rank-scheduled delivery: sa-toff imchxgrdky6zabCHVIXZJsot+q28_1.e, keep 9+9, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zabCHVIXZJsot+q28_1.e` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`kl_l6e_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zabCHVIXZJsot+q28_1.e", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 22105.534 | 430 | 9505379.620 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `6647e90cfe7935f0d329a2f458482c475166932fc33ddbff4dc8069c91aa12d0`
- Lane seed: `720910d9604135fb712246250d02a5704ba4d799023b97b1e521f05804a2d05c`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
