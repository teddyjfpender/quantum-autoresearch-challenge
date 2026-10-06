# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCAXZJtRNOBWMenFsY, keep 9+9, blocks 8/3

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
| `FEMOCO_SA_INNER_A` | `8` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCAXZJtRNOBWMenFsY` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`pf_l4tp_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zabCAXZJtRNOBWMenFsY", 9, 8, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 18059.586 | 484 | 8740839.624 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `2b849df2c3837fd103208343901604eaba0c39d6aa0911af2f3e2902b58ac008`
- Lane seed: `efb24368d587090a6f334c27243b3478ae74de5275d4b7ed3d217d35c43e146a`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
