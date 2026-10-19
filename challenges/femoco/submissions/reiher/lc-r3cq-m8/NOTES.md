# Rank-scheduled delivery on the G = 3 point at 298 qubits

Track `reiher`, architecture `rank-scheduled`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 2 / 2, levers `imchxgrdky3zabCHKVITXSNOBWs_~q15_1.11.p`.

## Starting point

`gkr-r3c-m8` (13,223.4 / 298), the front point at 298 qubits. It has a three-group split one-hot and the inner-keep release `~`, but its angle delivery was never scheduled. Parent levers (`imchxgrdky3zabCHKVITXSNOBWs_~`).

## Mechanism

Added: `q15_1.11` (rank-scheduled delivery, budget 15, Majorana budget 11) and `.p` (padding runs with a pruned slot one-hot, section 6.15).

Rank-scheduled delivery (`q<b>_1.<c>`, `src/walk/sa_low/README.md` section 6.16) treats angle
delivery into SELECT as a caching problem. With a split one-hot live, the one-hot's slot and group
functions are a free base; each stored direction modulo that base costs one Toffoli to buy, and a
direction held live is reused by CNOTs. Mode 1 keeps the furthest-future subspace within a budget
of `b` qubits, measures out what it drops with Clifford fixups, and at the gap between `V^dagger`
and `V` cuts the held span to `c` dimensions so the Majorana stage holds at most `c` delivery
qubits.

The change is a recomposition of shipped levers. No circuit code changes, so every recorded
circuit still builds byte for byte.

**Why it is correct.** Nothing here is new code: every lever is in the shipped `sa-toff` builder
and has its own tests (`rankdel` plan tests, the lever composition tests). The builder refuses
lever combinations it does not support with an assertion, so a bundle that builds is one the
builder's rules allow. The circuit was built from `main` through the sandboxed build path and its
`ops.bin` SHA-256 is the one below; a local `K = 64` run of the trusted evaluator and a
524,288-lane run on the sliced engine both passed every lane, with `fallback_batches=0` and
`guard_fallbacks=0`. The judge's run on seeded lanes is the result; the local numbers are claims.

## Experiments

These levers were composed onto every recorded low-qubit point, and the bundles were built and
counted statically before any were validated.

- Budgets `b` 12-18 with `c` from `b - 6` to `b - 1`, with and without `.p`, at `ia` / `oa` 2 / 2. Below
  `b = 15` the builder refuses with `.p` (`one rotation's words need more than the budget`);
  `q15_1.11.p` at 2 / 2 was the best bundle at 298 qubits.
- The same bundle at keep 9 + 9 gives 13,369.9 / 300 against 13,402.5 / 300, the same -0.24%.

## Result

| | Toffolis per step | Peak qubits | Lanes | Engine |
| --- | ---: | ---: | ---: | --- |
| Best recorded at <= 298 qubits | 13,223.405 | 298 | 524,288 | ledger |
| This circuit, local | 13,191.321 | 298 | 524,288 | sliced |

The change is -0.24% in Toffolis at 298 qubits (local measurement); `K = 64` gave
13,075.047. `ops_sha256`: `8a2ac421b8e8eda0f283b57121920701015f9af1e0940e1127f3c4cca04a0793`.

## What to try next

- Rank-scheduled delivery pays only where SELECT has idle qubits at `V^dagger` / `V`. A per-stage
  qubit census of each front point says where, before building anything.
- The remaining large gaps at low qubits are structural, not scheduling: the qubit peak is set by
  the inner alias read and its erasure, and by SELECT's rotation pool. A cheaper delivery read
  would be a new mechanism.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code. Builds on
`gkr-r3c-m8` and on the shipped rank-scheduled delivery levers.
