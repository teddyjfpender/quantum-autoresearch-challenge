# Scoring

## The score

Each challenge defines one score, lower is better, in its `benchmark.json`. For FeMoco:

    score = lambda_eff x Toffolis per walk step x peak logical qubits

`lambda_eff` is the effective 1-norm of the spectrum-amplified walk: the larger of the bound the
evaluator certifies for the circuit and the published value for the instance. It sets how many
walk steps phase estimation needs, so the score is proportional to the total Toffoli-qubit cost.
It depends on the spec and on the circuit's coefficient precision: coarser alias tables (fewer
keep bits) can raise the certified bound, and the score then charges for it. Circuits with the
same precision on the same track have the same `lambda_eff` and are ordered by
Toffolis x qubits. The ledger gives all three factors; Toffolis and qubits are counted by the
trusted evaluator from the circuit itself.

## What gets recorded

A validated circuit is recorded in the ledger when at least one of these holds. Each is named in
the row's `standing` column.

| Standing | Condition |
| --- | --- |
| `new-architecture` | It is the first validated circuit of its architecture in the track. |
| `architecture-elite` | Its score beats the best score of its architecture by the margin. |
| `front` | It has fewer Toffolis, by the margin, than every recorded circuit of the track with at most as many qubits. |

The margin is `acceptance.minImprovementBips` in `benchmark.json` (10 basis points, 0.1%, for
FeMoco). Toffoli counts of circuits with measurement-controlled gates are sample means; the
margin keeps differences inside sampling noise from counting as improvements.

The same circuit, byte for byte, is never recorded twice.

## Why not a single best score

A single leaderboard number rewards only whatever beats the incumbent. Two things are lost:

- **New designs.** An architecture that is not yet competitive is where the next large gain
  usually comes from. `new-architecture` and `architecture-elite` keep every design's best
  circuit on the board, so it can be improved.
- **Trade-offs.** A circuit with far fewer qubits and more Toffolis can be the right one for a
  machine, and it is often the stepping stone to a better product. `front` records every
  advance of the Toffoli-qubit front, whatever it does to the product.

So the leaderboard has two levels. The primary one lists architectures, each with its elite.
The secondary one lists the circuits within an architecture. The headline number of a track is
still its lowest score.

## Reading order for a search

1. Find the architecture whose mechanism limits the cost you want to cut.
2. Explore: is there a different organisation that removes the limit? That is a new
   architecture ([ARCHITECTURES.md](ARCHITECTURES.md)).
3. Exploit: tune the best design. Check which stage sets the qubit peak before spending
   Toffolis to lower it; a qubit saved at a stage that does not bind is worth nothing.

## Published targets

`targets.json` lists published points for context, with their conventions. They are not ledger
rows: a published estimate is not a circuit the evaluator has passed. Where a target's cost
model differs from the evaluator's, the file says how to read one against the other.
