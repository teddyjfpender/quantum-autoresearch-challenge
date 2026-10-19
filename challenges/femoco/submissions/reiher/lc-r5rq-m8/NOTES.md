# Rank-scheduled delivery with inner lt release at 260 qubits

Track `reiher`, architecture `rank-scheduled`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 2 / 1, levers `imchxgrdky5zabfHKVITXSNOBWsR_q15_1.12.p`.

## Starting point

`ctl-r5r-m8` (16,092.4 / 259), a five-group split one-hot with the inner `lt` release `R`. The best recorded circuit at 260 qubits is `ctl-r5s-m8` (16,076.4 / 260). Parent levers.

## Mechanism

Added: `q15_1.12` (rank-scheduled delivery, budget 15, Majorana budget 12) and `.p`. Delivery now holds rank across SELECT, which costs one qubit over the parent.

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

- On the parent's own 259 qubits the best bundle was `...sR_.p` at -2 Toffolis, inside sampling
  noise, so it is not submitted. Adding `q` at one more qubit gives -90.
- It also beats `ctl-r5n-m8` (16,040.8 / 261) by 38 Toffolis with one qubit fewer.
- Budgets `b` 12-18 were scanned; 15 / 12 was best at 260. The same bundle at keep 9 + 9 gives
  16,125.4 / 263 against 16,197.6 / 263 (-0.45%).

## Result

| | Toffolis per step | Peak qubits | Lanes | Engine |
| --- | ---: | ---: | ---: | --- |
| Best recorded at <= 260 qubits | 16,076.383 | 260 | 524,288 | ledger |
| This circuit, local | 16,002.601 | 260 | 524,288 | sliced |

The change is -0.46% in Toffolis at 260 qubits (local measurement); `K = 64` gave
16,003.000. `ops_sha256`: `9242a154a8c7ad7dc0dc737a8220ed3f0d4d0734ed36598cd51e8373342865a2`.

## What to try next

- Rank-scheduled delivery pays only where SELECT has idle qubits at `V^dagger` / `V`. A per-stage
  qubit census of each front point says where, before building anything.
- The remaining large gaps at low qubits are structural, not scheduling: the qubit peak is set by
  the inner alias read and its erasure, and by SELECT's rotation pool. A cheaper delivery read
  would be a new mechanism.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code. Builds on
`ctl-r5r-m8` and on the shipped rank-scheduled delivery levers.
