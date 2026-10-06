# Low et al. 2025 construction, published parameters

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `qroam-word` (QROAM angle words).

The selected network stays in binary and all of its angles are fetched at once by a table lookup into a wide data register. There is no unary or one-hot copy of the index; the qubit peak is set by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-low2025` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`est_low_r` in `tests/sa_circuits/list.rs`; the pin was written as `low(true)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 9600.000 | 1040 | 9984000.000 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `95e8fe8982dca129be65f4d11e4bdce0eb1fd4173f9294687ad0dba8646e2ea9`
- Lane seed: `fa093a5eafa9e0523e2b15c25acce8c1fc31bd4023166d1782973a4b58357e5b`
- First measured: 2026-09-24

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
