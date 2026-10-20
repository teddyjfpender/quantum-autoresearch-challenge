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

At this draft's recording, full-size all-input checks are still in progress. **Neither
headline circuit is yet claimed certified by this new checker.** Real small Rust-generated
circuits have complete certificates, including a three-orbital sign-normalized circuit.
The tests reject altered exports, wrong coefficient magnitudes/normalizations, spin-keyed
defects, and a defect reached only on a conjunction of 32 independent outcomes.

The large-run obligation is exact gate-word/scalar-phase agreement and restoration for
every control, every uniform input, every independent second-pass value and every HMR
assignment. An incomplete partition run never counts as a whole-domain certificate.
Reflection certificates and the deeper samples documented in SAMPLING.md remain separate
evidence; their passing status is not promoted into an operator-equivalence certificate.

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

The lower-bound witnesses concern the operator norm of the uncorrected fit, not its
ground-energy error. The large upper bounds likewise do not establish a large ground-energy
error. These approximation errors, coefficient rounding and its identity correction,
state preparation/selection, QPE confidence/error and physical gate synthesis must be
accounted separately. The 1.15 / 0.89 mHa sampling figures quantify a different issue:
possible implementation defects relative to the rounded encoded operator.

`certifies_energy_error` and `chemical_accuracy_certified` remain **false**.
