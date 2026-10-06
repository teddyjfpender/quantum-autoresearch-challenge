# Split one-hot with paired corrections: sa-toff imchxgrdky6zZabfHKVITXSRNOBWUs_, keep 9+9, blocks 3/1

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
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `1` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zZabfHKVITXSRNOBWUs_` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`ctl_r6_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zZabfHKVITXSRNOBWUs_", 9, 3, 1)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 17993.659 | 256 | 4606376.704 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `37da939941a2da0fbaae377e5f5b979015228b39fba9769e8ad43c92e09a6cd2`
- Lane seed: `dce216d86e784d927da3e986fc96947d660ddb15f36fc04032de8a42c7467606`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
