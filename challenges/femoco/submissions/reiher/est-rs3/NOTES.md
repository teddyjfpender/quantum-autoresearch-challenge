# Streamed angle words: sa-pareto chunks 3, keep 9+9, blocks 4/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `streamed` (Streamed angle words).

Angles are still fetched by lookups over a binary index, but only a fraction of a network's angle word is ever live: the data register is time-multiplexed along the rotation chain.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-pareto` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `4` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_CHUNKS` | `3` |
| `FEMOCO_SA_CARRIES` | `0` |
| `FEMOCO_SA_DROP_ALT` | `1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`est_rs3` in `tests/sa_circuits/list.rs`; the pin was written as `pareto(3, 4, 2, 0, true)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 12101.000 | 494 | 5977894.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `ea3ee0126013f55f5007152ffa0cea81b5b97357615ddec78e2de752d627b3ee`
- Lane seed: `42d7928cb5c89fa2b17ec7895b66c7bb39ff2c0a72c0ce3d21ed21c7979bcd44`
- First measured: 2026-09-24

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
