# One-hot angle delivery with folded inner read, hosted index and composed alias witnesses: sa-toff imchxgrdkyECHKVIXtRNOBW+-_.ebfijklu, keep 8+8, blocks 3/3

Track `li`, architecture `onehot`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 3 / 3, levers `imchxgrdkyECHKVIXtRNOBW+-_.ebfijklu`.

## Goal and starting point

Starts from the recorded `kl-l3q618p-m9` (12,821.674 Toffolis per step at 1166 qubits, score 14,950,071.9).
The goal was a lower-Toffoli point on the front, not a new score record: a census of the two
score elites shows both at a structural optimum of this lever family (every stage at the peak on
Reiher; the sole SELECT plateau on Li priced at or above the product's break-even of about 37
Toffolis per qubit), while the recorded front away from the elites was built before several
levers existed and never had them composed on.

## Mechanism

The Li unsplit one-hot (`E`, 931 slots) read with the item one-hot family (`H K V I X t`), the
folded inner read `-`, the hosted index `_`, donor padding `+` and the composed alias-table
witnesses `.b .f .i .j .k .l` with the reconstructed outer `hi` `.u`. The recorded low-Toffoli
Li points use the paired unary lookup (`M n F`) at slot width 8 or 9 and the whole-span rank
schedule `q618_1`; with the one-hot unsplit the span is free, and the item one-hot read with the
witnesses is cheaper than the paired lookup at this width. Keep 8 + 8 (the class rule's minimum).

The change is a recomposition of shipped levers. No circuit code changes, so every recorded
circuit still builds byte for byte (`cargo test --release --locked --test sa_digests`).

**Why it is correct.** Every lever is in the shipped `sa-toff` builder and has its own tests
(exact small specs plus mutants the evaluator rejects, `src/walk/sa_low/tests_*.rs`). The
builder refuses lever combinations it does not support with an assertion, so a bundle that
builds is one the builder's rules allow. The static ledger (`tests_combo::combo_scan`) gave
12,592.5 Toffolis per step at 1166 qubits; both trusted evaluations below passed every lane.

## Architecture

`onehot`: the unsplit one-hot with the angle register embedded in it (`E`); the levers
added are refinements within it (lookup layout, erasure and release choices), not a new
organisation of the step.

## Experiments

Static scans with `tests_combo::combo_scan` (seconds per bundle) over the neighbourhood of every
recorded front point: single, pair and triple toggles of the levers `+ - _ s O J W N B R U K T D P
t S ~ A o l` and the extensions `.e .d .u .p .g .c .h<k>` (Li: `.b .f .i .j .k .l .a`), the
block exponents 1 to 4 (8 and 9 for the paired lookup), the keep split 9 + 9, and the rank
budgets and Majorana parks around each ladder rung; about 200,000 bundles in all, of which the
builder refused about half. Nothing in that space lowers either score elite (Reiher 11,462.5 x
313, Li 17,535.5 x 471). This bundle is the lowest-Toffoli buildable point at or under 1166
qubits on its track.

## Result

| | Toffolis per step | Peak qubits | Score |
| --- | ---: | ---: | ---: |
| `kl-l3q618p-m9` (recorded) | 12,821.674 | 1166 | 14,950,071.9 |
| `fa-l3q618i9-m9` (recorded) | 12,879.512 | 1166 | 15,017,511.0 |
| `rdl-l3q618-m9` (recorded) | 13,089.566 | 1168 | 15,288,613.1 |
| `pf-l1c-m8` (recorded) | 13,116.502 | 1206 | 15,818,501.4 |
| `pf-l1c-m9` (recorded) | 13,189.020 | 1210 | 15,958,714.2 |
| This circuit, reference engine, 4,096 lanes | 12,591.315 | 1166 | 14,681,472.9 |
| This circuit, sliced engine, 524,288 lanes | **12,592.479** | **1166** | **14,682,830.8** |

Both runs passed every lane (`fallback_batches = 0`, `guard_fallbacks = 0` on the sliced run).
`ops_sha256`: `d5f876dab511c0611afbd29a2cf1a0f530291314750c802a7bfc7128704e2d2f`; `lanemap_sha256`: `ddb59e497bbe56e407cea3d67882c851cb82d06d6de67a24c31d259d46634624`. Local lanes are public; the
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
