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

## Completed exact symbolic proof

The promoted **Reiher low-Toffoli circuit** now has a completed exact symbolic quantum proof:
[exact-reiher-fewest-toffoli.json](exact-reiher-fewest-toffoli.json). All 256 exhaustive
partitions passed. The proof covers all `2^80` control/uniform/second-pass inputs and all
2,206 independent HMR outcome bits across 1,722,303 lowered operations. Its ops, lane-map,
family and payload digests match the candidate in the validation table above.

The checker proves restoration, every reset, final clean ancillas, the nested reflection
interface, exact system rotations and Pauli operations, and the scalar phase including all
measurement corrections. It uses canonical Boolean functions and exact integer-angle gate
identities against the independently decoded payload. No numerical quantum comparison or
sampled representative is needed for this certificate. All partitions ran with
`dd==0.6.0` (CUDD 3.0.0), `--partition-bits 8 --jobs 4`; the report binds the input and checker
source by SHA-256 and retains each partition's result and timing.

The earlier monolithic SMT timeout is retained in `validation-results.json` as the original
validation snapshot; it is superseded for this candidate by the completed BDD/algebra proof.
The other seven candidates retain their deterministic/numerical evidence and do not yet have
a completed full symbolic certificate. The proof is local audit evidence, not a ledger entry.

For this follow-up, all 58 Python tests, five Rust coverage tests, formatting, targeted trusted
evaluator clippy and the signed contract check passed. New tests include exact cyclotomic
Fock-matrix checks and rejection of angle, sign, spin, pivot, cleanup and measurement mutants.
The trusted evaluator built with the standard release profile; the coverage tests used LTO
disabled after the local thin-LTO linker again failed with an undefined `main` symbol.

See the [verification guide](../tools/verification/README.md) for the algebra, trust boundary,
supported structures and reproduction commands. The certificate concerns the declared logical
gates and pinned finite-precision spec. No new DFTHC or physical rotation-error bound is claimed.
