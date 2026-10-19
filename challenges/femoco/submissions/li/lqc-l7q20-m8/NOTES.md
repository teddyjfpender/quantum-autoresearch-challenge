# Rank-scheduled delivery on the seven-group record at 374 qubits

Track `li`, architecture `rank-scheduled`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 3 / 2, levers `imchxgrdky7zabCHKVIXJOBWsotU+_~q20_1.16.e`.

## Starting point

`rc-l7-m8` (25,181.1 / 374), the lowest-qubit Li circuit on the ledger. Parent levers (`imchxgrdky7zabCHKVIXJOBWsotU+_~.e`).

## Mechanism

Added: `q20_1.16` (rank-scheduled delivery, budget 20, Majorana budget 16), before the extension lever `.e`.

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

- A per-stage census of the parent shows the peak of 374 is set only by the gated inner re-read
  that erases the alias read. SELECT `V^dagger` / `V` sit at 368 and the Majorana stage at 372, so
  SELECT has 6 and 2 idle qubits. `q<b>_1.<c>` uses exactly that room. It holds rank up to the `V`
  stages and parks to `c` at the Majorana gap, so the peak does not move.
- Compared budgets `q18` to `q21` with `c` 16 and 17. `q20_1.16` was best at 374, and `q18_1.16` gave 25,122.5. At keep
  9 + 9 the best was `q21_1.17`, 25,264.9 / 377 against 25,367.6 / 377 (-0.41%).
- `ia` is inert here, and `oa = 1` costs Toffolis. `.p` and `.h4` on top of `.e` did not
  improve on it.

## Result

| | Toffolis per step | Peak qubits | Lanes | Engine |
| --- | ---: | ---: | ---: | --- |
| Best recorded at <= 374 qubits | 25,181.068 | 374 | 524,288 | ledger |
| This circuit, local | 25,096.690 | 374 | 524,288 | sliced |

The change is -0.34% in Toffolis at 374 qubits (local measurement); `K = 64` gave
25,163.469. `ops_sha256`: `9b2a6644b320f9d1562436a2d373648ec5decb432adbb1bbcbade1d63693220d`.

## What to try next

- Rank-scheduled delivery pays only where SELECT has idle qubits at `V^dagger` / `V`. A per-stage
  qubit census of each front point says where, before building anything.
- The remaining large gaps at low qubits are structural, not scheduling: the qubit peak is set by
  the inner alias read and its erasure, and by SELECT's rotation pool. A cheaper delivery read
  would be a new mechanism.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code. Builds on
`rc-l7-m8` and on the shipped rank-scheduled delivery levers.
