# Split one-hot with paired corrections: sa-toff imchxgrdky4zabCAXZJBMnFsYUt, keep 9+9, blocks 8/3

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zabCAXZJBMnFsYUt` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`pf_l4t_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zabCAXZJBMnFsYUt", 9, 8, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17855.525 | 492 | 8784918.300 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `064126088929e5c9064d782317c7ad83fb6f9509e362e7c849f57537c5f27fad`
- Lane seed: `cfea4c3e215494854399c68d7d69df7e98eed05df931192410b2eef44b6bc4fa`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
