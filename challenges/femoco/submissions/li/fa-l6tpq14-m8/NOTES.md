# Rank-scheduled delivery: sa-toff imchxgrdky6zabCHVIXZJsot+q14_1, keep 8+8, blocks 3/2

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
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zabCHVIXZJsot+q14_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`fa_l6tpq14_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zabCHVIXZJsot+q14_1", 8, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 22245.298 | 413 | 9187308.074 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `7b5abba501c226fdab2dac6af98fa2af211f5ee4b05432812a9727f812f020a2`
- Lane seed: `7c7f238dc626a79d3cf3ab2a6fdcffa7946aca8e6c43b1232d25ea30937f8b2c`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
