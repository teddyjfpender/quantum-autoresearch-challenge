# Rank-scheduled delivery: sa-toff imchxgrdky3zabCAXJBMnFsUtq618_1.p, keep 9+9, blocks 9/3

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
| `FEMOCO_SA_INNER_A` | `9` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky3zabCAXJBMnFsUtq618_1.p` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`kl_l3q618p_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky3zabCAXJBMnFsUtq618_1.p", 9, 9, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 12821.674 | 1166 | 14950071.884 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `6bcb6a20a59e1c1d9f8cfc8b1e144066c829e825d0057818106fa855f0bc58a1`
- Lane seed: `88dd29c40ae70f327cfcabf99afd93ecdfc3affce64c45f98fcc982270f69371`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
