# Split one-hot with paired corrections: sa-toff imchxgrdy4zabCAXZJBMnFsYU, keep 8+8, blocks 8/3

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
| `FEMOCO_SA_INNER_A` | `8` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdy4zabCAXZJBMnFsYU` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`ltn_lc3_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdy4zabCAXZJBMnFsYU", 8, 8, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17780.497 | 498 | 8854687.506 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `ae79536ea9200a596601f056dc75bcdf1ee8eb3a307d99b149c94304f964bf6a`
- Lane seed: `eaf1bd3d1f49dd86243d39569b289f0fde265d5615328509e3822e5169c364b2`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
