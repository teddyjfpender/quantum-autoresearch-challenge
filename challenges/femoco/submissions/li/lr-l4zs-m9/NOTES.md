# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCHVIXZJs, keep 9+9, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCHVIXZJs` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`lr_l4zs_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_mua("imchxgrdky4zabCHVIXZJs", (9, 9), 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 18501.425 | 499 | 9232211.075 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `a35c986c2872c2b279aca8da958cf16e9e5b6eb6f6e14c05a7e2b5a7fa7c3e92`
- Lane seed: `630879b52e4cf960f3ad1bf13dfb6df46ceae1e12e8ccad506a88762dfd346de`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
