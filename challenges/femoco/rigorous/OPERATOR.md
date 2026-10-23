# Encoded-operator validation of the estimated-track leaders

This is local maintainer evidence for `reiher/outer-hi-reconstruct-r1` and
`li/composed-alias-m8`, selected from the ledger at base commit `fb3d4ac`. Their rebuilt
operations, lane maps and family files match all three ledger hashes. No ledger row is
created or relabelled. The [method and trust base](../tools/verification/OPERATOR.md) state
the exact operator being checked and the distinction from chemistry accuracy.

## Exact coefficient evidence

The [generated audit](operator-coefficients.json) independently reads the binary alias
tables, counts every preimage, and expands all 802,116 / 3,736,654 ordered off-diagonal
terms with exact rational arithmetic. Both coefficient L1 errors agree **exactly** with
the evaluator's `rounding_error_exact`. The audit retains the diagonal identity correction.

| Submission | Off-diagonal coefficient L1, mHa | Identity shift, mHa | Encoded-to-payload norm upper bound, mHa |
| --- | ---: | ---: | ---: |
| reiher/outer-hi-reconstruct-r1 | 149.279 | +1.111 | 150.389 |
| li/composed-alias-m8 | 868.133 | +3.749 | 871.882 |

All displayed bounds are rounded upward; decisions use the JSON's exact rationals.
The norm upper bounds concern the **declared rounded encoded operator** versus the stored
integer-angle Hamiltonian, conditional on circuit equivalence. They are conservative bounds,
not measured ground-energy errors. Neither estimated-track map satisfies the rigorous
0.1 mHa coefficient rule. This does not contradict a successful estimated-track submission.

Fresh 4,096-lane sliced checks pass with an evaluator built without `walk`, and the large
streamed exports are byte-identical to the symbolic inputs being checked. These are sampled
regressions; they do not certify the complete quantum operator.

## Full circuit equivalence

The [Reiher certificate](operator-reiher.json) and [Li certificate](operator-li.json)
both establish **whole-domain encoded-operator equivalence in the documented ideal gate
model**. All 256 exhaustive address partitions passed for each circuit. All matched the
payload reference; the alternative sign-normalized reference was unnecessary for these
two checks. These are computational certificates on the stated trust base, not
proof-assistant-checked theorems.

| Circuit | Control values | Uniform bits | Independent second-pass bits | Independent HMR bits | Partitions proved |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reiher | 2 | 32 | 14 | 4,622 | 256 / 256 |
| Li | 2 | 33 | 15 | 2,068 | 256 / 256 |

The [complete CI run](https://github.com/teddyjfpender/quantum-autoresearch-challenge/actions/runs/39426490663)
rebuilt the pinned submissions from clean source commit
`bac4b2e14b9c0dd28dd2ceddd3e0c1a717cf8dfa`. Sixteen disjoint batch reports per molecule
were assembled only after coverage, binding and matching metadata checks. The downloaded
`operator-certificates` archive (artifact 12267092931) had verified SHA-256
`4d5491a45f7361af5bf8ed8739c6886a071c5a6abcf46630f54878c5670fbf21`.
Each retained certificate pins the circuit files, payload, symbolic input, four checker
dependencies and assembly checker. PR updates can reuse these reports only after the
committed-evidence tests pass; a manual workflow dispatch always reruns the proof.

The checks prove exact gate-word/scalar-phase agreement and restoration for every control,
every uniform input, every independent second-pass value and every HMR assignment.
An incomplete partition run never counts as a whole-domain certificate. Reflection checks
and the deeper samples in SAMPLING.md remain separate evidence. Real small Rust-generated
circuits additionally exercise a three-orbital sign-normalized reference. Adversarial tests
reject altered exports, coefficient magnitudes/normalizations, spin-keyed defects, and a
defect reached only on a conjunction of 32 independent outcomes.

## Original chemistry target

The coefficient audit records hashes and bounds from the existing committed interval
chemistry reports. It **does not recompute the original integrals** or create a new
chemistry certificate.

| Existing interval quantity | Reiher | Li |
| --- | ---: | ---: |
| Fit norm upper bound, **Ha** | <213.690 | <71.160 |
| Fit norm lower bound after any scalar shift, mHa | >47.500 | >21.340 |
| Rotation/preprocessing norm upper bound, mHa | <52.935 | <368.441 |
| Ground-energy accuracy decided | No | No |

On the stated fixed-electron sector, with consistent scalar conventions, the triangle
inequality adds the encoded-to-payload, rotation/preprocessing and factorization norm
bounds. For these estimated maps, that accounting must use the actual 150.389 / 871.882
mHa coefficient-plus-identity bounds above. The existing chemistry reports' illustrative
0.2 mHa allowance assumes an accepted **rigorous** coefficient map and cannot be reused
for these estimated maps.

The lower-bound witnesses concern the operator norm of the uncorrected fit, not its
ground-energy error. The large upper bounds likewise do not establish a large ground-energy
error. These approximation errors, coefficient rounding and its identity correction,
state preparation/selection, QPE confidence/error and physical gate synthesis must be
accounted separately. The 1.15 / 0.89 mHa sampling figures quantify a different issue:
possible implementation defects relative to the rounded encoded operator.

`certifies_energy_error` and `chemical_accuracy_certified` remain **false**.
