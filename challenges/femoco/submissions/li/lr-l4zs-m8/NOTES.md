# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCHVIXZJs, keep 8+8, blocks 3/2

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `li`, architecture `onehot-split` (Split one-hot with paired corrections).

The unary register is factored into slots times groups, and angle delivery is a Clifford base load plus Toffoli corrections whose count is set by the exclusivity of the group bits. Qubits fall with G while Toffolis per delivered bit rise with it.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `8` |
| `FEMOCO_SA_MU_I` | `8` |
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `2` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCHVIXZJs` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`lr_l4zs_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_mua("imchxgrdky4zabCHVIXZJs", (8, 8), 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 18375.476 | 496 | 9114236.096 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `63e4c09de14c7f3a4ced7bc36f17947f28fdb0e2af1e78a6c7184c8e41eac9a8`
- Lane seed: `5ffdc3ac477472e3cce13bcd94e1d22cdf1b27afcfa2f159bc1f8d442e6b2778`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
