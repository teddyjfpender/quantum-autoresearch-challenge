# Split one-hot with paired corrections: sa-toff imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4, keep 9+9, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rc_l5_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 20882.252 | 419 | 8749663.588 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `75d99c457ada44bfe14d4b0d739541cc8a29f8f52cefe0484a2f244a8ed0634a`
- Lane seed: `a0d4c841b696f8fb9322b26f91f5f3d0f2032499d44d3c3aa4deebdfe7e5e621`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
