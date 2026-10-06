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
| `FEMOCO_SA_CARRIES` | `3` |
| `FEMOCO_SA_DROP_ALT` | `0` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`est_r1` in `tests/sa_circuits/list.rs`; the pin was written as `pareto(1, 4, 2, 3, false)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 9459.000 | 1050 | 9931950.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `b7000aa37e7a4e862c72f7afdac17cf1bb99b3fdbafb55a9130bdf85c49cd858`
- Lane seed: `3cc562876bb47979747b55c7e30be23bb4b8016c0a417e71b6847543323074a4`
- First measured: 2026-09-24

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
