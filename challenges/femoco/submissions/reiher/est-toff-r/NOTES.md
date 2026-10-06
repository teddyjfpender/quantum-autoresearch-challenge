# QROAM angle words: sa-toff all, keep 9+9, blocks 4/2

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
| `FEMOCO_SA_TWEAKS` | `all` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`est_toff_r` in `tests/sa_circuits/list.rs`; the pin was written as `toff("all")`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 9364.000 | 1028 | 9626192.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `58c5a23ae6877f9bf0b12a7d7199a6e2bc27556ef8e688ad5625e708a406f85d`
- Lane seed: `eaa28fc33d7896501dec8d87627c878659823e7723e2ce6ebc278afea043b433`
- First measured: 2026-09-24

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
