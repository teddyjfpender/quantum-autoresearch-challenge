# One-hot angle delivery: sa-toff imchxgrdkyEHKVIXtRNOBW, keep 8+8, blocks 3/3

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `onehot` (One-hot angle delivery).

A unary copy of the selected network exists for the length of SELECT, so delivering an angle costs Clifford gates only. The Toffoli cost of delivery moves into writing and erasing the one-hot register, and the qubit peak is set by the number of stored networks, not by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `8` |
| `FEMOCO_SA_MU_I` | `8` |
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdkyEHKVIXtRNOBW` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`ltn_r1t_m8` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdkyEHKVIXtRNOBW", 8, 3, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 8865.485 | 518 | 4592321.230 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `f184ebaedcbe079659ae4f0aa396e912e1ca88551546af62444f79f278b6ccf5`
- Lane seed: `d1443137965e5b918f1a1c811b4c38ac25008bb5a8f4ea1ad7c84d354adb5b1a`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
