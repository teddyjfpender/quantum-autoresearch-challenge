# One-hot angle delivery: sa-toff imchxgrdkyECHKVIXtRNOBW+, keep 9+9, blocks 3/3

Recorded before the challenge opened and part of its starting board. This note is generated from
the circuit's build parameters and its validation run; it states facts, not a research narrative.

## What it is

Track `reiher`, architecture `onehot` (One-hot angle delivery).

A unary copy of the selected network exists for the length of SELECT, so delivering an angle costs Clifford gates only. The Toffoli cost of delivery moves into writing and erasing the one-hot register, and the qubit peak is set by the number of stored networks, not by the angle word.

## Build

| Knob | Value |
| --- | --- |
| `FEMOCO_WALK_ARCH` | `sa-toff` |
| `FEMOCO_SA_MU_O` | `9` |
| `FEMOCO_SA_MU_I` | `9` |
| `FEMOCO_SA_INNER_A` | `3` |
| `FEMOCO_SA_OUTER_A` | `3` |
| `FEMOCO_SA_TWEAKS` | `imchxgrdkyECHKVIXtRNOBW+` |

The track sets `FEMOCO_WALK_SPEC`. For the `sa-toff` builder, each letter of the lever string is
documented in `src/walk/sa_low/README.md`, section 6. The circuit is also pinned byte for byte as
`fa_r1tr_m9` in `tests/sa_circuits/list.rs`; the pin was written as `toff_rp("imchxgrdkyECHKVIXtRNOBW+", 9, 3, 3)`.

## Result

| Toffolis per step | Peak qubits | Score (Toffolis x qubits) | Lanes | Engines |
| ---: | ---: | ---: | ---: | --- |
| 8827.483 | 520 | 4590291.160 | 524288 | `reference-4096+sliced-524288` |

- `ops_sha256`: `fdbdf8dc7b62253999d95893b318e36affe781c08cbff2df2b4746e201b1dc00`
- Lane seed: `1b473b542e8a142dc4dd3bef49867b67fbdd967e6e6a3b48e81e35b4e307b515`
- First measured: 2026-10-02

Validated by the judge pipeline: a screen on the reference engine and a full run on the sliced
engine, both on lanes seeded by the ledger key, with no engine fallback.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code.
