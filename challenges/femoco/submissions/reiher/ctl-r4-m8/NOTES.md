# Split one-hot with paired corrections: sa-toff imchxgrdky4zZabfHKVITXSRNOBWs_, keep 8+8, blocks 2/1

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
| `FEMOCO_SA_INNER_A` | `2` |
| `FEMOCO_SA_OUTER_A` | `1` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zZabfHKVITXSRNOBWs_` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`ctl_r4_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zZabfHKVITXSRNOBWs_", 8, 2, 1)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 14702.629 | 275 | 4043222.975 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `c152be62a572e9a38bf8fc8e2d047c3da658bc2cb03b9581c87355b46f78043e`
- Lane seed: `6da1eab61cc73036e2889562cbc0f1258abbd2bb73a14eee6a5ce141e9586a37`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
