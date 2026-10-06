# Split one-hot with paired corrections: sa-toff imchxgrdky6zZabCHKVITXSRNOBWs, keep 8+8, blocks 2/1

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zZabCHKVITXSRNOBWs` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`fa_r260_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zZabCHKVITXSRNOBWs", 8, 2, 1)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17612.741 | 260 | 4579312.660 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `0970971c6d3b002c38467214685e979d4041aadecd1f5ae190bd5593b05bf2b1`
- Lane seed: `89820c672d37dac3f525f78f1a101f9d77659f70f248fcad524d34b5f907e121`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
