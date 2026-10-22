# What sampled validation does and does not establish

A ledger row says that every sampled lane of a circuit passed. This document states what
follows from that, what does not, and what it would take to close each gap. The numbers come
from [`tools/analysis/walk_defect.py`](../tools/analysis/walk_defect.py).

**In short.** At the judge's full stage (`2^19` lanes) a row bounds the distance between the
operator the circuit encodes and the one the spec declares at 37 mHa (Reiher) and 114 mHa
(Li), reported as `metrics.bound_mHa`, and only under an assumption that sampling cannot
check: that the step is Hermitian on the lanes that were not drawn. The reflection check
(section 4.1) proves that assumption for a circuit, without sampling; from `2^24` lanes the
evaluator runs it by default and refuses a circuit it cannot certify, and `score.json` says
whether a run's bound is unconditional (`metrics.sampling_limits.bound_certified`). At `2^24`
lanes the bound is 1.15 mHa on Reiher, below chemical accuracy (1.6 mHa), and 3.55 mHa on Li,
which needs `2^26` (0.89 mHa). The statistics were re-derived and are sound and nearly tight
(section 1); the one term they left out, the Gaussian tracker's tolerance, is now named and
added (`7.3e-7` and `3.7e-6` Ha). A `2^24` full stage with the check required is proposed as
the next contract epoch (VALIDATION.md); what holds it back is the check's memory and time on
the Li track, measured in sections 4.1 and 4.2. No row certifies the energy of FeMoco: the
bound is about the encoded operator against the spec's, not about the chemistry or the
rotation approximation behind the spec, and `score.json` says so
(`metrics.sampling_limits.certifies_energy_error: false`).

## 1. What a passing sample bounds

Every sampled lane passed. With confidence `1 - 2 delta` (`delta = 1e-6`) the operator weight
that sits on wrong control-1 lanes is at most

`g = 2 f_p1 + f_d1`, with `f = ln(1/delta) / K` over each class of control-1 lanes,

(DESIGN.md section 16.4; `metrics.sampling_limits.wrong_weight_bound`). At `K = 2^19` each
class holds about `K / 4` lanes and `g = 3.2e-4`; at the full stage `K = 2^24`, `g = 9.9e-6`.
What `g` implies depends on what the wrong lanes do.

The statement was re-derived for the `v3` epoch (DESIGN.md 16.4 gives each step). It is
sound and nearly tight: the per-class fraction is the Bernoulli bound `1 - delta^(1/K)` in its
linear form, looser by `5e-5` relative at `2^19` and `2e-6` at `2^24`
(`wrong_weight_bound_exact`); lanes are equiprobable in the operator's measure, since the
alias tables give a term its weight by how many lanes name it, so counting wrong lanes is
weighting them; the factor 2 on the paired class is the inner reflection's `2 |+><+| - I`;
and the step from weight to norm is the triangle inequality with a unitary lane operator at
most 2 from its reference, attained by a sign flip. A re-balanced draw (`sqrt(2) : 1` paired to
diagonal) would gain 3%. The lever is the lane count: `g ~ 12 ln(1/delta) / K`.

One term is not in `g`. A lane the Gaussian tracker passed is right only to its tolerance
(`AD_TOL = 1e-11`, DESIGN.md section 15.2): the rotated operator may differ from the reference
by up to `(pi/2) sqrt(n) (2 n AD_TOL) / sqrt(8)` per lane, `1.3e-8` at `n = 108` and `2.1e-8`
at `n = 152`, which over the whole operator is `7.3e-7` Ha (Reiher) and `3.7e-6` Ha (Li). It is
not in `rounding_error` (which is exact, about the lane map's counts) and not in
`implied_error_bound` (which is about lanes not drawn). `score.json` names it
(`metrics.sampling_limits.tracker_tolerance_bound_mHa`) and adds it: `metrics.bound_mHa` and
`certified_bound_mHa` are `implied_error_bound` plus this term. It does not change the
acceptance rule; folding it into the rule itself would be a contract change.

### 1.1 The bound per lane count

`python3 tools/analysis/walk_defect.py --lanes`. Single draw, confidence `1 - 2 delta`; the
lane time is the sliced engine on an Apple M5 Max at four threads, the judge runner's core
count, without the reflection check (section 4.2 has measured runs).

| Lanes | `g` | Reiher bound, mHa | Reiher lanes, min (4 thr) | Li bound, mHa | Li lanes, min (4 thr) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| `2^19` | 3.16e-04 | 36.90 | 0.0 | 113.67 | 0.2 |
| `2^20` | 1.58e-04 | 18.45 | 0.1 | 56.83 | 0.3 |
| `2^21` | 7.91e-05 | 9.22 | 0.2 | 28.42 | 0.6 |
| `2^22` | 3.95e-05 | 4.61 | 0.4 | 14.21 | 1.3 |
| `2^23` | 1.98e-05 | 2.31 | 0.8 | 7.10 | 2.6 |
| `2^24` | 9.88e-06 | 1.15 * | 1.6 | 3.55 | 5.1 |
| `2^25` | 4.94e-06 | 0.58 * | 3.2 | 1.78 | 10.2 |
| `2^26` | 2.47e-06 | 0.29 * | 6.4 | 0.89 * | 20.5 |
| `2^27` | 1.24e-06 | 0.14 * | 12.7 | 0.44 * | 40.9 |
| `2^28` | 6.18e-07 | 0.07 * | 25.4 | 0.22 * | 81.9 |

Chemical accuracy needs `2^23.5` lanes on Reiher and `2^25.2` on Li, so `2^24` and `2^26`
are the smallest powers of two. The proposed full stage is `2^24` on every track: the smallest
count that puts one track below chemical accuracy and keeps the Li run, reflection check
included, inside the judge's 120-minute job in time (section 4.2). Below 1.6 mHa on Li needs
the `2^26` run of section 4.2, which the re-validation workflow can make (`full_samples`).

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
113.7 mHa (Li) at the full stage, 1.15 and 3.55 mHa at `2^24` lanes.

**Sampling does not check the condition.** A sampled lane is checked exactly, so it is
Hermitian there. Nothing in the sample constrains an unsampled lane. The reflection check of
section 4 (item 2) is a separate, static analysis that establishes the condition for a
circuit; where it certifies, this bound holds for the encoded operator with no assumption
about the lanes, at the confidence of `g`, and `metrics.sampling_limits.bound_certified` is
`true`. From `2^24` lanes the evaluator runs it and refuses a circuit it does not certify, so
every bound reported at that depth is of this kind.

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

**The outcome-coverage mode** (`eval_circuit --outcome-coverage`, opt-in; DESIGN.md section 9)
checks that every block under a classical condition was actually run. A *site* is each
`PushCondition` block and each op with a condition of its own. In the mode:

- The engine counts, per site, the lanes on which the site executed, separately for control 1
  and control 0. The counts are taken from the masks that gate the ops and are read only after
  every lane passed, so an execution counted is an execution on a lane that passed every check.
- A trusted static pass follows each classical bit as a function of the measurement outcomes. A
  bit is 0, a conjunction of GF(2)-affine forms of the outcomes, or not followed. Each rule is
  exact or gives "not followed". A site whose condition is an inconsistent linear system can
  never execute and is exempt (*proved unreachable*).
- A site with fewer than 256 control-1 executions on the uniform lanes gets *directed lanes*:
  1,024 lanes drawn as the main sample is, with outcomes uniform on the solutions of that
  site's condition. Every per-lane check applies to them. They exist only for sites whose
  condition the static pass follows.
- The run is refused unless every site not proved unreachable has at least 256 control-1
  executions on lanes that passed. A site whose condition is not followed and that the uniform
  lanes leave short is refused, not passed (`unexercised`, with the op index).
- The run is also refused if an outcome can steer an op without a site (next paragraph).

**A site is a classical condition, and nothing else.** An outcome can be copied into a qubit by
a conditioned write (`X a if m`) and then act as a quantum control. The ops it steers carry no
condition and are not sites. The rare block above can be rewritten this way: copy each of `n`
outcomes into a qubit (`n` conditioned `X`, each a site that runs on half the lanes), take the
AND of the qubits with unconditioned Toffolis, and control the block on the AND qubit. The
block still runs on a fraction `2^-n` of lanes, but no site is rare, no directed lanes are
drawn, and the per-site rule passes. The rewrite costs about `2n` Cliffords and `n` Toffolis.

So the mode has a second static pass (`sim::condition::qubit_outcomes`). It marks every
non-system qubit that is written while a classical condition is in force (the op's own or an
enclosing block's), spreads the mark to the target of an `X`-type gate with a marked control,
and moves it with a `Swap`. Only an `Hmr` with no condition in force removes a mark, because
it sets the qubit to 0 on every lane; `R` does not, because it checks the qubit and leaves it
as it is. An op *reads* a marked qubit when the qubit is one of its controls, an operand of its
phase, a bit of its angle register, the qubit it measures or frees, or is still marked at the
end of the stream. The run is refused if any op reads a marked qubit
(`outcome-coverage: outcome-in-qubit: op I ...`). The rule knows nothing about what a condition
evaluates to, so it does not depend on the first static pass.

`--outcome-coverage=sites` applies the per-site rule alone. It reports the reads
(`metrics.outcome_coverage.qubit_outcomes`) and does not refuse them.

**What a pass establishes.** For every conditioned site not proved unreachable, at least 256
control-1 lanes on which that site executed went through the full per-lane checks and passed.
Per site, at confidence `1 - delta`, the fraction of failing control-1 (lane, outcome) pairs
among those that execute the site is at most `ln(1/delta) / n1`, which is 5.4% at the floor
`n1 = 256` (`metrics.outcome_coverage.per_site_bound`). A block that is wrong whenever it
executes cannot pass. A block wrong on a fraction `q` of its executions escapes with
probability `(1 - q)^256`, which is below `1e-6` even against `2^40` re-drawn seeds once `q`
exceeds about 16%.

And no op reads a qubit written under a classical condition. It follows, by induction over the
ops, that on every lane and for every outcome record each qubit an op reads holds a function of
the lane alone. An outcome then acts on the step only through the conditions of the counted
sites and through the signs the measurements themselves add. A sites-only pass that reports
reads establishes the first paragraph only.

**What it does not establish.**

- A sites-only pass says nothing about the ops that read a marked qubit. A rare block reached
  through a qubit is what it cannot see, and such a run's `statement` and
  `sampling_limits.measurement_outcomes` say so.
- The qubit rule refuses honest circuits. A fix-up `X` under an outcome, or any gate inside a
  condition block that writes a qubit, marks the qubit although the value it leaves may be the
  same for every outcome. Showing that it is needs the outcomes followed through the qubits,
  which the reflection check of section 4.1 does symbolically. A refusal here is not a finding
  that the circuit is wrong.
- It is a statement per site. Two blocks that are each right but wrong together on a rare joint
  outcome pattern are covered only as far as uniform sampling covers the pattern.
- It does not lower `f_bound` or the `T f` figure above for defects spread thinly over common
  branches.
- It says nothing about unsampled uniform-register values (2.1 and 2.2), and
  `certifies_energy_error` stays `false`.
- Control-0 executions are reported, not required.
- The proved-unreachable exemption rests on the static pass. Its rules are exact, an exhaustive
  test checks them against every outcome vector of random programs, and the run is refused if
  an exempt site executes on any lane or a directed site misses a lane of its own stratum. An
  error in the rules that these guards also miss would exempt a live block.
- The qubit rule has no such guard at run time. It rests on its own short rule set and on a
  test of the same kind: over every outcome vector of random programs, each read whose value
  differs between two outcome vectors is reported.
- Directed lanes use outcome patterns chosen by the evaluator. The harness already requires
  every sampled lane to be right for whatever outcomes it is given, so this is the same
  standard; a circuit that relies on outcomes being correlated would fail the mode.
- An honest circuit with a rare condition that is not a conjunction of affine forms (a product
  of outcomes, say) is refused until the static pass is extended. That is a refusal to certify,
  not a finding that the circuit is wrong.
- The static pass works within fixed budgets: `2^31` entries touched, 4,096 outcomes per
  equation, `2^24` entries held at once. Past them a condition is not followed. That can turn
  a pass into a refusal and never the reverse. The budgets bound the pass's time and memory
  on a forged stream (`tests/adversarial_coverage_memory.rs`).

The Toffoli and qubit counts, `samples` and every bound still come from the uniform lanes only.

**What the ledger's circuits get**, measured at `2^19` lanes:

| Circuit | Sites | Proved unreachable | Conditioned qubit writes | Reads of a marked qubit | Mode | `=sites` |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| reiher/est-low-r (Low et al. baseline) | 777,724 | 100 | 0 | 0 | passes | passes |
| reiher/outer-hi-reconstruct-r1 (best Reiher) | 86,433 | 4 | 124,390 | 1,307,151 | refused | passes |
| li/composed-alias-m8 (best Li) | 182,524 | 10 | 310,801 | 2,300,772 | refused | passes |

Every site not proved unreachable runs on about a half or a quarter of the lanes, so none needs
directed lanes. The two headline circuits run whole gate sequences inside condition blocks, so
outcomes reach their qubits and the mode refuses both (`outcome-in-qubit`). For them only the
per-site statement is available, and it does not exclude a rare block reached through a qubit.
The baseline writes no qubit under a condition and passes the mode as it stands.

## 3. What a row therefore says

- Every sampled lane applies the declared operator, restores its ancillas and leaves no phase.
- The Toffoli and qubit counts are those of the sampled executions.
- The encoded operator is within `metrics.bound_mHa` of the declared one if the unsampled
  lanes are Hermitian; with no assumption about the lanes only on a run whose reflection check
  certified (`metrics.reflection.status` is `certified`, `sampling_limits.bound_certified` is
  `true`), which every run from `2^24` lanes has by default and a full-stage row has only if
  the flag was given. A certified check also shows that no outcome pattern changes the
  operator (2.3, for correctness).
- **Not** an energy error of FeMoco, and **not** that every conditioned block was executed
  for its gate count. The last holds only for a run made in the outcome-coverage mode (2.3),
  whose `score.json` then carries `metrics.outcome_coverage`, and then for blocks under a
  classical condition, site by site. It covers blocks reached through a qubit only when that
  run reports no read of a marked qubit (`qubit_outcomes.covered: true`), which every pass
  that is not sites-only does. Ledger rows are made without that flag.

For comparing circuits built by the same builders under the same rule, which is what the
board does, this is the standard the challenge started from (every drawn lane passes), with
its bound on the encoded operator now reported in every `score.json` together with whether it
is unconditional. It is not a claim about a ground-state energy.

### 3.1 The evaluator's version

The outcome-coverage mode is opt-in, and the reflection check is opt-in below `2^24` lanes and
required from `2^24` on (`eval_circuit --reflection-check off|report|require` overrides either
way). In mode `off` no code of either runs, and a run's `score.json` differs from the one
`main` wrote before this work by the block `metrics.sampling_limits`, `metrics.bound_mHa` and,
from the `v3` epoch, the bound fields of `sampling_limits` (`bound_certified`,
`certified_bound_mHa`, `below_chemical_accuracy`, `wrong_weight_bound_exact`).
`tests/outcome_coverage.rs` pins the toy circuit's `score.json`; on the three circuits of 2.3
the two evaluators were compared at `2^19` lanes on the sliced engine and agree on every other
key, the wall clock aside. The *verifier digest* is not an output of a run. It is a hash over
every trusted file (`python3 challenge.py verifier-digest femoco`), and this work changes
trusted files. So:

- Every row of `results.tsv` and `evaluator.lock.json` carry the digest of the evaluator that
  made them (`077abe9f...`). The evaluator after this work has another. Rows are not edited.
- After a merge, `challenge.py revalidate` reports `verifier_sha256` as differing for every
  existing row, and the judge and re-validation workflows build the evaluator from source
  until the release workflow has cut a binary for the new digest and regenerated the lock
  ([spec/VALIDATION.md](../../../spec/VALIDATION.md)).
- A headline row that is to be compared under the new evaluator has to be re-validated under
  it. Its counts are not expected to change (they did not on the three circuits above), but
  the digest the row carries will.
- VALIDATION.md ties any change of the evaluator to a new contract epoch. The reporting
  fields and the `2^24` default of the reflection check change no judged output at the
  current full stage (`2^19`), so whether they alone start an epoch is a maintainer decision;
  the proposed `femoco-certified-bound-v3` epoch (VALIDATION.md, "Changing the contract")
  would raise the full stage to `2^24` with the check required, once the Li memory and time
  of 4.1 and 4.2 fit the judge's runner. No spec, acceptance rule, score or per-lane check
  changes; the counts of a re-validated row are not expected to.

## 4. Closing the gaps

In order of strength.

1. **A symbolic check over the whole domain.** The checker in `tools/verification` (where
   present) compares the gate word with the spec for every input and every measurement
   outcome. A circuit with such a certificate has no unsampled lanes, and none of section 2
   applies to it. This is the only route here that supports a claim about a final result.
2. **A step proved to be a reflection.** Implemented as the opt-in reflection check
   (`eval_circuit --reflection-check report|require`, `src/sim/reflection.rs`, rule set
   `reflection-v3`, DESIGN.md section 16.6). See 4.1 below for what it establishes and what
   the ledger's circuits get.
3. **More lanes.** With the step proved a reflection, `2^24` lanes bound the Reiher error at
   1.15 mHa and `2^26` the Li error at 0.89 mHa (section 1.1). Without it, lanes alone do not
   reach chemical accuracy at any practical count. The re-validation workflow takes a lane
   count, so a headline row can be re-run deeper than the full stage today. Section 4.2 gives
   the measured cost.
4. **Coverage of conditioned blocks.** Implemented as the opt-in outcome-coverage mode of 2.3:
   every conditioned site not proved unreachable must execute on at least 256 control-1 lanes
   that pass, with directed lanes for the sites uniform outcomes rarely reach, and uniform
   outcomes alone for the cost count; and no op may read a qubit written under a classical
   condition. For a circuit the mode passes, this removes the case of 2.3 in which a block
   under a classical condition is never or almost never run, and the second rule excludes the
   same block reached through a quantum control. A sites-only pass removes the first only, and
   the two headline circuits get no more than that. It does not remove the `T f` figure for
   defects on common branches, and it
   is per site, not per combination of outcomes. All-zero, all-one and biased outcome strata
   were measured and rejected: on the three circuits above they reach fewer sites than uniform
   outcomes do (all-one leaves 5,636 to 11,769 sites unexecuted that uniform reaches), and their
   lanes are not samples from a distribution a bound can be stated over.

Until item 1 holds for a circuit, its row is a sampled regression result and is labelled as
one. Item 2 removes one assumption from the bound of 2.1; it does not make the bound small.

### 4.1 The reflection check

**Why not a syntactic rule.** The first idea was to require the second half of the step to be
the op-for-op adjoint of the first. The ledger's circuits are nowhere near that shape, for good
reasons: fewer than 0.1% of their ops sit in a mirrored position; about half of their
measurement brackets are lookup erasures, whose correctness is an identity of table data and
not a mirror; many brackets cross the `Reflect`; and the two inner copies are deliberately not
each other's adjoint (the second applies `M^dagger` by reading a pass flag). A palindrome rule
would reject every circuit and say nothing about the ones it rejected.

**What is implemented.** A symbolic execution of the op stream under the simulator's lane
semantics. Every non-system wire is a Boolean function of free variables: the lane's control
and uniform bits, the inner bits after the `Reflect`, and one variable per `Hmr` outcome. Each
function is held as a reduced ordered binary decision diagram, a canonical form, so "this wire
is 0 on every lane and for every outcome record" is one comparison. It is exact (no sampling,
no floating point) and it covers every lane and every outcome record at once. It never reads a
sampled lane. The stream is run twice, the second time with the inner register's two values
exchanged, which gives the operator the first run's adjoint must equal.

**What `certified` establishes**, in this order:

1. *Registers restored.* Every `R` frees a qubit that is 0. The inner register holds its
   initial value at the `Reflect`. At the end the control and outer uniform bits hold their
   initial values, the inner register its second-pass value, and every ancilla is 0.
2. *Outcome independent.* The lane's sign, every system op's control and every rotation angle
   are functions that no measurement outcome changes. This is a property of the function, not
   of how it is written: an outcome may be copied into a qubit, used and undone.
   So the lane operator `L(g, a, b)` depends on the lane only (`g`: control and outer bits; `a`, `b`:
   the inner value before and after the `Reflect`). This also closes 2.3 for the operator: a
   rare outcome pattern cannot change it.
3. *Hermitian.* `L(g, a, b)^dagger = L(g, b, a)` as an identity in the variables. With 1 and the
   decomposition of DESIGN.md section 16.2 this is Hermiticity of the step on the space where
   every ancilla is 0. The step is a unitary there, so it is a reflection.

When certified, `score.json` says so: `metrics.reflection.status = "certified"`,
`metrics.sampling_limits.hermitian_on_every_lane = "certified"`, and
`implied_error_bound_holds_if` no longer names the Hermiticity condition. The bound
`2 Lambda g` then holds for the encoded operator at the confidence of `g`, and the square-root
effect of 2.2 does not apply to that circuit.

**What it does not establish.**

- That the step encodes the spec's operator. A step can be Hermitian and wrong (a sign on
  some lanes, applied symmetrically, is certified; `tests/reflection.rs` pins this). The
  operator is still a sampled statement, so `certifies_energy_error` stays `false` and the
  bound stays at tens of millihartree at the default sample.
- Executed gate counts on rare outcome patterns (a cost question, not a correctness one).
- Anything for a circuit it does not certify. `not certified` means not proven within the
  rule set. A refusal in stage 1 or 2 is exact and comes with a lane and outcome record that
  shows it. A refusal in stage 3 is not a finding that the step is not Hermitian, because the
  comparison there is incomplete (below).

**It fails closed.** The result is `certified` or `not certified` with the first obstruction
(op index, kind, reason). Outside the rule set, and therefore refused: `S`/`Sdg`; a rotation
under a classical condition; an outcome that reaches a system op; an `R`, `Reflect` or
end-of-stream condition that does not hold; a sign, control or angle that depends on an
outcome; a mismatch between the lane operator and its adjoint; an exhausted work budget or
node store; a lane map that is not nested. The budget counts decision-diagram steps and nodes,
never wall clock, so the verdict is deterministic. When the check finds that something it needs
to be zero is 1 somewhere, the refusal gives one lane and outcome assignment on which it is 1,
which can be replayed on the lane engine.

**The comparison of stage 3 is incomplete, deliberately.** Two examples of true identities it
does not find:
system ops that commute are not reordered, so a SELECT written as a loop of disjoint
controlled terms is refused where one data-loaded rotation network is accepted; and an angle
register holding `2^beta - a` is not recognised as `-a`, so the inverse rotation must be
written as `Z G Z` from the same register (as Low et al.'s construction does).

**What the ledger's circuits get** (built from their manifests, `ops.bin` equal to the ledger
digest, checked by trusted code):

| Circuit | Result | Detail |
| --- | --- | --- |
| reiher/est-low-r (Low et al. baseline) | certified | 984 resets proved, all 5,008 outcomes proved absent from the operator; 330 system ops, 218 normal-form elements compared; 137 s, 5.7 GB |
| li/low2025-reference (Low et al. baseline) | certified | 1,369 resets proved, all 7,868 outcomes proved absent from the operator; 462 system ops, 306 normal-form elements compared; 11 min, 12.6 GB |
| reiher/outer-hi-reconstruct-r1 (best Reiher) | certified | 3,301 resets proved, all 4,622 outcomes proved absent from the operator; 330 system ops, 218 normal-form elements compared; 12 min, 2.7 GB |
| li/composed-alias-m8 (best Li) | certified | 251 resets proved, all 2,068 outcomes proved absent from the operator; 462 system ops, 306 normal-form elements compared; 39 min, 5.4 GB |

Times and peak memory are for one core of a laptop, both runs of the stream included. For
these four circuits the bound of 2.1 no longer rests on the Hermiticity assumption, and a run
with the flag reports `hermitian_on_every_lane: "certified"`. The ledger's other rows have
not been run through rule set v3; a recorded row is not changed by any of this, because the
check is opt-in and reads the same `ops.bin`.

*What changed with rule set v3.* Under v2 the two headline circuits were not certified: the
check ran out of work proving the Reiher circuit's sign independent of its outcomes one at a
time, and out of diagram store on one inner bit of the Li circuit. v2 held a wire as a
polynomial over named sub-expressions and built a decision diagram only for an identity it
could not settle otherwise, from scratch each time. v3 holds every wire as a decision diagram
from the start. A dependence on an outcome that cancels is then gone when it cancels, an
ancilla is 0 when its diagram is the 0 node, and the operator with the two inner values
exchanged comes from running the stream a second time, not from rewriting the first run. v2's
refusals of these two circuits were limits of v2 and not defects of the circuits. v3 agrees
with v2 on everything v2 decided: both baselines, and every circuit of `tests/reflection.rs`.

*What it costs.* v3 holds table data explicitly: a register filled by a lookup is a diagram
that grows with the table. So the baselines, which v2 certified in seconds, take minutes, and
the check needs between 3 and 13 GB. It is deterministic and has no tuning per circuit, but it
is not a check to run on every submission by default.

**How far to trust it.** The check is about 2,300 lines of trusted code, its unit tests
included. Its tests (`tests/reflection.rs` and the unit tests of the decision diagrams):

- The decision diagrams are compared with truth tables: both operations, canonical form,
  collection, which variables a function depends on, and the counterexample.
- The normal form and the adjoint the check builds are compared with dense matrices on random
  words of system ops, including a `SpinSwap` that does not execute.
- On about 500 random circuits it certifies, a dense simulation of every lane and every
  outcome record finds exactly what the certificate says (the converse is never asserted).
- A rotation whose compute and uncompute disagree on one uniform value, the construction of
  2.2, passes the sampled check at 64 lanes and is refused.
- Each of the sixteen obstruction kinds of DESIGN.md section 16.6 has a circuit that is
  refused with that kind. Seven lane-semantics rules that no kind names (an op's own
  condition inside a condition block, a conditioned `Swap`, a conditioned `Neg`, a phase on a
  qubit that holds an outcome, an outcome mixed into a value with lane bits, the qubit and
  the bit of a conditioned `Hmr`) each have a stream that is refused paired with a variant
  that cancels and is certified, and the dense simulation agrees with both.
- A node store too small refuses with kind `budget` or gives exactly the verdict of a large
  one, on the tiny circuit and on each of its mutants.

The tests were themselves tested. `tools/analysis/reflection_mutants.py` applies 72
single-rule alterations of the check, one at a time, in a scratch copy (a comparison skipped,
a condition ignored, a term dropped, a collection that forgets a root), and each makes at
least one test fail. On the first run ten did not: nine now have a test written for them, and
the tenth altered unreachable code, which was removed. This is testing, not a proof of the
implementation. The check is one implementation, by one author, of semantics that the lane
engine implements separately; where both have run, they agree.

**Status.** From `2^24` lanes on the evaluator's default mode is `require`: a circuit that is
not certified is a `FAIL` row, named with its first obstruction. Below `2^24`, and so at the
judge's full stage of `2^19` today, the default is `off`; `--reflection-check report` runs it
and reports without refusing, `require` refuses, and `off` turns it off at any count. The
acceptance rules of `benchmark.json` are unchanged. Requiring it at the judge's full stage is
part of the proposed epoch (VALIDATION.md).

**Would requiring it exclude an architecture?** Every architecture on the ledger has a
circuit the check certifies: measured on the best row of each architecture on each track plus
the two baselines, built from their manifests (`ops.bin` equal to the ledger digest), on an
Apple M5 Max (18 cores, 64 GB) on 2026-10-22:

| Circuit (architecture) | Result | Wall / CPU | Peak memory |
| --- | --- | ---: | ---: |
| reiher/est-low-r (qroam-word, baseline) | certified | 405 s / 255 s | 4.4 GB |
| reiher/est-toff-r (qroam-word) | certified | 416 s / 254 s | 6.8 GB |
| reiher/est-rs3 (streamed) | certified | 377 s / 245 s | 4.3 GB |
| reiher/fa-r1tr-m8 (onehot) | certified | 224 s / 159 s | 5.7 GB |
| reiher/pf-r3p-m8 (onehot-split) | certified | 1,029 s / 672 s | 4.2 GB |
| reiher/outer-hi-reconstruct-r1 (rank-scheduled, best) | certified | 1,519 s / 1,140 s | 13.4 GB |
| li/low2025-reference (qroam-word, baseline) | certified | 1,494 s / 979 s | 28.3 GB |
| li/pf-l1c-m8 (onehot) | certified | 1,228 s / 744 s | 15.3 GB |
| li/composed-alias-m8 (onehot-split, best) | certified | 3,243 s / 2,804 s | 10.7 GB |
| li/kn-l4fhq24-m9 (rank-scheduled) | certified | 5,344 s / 4,881 s | 12.2 GB |

Wall and CPU are `/usr/bin/time` on macOS with up to ten checks and a 16-thread lane run
sharing 18 cores, so wall is an upper estimate; the peak memory is the resident set as macOS
reports it and is well above the 2.7 to 12.6 GB measured earlier for rule set v3 on a Linux
laptop. The verdicts are what matters here: no architecture is excluded.

What it would exclude is a circuit written outside the rule set above (an `S` gate, a
conditioned rotation, an outcome that steers a system op, a non-palindromic SELECT), and such
a refusal names the op. The rules are the discipline that every recorded architecture already
follows, so a submitter can check a new design with `--reflection-check report` at the quick
tier before submitting. Memory is the practical limit: the judge's `ubuntu-24.04` runner has
16 GB, and the Li circuits measured above need 11 to 28 GB as macOS counts them, so the check
cannot be required on the Li track until it runs on a larger runner or its node store is
sized to the runner (a store too small refuses with kind `budget`, never a wrong verdict).

### 4.2 Deep runs of the headline circuits, and what the judge pays

The two best circuits of the ledger, rebuilt from their manifests (`ops.bin` equal to the
ledger digest), evaluated by this evaluator with `--engine sliced --reflection-check report` at
the lane counts of 4, item 3:

| Circuit | Lanes | Toffolis / qubits | `implied_error_bound` | grinding bound | Reflection | Wall, peak memory |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| reiher/outer-hi-reconstruct-r1 | `2^24` | 11,462.48 / 313 | 1.15 mHa | 3.47 mHa | certified | 14 min, 2.9 GB |
| li/composed-alias-m8 | `2^26` | 17,535.51 / 471 | 0.89 mHa | 2.67 mHa | certified | 54 min, 4.8 GB |

Both pass every lane, with no fallback batch, and both `score.json` report
`hermitian_on_every_lane: "certified"`. So for these two circuits the operator the step
encodes is within 1.15 and 0.89 mHa of the spec's (`2 Lambda g`, section 2.1), at the
confidence of `g`, with no assumption about the unsampled lanes. The grinding bound, which
allows a submitter `2^40` draws of the seed (`metrics.grinding_draws_log2`), is within
3.5 mHa. The Toffoli count is a mean over the sampled lanes, and it agrees with the ledger
rows' (11,462.56 and 17,535.50 at `2^19` lanes) to within 0.001%; the qubit counts are equal. Times are for four threads of a laptop, the
reflection check included (about 12 and 39 minutes of each).

These runs used a local seed, not the judge's, and they are not ledger rows. The bound is
about the encoded operator, not about energy, and `certifies_energy_error` stays `false`
(4.1, "What it does not establish").

**Measured cost of the lanes** (sliced engine, same two circuits, Apple M5 Max, 2026-10-22,
`--reflection-check off`; the reflection check is a separate single-threaded cost, above):

| Circuit | Lanes | Threads | Wall | Peak memory | `implied_error_bound` |
| --- | ---: | ---: | ---: | ---: | ---: |
| reiher/outer-hi-reconstruct-r1 | `2^19` | 16 | 2.1 s | 0.6 GB | 36.8 mHa |
| | `2^22` | 16 | 9.3 s | 0.7 GB | 4.61 mHa |
| | `2^24` | 16 | 38 s | 1.0 GB | 1.15 mHa |
| | `2^19` | 4 | 6.8 s | 0.3 GB | 36.8 mHa |
| | `2^22` | 4 | 28 s | 0.5 GB | 4.61 mHa |
| | `2^24` | 4 | 192 s (loaded: six reflection checks running; 95 s from the per-lane rate) | 0.6 GB | 1.15 mHa |
| li/composed-alias-m8 | `2^19` | 16 | 8.0 s | 0.9 GB | 113.7 mHa |
| | `2^22` | 16 | 33 s | 1.3 GB | 14.2 mHa |
| | `2^24` | 16 | 138 s | 1.7 GB | 3.55 mHa |
| | `2^26` | 16 | 567 s | 3.1 GB | 0.89 mHa |
| | `2^19` | 4 | 24 s | 0.6 GB | 113.7 mHa |
| | `2^22` | 4 | 91 s | 1.0 GB | 14.2 mHa |
| | `2^24` | 4 | 322 s (loaded as above; 310 s from the per-lane rate) | 1.2 GB | 3.55 mHa |

The screen stage (reference engine, 4,096 lanes, 4 threads) takes 8 s (Reiher) and 20 s (Li).

The engine is linear in lanes once the fixed cost of reading `ops.bin` (about 1 s) is out:
per lane, Reiher costs 1.9 us at 16 threads and 5.7 us at 4, Li 6.8 us and 18 us. Four
threads are 2.9 to 3.5 times slower than sixteen, so the engine also scales nearly linearly
in threads at this size. Peak memory is below 2 GB at `2^26`.

**The judge's budget.** The `evaluate` job runs on a 4-vCPU, 16 GB `ubuntu-24.04` runner with
a 120-minute limit and runs the screen stage, then the full stage, then the reflection check
in the same process, one after the other. Taking the runner at half the per-core speed of
this machine (an estimate to be replaced by a measured judge run):

| Track | Lanes | Lanes, this machine, 4 threads | Lanes, runner (x2) | Reflection check, runner (x2 CPU time) | Total | Bound (certified) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reiher | `2^24` | 3.2 min (loaded) | 6.5 min | 38 min | about 45 min | 1.15 mHa |
| Li | `2^24` | 5.4 min | 11 min | 60 to 95 min (2,800 s CPU here under load; 39 min on one laptop core earlier) | 70 to 105 min | 3.55 mHa |
| Li | `2^26` | 21 min (from the per-lane rate) | 42 min | 60 to 95 min | 100 to 140 min | 0.89 mHa |

So in time, `2^24` fits on both tracks with margin and makes Reiher certified below chemical
accuracy; Li below 1.6 mHa needs `2^26`, which fits only with the reflection check run
concurrently with the lanes (it is single-threaded and the engine then has three cores), a
larger runner, or a job split: lanes in one job, the certificate in another, joined by the
circuit's digests. In memory, the check on the Li circuits does not fit the runner (4.1).
That is why the full stage is still `2^19` and the epoch is a proposal. The re-validation
workflow (180-minute limit, `full_samples`) can make the `2^24` and `2^26` rows today with
`--reflection-check require` on a machine with the memory.
