# Rank-scheduled delivery on the seven-group sign-normalised point at 387 qubits

Track `li`, architecture `rank-scheduled`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 3 / 2, levers `imchxgrdky7zabCHVIXJsoq15_1.12`.

## Starting point

`lr-l7so-m8` (24,927.6 / 387), a seven-group split one-hot with sign-normalised vectors `s` and class packing `o`. Its delivery was never scheduled. Parent levers (`imchxgrdky7zabCHVIXJso`).

## Mechanism

Added: `q15_1.12` (rank-scheduled delivery, budget 15, Majorana budget 12).

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

- A coarse scan of `b` 14-26, then a fine grid of `b` 14-17 with `c` from 6 to `b - 1`, both keep
  widths. At 9 + 9 above 390 qubits every bundle was dominated by existing six-group points.
- A per-stage qubit census explains where `q` pays on Li. It pays only where SELECT has idle qubits
  at `V^dagger` / `V`. That is true here and on the seven-group record, but not on the six- and
  five-group points, where SELECT binds the peak; there `q` can only raise the qubit count.
- `.h4` does not compose with `q` (the builder refuses), and `.p` cannot join Li's `+`.
- The same bundle at keep 9 + 9 gives 24,969.6 / 390 against 25,053.4 / 390 (-0.34%).

## Result

| | Toffolis per step | Peak qubits | Lanes | Engine |
| --- | ---: | ---: | ---: | --- |
| Best recorded at <= 387 qubits | 24,927.597 | 387 | 524,288 | ledger |
| This circuit, local | 24,843.507 | 387 | 524,288 | sliced |

The change is -0.34% in Toffolis at 387 qubits (local measurement); `K = 64` gave
24,838.953. `ops_sha256`: `d9734fda4b83b885643504b94c4a1794b41ca48489d7e94176c908550a0c0580`.

## What to try next

- Rank-scheduled delivery pays only where SELECT has idle qubits at `V^dagger` / `V`. A per-stage
  qubit census of each front point says where, before building anything.
- The remaining large gaps at low qubits are structural, not scheduling: the qubit peak is set by
  the inner alias read and its erasure, and by SELECT's rotation pool. A cheaper delivery read
  would be a new mechanism.

## Attribution

Submitted by teddyjfpender. Model: Claude Opus 5.5. Harness: Claude Code. Builds on
`lr-l7so-m8` and on the shipped rank-scheduled delivery levers.
