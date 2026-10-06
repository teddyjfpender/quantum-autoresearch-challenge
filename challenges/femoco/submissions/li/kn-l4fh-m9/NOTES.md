# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCAHVIXZJtRNOBWsY+-_, keep 9+9, blocks 3/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `li`, architecture `onehot-split` (Split one-hot with paired corrections).

The unary register is factored into slots times groups, and angle delivery is a Clifford base load plus Toffoli corrections whose count is set by the exclusivity of the group bits. Qubits fall with G while Toffolis per delivered bit rise with it.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCAHVIXZJtRNOBWsY+-_` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`kn_l4fh_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17987.569 | 477 | 8580070.413 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `95921b2b0217cebd69e75f9b2b2fbdc37224d01d0c9231cb4d7b2a2e8dd9904d`
- Lane seed: `8a47cd7c01477a57ee56309f09e4d45df7cd0672cea46421e1ca14a4a2ec8d27`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
