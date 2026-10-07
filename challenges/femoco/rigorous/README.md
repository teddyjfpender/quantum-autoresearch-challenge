# Rigorous frontier promotion evidence

Eight candidates rebuild the strongest estimated-track choices and each Low-layout reference
under the rigorous coefficient specs. They use 19/19 keep bits on Reiher and 20/21 on Li. The
source manifests and exact build knobs are pinned in [promotions.json](promotions.json).
These are local maintainer checks, not ledger rows: the staging tracks have no recorded
baseline or leaderboard until circuits are submitted and judged.

Each candidate passed 4,096 reference-engine lanes, 524,288 sliced-engine lanes and every
deterministic lane below. The exact coefficient 1-norm errors are 0.087779 mHa (Reiher) and
0.097685 mHa (Li), against a 0.1 mHa limit on the stored Hamiltonian. Resource counts are
sampled means; deterministic coverage does not change them.

| Molecule | Promoted source role | Toffolis / step | Peak qubits | Toffolis x qubits | Deterministic lanes |
| --- | --- | ---: | ---: | ---: | ---: |
| Reiher | baseline | 10,020.000 | 1,080 | 10,821,600 | 4,523,712 |
| Reiher | fewest-toffoli | 9,486.316 | 572 | 5,426,173 | 4,523,712 |
| Reiher | best-product | 12,253.415 | 346 | 4,239,682 | 4,523,712 |
| Reiher | fewest-qubits | 18,513.768 | 286 | 5,294,938 | 4,523,712 |
| Li | baseline | 14,788.000 | 1,393 | 20,599,684 | 17,940,384 |
| Li | fewest-toffoli | 13,442.525 | 1,201 | 16,144,473 | 17,940,384 |
| Li | best-product | 19,088.368 | 578 | 11,033,077 | 17,940,384 |
| Li | fewest-qubits | 27,586.076 | 412 | 11,365,463 | 17,940,384 |

The deterministic lanes are one representative of every reachable term pair under both
controls and every value of the outer and inner spin bits, plus the alias comparator
boundaries: 1,693,848 term-pair and 568,008 boundary cases on Reiher, 7,666,832 and 1,303,360
on Li, each under both controls. On the machine that produced this evidence (four threads)
the full stage with coverage took 34 to 49 seconds per Reiher candidate and 5 to 7 minutes
per Li candidate.

[validation-results.json](validation-results.json) holds the exact errors, artifact digests,
coverage counts, timings, and the commit and toolchain that produced them. Every field of it
and of the certificate below is written by
[`tools/repro/promote_rigorous.py`](../tools/repro/promote_rigorous.py), from a clean
checkout; the repository's tests check that the files match the current promotion catalogue
and checker and that the table above quotes them.

## Symbolic check of the Reiher low-Toffoli circuit

[exact-reiher-fewest-toffoli.json](exact-reiher-fewest-toffoli.json) is a certificate from the
symbolic gate-word checker for the Reiher fewest-Toffoli candidate. All 256 partitions were
proved, covering all `2^80` control/uniform/second-pass inputs and all 2,206 independent
measurement-outcome bits over 1,722,303 lowered operations. Its ops, lane-map, family and
payload digests are those of the candidate in the table, and the checker recomputed the first
three from the circuit files.

What the certificate shows: in the evaluator's lowered-op model, for every such input and
outcome, the circuit restores its control, uniform register and ancillas, every reset is
clean, and the system gate word equals the reference word built from the pinned `sa.bin`,
with the expected scalar phase including every measurement correction. No numerical
tolerance or sampled lane enters.

What it rests on: the evaluator's lowering and export, a small set of gate identities
(tested on adjacent modes only), the spin-swap gate taken as an opaque inverse pair, the
reflection as an interface, the checker's own transcription of the spec, and CUDD. It is
not a machine-checked proof, and it says nothing about coefficient magnitudes beyond the
rounding rule above, the original chemistry approximation, or rotation synthesis. The full
statement is in the [verification guide](../tools/verification/README.md#trust-base).

The other seven candidates have the deterministic and sampled evidence above and no
symbolic certificate. The run used `dd` 0.6.0 with CUDD, `--partition-bits 8 --jobs 4`, and
took under six minutes.

## Energy audit of the published factors

The separate [chemistry checker](../tools/chemistry/README.md) computes interval bounds from
the pinned original integrals and the published DFTHC factors. Its
[Reiher](energy-reiher.json) and [Li](energy-li.json) reports give determinant witnesses for
a lower bound on the operator norm of the fit residual of the published, unrounded factors:
more than 47.500 mHa and 21.340 mHa respectively, whatever scalar is subtracted. So those
factors cannot meet a chemical-accuracy budget stated as an operator norm over the whole
electron-number sector.

This does not determine their ground-energy errors, and at the current rotation precision it
establishes nothing about the rounded Hamiltonians the circuits encode. Both reports say
`chemical_accuracy_certified: false`. The audit also holds validated rotation-precision
proposals, which are not circuit specs, and Slater energy bounds for the stored Hamiltonian.
