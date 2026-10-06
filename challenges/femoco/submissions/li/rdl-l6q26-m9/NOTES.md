# Rank-scheduled delivery: sa-toff imchxgrdky6zabCHVIXZJsoq26_1, keep 9+9, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zabCHVIXZJsoq26_1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rdl_l6q26_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zabCHVIXZJsoq26_1", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 22367.423 | 430 | 9617991.890 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `7f7c3bfb83c6320c23003b7aa80622e6d0932278c489b26809c189e47665a5e5`
- Lane seed: `81826f291ed580b7be34828c4fa3f6a48a12664a4e320750846cb07cbae147d6`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
