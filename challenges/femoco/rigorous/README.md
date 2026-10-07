# Rigorous frontier promotion evidence

Eight candidates rebuild the strongest estimated-track choices and each Low-layout reference
under the existing rigorous coefficient specs. They use 19/19 keep bits on Reiher and 20/21
on Li. The source manifests and exact build knobs are pinned in
[promotions.json](promotions.json). These are local maintainer checks; the staging tracks
await fresh authenticated circuit submissions before they have recorded baselines or a
rigorous leaderboard.

Each candidate passed 4,096 reference-engine lanes, 524,288 sliced-engine lanes and every
deterministic term-pair/boundary case below. The exact coefficient 1-norm errors are
0.087779 mHa (Reiher) and 0.097685 mHa (Li), against a 0.1 mHa limit on the stored Hamiltonian.
Resource counts are sampled means; deterministic coverage does not change them.

| Molecule | Promoted source role | Toffolis / step | Peak qubits | Toffolis x qubits | Deterministic lanes |
| --- | --- | ---: | ---: | ---: | ---: |
| Reiher | baseline | 10,020.000 | 1,080 | 10,821,600 | 2,242,548 |
| Reiher | fewest-toffoli | 9,486.316 | 572 | 5,426,173 | 2,242,548 |
| Reiher | best-product | 12,253.415 | 346 | 4,239,682 | 2,242,548 |
| Reiher | fewest-qubits | 18,513.768 | 286 | 5,294,938 | 2,242,548 |
| Li | baseline | 14,788.000 | 1,393 | 20,599,684 | 8,954,090 |
| Li | fewest-toffoli | 13,442.525 | 1,201 | 16,144,473 | 8,954,090 |
| Li | best-product | 19,088.368 | 578 | 11,033,077 | 8,954,090 |
| Li | fewest-qubits | 27,586.076 | 412 | 11,365,463 | 8,954,090 |

[validation-results.json](validation-results.json) retains exact errors, artifact digests,
coverage counts, the implementation tree and verification settings. The signed ledger is
unchanged. A clean trusted release build and the standard-profile coverage tests passed;
all 266 historical byte-identity pins, formatting, clippy and the signed contract passed.
The full Rust suite passed with LTO disabled, and 44 Python tests passed. Parallel thin-LTO
linking in the local environment failed; clean serial release builds and pin checks passed.
The release profile in the repository is unchanged.

The full Reiher low-Toffoli symbolic attempt exceeded a 600-second wall budget and is
**inconclusive**. The small exact SMT regressions prove correct measured uncomputation and
reject phase, cleanup, controller, measurement-dependent trace and alias-route mutants.
Neither the numerical term checks nor this proof attempt certify full quantum equivalence.
See the [verification guide](../tools/verification/README.md) for the input domains, proof
obligations, limits and reproduction commands. No new DFTHC or rotation-error bound is claimed.
