# QROAM angle words: sa-toff imchxL, keep 9+9, blocks 4/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `qroam-word` (QROAM angle words).

The selected network stays in binary and all of its angles are fetched at once by a table lookup into a wide data register. There is no unary or one-hot copy of the index; the qubit peak is set by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `4` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxL` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`est_toffl_r` in `tests/sa_circuits/list.rs`; the pin was written as `toff("imchxL")`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 9340.000 | 1044 | 9750960.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `9644540e5173efa1497dd2498fd60b487866b733376b07021ad580e41a5ae092`
- Lane seed: `44bcf2be723527e25fe4620d512c83c51b76918971abfb6c37e072dfa107bf1c`
- First measured: 2026-09-24

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
