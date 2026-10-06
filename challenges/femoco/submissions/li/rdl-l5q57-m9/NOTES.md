# Rank-scheduled delivery: sa-toff imchxgrdky5zabCHVIXJtRNOBWsq57_1, keep 9+9, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky5zabCHVIXJtRNOBWsq57_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdl_l5q57_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky5zabCHVIXJtRNOBWsq57_1", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 19801.400 | 480 | 9504672.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `b381aa78e9e421bc0af9aed62eb78ede31f7dd143c6f4506f610234628b997a6`
- Lane seed: `22b3dda8a2f4d32e35f4d0d98d7ee2fea56b7fa4282e8622934edda426a5956a`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
