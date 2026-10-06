# Rank-scheduled delivery: sa-toff imchxgrdky3zabCHKVIDXtNBWsq20_1, keep 9+9, blocks 2/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky3zabCHKVIDXtNBWsq20_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdr_r3q20b_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky3zabCHKVIDXtNBWsq20_1", 9, 2, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 11552.579 | 323 | 3731483.017 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `5988bef6addabd21127e81ed04b947f953d6dada549649f1b45eff0c896bac8f`
- Lane seed: `0a556918d9621da9e89f1e93269a3effdf9b38dba7dff303ef8e3ca4dbcb5579`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
