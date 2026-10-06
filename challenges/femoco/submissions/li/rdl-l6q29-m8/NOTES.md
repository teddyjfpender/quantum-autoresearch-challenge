# Rank-scheduled delivery: sa-toff imchxgrdky6zabCHVIXZJsoq29_1, keep 8+8, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zabCHVIXZJsoq29_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdl_l6q29_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zabCHVIXZJsoq29_1", 8, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 22197.522 | 430 | 9544934.460 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `157f47c3e1362e80914bdb0285d2a97e8b00bdaf0f4be7fd9d8db336f16c91a8`
- Lane seed: `3e34de068784651978f2d1fe5dc9889c8593f3c1b8d247a526a1ff59d16e1d36`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
