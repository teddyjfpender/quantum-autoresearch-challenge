# Split one-hot with paired corrections: sa-toff imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4, keep 8+8, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rc_l6_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4", 8, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 22798.910 | 393 | 8959971.630 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `6ff2fab3454c74b41aa8b79401943cad1e86179e9c209c0db353be088ffd5acb`
- Lane seed: `be63e634bf7e7cab2f171e4288e33524a90fd74e05ca5c9948d2a2d2d6701a6d`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
