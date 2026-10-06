# Split one-hot with paired corrections: sa-toff imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4, keep 8+8, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`kl_l5fhe_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4", 8, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 19791.448 | 424 | 8391573.952 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `831604c2ba5dbeaab532c13a86c64b24fce9493f832f70caff613551e2797648`
- Lane seed: `649aa450b4a610fba6685ec820463d19d125f754e40b697d72fd98f81749a0ca`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
