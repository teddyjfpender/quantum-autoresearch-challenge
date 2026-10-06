# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCAXZJBMnFsYU, keep 8+8, blocks 8/3

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCAXZJBMnFsYU` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`ltn_lcbmsyu_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zabCAXZJBMnFsYU", 8, 8, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17839.628 | 490 | 8741417.720 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `093dcbecccc8148e90230ec6fee853c671209f0e833ec03c170fb6229596c0b2`
- Lane seed: `5eba012ef5fad8a97a8318834ff659b0e29fb5a8bea1dc5c8f2ceaa3a4d90a77`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
