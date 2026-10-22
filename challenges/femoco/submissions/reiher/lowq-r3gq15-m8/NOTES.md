# Rank-scheduled delivery at budget 15 with padding runs, four-group item one-hot and hosted index: sa-toff imchxgrdky3zabCHVITXSNOBWs_q15_1.11.p, keep 8+8, blocks 2/2

Track `reiher`, architecture `rank-scheduled`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 2 / 2, levers `imchxgrdky3zabCHVITXSNOBWs_q15_1.11.p`.

## Goal and starting point

Starts from the recorded `lc-r3cq-m8` (13,191.725 Toffolis per step at 298 qubits, score 3,931,134.0).
The goal was a lower-Toffoli point on the front, not a new score record: a census of the two
score elites shows both at a structural optimum of this lever family (every stage at the peak on
Reiher; the sole SELECT plateau on Li priced at or above the product's break-even of about 37
Toffolis per qubit), while the recorded front away from the elites was built before several
levers existed and never had them composed on.

## Mechanism

The 298-qubit rank-scheduled point `lc-r3cq-m8` with its item-clear lever `K` removed: without `K`
the item index is not rebuilt from the item one-hot after the read, which here lowers the inner
read stage, and the outer witness and release levers keep the SELECT side at the same height.
The scan found it as the lowest-Toffoli bundle at or under 306 qubits; it dominates `rp-r3y-m8`
(12,659.4 at 311).

The change is a recomposition of shipped levers. No circuit code changes, so every recorded
circuit still builds byte for byte (`cargo test --release --locked --test sa_digests`).

**Why it is correct.** Every lever is in the shipped `sa-toff` builder and has its own tests
(exact small specs plus mutants the evaluator rejects, `src/walk/sa_low/tests_*.rs`). The
builder refuses lever combinations it does not support with an assertion, so a bundle that
builds is one the builder's rules allow. The static ledger (`tests_combo::combo_scan`) gave
12,589.5 Toffolis per step at 306 qubits; both trusted evaluations below passed every lane.

## Architecture

`rank-scheduled`: a split one-hot with a held subspace of angle directions under the budget `q<b>`; the levers
added are refinements within it (lookup layout, erasure and release choices), not a new
organisation of the step.

## Experiments

Static scans with `tests_combo::combo_scan` (seconds per bundle) over the neighbourhood of every
recorded front point: single, pair and triple toggles of the levers `+ - _ s O J W N B R U K T D P
t S ~ A o l` and the extensions `.e .d .u .p .g .c .h<k>` (Li: `.b .f .i .j .k .l .a`), the
block exponents 1 to 4 (8 and 9 for the paired lookup), the keep split 9 + 9, and the rank
budgets and Majorana parks around each ladder rung; about 200,000 bundles in all, of which the
builder refused about half. Nothing in that space lowers either score elite (Reiher 11,462.5 x
313, Li 17,535.5 x 471). This bundle is the lowest-Toffoli buildable point at or under 306
qubits on its track.

## Result

| | Toffolis per step | Peak qubits | Score |
| --- | ---: | ---: | ---: |
| `rp-r3y-m8` (recorded) | 12,659.411 | 311 | 3,937,076.8 |
| This circuit, reference engine, 4,096 lanes | 12,589.008 | 306 | 3,852,236.5 |
| This circuit, sliced engine, 524,288 lanes | **12,589.486** | **306** | **3,852,382.7** |

Both runs passed every lane (`fallback_batches = 0`, `guard_fallbacks = 0` on the sliced run).
`ops_sha256`: `f84a24398686dc0962352b6e990d5eae7a6ca693d958db6dd401bb756026333b`; `lanemap_sha256`: `ad9002d6b0b1560114a1ad0a25ef1267b48b0db18dffb7ebfc5e5047ca4611c3`. Local lanes are public; the
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
