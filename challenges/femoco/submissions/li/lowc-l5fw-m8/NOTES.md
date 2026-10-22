# Five-group split one-hot with the composed alias witnesses: sa-toff imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4bfijklu, keep 8+8, blocks 3/3

Track `li`, architecture `onehot-split`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 3 / 3, levers `imchxgrdky5zabCHKVIXJtRNOBWPsU+-_.eh4bfijklu`.

## Goal and starting point

Starts from the recorded `kl-l5fhe-m8` (19,791.448 Toffolis per step at 424 qubits, score 8,391,574.0).
The goal was a lower-Toffoli point on the front, not a new score record: a census of the two
score elites shows both at a structural optimum of this lever family (every stage at the peak on
Reiher; the sole SELECT plateau on Li priced at or above the product's break-even of about 37
Toffolis per qubit), while the recorded front away from the elites was built before several
levers existed and never had them composed on.

## Mechanism

The recorded five-group point with the composed alias witnesses `.b .f .i .j .k .l` and the
reconstructed outer `hi` `.u` added (they were recorded only on the four-group product point),
and the outer block exponent raised to 3. The witnesses cut the folded inner read, `.u` the outer
read; the five-group SELECT plateau is unchanged.

The change is a recomposition of shipped levers. No circuit code changes, so every recorded
circuit still builds byte for byte (`cargo test --release --locked --test sa_digests`).

**Why it is correct.** Every lever is in the shipped `sa-toff` builder and has its own tests
(exact small specs plus mutants the evaluator rejects, `src/walk/sa_low/tests_*.rs`). The
builder refuses lever combinations it does not support with an assertion, so a bundle that
builds is one the builder's rules allow. The static ledger (`tests_combo::combo_scan`) gave
19,557.5 Toffolis per step at 424 qubits; both trusted evaluations below passed every lane.

## Architecture

`onehot-split`: a split one-hot with a group count and no rank budget; the levers
added are refinements within it (lookup layout, erasure and release choices), not a new
organisation of the step.

## Experiments

Static scans with `tests_combo::combo_scan` (seconds per bundle) over the neighbourhood of every
recorded front point: single, pair and triple toggles of the levers `+ - _ s O J W N B R U K T D P
t S ~ A o l` and the extensions `.e .d .u .p .g .c .h<k>` (Li: `.b .f .i .j .k .l .a`), the
block exponents 1 to 4 (8 and 9 for the paired lookup), the keep split 9 + 9, and the rank
budgets and Majorana parks around each ladder rung; about 200,000 bundles in all, of which the
builder refused about half. Nothing in that space lowers either score elite (Reiher 11,462.5 x
313, Li 17,535.5 x 471). This bundle is the lowest-Toffoli buildable point at or under 424
qubits on its track.

## Result

| | Toffolis per step | Peak qubits | Score |
| --- | ---: | ---: | ---: |
| `kl-l5fhe-m8` (recorded) | 19,791.448 | 424 | 8,391,574.0 |
| `kl-l5fhh-m8` (recorded) | 19,851.654 | 424 | 8,417,101.3 |
| `kn-l5fhq36-m8` (recorded) | 19,571.567 | 450 | 8,807,205.2 |
| `rdl-l5q30-m8` (recorded) | 19,997.518 | 450 | 8,998,883.1 |
| This circuit, reference engine, 4,096 lanes | 19,557.669 | 424 | 8,292,451.5 |
| This circuit, sliced engine, 524,288 lanes | **19,557.582** | **424** | **8,292,414.8** |

Both runs passed every lane (`fallback_batches = 0`, `guard_fallbacks = 0` on the sliced run).
`ops_sha256`: `5dc8f5adc37acaed6964bbaa0393883354e20681c8946b9c589e282e17ea64bd`; `lanemap_sha256`: `ddb59e497bbe56e407cea3d67882c851cb82d06d6de67a24c31d259d46634624`. Local lanes are public; the
judge's seeded run establishes the standing.

## What to try next

- The same recomposition applies rung by rung along the Reiher rank ladder (`q70` to `q210`,
  3/3 blocks, `+-_` and `.edu`: 2.4 to 4.5 percent lower Toffolis at equal qubits, static) and
  along the Li three-group ladder (`imchxgrdky3zabCAHVIXJtRNOBWsU+-_q<b>_1`, 596 to 1,154 qubits).
- Neither score elite moves: on Reiher the PREPARE plateau (item one-hot 135 + read transient 20)
  and the SELECT plateau (one-hot 115 + held span 17 + inner residue 24) are both at 313 and the
  cheapest levers that cut either (the four-group item one-hot at about 13 Toffolis per qubit on the
  PREPARE side, `f` at 33 per qubit on the SELECT side) do not pay together; on Li the SELECT
  plateau alone is the peak and the group exchange (G = 4 to 5: 44 Toffolis per qubit) sits at the
  product's break-even. A cheaper inner-keep release or a sub-one-Toffoli-per-bit item read would be
  the mechanisms that move them.

## Attribution

Claude Mythos 5.1 in Claude Code, directed by Teddy Pender. The parent circuits and every lever
keep their recorded attribution.
