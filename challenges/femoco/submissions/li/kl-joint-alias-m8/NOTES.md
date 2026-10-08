# Joint sparse keep and aligned alt inner alias

This candidate builds on the recorded Li circuit `li/kl-aligned-inner-m8`. It retains that circuit’s spectrum-amplified encoding, four-group split one-hot angle delivery, item one-hot inner read, folded index, measured erasure, and eight-bit outer and inner alias draws. It changes only the arrangement of the 285 Li square inner alias tables under the new default-off `.f` extension, which requires the recorded `.b` and `+` levers. Existing build strings retain their exact operation streams.

## Mechanism and correctness

The recorded `.b` arrangement concentrates keep bit 7 in eight inner bucket rows. Its alt bit 5 still varies over many folded leaves. The joint witness preserves the high keep concentration with two extra permitted rows (24 and 34), and aligns alt bit 5 with the shared top donor on rows 18 through 37 in every table. The paired folded read can then omit twelve Toffolis per copy. Each table has 64 keep and 64 alt bytes, has 58 item counts that are exact floor or ceiling of the pinned ideal, sums to 16,384 lanes, and keeps padding rows 61–63 on one donor. The trusted estimated-class check accepts the resulting lane map. The existing reversible read and erasure gadgets are unchanged.

The witness is embedded as Rust source in `src/walk/sa_low/jointalias.rs`; the build needs no solver, network, or runtime data file. The parser verifies every row's shape, index range, padding, and donor condition. Focused tests enumerate a small exact alias table, detect a corrupt alternate target, verify all pinned joint-table constraints, and reject a pinned table-count mutant. This is a table-data layout change using the existing reversible gadgets.

The final exact witness preserves **every item count in all 285 inner square tables** from `.b`. The resulting exact rounding error remains **0.5603668293**. An earlier S10 witness met the bit-support constraints but changed counts in every table and doubled rounding error to 1.1623491384; it was replaced before submission. The pinned test compares every count vector and the exact rounding-error value with the recorded parent.

## Cost model and early screens

The parent static circuit is 17,675.5 Toffolis per step at 471 peak logical qubits. The joint table read is 793 Toffolis per copy instead of 805; all other measured stages are unchanged. The static candidate is 17,651.5 at Q=471, a 24-C improvement. Against the recorded parent full sliced measurement of 17,675.392 × 471, the measured product improves by 0.136% rounded. This is an unranked local measurement; only the pull-request judge establishes standing. A stricter eight-row high-keep witness was attempted, but one hard table remained solver-unknown in a bounded retry; the ten-row certified layout is the submitted candidate.

An architecture screen considered a persistent nonlinear angle code for the 15 central rotations. Its best code had D′=210 with at least three extra live qubits; even ideal encode/decode floors cost 210+207 Toffolis per copy, and the current synthesis costs about 315 per encode. It offered no credible path to a >1% product gain at the existing 471-qubit peak, so it was not introduced into the recorded circuit.

## Validation

The exact small-table count and corrupt-alt test passed. The pinned structural test verified all 285 tables, the exact floor/ceiling count certificate, accepted estimated class, and a rejected count mutant. All **266 recorded-circuit digests passed**, showing the default-off change preserves recorded builds.

The trusted 4,096-lane reference evaluation passed at **17,652.356 Toffolis/step × 471 qubits = 8,314,259.676**. The trusted 524,288-lane sliced evaluation passed at **17,651.395 × 471 = 8,313,807.045**, with zero fallback batches or guard fallbacks. Both report rounding error 5.604e-1, equal to the recorded parent. The slice result is the comparable full measurement; build and evaluator wall times are not the score.

## Attribution

Developed by GPT-6 in Codex under Teddy Pender's direction. The parent design and previous levers remain credited to their recorded submitters. Research discussion: https://github.com/teddyjfpender/quantum-autoresearch-challenge/discussions/4.
