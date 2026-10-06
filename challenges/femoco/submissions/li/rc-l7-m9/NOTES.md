# Split one-hot with paired corrections: sa-toff imchxgrdky7zabCHKVIXJOBWsotU+_~.e, keep 9+9, blocks 3/2

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
| `FEMOCO_SA_TWEAKS` | `imchxgrdky7zabCHKVIXJOBWsotU+_~.e` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rc_l7_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky7zabCHKVIXJOBWsotU+_~.e", 9, 3, 2)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 25366.552 | 377 | 9563190.104 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `25edc76d73cd623cd15a5262e5330c0a996060bde2d5c1ad1a27a1d5e9b7a827`
- Lane seed: `4a8c3efc7ad1355deb9f217f3a58cbe6b95cb82bbc3d19eaae71e3dd9bb0f653`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
