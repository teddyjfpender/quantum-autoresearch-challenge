# What sampled validation does and does not establish

A ledger row says that every sampled lane of a circuit passed. This document states what
follows from that, what does not, and what it would take to close each gap. The numbers come
from [`tools/analysis/walk_defect.py`](../tools/analysis/walk_defect.py).

**In short.** At the default `2^19` lanes a row is a strong regression check and a weak
energy statement. It bounds the operator error at 37 mHa (Reiher) and 114 mHa (Li), and only
under an assumption that sampling cannot check. Without that assumption a deliberately built
circuit can pass most draws and still move the energy that phase estimation reports by tens
of millihartree. No row certifies chemical accuracy, and `score.json` says so
(`metrics.sampling_limits.certifies_energy_error: false`).

## 1. What a passing sample bounds

Every sampled lane passed. With confidence `1 - 2 delta` (`delta = 1e-6`) the operator weight
that sits on wrong control-1 lanes is at most

`g = 2 f_p1 + f_d1`, with `f = ln(1/delta) / K` over each class of control-1 lanes,

(DESIGN.md section 16.4; `metrics.sampling_limits.wrong_weight_bound`). At `K = 2^19` each
class holds about `K / 4` lanes and `g = 3.2e-4`. What `g` implies depends on what the wrong
lanes do.

## 2. Three kinds of defect

| Defect on the unsampled lanes | Effect on the energy | Lanes for a 1.6 mHa statement |
| --- | --- | --- |
| Hermitian: the step is still a reflection | at most `2 Lambda g`, linear | `2^23.5` Reiher, `2^25.2` Li |
| Not Hermitian, tuned to resonance | about `lambda_eff sqrt(2 g)`, a square root | `2^35.8` Reiher, `2^37.8` Li |
| Reached only on a rare measurement outcome | a failure per step with probability up to `f` | see 2.3 |

### 2.1 Hermitian defects: the linear bound

If the block encoding is Hermitian on every lane, the step `C` is a reflection whatever the
wrong lanes do, qubitization is exact for the operator `A'` that the circuit actually encodes,
and `||A' - A|| <= 2 Lambda g`. This is `implied_error_bound`: 36.9 mHa (Reiher) and
113.7 mHa (Li) at the default sample.

**The condition is not checked.** A sampled lane is checked exactly, so it is Hermitian there.
Nothing constrains an unsampled lane.

### 2.2 Non-Hermitian defects: the square root

Let a defect act on a set `B` of lanes of weight `f` and make the step non-Hermitian there.
The defected walk has eigenvectors localized on `B`. While the step is a reflection they do
not couple to the walk's ground subspace. When it is not, they couple with amplitude about
`sqrt(f)`, because the ground walk state has weight `f` on `B`. If the localized eigenvalue is
placed on the ground eigenphase `e^{i theta0}`, the two levels are degenerate and the coupling
splits them by about `sqrt(2 f)` in phase. Phase estimation then reports an energy displaced
by about `lambda_eff sqrt(2 f)`.

In the flat model this is exact: the walk is a rank-one perturbation of a diagonal unitary and
its spectrum on the start state solves one secular equation (the script's docstring). The
displaced peaks, in mHa, at the fraction `f = 1.9e-6` that passes `2^19` lanes 61% of the time:

| Defect on a fraction `1.9e-6` of lanes | Reiher | Li |
| --- | ---: | ---: |
| sign (Hermitian) | 0.22 | 0.68 |
| phase `i` (not Hermitian, off resonance) | 0.27 | 0.78 |
| tuned to one branch | 41.7 | 85.3 |
| tuned to both branches | 29.5 | 60.2 |

The fitted exponents are 1.00 for the first two and 0.50 for the tuned defects. With both
branches tuned, no phase-estimation outcome lands within 1.6 mHa. Phase estimation runs about
`T = pi lambda_eff / (2 epsilon)` steps (21,000 and 43,000) and resolves splittings down to
`f` of order `1 / T^2`, which is why the lanes needed grow as `T^2` and not `T`.

**What it takes to build one.** The localized eigenvalue must sit within the splitting of
`theta0`, a few milliradians. Sign errors, a missed `S` gate and the `Z/8` phases cannot do
this: in the flat model their eigenvalues are eighth roots of unity, at least 0.24 rad from
`theta0` on both tracks, so they stay linear. A continuous angle can: a rotation whose compute and uncompute
disagree on rare lanes, at the resolution of the rotation register. This is a deliberate
construction, not a plausible accident.

`metrics.sampling_limits.non_hermitian_phase_scale_rad` reports `sqrt(2 g)` for the run's own
`g`: 0.025 rad at the default sample, about 540 mHa (Reiher) and 1,100 mHa (Li) when
multiplied by `lambda_eff`. It is the scale of the effect in this model at the confidence of
`g`, not a certified bound.

### 2.3 Rare measurement branches

Measurement outcomes are drawn uniformly on every lane. A block conditioned on one outcome
bit runs on half the lanes. A block conditioned on a conjunction of `k` outcome bits runs on a
fraction `2^-k` of them, and beyond about 19 bits on none. Such a block is not validated.

A defect there is stochastic, not coherent: it strikes a step with probability up to `f` and
a run of `T` steps with probability about `T f`. At the default bound that is 0.67 (Reiher)
and 0.90 (Li). It scales with `T`, not `T^2`.

## 3. What a row therefore says

- Every sampled lane applies the declared operator, restores its ancillas and leaves no phase.
- The Toffoli and qubit counts are those of the sampled executions.
- **Not** that the unsampled lanes are Hermitian, **not** an energy error, **not** that every
  conditioned block was executed.

For comparing circuits built by the same builders under the same rule, which is what the
board does, this is the standard the challenge started from (every drawn lane passes). It is
not sufficient for a claim about a ground-state energy.

## 4. Closing the gaps

In order of strength.

1. **A symbolic check over the whole domain.** The checker in `tools/verification` (where
   present) compares the gate word with the spec for every input and every measurement
   outcome. A circuit with such a certificate has no unsampled lanes, and none of section 2
   applies to it. This is the only route here that supports a claim about a final result.
2. **A step that is a reflection by construction.** Today the two inner copies must be
   identical op for op (DESIGN.md section 16.3). If instead the second half of the step had to
   be the syntactic adjoint of the first, the step would be Hermitian on every lane whatever
   the lanes do, the linear bound of 2.1 would hold unconditionally, and `2^24` to `2^26`
   lanes would give a real statement. It needs adjoint rules for measurement-based
   uncomputation and for rotation angles, and it changes the contract for every builder. Not
   implemented.
3. **More lanes.** Under the Hermitian assumption `2^24` lanes bound the Reiher error at
   1.2 mHa and `2^26` the Li error at 0.9 mHa. Without it, lanes alone do not reach chemical
   accuracy at any practical count. The re-validation workflow takes a lane count, so a
   headline row can be re-run deeper; at the measured sliced-engine rate `2^24` lanes take
   about 4 minutes (Reiher) and 20 minutes (Li) on four threads.
4. **Stratified measurement outcomes.** Force every classically conditioned block to run on a
   fixed share of the validation lanes, keeping uniform outcomes for the cost count. This
   removes 2.3. Not implemented.

Items 2 and 4 are open design work. Until one of 1 or 2 holds for a circuit, its row is a
sampled regression result and is labelled as one.
