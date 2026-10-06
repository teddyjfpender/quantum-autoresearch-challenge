# Rank-scheduled delivery: sa-toff imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12, keep 8+8, blocks 2/1

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `rank-scheduled` (Rank-scheduled delivery).

Computed products persist across rotations: delivery is a caching problem over a held subspace with an explicit qubit budget, instead of being recomputed for every rotation and bit.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `8` |
| `FEMOCO_SA_MU_I` | `8` |
| `FEMOCO_SA_INNER_A` | `2` |
| `FEMOCO_SA_OUTER_A` | `1` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`rc_r4q_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12", 8, 2, 1)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 15140.783 | 268 | 4057729.844 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `bcd0c7a83b2917800d660b59a164fa9b43f161b7e884d82627e16e4367211de8`
- Lane seed: `ad3864c22e67bed1f0c09adc0e341740d0ad2fe22dbb0042af9fb42e3884990f`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
