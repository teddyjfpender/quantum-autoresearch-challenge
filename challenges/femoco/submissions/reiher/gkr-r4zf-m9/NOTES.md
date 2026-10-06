# Split one-hot with paired corrections: sa-toff imchxgrdky4zZabfHKVITXSNOBWs_~, keep 9+9, blocks 2/1

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `onehot-split` (Split one-hot with paired corrections).

The unary register is factored into slots times groups, and angle delivery is a Clifford base load plus Toffoli corrections whose count is set by the exclusivity of the group bits. Qubits fall with G while Toffolis per delivered bit rise with it.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `2` |
| `FEMOCO_SA_OUTER_A` | `1` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zZabfHKVITXSNOBWs_~` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`gkr_r4zf_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zZabfHKVITXSNOBWs_~", 9, 2, 1)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 15430.861 | 270 | 4166332.470 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `98d0b783a327469b37fa6c911e64e8f4375b63bf2c053f3f1e899eb6daa275ec`
- Lane seed: `a4736d2f56f1a322f245fc69d301a0622da7fda6f53515f9d5818eda94e293ea`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
