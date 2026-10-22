# One-hot angle delivery with folded inner read and hosted index: sa-toff imchxgrdkyECHKVIXtRNOBW+-_.edu, keep 8+8, blocks 3/3

Track `reiher`, architecture `onehot`. Builder `sa-toff`, keep 8 + 8, blocks
`ia` / `oa` 3 / 3, levers `imchxgrdkyECHKVIXtRNOBW+-_.edu`.

## Goal and starting point

Starts from the recorded `fa-r1tr-m8` (8,755.465 Toffolis per step at 517 qubits, score 4,526,575.4).
The goal was a lower-Toffoli point on the front, not a new score record: a census of the two
score elites shows both at a structural optimum of this lever family (every stage at the peak on
Reiher; the sole SELECT plateau on Li priced at or above the product's break-even of about 37
Toffolis per qubit), while the recorded front away from the elites was built before several
levers existed and never had them composed on.

## Mechanism

This is the unsplit-one-hot family (`E`: the angle register lives inside the one-hot, 324 slots on
Reiher) with the release levers `R N O B W`. Four shipped levers that post-date the recorded
unsplit points are added: the folded inner read `-` (the inner alias word read from the
`i_0`-refined item one-hot, half the inner iteration for 45 extra qubits during the read only),
the hosted unchosen index `_` (the alt slot moves into the inner index register through RPREP
and SELECT: `k_i` qubits off the SELECT plateau for `2 k_i` Toffolis per copy), the paired item
one-hot erasure `.e`, the sparse high keep bit `.d` and the reconstructed outer `hi` field
`.u`. At this point the peak is the Majorana stage of SELECT (one-hot 324 plus the inner
residue), 50 to 60 qubits above the PREPARE stages, so `-`'s read-time qubits are free and `_`
takes five qubits off the peak.

The change is a recomposition of shipped levers. No circuit code changes, so every recorded
circuit still builds byte for byte (`cargo test --release --locked --test sa_digests`).

**Why it is correct.** Every lever is in the shipped `sa-toff` builder and has its own tests
(exact small specs plus mutants the evaluator rejects, `src/walk/sa_low/tests_*.rs`). The
builder refuses lever combinations it does not support with an assertion, so a bundle that
builds is one the builder's rules allow. The static ledger (`tests_combo::combo_scan`) gave
8,615.7 Toffolis per step at 512 qubits; both trusted evaluations below passed every lane.

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
313, Li 17,535.5 x 471). This bundle is the lowest-Toffoli buildable point at or under 512
qubits on its track.

## Result

| | Toffolis per step | Peak qubits | Score |
| --- | ---: | ---: | ---: |
| `rdr-r3q210b-m8` (recorded) | 8,832.673 | 510 | 4,504,663.2 |
| `rdr-r3q210-m8` (recorded) | 8,832.522 | 513 | 4,531,083.8 |
| `fa-r1tr-m8` (recorded) | 8,755.465 | 517 | 4,526,575.4 |
| `pf-r1cn-m8` (recorded) | 8,729.394 | 521 | 4,548,014.3 |
| `pf-r1c-m8` (recorded) | 8,722.565 | 528 | 4,605,514.3 |
| This circuit, reference engine, 4,096 lanes | 8,614.535 | 512 | 4,410,641.9 |
| This circuit, sliced engine, 524,288 lanes | **8,615.657** | **512** | **4,411,216.6** |

Both runs passed every lane (`fallback_batches = 0`, `guard_fallbacks = 0` on the sliced run).
`ops_sha256`: `2df219e15e78681ed353873770ee99ac1aab5a3fa3c78b7e942ce8987f058390`; `lanemap_sha256`: `8ed559d79996a3425d970c566a1bfb6ebfd5d72918da129adfc58b5c5a542758`. Local lanes are public; the
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
