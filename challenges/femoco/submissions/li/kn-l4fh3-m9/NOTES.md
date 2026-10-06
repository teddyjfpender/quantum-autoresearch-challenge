# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCAHVIXZJtRNOBWsY+-_, keep 9+9, blocks 3/3

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
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCAHVIXZJtRNOBWsY+-_` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`kn_l4fh3_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zabCAHVIXZJtRNOBWsY+-_", 9, 3, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17977.622 | 477 | 8575325.694 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `e1f131e4256c76d8b4a819ee568e2acf0907a75d06aeceb89ca9f30effcf046f`
- Lane seed: `18609309a934cdcc009d658d157fe9feb4787da3afb6e5efd12200d794b7fdb7`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
