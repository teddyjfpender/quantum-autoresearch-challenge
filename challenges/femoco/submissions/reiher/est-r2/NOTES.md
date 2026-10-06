# QROAM angle words: sa-pareto chunks 1, keep 9+9, blocks 4/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `qroam-word` (QROAM angle words).

The selected network stays in binary and all of its angles are fetched at once by a table lookup into a wide data register. There is no unary or one-hot copy of the index; the qubit peak is set by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-pareto` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `4` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_CHUNKS` | `1` |
| `FEMOCO_SA_CARRIES` | `0` |
| `FEMOCO_SA_DROP_ALT` | `1` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`est_r2` in `tests/sa_circuits/list.rs`; the pin was written as `pareto(1, 4, 2, 0, true)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 9525.000 | 1016 | 9677400.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `bd05174b511c91e2eacc9d193ac4577452f7e169c42f694171defb9b07a4afc1`
- Lane seed: `6084aca9e29c91558b1d8f8b2dbf5d38b7e0ce1e82e483a38a31b57a58e7faf0`
- First measured: 2026-09-24

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
