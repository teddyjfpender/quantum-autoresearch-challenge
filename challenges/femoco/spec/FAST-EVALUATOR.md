# Fast lane evaluator: the `sliced` engine and its equivalence harness

`eval_circuit` validates a submission on Fiat-Shamir-sampled lanes. The code that runs those
lanes is a *lane engine*. Two engines are built:

- `reference` (`src/sim/validate.rs`, `src/sim/lanes.rs`, `src/sim/gaussian*`): the default and
  the oracle.
- `sliced` (`src/fastsim/`), selected with `eval_circuit --engine sliced`: a faster engine that
  must equal the reference on every observable. It is tooling.

This document describes the sliced engine as built, the obligations it must meet (section 5),
the folding tier and its guard band (sections 6 and 6.1), the harness that compares engines
(section 10) and the evidence and limits of that comparison. Section numbers are stable: code
comments cite sections 5, 6, 6.1 and 10.

Every number is labelled **measured** (with its conditions) or **estimate**. Machine for every
measurement: Apple M4 Max (10 performance and 4 efficiency cores, 36 GiB), macOS 15, rustc
1.93.0 (the pinned toolchain), release profile as in `Cargo.toml`, `eval_circuit` built with
`--no-default-features`, thread count set with `RAYON_NUM_THREADS`, no other heavy job running
(the `tools/heavy.sh` lock held).

## 0. Summary

1. **The reference is already bit-sliced** (512 lanes per batch). On the spectrum-amplified
   (SA) circuits 90-93% of its lane time is each lane's final Gaussian-tracker check
   (`GaussianLane::finish`), so wider bit-slicing alone gains little (section 2).
2. **The sliced engine** runs the same batches bit-sliced without a tracker, records each
   lane's exact tracker inputs, and computes the final check once per distinct input sequence,
   with kernels that perform the reference's float operations in a different schedule. Any
   batch in which anything fails is re-run by the reference's own code (section 4).
3. **The folding tier** replaces the `Ad` residual by a cheaper, numerically different
   computation, used only when a rigorous bound proves that the reference's residual passes;
   otherwise the exact residual is computed. The vacuum-overlap check is always exact
   (sections 6 and 6.1).
4. **Measured:** at `K = 2^19` on 4 threads, 7.76 s against 571.29 s on a Reiher SA circuit
   and 36.68 s against 1,572.79 s on a Li SA circuit, with identical `score.json` apart from
   `eval_seconds` (section 11).

### 0.1 What is and is not audited

- **The reference engine is the oracle.** `eval_circuit` uses it unless `--engine` is given,
  and `benchmark.sh` does not pass `--engine`.
- **How the judge uses the two engines.** Every submission is first screened by the reference
  engine on 4,096 lanes, then validated in full by the sliced engine on 524,288 lanes with the
  same seed (`benchmark.json`, `validation`). A ledger row therefore rests on the reference
  engine for its screen and on the sliced engine for its full sample. The `revalidate`
  workflow can repeat the full stage on the reference engine for any recorded circuit.
- **The sliced engine is not independently audited** against section 5. Its agreement with the
  reference is evidenced by testing only: the equivalence harness of section 10, on the
  circuit classes listed in sections 10.3, 10.4 and 10.6.
- **`score.json` does not name the engine.** `eval_circuit --engine NAME` prints the engine
  name and the engine's counters on stdout only. The ledger records the engines and sample
  sizes of both stages in its `engine` column.
- **The guard band's derivation (section 6.1) is an argument on paper plus tests.** The harness
  checks the engine's behaviour at the threshold; it cannot check the derivation.
- **What the harness does not show** is listed in section 10.5. In short: it is testing, not
  proof; the fuzzer's references and rotation precision are narrower than the real circuits';
  the recorded runs are the reduced (`--quick`) configuration; and only thread counts 1 and 4
  are compared by default.
- **The harness itself is checked** by seven deliberately wrong engines, each of which it must
  report as different (section 10.2).

## 1. Measured baseline (reference engine)

Two SA circuits are used throughout, both of the `sa-toff` architecture on the rigorous-class
fixture specs:

- **Reiher SA circuit:** `FEMOCO_WALK_ARCH=sa-toff`, `FEMOCO_WALK_SPEC=reiher-sa-v1`,
  `FEMOCO_SA_TWEAKS=imchxlgr`; 1,650,241 ops, 657 qubits, `n = 108` modes.
- **Li SA circuit:** `FEMOCO_WALK_ARCH=sa-toff`, `FEMOCO_WALK_SPEC=li-sa-v1`,
  `FEMOCO_SA_TWEAKS=imchxgrvd`, `FEMOCO_SA_INNER_A=4`; 5,705,098 ops, `n = 152` modes.

**Measured** `eval_circuit --samples K` wall time (the "eval time" line, which excludes reading
`ops.bin`), reference engine. The `K = 2^19` columns are **estimates** (a linear fit through
the two measured points); direct runs are in section 11.

| circuit | K = 2^12, 1 thr | K = 2^12, 4 thr | K = 2^14, 1 thr | K = 2^14, 4 thr | est. K = 2^19, 1 thr | est. K = 2^19, 4 thr |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reiher SA | 15.84 s | 4.46 s | 64.50 s | 17.60 s | 2,080 s | 560 s |
| Li SA | 44.67 s | 12.40 s | 180.51 s | 48.91 s | 5,800 s | 1,560 s |

To reproduce: build the circuit with `build_circuit` (knobs as compile-time environment
variables, as in `tools/repro/`) in a scratch directory that links `specs/` and `taxonomy/`,
then run `RAYON_NUM_THREADS=t eval_circuit --samples K --root <dir> [--engine sliced]` under
`tools/heavy.sh`.

## 2. Where the reference's lane time goes

**Measured** with timers in a scratch copy of the reference (single thread, `K = 2^12`; not
part of this repository) and confirmed by a sampling profile:

| circuit | final check (`finish`) | of which `ad_residual` | of which Pfaffian | Givens + SpinSwap tracking | bit-sliced classical ops | Hmr (XOF + apply) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reiher SA | 14.37 s (90.6%) | 6.19 s | 8.16 s | 1.04 + 0.28 s (8.3%) | ~0.07 s (0.4%) | 0.003 s |
| Li SA | 41.36 s (92.7%) | 18.08 s | 23.26 s | 2.22 + 0.63 s (6.4%) | ~0.27 s (0.6%) | 0.005 s |

Each lane ends with 219 (Reiher) or 307 (Li) Majorana vectors of length `n`: four chunks, each
one frame handed to the tracker at a `Givens`, plus 1-4 reference vectors. The frames are `Z`
strings on the odd-class modes, which the SA circuits emit around `V^dagger` to flip the sign
of its rotation angles; each `Z_q` is two Majoranas (`gamma_2q gamma_2q+1 = i Z_q`).
`ad_residual` costs `4 n^2 |ys|` flops in sequential `fold(-0.0, +)` dot products; the
Pfaffian is a complex Gaussian elimination with row pivoting of about `|ys|^3 / 6` updates.

## 3. Lane structure

The per-lane bits (non-system qubits, classical bits, the `Z/8` phase, the Pauli frame) are
sliced by the reference already. The non-boolean state is one `GaussianLane` per lane that ran
a `Givens` or `SpinSwap`. Tallies are `popcount(base & cond)` and never depend on qubit
values. Compile, liveness, `Q_peak`, static facts, depth, the lane-map rounding check and
`f_bound` are computed once per circuit by shared code, which the sliced engine reuses.

**Repeated tracker inputs (the main exact lever).** Fix a lane's tracker input sequence: every
`(before frame, p, q, angle)` handed to `givens_modes`, the final frame and the reference
operator. `finish` is a deterministic function of that sequence, so lanes that share it get
identical results. Distinct sequences among sampled lanes (**measured**):

| circuit | K = 2^12 | K = 2^14 | K = 2^16 | K = 2^19 | reduction at 2^19 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reiher SA | 2,507 | 6,736 | 17,073 | 43,259 | 12.1x |
| Li SA | 3,098 | | | 108,146 | 4.85x |

Growth is sublinear, about `K^0.45` between `2^16` and `2^19` on Reiher SA.

## 4. The sliced engine (`--engine sliced`)

### 4.1 Boundary and modules

The engine replaces exactly one function, `validate::run_with`: same `Context`, same optional
`Nested`, same `&[Lane]`, an `Outcome` back. Everything before it (`lanemap::parse`, the
lane-map check, compile, `fiat_shamir::sample*`) and after it (static facts, the taxonomy
check, the score arithmetic, `score_json`) is the reference's code, called unchanged. Every
`score.json` field other than the wall-clock `eval_seconds` is therefore identical when the
`Outcome` is identical.

Modules under `src/fastsim/`; the reference path imports none of them:

| module | role |
| --- | --- |
| `exec.rs` | Bit-sliced execution of a pass of 1, 2, 4 or 8 reference batches, one outcome stream per batch. Records each lane's tracker calls instead of running a tracker. |
| `mod.rs` | `run_probed` (a `validate::Engine`): passes in parallel, the reference's end-of-lane checks, the compute-once memo from trace key to verdict, re-run of failing batches by `validate::run_batch`, counters. |
| `gauss.rs` | `FastLane`, a field-for-field copy of the reference tracker state with the fast final-check kernels and running error bounds; `fold_probe`: exact overlap, guarded residual. |
| `fold.rs` | `FoldLane`, the bound `gap`, `kappa` and the certificate rule (section 6.1). |
| `kernels.rs` | The residual and overlap kernels, equal to the reference up to zero signs. |
| `pfreal.rs` | The reference's Pfaffian elimination on real coefficients: the default overlap kernel for a single-component contraction matrix (section 11). |
| `pfblock.rs` | The reference's elimination on complex entries with delayed (panelled) updates and linked columns; the `FEMOCO_FASTSIM_REAL=0` path. |
| `prof.rs` | Phase timers (`FEMOCO_FASTSIM_PROF`, stderr only). |

### 4.2 Memoized tracker on exact input traces

**Recording.** During bit-sliced execution, each lane on which a `Givens` or `SpinSwap`
executes appends one record to its trace: the exact tracker call the reference would make
(`before` frame, `p`, `q`, angle).

- **Frames.** The reference hands the tracker the lane's frame and then clears it. The executor
  keeps a per-lane dirty flag: a lane whose frame planes have not changed since its last
  hand-off has the identity frame (98.7% of hand-offs on Reiher SA, **measured**), and nothing
  is extracted for it. Otherwise the frame is extracted exactly as the reference builds it.
- **Clearing.** The frame planes are cleared for all executing lanes with one masked pass.
- **SpinSwap.** The reference's sequence of `n / 2` calls is reproduced call for call.

At the end of the batch the trace is closed with the final frame (phase included) and the
reference operator (or none, for a control-0 lane).

**Key and memo.**
- **Key.** The full canonical byte encoding of the trace. The memo is a hash map keyed by the
  full key bytes, so a hash collision cannot change a result. Keys take about 92 MB (Reiher SA)
  and 316 MB (Li SA) at `K = 2^19` (**measured**).
- **Value.** The verdict of the reference's `finish` on that trace, computed once per distinct
  key by `gauss::verdict`: the kernels of section 4.4 and the guarded fold of section 6.1, or
  the reference tracker itself when a kernel declines.
- **Concurrency.** 64 shards of `Mutex<HashMap>` holding `OnceLock` cells. A value is a pure
  function of its key, so thread scheduling cannot change a result.
- **No tracker.** A lane without a tracker takes the reference's Pauli path (`check_identity`,
  `compare`), called unchanged.

### 4.3 Bit-sliced executor

The state layout, every classical op, the phase arithmetic, the `Sys` / `SysS` frame rules,
`Hmr`, strict `R`, `Reflect` and every tally are those of `src/sim/lanes.rs`. A pass covers 1,
2, 4 or 8 consecutive reference batches (`Exec<WW>` with `WW = 8, 16, 32` or `64` words), so
each op is decoded and dispatched once per pass. The reference's 512-lane batch stays the unit
of outcome streams, failure accounting and fallback. The pass width is chosen from the thread
count and the plane count so that every thread has work and the planes stay in cache; it is a
performance choice only, and `FEMOCO_FASTSIM_PASS` fixes it. A pass that does not pass as a
whole is retried batch by batch before any batch goes to the reference.

### 4.4 Kernels of the final check

Two finite floats are *equal up to zero sign* when they are bit-identical or both zeros.
Replacing inputs of `+`, `-`, `*` or `/` (by a non-zero divisor) by values equal up to zero
sign gives a result equal up to zero sign, and `abs`, `hypot`, `==` and `total_cmp` on
non-negative norms do not see zero signs. The kernels use only that:

- **`ad_residual`.** Every basis column's reflection chain runs the reference's float
  operations in the reference's order (sequential `Sum` fold from the toolchain's own start
  value, then `d * w - y`, no FMA). Columns are independent and run side by side. Terms with an
  exactly-zero factor are skipped in place.
- **Overlap (Pfaffian).** The contraction matrix's upper triangle is computed as the reference
  computes it, each distinct vector's dot products once. The elimination follows the
  reference's pivot rule (`hypot`, `total_cmp`, last maximum wins) with delayed updates, and by
  default on real coefficients (section 11).

The values returned equal the reference's up to zero sign, and the verdict (`off <= AD_TOL`,
the nearest eighth root of unity to the overlap) is identical. Every kernel checks finiteness
of its inputs and of every value it reads; when anything is not finite it declines and the
caller uses a kernel that performs the reference's operations exactly, or the reference
tracker itself.

### 4.5 Outcome stream

Batch `b` reads 64 bytes of `hmr_stream(key, b)` per `Hmr` op it reaches, in op order; lane
`512 b + 64 j + l` takes bit `l` of little-endian word `j`. Every bit reaches a reported field
(the fix-up's executed count goes into `cliffords`), so the SHAKE256 stream is generated in
full. The engine calls the reference's `hmr_stream` and reads it in the reference's order.

### 4.6 Failure handling (exact by fallback)

A 512-lane batch in which anything fails (a dirty `R`, a tracker error, a `Reflect` mismatch,
any per-lane check) is re-run by the reference's own `run_batch`, and that `Outcome` replaces
the fast one. The fallback is needed because the reference's failure accounting is
batch-shaped: an execution error stops its batch, counts once, leaves the batch's remaining
lanes unchecked and its tally partial, and messages carry formatted floats. `Outcome::merge`
is order-independent (sums, plus the first failure by sample index), so the merged `Outcome`
is the reference's. A rejected circuit costs reference time only on its failing batches.
There is no "fail fast" mode: it would change the reported failure count.

### 4.7 Threads

Rayon over passes, with the memo shared. Results do not depend on the thread count: the outcome
streams are per batch and the memo values are pure functions of their keys.

### 4.8 Switches and counters

Environment variables read by the engine. None changes a verdict.

| variable | effect |
| --- | --- |
| `FEMOCO_FASTSIM_FOLD=0` | Turns the guarded fold off; the exact residual decides every key. |
| `FEMOCO_FASTSIM_FOLD_MIN=k` | Folds only lanes with at least `k` tracked vectors (default 16). `0` folds every lane. |
| `FEMOCO_FASTSIM_PASS=1\|2\|4\|8` | Fixes the batches per pass. |
| `FEMOCO_FASTSIM_REAL=0` | Uses the complex elimination (`pfblock`) instead of the real-coefficient one. |
| `FEMOCO_FASTSIM_PANEL=p` | Panel size of the delayed updates. |
| `FEMOCO_FASTSIM_SELFCHECK` (set) | Also computes the exact residual on every key and asserts `\|off_ref - off_fold\| <= g`; panics on a violation. |
| `FEMOCO_FASTSIM_STATS` (set) | Prints the counters on stderr. |
| `FEMOCO_FASTSIM_PROF` (set) | Prints phase timers on stderr. |

Counters (`fastsim::last_stats`, printed by `eval_circuit --engine sliced` on its `counters`
line, never in `score.json`): `batches`, `fallback_batches` (re-run on the reference),
`keyed_lanes`, `distinct_keys`, `key_bytes`, `fold_certified`, `guard_fallbacks` (fold tried,
guard declined), `guard_exact_small` (too few vectors to fold) and `fold_max_g_attos` (the
largest `g`, in units of 1e-18).

## 5. Equivalence obligations

For every op stream, spec, lane map, seed and `K`, at every thread count, the sliced engine's
`Outcome` must equal the reference's in every observable: `rejection()` (the full string) and,
when that is `None`, the `Tally` (all nine fields) and `lanes`. Concretely:

1. **Same lanes.** `fiat_shamir::sample` and `sample_nested` are called unchanged, inside the
   same `with_audit` scope, on the calling thread. `hmr_key` is taken from that sample.
2. **Same outcome bits.** The byte and bit mapping of section 4.5, per original lane index.
   Lanes are not permuted.
3. **Same op semantics.** Masks and the condition stack; `Hmr` under a false condition (qubit
   and bit untouched, no phase, not counted, outcome bytes still read); strict `R`; `Z/8` phase
   arithmetic; the `Sys` and `SysS` frame rules; `Givens` and `SpinSwap` hand-off and clearing;
   `Reflect`'s want and flip checks; lazy tracker creation.
4. **Same tallies.** All are integer popcount sums over the same masks. The derived
   `score.json` floats are then identical, because shared code computes them from identical
   integers.
5. **Same per-lane check order and messages.** Control and uniform restored, then the first
   dirty ancilla by id, then the tracker `finish` or the Pauli comparison. On any failure the
   messages come from the reference's re-run of the batch (section 4.6).
6. **Same floats where they are observable.** Kernels must equal the reference up to zero
   signs, which requires no FMA, the same summation order including the `fold(-0.0, +)` start
   (a property of `f64`'s `Sum` on the pinned toolchain 1.93; `kernels::sum_start` reads it from
   the standard library, and it must be re-checked on any toolchain change) and the same pivot
   rule. The one numerically different computation, the folded residual, may decide a lane only
   under the certificate of section 6.1.
7. **Same static results.** `Q_peak`, segment peaks, depth, Toffoli depth, rounding error,
   `f_bound`, the grinding bounds, facts and taxonomy are shared code, not reimplemented.
8. **`score.json`.** Every field is identical except `eval_seconds`. The engine choice is
   printed on stdout and not added to `score.json`, so that file stays comparable byte for
   byte apart from `eval_seconds`.

**Argument, by part.** Sampling, compile, static facts and scoring are the reference's code.
Execution, tallies and outcome bytes are the reference's operations per op (unit-tested per op
kind against `Lanes` on random states, and per run at four pass widths). A lane's verdict is a
pure function of its trace key, which holds every tracker input. The overlap kernel equals the
reference's up to the sign of a zero, which no later operation observes. The residual is
either the exact kernel's or certified by the guard. Any batch with a failing lane is re-run by
the reference, so failure counts, first failures and messages are the reference's own.

## 6. The folding tier

**The fold.** A frame handed to the tracker, as `frame_to_majoranas` writes it
(`w^k gamma_m1 ... gamma_md`, sorted), equals `w^(k + 2 #pairs) M Z_S`:

- every pair `gamma_2q gamma_2q+1 = i Z_q` is adjacent in sorted order and commutes with every
  other Majorana;
- `M` is the product of the unpaired Majoranas, in order;
- `Z_S` is number-conserving and fixes the vacuum: `Ad(Z_S) = D_S` (the sign flip of the modes
  in `S`) on both Majorana blocks, and `Z_S |vac> = |vac>`.

In the tracked form `O = X P`, `Z_S X P = (Z_S X Z_S^-1)(Z_S P)`: flip the `S` components of
every held vector and the `S` rows of the mode rotation `U`. Both are exact float operations.
`FoldLane` (`src/fastsim/fold.rs`) keeps the folded rotation and the unpaired vectors only:
about 2 vectors per lane on the SA circuits, against about `2n` for the reference. The
rotations are the reference's float operations.

The operator is the same in exact arithmetic, and so are the `Ad` residual and the vacuum
overlap. Only the floating-point evaluation differs. In a shadow-tracker measurement
(**measured**, single thread, `K = 2^10`) the folded tracker plus check took 0.103 s against
3.96 s (Reiher SA) and 0.198 s against 11.19 s (Li SA); verdicts agreed on all 1,024 lanes of
each; the largest `|off_fold - off_ref|` was 6.2e-15 and 2.3e-14.

**Why the fold cannot simply replace the check.** The verdict is `off <= AD_TOL = 1e-11` and
`|<vac|Omega|vac> - w^0| <= PHASE_TOL = 1e-6`, computed in floating point. Honest lanes stay
at or below 3e-14 (**measured**), but a circuit can be constructed whose residual sits within
rounding of `1e-11` (section 10.4 does so). On such a lane two float evaluations can disagree.

**As implemented.** The fold is used only for the `Ad` residual, and only under the guard band
of section 6.1: a rigorous bound `g` on the difference between the folded and the reference
residual; the folded result decides a lane only when it is more than `g` below the threshold;
otherwise the exact residual kernel decides. The vacuum overlap is always computed by the exact
kernel. Lanes with fewer than 16 tracked vectors are not folded (`FEMOCO_FASTSIM_FOLD_MIN`).

## 6.1 The guard band

**Scope: the `Ad` residual only.** No rigorous guard is available for the vacuum-overlap check
at sub-cubic cost. The reference computes the overlap by Gaussian elimination with row
pivoting; its forward error is bounded by the backward error times the growth of the
eliminated entries, and growth depends on the pivot path, which depends on the reference's own
floats (pivot ties are common: the same vector often occurs in two chunks). In exact
arithmetic the entries after `t` steps are ratios of sub-Pfaffians
`Pf(A[T + {i, j}]) / Pf(A[T])`, whose denominator has no lower bound beyond `(|Pf(A)| / m)^t`,
so an adversarial circuit can in principle drive the growth to `m^t`. Knowing the growth means
running the reference's elimination, the cost the fold would avoid. The overlap therefore
stays exact. Honest SA lanes never come near its tolerance (`1e-6` against observed errors
near `1e-14`); this is a gap in what can be proved.

**Common ideal.** Let `(c_t, s_t)` be the computed cosine and sine of rotation `t` (the same
floats in both engines), `rho_t = sqrt(c_t^2 + s_t^2)`, and `G*_t = G_t / rho_t` the exactly
orthogonal rotation. Define every ideal quantity with the `G*_t` in exact arithmetic. For a
pair, the reference's two vectors give, per block,
`(2 w w^T - I)(-I) = I - 2 w w^T = P* D_q P*^T` for `w = P* e_q`, which is what the fold's flip
contributes. Both engines therefore approximate the same ideal matrix `M*_b = Ad(Omega)` on
each block `b`. The reference operator's vectors are the same computed floats in both engines.

**What is bounded.** For one engine and column `j` of block `b`, let `y_hat` be the computed
result of the reference's reflection chain starting from the computed `U e_j`. `gap` bounds
`|| sigma_b y_hat - M*_b e_j ||_2` over all `b, j`. Then `|off_ref - off_fold|` is at most
`gap_ref + gap_fold`, up to the final rounding of `|sigma v - delta|` (a relative `u`). With
`u = 2^-53`:

1. **One rotation.** For computed components `(a, b)` at `(p, q)`, the computed rotation
   differs from `G* (a, b)` by at most `kappa_t (|a| + |b|)` in the 2-norm, with
   `kappa_t = (2u + u^2)(1 + nu_t) + nu_t` and `nu_t >= |c^2 + s^2 - 1| >= |rho - 1|` computed
   exactly with FMA two-products (`fold::kappa`). Earlier errors pass through `G*` unchanged in
   norm. A rotation of two exact zeros adds nothing.
2. **Tracked vectors.** Every chunk vector starts as an exact basis vector, so
   `eps_i = sum_t kappa_t (|a_t| + |b_t|)` over the rotations after its hand-off bounds
   `|| w_hat_i - w*_i ||_2`. The fold's sign flips are exact. The reference operator's rotated
   vectors get the same bound by replaying their network; a monomial's vectors are exact.
3. **Mode rotation.** A priori: each rotation adds at most `sqrt(2) kappa_t (1 + eps_U)` to
   `eps_U >= || (U_hat - U*) e_j ||`.
4. **A chunk's reflections.** The vectors of one chunk in one block are columns `W` of one
   computed rotation product applied to distinct basis vectors, with orthonormal ideals `W*`.
   With `phi = (sum eps_i^2)^(1/2) >= || W - W* ||_F`, the compact WY form gives
   `prod_i (I - 2 w_i w_i^T) = I - 2 W (I + 2E)^-1 W^T`, `E` the strict upper triangle of
   `W^T W`; `|| E ||_2 <= (2 phi + phi^2) / sqrt(2) =: e`; and the chunk's operator differs from
   `I - 2 W* W*^T` by at most `err = 2 [ (1 + phi)^2 2e / (1 - 2e) + 2 phi + phi^2 ]` (requires
   `2e < 1/2`). This grows like the square root of the chunk size.
5. **All factors.** Per block, `|| Y_hat - Y* || <= prod_g (1 + err_g) - 1 =: T`, and the
   column's input term is `(1 + T)(1 + eps_U) - 1`.
6. **The chain's own rounding.** At a step with vector `w` (bound `eps_w`) on a column `y`, the
   dot product of `k` live terms is within `gamma_k ||w|| ||y||` (`gamma_k = k u / (1 - k u)`),
   and the update is within `(2u + u^2)|d w_r| + u |y_r|`. The local error is at most
   `lambda ||y_hat||`, `lambda = 2 gamma_k w2 + 2 (2u + u^2)(1 + gamma_k) w2 + u`,
   `w2 = (1 + eps_w)^2`; earlier errors grow by at most `1 + 2 (w2 - 1)`. `k` is counted on a
   superset of the column's non-zero rows.
7. **Outward rounding.** Every bound is computed in floats from non-negative terms, multiplied
   by `1 + 1e-6`, plus `1e-300` for gradual underflow.

**The rule.** With `g = (gap_ref + gap_fold)(1 + 4u)`, the fold's residual is used only when
`(off_fold + g)(1 + 8u) <= AD_TOL` (`fold::certifies`). That proves `off_ref <= AD_TOL`.
Otherwise the engine computes the residual with the exact kernel and counts a fallback. A
failing fold never decides anything. Non-finite values never certify.

**Equivalence.** The verdict is `phase_ok && (certified || off_exact <= AD_TOL)`, where
`phase_ok` is the exact overlap check. A certificate implies `off_ref <= AD_TOL`, and every
other case uses the exact residual, so the verdict equals the reference's.

**Unit tests** (`src/fastsim/tests.rs`, `fold_*`): mirrored random lanes with random Pauli and
`Z`-string frames, long `Z`-string lanes (the SA shape), `Rotated` references (correct and one
unit off). Each case checks `|off_ref - off_fold| <= g`, that a certificate implies
`off_ref <= AD_TOL`, and that the verdict equals `GaussianLane::finish`. A 120,000-rotation
lane, where `g` exceeds the margin, must fall back; the rule is tested at the threshold itself.

**Measured** (`FEMOCO_FASTSIM_SELFCHECK`, `K = 2^11`, both residuals on every distinct key):

| circuit | keys | certified | fallbacks | max `g` | max `\|off_ref - off_fold\| / g` |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reiher SA | 1,496 | 1,496 | 0 | 5.5e-12 | 2.2e-3 |
| Li SA | 1,689 | 1,689 | 0 | 8.8e-12 | 3.7e-3 |

The bound is loose by a factor of about 300 against the observed differences but stays under
`AD_TOL` on every key. The Li SA margin is the narrowest (largest `g` 9.06e-12 at `K = 2^19`):
a Li-size circuit with somewhat more rotations after its hand-offs would start to fall back,
which costs time, never a verdict.

## 8. Limits and cautions

- **Wider bit-slicing is not the lever on SA.** The classical part is under 1% of reference
  time; the cost is the per-lane Gaussian check.
- **The overlap is the floor.** The SA lanes' contraction matrices are 99% dense (`m` about 220
  for Reiher, 304 for Li). The reference's elimination per distinct key is the largest
  remaining phase, and no exact method avoids it (section 6.1). Going further would need an
  overlap check defined by a computation with a provable error bound: a change of verifier
  version, not of tooling.
- **The memo needs repeated traces.** A circuit with as many distinct traces as lanes gets no
  benefit from it. The pass width is limited by the number of classical-bit planes (bits are
  not renamed by liveness).
- **The outcome stream is a protocol floor** (SHAKE256, about 1 GB/s per core, **measured**).
- **Toolchain dependence.** The kernels rely on float properties of the pinned toolchain
  (obligation 6). Re-run the harness after any toolchain change.
- **No persistent memo.** A cross-run cache would make a cache file an input to verdicts.

## 10. Equivalence harness

Tooling. The harness never changes a verdict: its hooks return the reference's own result and
only panic when a second engine differs.

### 10.1 Interface

| item | where | what |
| --- | --- | --- |
| `Engine` | `src/sim/validate.rs` | `fn(&Context, Option<&Nested>, &[Lane], Option<&mut [u8]>) -> Outcome`. When the last argument is given (one byte per lane, sample order) the engine writes each lane's verdict. |
| `reference` | `src/sim/validate.rs` | The reference as an `Engine`. |
| verdict bytes | `src/sim/validate.rs` | `0..10`: the failure category (index into `CATEGORIES`). `VERDICT_PASS` (100). `VERDICT_UNCHECKED` (101): an execution error stopped the lane's batch; only the lane that raised it carries a category. |
| `evaluate_with` | `src/score.rs` | `evaluate` with the engine as a parameter and an optional `Probe` (lanes, verdicts, the `Outcome`'s `Debug` text). |
| `equiv::ENGINES`, `equiv::engine` | `src/equiv.rs` | The registry `eval_circuit --engine` reads: `reference` and `sliced`. |
| `fastsim::run_probed` | `src/fastsim/mod.rs` | The sliced engine as an `Engine`. A batch the fast path passes is all `VERDICT_PASS`; any other batch gets the reference's own verdict bytes from its re-run. |
| `equiv::engine_counters` | `src/equiv.rs` | The engine's counters after its most recent run (section 4.8). |
| `equiv::evaluate_checked`, `run_checked`, `run_nested_checked` | `src/equiv.rs` | Drop-in replacements for `score::evaluate`, `validate::run` and `run_nested`. With `FEMOCO_EQUIV_ENGINE` set they also run that engine at each `FEMOCO_EQUIV_THREADS` count and panic on any difference; a misspelt engine name is an error, not a skip. |
| `eval_circuit --engine NAME` | `src/bin/eval_circuit.rs` | Selects the engine (default `reference`). Printed on stdout; never written to `score.json`. |
| `eval_circuit --dump-verdicts FILE` | `src/bin/eval_circuit.rs` | Writes the verdict bytes, one per sampled lane. |
| `eval_diff` | `src/bin/eval_diff.rs` | The differential runner on an artifact directory. Flags: `--root`, `--samples` (default 4096), `--candidate NAME[,NAME...]`, `--threads` (default `1,4`), `--ref-threads` (default 4), `--seed MODE[,MODE...]`, `--json FILE`, `--quiet`. It writes no `score.json` and no `results.tsv` row. Exit status 0 when equal everywhere, 1 on any difference, 2 on a usage or input error. |

Environment variables of the harness (all optional):

| variable | meaning | default |
| --- | --- | --- |
| `FEMOCO_EQUIV_ENGINE` | the candidate engine | unset: no second engine in the hooks; `reference` in the `equiv_*` tests |
| `FEMOCO_EQUIV_THREADS` | the candidate's thread counts, comma separated | `1,4` |
| `FEMOCO_EQUIV_REF_THREADS` | the reference's thread count | 4 |
| `FEMOCO_EQUIV_K` | sampled lanes per artifact run | 4096 |
| `FEMOCO_EQUIV_SEEDS` | seed modes per artifact | 3 |
| `FEMOCO_EQUIV_LOG` | a JSONL file every comparison is appended to | none |
| `FEMOCO_EQUIV_FUZZ_CASES`, `FEMOCO_EQUIV_FUZZ_SEED` | fuzz cases and generator seed | 300, 1 |
| `FEMOCO_EQUIV_MUTANT_PER`, `FEMOCO_EQUIV_MUTANT_SEEDS` | points per mutation, seed modes per mutant | 1, 1 |
| `FEMOCO_EQUIV_SA_MUTANTS` | cap on mutants of the full-size SA sweep | 48 |

### 10.2 What is compared

For each input, at each candidate thread count, against the reference:

1. **The `evaluate` result.** The same rejection string, or the same `score.json` with only
   `eval_seconds` removed, compared as serialized text (floats bit for bit, signed zeros
   included).
2. **The lanes handed to the engine**, in order.
3. **The verdict vector.**
4. **The `Outcome`'s `Debug` text**: all nine tally fields, the failure count per category and
   the first failure of each category with its message.

This is stricter than section 5, which asks only for the observables.

**Seed modes.** The ordinary Fiat-Shamir stream, a server seed (`--server-seed`: the audit
domain at round 0) and a fresh-seed audit beacon (`with_audit` at a nonzero round).

**Threads.** Only the engine moves onto a pool of the requested size. Parsing, sampling and
scoring stay on the calling thread.

**Self-test engines** (`equiv::SELFTEST_ENGINES`). Each is the reference with one deliberate
difference; the harness must report every one. `eval_circuit --engine` rejects them.

| engine | difference |
| --- | --- |
| `selftest-tally` | one more Clifford |
| `selftest-verdict` | lane 0's verdict flipped in the dump only |
| `selftest-accept` | every failure cleared |
| `selftest-hmrkey` | a different `Hmr` key |
| `selftest-message` | a reworded first message |
| `selftest-threads` | a tally that depends on the thread count |
| `selftest-unguarded` | the tracker residual read `equiv::UNGUARDED_ERROR = 4e-15` low: a folded check without its band. Caught by the guard cases (10.4), not by the fuzzer, whose residuals are far from the threshold. |

### 10.3 Coverage

`tools/fastsim/equivalence.sh --candidate NAME [--quick]` runs the stages under the
machine-wide lock, appends every comparison to `equiv.jsonl` in the run's log directory and
summarises; its header comment lists the stages and options of the current script. The test
targets it drives:

| test | inputs |
| --- | --- |
| `tests/equiv_fuzz.rs` | Random small op streams. Each case starts from a valid circuit built to pass every lane: a controlled select of random Hermitian Majorana monomials, uncomputed by measurement with a conditioned fix-up or by `CCX` and `R`, inside mirrored noise that uses every lowered op kind (`Givens` undone by the negated angle, `SpinSwap` / `SpinSwapDg`, condition blocks, nested `Reflect` copies, a swap bit). Most cases are then broken by 1-3 mutations and some replaced by unconstrained streams. `tracker_edges_agree` builds the tracker rejections on purpose (more than `MAX_FACTORS` factors, an `S` power at a hand-off, a `SpinSwap` after a system `S`). The self-test engines are run through this fuzzer first. |
| `tests/equiv_guard.rs` | Near-threshold adversarial circuits for the guard band (10.4) and the `selftest-unguarded` check. |
| `tests/equiv_mutants.rs` | Op-stream mutation sweeps of the four small fixture circuits (flat, DF, THC, nested), the `lambda_decl` sign byte, and (ignored by default) a sweep of the full-size Reiher SA circuit of section 1. |
| `tests/equiv_pinned.rs` (ignored by default) | Every SA circuit pinned in `tests/sa_circuits/list.rs`: rebuilt, its four digests checked, then compared under each seed mode and thread count. |
| the other test targets | Every test that evaluates through `equiv::evaluate_checked`, `run_checked` or `run_nested_checked`, run with `FEMOCO_EQUIV_ENGINE` set: the fixtures' named mutants, the adversarial suites, the SA tests and the walk library's tests. |

**Mutation operators** (`tests/equiv_common`, `Mut`). For each op kind present: drop,
uncondition, retarget, recontrol, recondition or duplicate one op. Kind-free: an `X` on an
ancilla; a Pauli or `S` on the system; a conditioned `Neg`; an adjacent swap; a `Givens` that
reads another register or acts on other modes; `SpinSwap` exchanged with `SpinSwapDg`; *rare
lanes* (an AND ladder over the control and `k` uniform bits, sized so that about 8, 2 or 0.5
lanes match, which applies a wrong term, leaves or frees a dirty ancilla, or applies a phase);
*random rare lanes* (an `S` or `Neg` inside nested conditions on fresh `Hmr` outcomes); twin
mutations of nested inner copies. Three verifier behaviours that are easy to get wrong each
have an operator: the `lambda_decl` sign byte; an `Hmr` under a never-set condition; and
liveness (`Hmr` with its own condition, an `R` or `Hmr` on a qubit not yet live, liveness
inside a condition).

### 10.4 Guard-band adversarial cases

The harness cannot check the derivation of `g`. It checks the engine's behaviour on inputs
built to sit at the threshold (`tests/equiv_guard.rs`).

**Construction.** With `beta = 32`, the group commutator
`G_01(a) G_12(b) G_01(-a) G_12(-b)` differs from the identity by a rotation of about
`theta_a theta_b`, `theta = 2 pi x / 2^32`. The residual is tunable in steps of
`theta_b * 2 pi / 2^32`: about `2e-18` for `b = 1`, far below the residual's own rounding. The
angle `a = a0 + s_low` is lane dependent (`CX` gates copy 12 uniform bits into the angle
register), and `a0` is calibrated by bisection on the reference tracker so that `AD_TOL` falls
inside the lane range. Lanes with control 1 and uniform bit 12 set also get a `-1`, a sign-flip
failure that the overlap decides after the `Ad` check.

**Cases.** Four gadgets (`b = 1`; `b = 64`; `b = 64` inside a random constant 60-rotation
network and its inverse; `b = 2^12`), each with three lane picks: *Mixed* (every batch has
failing lanes, so batch fallback is exercised); *Below* (only passing lanes nearest the
threshold, so the fast path's own check decides); *OneAbove* (in each 512-lane batch, 511
passing near lanes and one lane just above `AD_TOL`: the false-accept direction). Per case,
2,048 lanes; 33 to 2,048 of them lie within `1e-13` of `AD_TOL` and up to 1,872 within `1e-15`.

**Assertions.** (1) Both engines agree on every verdict and on the whole `Outcome`, at 1 and 4
threads. (2) The residual computed in the test with the reference tracker predicts every
reference verdict, so the lanes really are decided at the threshold. (3) The reference verdicts
include passes, residual failures and sign flips where the pick asks for them. (4) If the
engine reports a counter with `guard` in its name, it is nonzero on the Below and OneAbove
picks whenever some lane's reference residual lies within the reported band width
(`fold_max_g_attos`) of `AD_TOL`. The sliced engine folds only lanes with at least 16 tracked
vectors, so these small cases reach the fold only with `FEMOCO_FASTSIM_FOLD_MIN=0`.
(5) `selftest-unguarded` is caught on the OneAbove and Mixed picks.

**The overlap threshold.** No case puts the vacuum-overlap distance near `PHASE_TOL`. When
`off <= AD_TOL`, `Omega` is a scalar to about `1e-11`, so the overlap is a unit phase to
comparable accuracy; a lane with an overlap distance near `1e-6` and a residual under `1e-11`
has not been constructed. This is an observation, not a proof. The sliced engine computes the
overlap exactly and needs no band there.

### 10.5 What the harness does not show

- **It is testing, not proof.** Equivalence is evidenced on the inputs above. An audit of the
  engine's code against section 5 has not been done.
- **References.** The fuzzer uses Majorana-monomial references. `Rotated` references are
  covered only through the real circuits and the fixtures.
- **Rotation precision.** Valid fuzz `Givens` use 3-bit angles, so that the negated angle is an
  exact inverse. Wider angles occur in the fixtures and the real circuits.
- **The pass width.** The harness cannot set it; run with each `FEMOCO_FASTSIM_PASS` value to
  cover it. The engine's unit tests run several widths.
- **Thread counts.** Only 1 and 4 by default (`FEMOCO_EQUIV_THREADS`).
- **Machines and toolchains.** Every recorded run is on the one machine and toolchain above.
- **Full runs.** The recorded runs (10.6) are `--quick` runs. No full-configuration run is
  recorded here.

### 10.6 Recorded evidence

- **Harness runs** (`tools/fastsim/equivalence.sh --candidate sliced --quick`; the test tree
  at that time was larger than the one shipped here). Every stage passed in each run: 6 of 6
  wrong engines caught by the fuzzer; fuzz 2,000 cases, 822,655 lanes, 0 differences; guard 12
  cases, 24,576 lanes (8,061 within `1e-14` of `AD_TOL`), 0 unequal, also with
  `FEMOCO_FASTSIM_FOLD_MIN=0`; the fixture sweeps; 22 test targets plus the library with
  20,854 hooked comparisons, 0 unequal (20,402 in the later runs on the real-coefficient
  kernel, which skipped two long exhaustive adversarial tests); 6 pinned SA circuits under
  three seed modes; `eval_circuit --engine sliced` against the default path and `eval_diff` on
  2 built artifacts and 16 single-op mutants of them. The runs also covered 3 baseline
  circuits of other encodings, which are not shipped here.
- **Confirmation-size runs** (section 11): on both SA circuits at `K = 2^19`, at 1 and 4
  threads, every sliced `score.json` was identical to the reference's apart from
  `eval_seconds`, with 0 batches re-run on the reference and 0 guard fallbacks.
- **Self-check runs** (section 6.1): the bound held on every key.

## 11. Overlap on real coefficients, and measured performance

Every entry of the reference's contraction matrix is purely real or purely imaginary: the
reference writes `(d, 0.0)`, `(0.0, d)` or `(0.0, -d)` according to the parities of the two
factors. This survives the whole elimination in floating point: every complex operation the
reference computes has one part formed only from products with an exact-zero factor (a zero),
and the other part equals one real operation up to the sign of a zero. `src/fastsim/pfreal.rs`
performs exactly those real operations: 2 multiplications and 2 additions per entry update
instead of 8 and 6. Every value read (pivot norms, heads, `tau`, `row`) and the overlap equal
the reference's up to zero sign, and the pivot choices are the reference's. The argument is in
the module docs of `pfreal.rs`; it rests on the sign rule of an update, on exact negation in
the head division, and on `hypot(x, 0) = |x|` bit for bit (tested on 400,000 values with both
zero signs, including subnormals and extremes). `real_pfaffian_matches_the_reference` runs 600
cases at 7 panel sizes, all equal up to zero signs. Matrices whose non-zero pattern splits
into several components keep the per-component complex path.

**Measured, whole engine** (`eval time`; reference and sliced at the same thread count; each
line one run; run-to-run variation about 10%):

| circuit | K | threads | reference | sliced | speedup | sliced lanes/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reiher SA | 2^19 | 4 | 571.29 s | 7.76 s | 73.6x | 67,560 |
| Reiher SA | 2^19 | 1 | 2,109.6 s | 30.27 s | 69.7x | 17,320 |
| Li SA | 2^19 | 4 | 1,572.79 s | 36.68 s | 42.9x | 14,290 |
| Li SA | 2^19 | 1 | not run (fit about 5,800 s, section 1) | 140.20 s | about 41x (**estimate**) | 3,740 |

At `K = 2^19` the Reiher SA run has 43,259 distinct keys for 524,288 lanes and the Li SA run
108,146. The speedup grows with `K` because keys grow about as `K^0.45`. With the complex
elimination (`pfblock`) the same Reiher SA run took 12.8 s on 4 threads and 51.9 s on 1, and
at `K = 2^14` the engine measured 10.4x (Reiher SA) and 8.2-8.5x (Li SA). On a
tensor-hypercontraction circuit of another encoding (not shipped here), which has as many keys
as lanes, the measured speedup was 2.3-2.6x.

**Where the sliced engine's time goes** (**measured**, single thread, `K = 2^14`,
thread-seconds from `FEMOCO_FASTSIM_PROF`):

| circuit | elimination | Gram | fold and guard | tracking | executor |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reiher SA | 1.09 s | 0.67 s | 0.93 s | 0.34 s | 0.27 s |
| Li SA | 3.88 s | 2.16 s | 2.61 s | 1.05 s | 0.72 s |

The elimination is the largest single phase, about a third of per-key time; the fold with its
guard and the Gram matrix are of the same order. No single phase dominates.
