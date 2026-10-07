# Sparse high keep bit in Reiher inner alias tables

This submission refines the recorded `kc-r3re-m8` rank-scheduled Reiher circuit. It keeps that
circuit's architecture, alias resolutions, two lookup block sizes, angle schedule, and qubit
budget. The new default-off `.d` extension changes only the 270 square inner alias tables at
`mu_i = 8`. It is discussed alongside broader PREPARE and angle-delivery options in
[Discussion #4](https://github.com/teddyjfpender/quantum-autoresearch-challenge/discussions/4).

## Mechanism and correctness

The estimated rounding class permits each item's lane count to be either the exact floor or the
exact ceiling of its ideal `8192 |w_i| / sum_j |w_j|`. The original builder chooses one largest-
remainder assignment and a Walker alias pairing. Neither choice is unique. An offline integer
search retained the original counts in 268 of 270 square tables and changed only four item
counts in total: in each of tables 20 and 117, one 128-lane item gains a lane and one other
item loses one. Both changes stay within exact floor/ceiling bounds. New alias pairings then
satisfy these constraints for every table:

1. All 32 bucket values are an exact Walker alias table. Own keep is in `0..255`; each alternate
   names one of the 28 real items, and padding buckets have keep zero.
2. The count of every inner item is a legal floor or ceiling, and all counts sum to 8192.
3. The four padding alternates match the existing donor-padded `+` table, preserving its top
   item-constant offset and skipped suffix.
4. The high bit of keep is zero at twelve shared own indices: `1, 2, 5, 9, 10, 11, 14, 17,
   19, 22, 25, 27`.

The data in `src/walk/sa_low/sparsealias.rs` is an inline Rust string with 270 hexadecimal rows
of 32 keep bytes followed by 32 alternate bytes. Its decoder runs only for the pinned estimated Reiher spec at
`(k_i, mu_i) = (5, 8)` with `+`. The trusted evaluator independently recomputes the exact
counts and validates every sampled lane against that lane map. A dedicated unit test checks
the full rounding class and mutates one keep by two lanes; the trusted rounding check rejects
the mutant. The `.d` knob is off in every recorded build, preserving their byte identities.

The two-group item one-hot read pays one paired Toffoli for a word bit at an inner index if any
square table needs that bit. The twelve shared zeros remove one high-keep-bit product per index
per inner copy. No selection, Givens, erasure, or qubit-lifetime path changes.

## Measurements

| Reiher bundle | Expected Toffolis/step | Peak qubits | Product |
| --- | ---: | ---: | ---: |
| Parent `kc-r3re-m8` | 11500.5 static (11500.479 recorded) | 313 | 3,599,649.927 recorded |
| This `.d` candidate | 11476.521 full sliced | 313 | 3,592,151.073 local |

The exact static stage ledger changes `copy: inner alias QROAM read` from 466.0 to 454.0
Toffolis per copy, and no other row. There are two copies, giving a 24.0 static Toffoli
reduction (0.209% at fixed Q). The candidate is a small additive gain, not a new architecture.
The final four-count payload's 4,096-lane reference run measured `C = 11477.767`, `Q = 313`;
the 524,288-lane sliced run measured `C = 11476.521`, `Q = 313`, with
`fallback_batches = 0`. The full local score is `3,592,151.073`, a 0.208% reduction from the
recorded parent score. The small difference
between the reference sample and full sliced count is expected from outcome-gated erasures.
The exact 1-norm rounding error is `0.1492367075` Ha for the parent map and `0.1492782215`
Ha for this map, an increase of `0.0000415140` Ha; this estimated-class track has no 1-norm
acceptance bound. Both full validation commands used the four-count payload.

## Checks and failed directions

- The pinned exact-rounding check and a two-lane mutant rejection pass.
- A complete 128-lane sampled circuit check passed on the initial legal table witness;
  the final payload passed both challenge validation engines as listed above.
- All 266 recorded circuit digest checks pass byte for byte.
- A stronger uniform seven-bit keep register was investigated. It is impossible even with
  arbitrary legal floor/ceiling rerounding: Reiher square table 47 has no Walker alias
  solution with all own keeps below 128 (exact integer solver, unrestricted alternate targets).
  This candidate instead removes sparse high-bit reads where every table can agree.
- Repartitioning the outer alias table alone cannot change the current QROAM Toffoli or qubit
  counts; its lookup and measured fixup traverse fixed dimensions. Random Walker pairing of
  the inner tables also left the paired read's global bit support unchanged.

The implementation was developed with GPT-6 in Codex. Local checks are evidence for
this candidate; the challenge judge determines any recorded standing.
