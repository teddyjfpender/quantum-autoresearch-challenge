# Split one-hot with paired corrections: sa-toff imchxgrdky5zabfHKVITXSNOBWsR_, keep 8+8, blocks 1/1

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `onehot-split` (Split one-hot with paired corrections).

The unary register is factored into slots times groups, and angle delivery is a Clifford base load plus Toffoli corrections whose count is set by the exclusivity of the group bits. Qubits fall with G while Toffolis per delivered bit rise with it.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `8` |
| `FEMOCO_SA_MU_I` | `8` |
| `FEMOCO_SA_INNER_A` | `1` |
| `FEMOCO_SA_OUTER_A` | `1` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky5zabfHKVITXSNOBWsR_` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`ctl_r5r_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky5zabfHKVITXSNOBWsR_", 8, 1, 1)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 16092.362 | 259 | 4167921.758 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `0c6309bf72b7dfb1eef1ade0a6dfb47f787988ceb00b068139442fe0da455935`
- Lane seed: `96b91b46ffe035253cdac96a2c37fb0994a4a298e8b44b0cce5c26031b383510`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
