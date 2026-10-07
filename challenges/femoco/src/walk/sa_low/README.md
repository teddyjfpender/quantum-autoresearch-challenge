# The spectrum-amplified walk step (`src/walk/sa_low`)

This directory builds the controlled walk step of Low, King, Berry, Han, DePrince, White, Babbush,
Somma and Rubin, *Fast quantum simulation of electronic structure by spectrum amplification*,
Phys. Rev. X 15, 041016 (2025), doi:10.1103/pb2g-j9cw (the journal version; arXiv:2502.15882 v1 is
superseded), on the `sos-sa` specs (spec/SPEC-SA.md: their DFTHC + BLISS factors from Zenodo
17066718), and a set of build-time levers on top of it. This file is the reference for every
architecture, knob and lever in the directory.

Contents:

1. [Conventions](#1-conventions)
2. [Architectures and knobs](#2-architectures-and-knobs)
3. [`sa-low2025`: the published step](#3-sa-low2025-the-published-step)
4. [`sa-lowq`: streamed angles](#4-sa-lowq-streamed-angles)
5. [`sa-pareto`: lean layout, chunked angles, narrow gadgets](#5-sa-pareto-lean-layout-chunked-angles-narrow-gadgets)
6. [`sa-toff`: the lever reference (`FEMOCO_SA_TWEAKS`)](#6-sa-toff-the-lever-reference-femoco_sa_tweaks)
7. [Lower bounds](#7-lower-bounds)
8. [Measured fronts on the two tracks](#8-measured-fronts-on-the-two-tracks)
9. [Measured circuits and their digests](#9-measured-circuits-and-their-digests)
10. [Tests and static scans](#10-tests-and-static-scans)

## 1. Conventions

**Specs.** The two tracks are `reiher-sa-est-v1` (N = 54 orbitals, the default) and `li-sa-est-v1`
(N = 76). They declare Low et al.'s estimated rounding class (spec/SPEC-SA.md section 14):
acceptance follows their App. D error model, an estimate and not a bound. Their rigorous twins
`reiher-sa-v1` / `li-sa-v1` carry the same `sa.bin` (the same factors and rotation bits, 16 on
Reiher and 15 on Li) under an exact 0.1 mHa 1-norm rounding rule. The only difference a walk sees
is the alias keep width that the rule forces:

| | keep bits, outer + inner (`Params::for_spec`) | rounding error |
| --- | --- | --- |
| `reiher-sa-est-v1` | 9 + 9 (Low et al.'s cost-model width); 8 + 8 also meets the class's rule | exact 1-norm 7.97e-2 Ha at 9 + 9, reported and not applied |
| `li-sa-est-v1` | 9 + 9; 8 + 8 also meets the rule | exact 1-norm 2.83e-1 Ha at 9 + 9, reported and not applied |
| `reiher-sa-v1` | 19 + 19 | 8.778e-5 Ha (exact) |
| `li-sa-v1` | 20 + 21 | 9.768e-5 Ha (exact) |

The class's rule needs a table resolution `b_equiv >= 9`. 8 + 8 meets it exactly
(`b_equiv(28, 13) = 9`, `b_equiv(324, 17) = 9` on Reiher; `b_equiv(58, 14) = 9`,
`b_equiv(361, 17) = 9` on Li); 9 + 9 resolves 10. The keep width is calibration, not a mechanism.

Several levers were first measured on the rigorous twins. Where a number below is from a twin, the
table or sentence names the spec. A lever's Toffoli and qubit effect on the estimated specs differs
from the twin's wherever it scales with the keep width or the alias word.

**Labels.**

- *measured*: the harness's counts (`./benchmark.sh`, K = 2^19 = 524,288 sampled lanes), or the
  same K on the fast lane engine (`eval_circuit --engine sliced`, spec/FAST-EVALUATOR.md) after a
  K = 64 check with the reference evaluator. Tables say which.
- *static*: the walk's expected-count ledger (`ledger.rs`) plus the harness's static peak pass.
  Static counts have equalled the harness's on every run measured (exactly when no Toffoli is
  outcome-gated, within sampling noise otherwise).
- *quoted*: printed in the paper. *derived*: a formula is given.

**Sampled means.** With lever `g`, the gated re-reads (`k`, `j`, `~`) or the narrow gadgets, some
Toffolis run only on one measurement outcome, so `C_step` is a sampled mean and carries decimals.

**Charge conventions.** The harness charges `2 (beta - 2)` Toffolis per Givens rotation (Lee et al.
2021, App. C); Low et al.'s resource script charges `2 beta`. "Their charge" below is
`C_step + 4 x Givens` (212 Givens per step on Reiher, 300 on Li: +848 / +1,200), the like-for-like
Toffoli count; `score.json` reports it as `low2025_givens_charge`. `SpinSwap` is charged `N + 1`
(the `+1` is the fermionic block-parity `CCZ`), theirs `N`.

**Qubit conventions.** `Q_peak` is the harness's true peak over the step, with the phase-gradient
register (16 / 15 qubits) and without a phase-estimation register. Low et al. count every
persistent register plus the largest temporary, also without a phase-estimation register: 1,132 /
1,454 (Table V). The true peak of their layout is 1,055 / 1,378 (derived). "+PE" adds the
Lee-convention register `2 ceil(log2(I + 1)) - 1` at their step counts (+31 / +33).

**Low et al.'s Table V point** (quoted): 10,203 Toffolis per step and 1,132 qubits (Reiher);
14,629 and 1,454 (Li).

## 2. Architectures and knobs

Every knob is a build-time environment variable read by `build_circuit` (`option_env!`).
`FEMOCO_WALK_SPEC` selects the spec (default `reiher-sa-est-v1`), `FEMOCO_WALK_ARCH` the
architecture (default `sa-low2025`).

| `FEMOCO_WALK_ARCH` | Emitter | Family (parent) | Knobs besides the common ones |
| --- | --- | --- | --- |
| `sa-low2025` | `mod.rs::emit`, `inner.rs` | `sa-low2025-spinswap` | none (the published construction) |
| `sa-low2025-bothspin` | the same | `sa-low2025-bothspin` | none; runs every network on both spins instead of `SpinSwap` |
| `sa-lowq` | `mod.rs::emit`, `inner.rs` | `sa-lowq-stream` (`sa-low2025-spinswap`) | `FEMOCO_SA_CHUNKS` (default 2), `FEMOCO_SA_OUTER_ERASE` (1), `FEMOCO_SA_DENSE` (1) |
| `sa-toff` | `toff.rs` | `sa-low2025-toff` (`sa-low2025-spinswap`) | `FEMOCO_SA_TWEAKS` (default `all`) |
| `sa-pareto` | `pareto.rs` | `sa-pareto` (`sa-low2025-spinswap`) | `FEMOCO_SA_CHUNKS` (1), `FEMOCO_SA_LEAN` (1), `FEMOCO_SA_CARRIES` (0), `FEMOCO_SA_DROP_ALT` (0), `FEMOCO_SA_NARROW` (empty) |

Common knobs (`Params::for_spec`):

| Knob | Meaning | Default |
| --- | --- | --- |
| `FEMOCO_SA_MU_O`, `FEMOCO_SA_MU_I` | outer and inner alias keep widths | the table in section 1 |
| `FEMOCO_SA_OUTER_A` | log2 of the outer QROAM block count (their `k1`) | 2 |
| `FEMOCO_SA_INNER_A` | log2 of the inner QROAM block count (their `k2`); with lever `G` the paired lookup's slot width | `k_i - 1` (4 Reiher, 5 Li); `sa-lowq` caps it at 4 once `C > 1` |

All families share the axes `encoding = sos-sa`, `lane_map = sa-nested-alias-v1`,
`lookup = qroam-clean`, `select = givens-sa-nested`, `uncompute = measurement-based`,
`reuse = serial`, `rotation = phase-gradient-givens` (taxonomy 1.3.0). The harness verifies
encoding, lane_map, select, uncompute and rotation; lookup and reuse are declared only (no
signature separates them). Every lever is a layout or erasure choice within these axes.

The lane map depends only on the spec, the keep widths and three padding levers (`p`, `+`, `.p`,
which rearrange padding buckets without changing any count): `mod.rs::lane_map`. One-body inner
tables are replaced by the equivalent `keep = 0, alt = i mod 2` table (the same counts), so a
one-body lane's inner item is the low inner index bit and needs no lookup.

`sa-low2025`, `sa-lowq`, `sa-toff` and `sa-pareto` use `SpinSwap` / `SpinSwapDg` (op kinds
32 / 33), the controlled spin swap defined for `sos-sa` specs in spec/SPEC-SA.md section 11.
`sa-low2025-bothspin` does not.

The `sa-toff` levers and `sa-pareto` are separate walks; `emit` refuses to combine them. Three
ideas exist in more than one walk, each with its own implementation:

- **Streaming the angles.** `sa-lowq` streams through one slot register with XOR transitions over
  a dense or padded index (`inner.rs`). `sa-pareto` uses ragged loads on `shared::angles`'s
  schedule (`pareto.rs`). Both read `FEMOCO_SA_CHUNKS`.
- **Deriving `id` from `b`.** `sa-toff` lever `i`; part of `sa-pareto`'s lean layout.
- **Keeping comparator carries.** `sa-toff` levers `l`, `L`; `sa-pareto`'s `FEMOCO_SA_CARRIES`.

```bash
./benchmark.sh                                                        # sa-low2025 on reiher-sa-est-v1
FEMOCO_WALK_SPEC=li-sa-est-v1 ./benchmark.sh
FEMOCO_WALK_ARCH=sa-toff FEMOCO_SA_TWEAKS=imchxlgr ./benchmark.sh
FEMOCO_WALK_ARCH=sa-pareto FEMOCO_SA_CHUNKS=3 FEMOCO_SA_DROP_ALT=1 ./benchmark.sh
```

`tools/repro/` holds scripts that rebuild and re-measure pinned circuits of the two tracks.

## 3. `sa-low2025`: the published step

Low et al.'s construction (their Fig. 2 and App. B), built as published.

### 3.1 The step

| Their subroutine | Here |
| --- | --- |
| `PREP_outer`: uniform over `x_o`, alias QROAM (`2^k1 = 4` blocks), test and swap | The uniform superposition is the harness's lanes (no Toffolis). A clean QROAM read (4 blocks) of the outer bucket's `keep \| own item \| alt item`, `lt = [draw < keep]`, alt swapped in. Item data: `lo \| hi \| q \| is_ob \| pos_e` (`tables.rs`). `en_ob = control AND is_ob`, `en_sq = control AND NOT is_ob`, `pass`. |
| inner PREP: iteration over `x_o` with QROAM over `b` (`2^k2` blocks) | A clean QROAM read at `(q, i)` with `2^(k_i - 1)` blocks over `i` (16 Reiher, 32 Li: their `k2`); own item `b = i` copied; test and swap. One-body lanes read nothing: their inner item is the low inner index bit. |
| RPREP: QROM of all `N - 1` angles over `N + R B` entries | One ragged unary read over `(hi, lo)`: `R` rows of `B`, then the one-body rows (`N + R B` leaves plus the row iteration). Angle registers are `beta - 1` qubits where every angle allows it (796 / 1,051 bits in all). |
| SELECT: `U`, controlled spin swap, Majorana, swap, `U^dagger` | `F^-s`, `V^dagger` (as `Z_S V_rev Z_S`, the same registers), the controlled Majorana on mode `(0, 0)`, `V`, `F^s`. `F` is `SpinSwap`. The spin `s` is `s1` (one-body) or `s0` (square), one Toffoli. The second copy's `Maj^dagger` is `CZ(sel_x, pass)`. |
| RPREP^dagger, inner PREP^dagger | Measurement-based: angles measured out with one phase fixup; the inner read's junk blocks are measured when read and their fixup is merged with the output's at the erasure (`qroam.rs`). |
| `R_T2` inner reflection, walk reflection | The harness's `Reflect` (charged `w - 2`) and walk reflection (`u - 2`). |

One step is: outer PREPARE; `nested_inner` (inner PREPARE, RPREP, SELECT, RPREP^dagger, inner
PREPARE^dagger), the `Reflect` on the inner register, and the identical copy, which applies
`M^dagger` by reading `pass`; outer UNPREPARE; the walk reflection.

Sizes: `E = R B + N` networks (leaves): `10 x 27 + 54 = 324` on Reiher, `15 x 57 + 76 = 931` on
Li. `k_i = 5 / 6` inner index bits.

### 3.2 Measured

| | reiher-sa-est-v1 | reiher-sa-v1 | li-sa-v1 |
| --- | ---: | ---: | ---: |
| `C_step` (Toffolis per step, harness charge) | **9,600** | **10,020** | **14,788** |
| `Q_peak` | **1,040** | **1,080** | **1,393** |
| executed CCX / Givens / SpinSwap per step | | 3,789 / 212 / 4 | 6,598 / 300 / 4 |
| `lambda_decl` (= Lambda) | | 58.343975 | 179.729577 |
| product `lambda_eff_used x C_step x Q_peak` (21.3674 / 43.6538, Low et al.'s `lambda_eff`) | | 2.3123e8 | 8.9925e8 |
| implied error bound (sampling, single draw) | | 0.037 Ha | 0.114 Ha |

(Measured, `./benchmark.sh`, K = 2^19. `li-sa-est-v1` was not measured for this architecture.)

Static per-stage counts (`tests::pinned_ledgers`) add up exactly to the harness's executed counts:
every Toffoli here is unconditioned, and `tests::walk_passes_on_an_exact_spec` checks the same
identity against `score::evaluate`.

`sa-low2025-bothspin` was measured on `reiher-sa-v1` only: **C_step 15,740, Q_peak 1,080**. Its
static Toffoli count on `li-sa-v1` is 22,284. Without `SpinSwap`, the spin sector costs 212 / 300
more Givens per step.

### 3.3 Against Table V, component by component (rigorous twins, Toffolis per step)

Theirs: reproduced from their resource script; totals quoted from Table V. Ours: measured on
`reiher-sa-v1` / `li-sa-v1`, split by the static ledger.

**Reiher** (their b_coeff 9, b_rot 16; ours: keep bits 19 / 19, rotation bits 16)

| Component | Theirs | Ours | Why they differ |
| --- | ---: | ---: | --- |
| Outer PREP + PREP^dagger | 280 | 439 | Their 72 for the uniform state is free here (lanes are uniform). Our QROAM word is 59 bits (19 keep + own item 20 + alt item 20) against their `b1 = 18` (9 keep + 9 index): about +105 in the swap network (`3 x 59` vs `3 x 18` plus their copy-out) and +10 per comparator; the item's fields are looked up rather than recomputed from `x_o`. |
| Inner PREP, both copies (read, test/swap, erasure, uniform) | 2,138 | 2,552 | Same iteration (540 leaves per read) and block count. Our word is 28 bits (19 keep + own sign/id 2 + alt `b` 5 + alt sign/id 2) against their `b2 = 15` (9 keep): +195 per read in the swap network. Their 80 for uniform states are free here. |
| RPREP + RPREP^dagger, both copies | 722 | 792 | Read 345 vs 324 (the ragged iteration's row level); erasure 51 vs 37. |
| SELECT rotations | 6,784 | 5,936 | Same 212 Givens. Charge convention: 28 per Givens here, 32 in their script. Under their charge ours is 6,784. |
| Controlled spin swap | 216 | 220 | `N + 1` vs `N`. |
| Majorana and controls | 14 | 6 | |
| Reflections | 49 | 75 | Harness charges `u - 2 = 52` and `w - 2 = 23`: the wider keep draws widen both registers. |
| **Total** | **10,203** | **10,020** | |
| Total under their Givens charge (derived) | 10,203 | **10,868** | |

**Li** (their b_coeff 9, b_rot 15; ours: keep bits 20 / 21, rotation bits 15)

| Component | Theirs | Ours | Why they differ |
| --- | ---: | ---: | --- |
| Outer PREP + PREP^dagger | 292 | 460 | As Reiher (word 64 bits vs 18). |
| Inner PREP, both copies | 2,962 | 3,976 | Word 31 bits vs 16, 32 blocks: +465 per read. |
| RPREP + RPREP^dagger, both copies | 2,006 | 2,156 | Read 981 vs 931, erasure 97 vs 72. |
| SELECT rotations | 9,000 | 7,800 | 300 Givens at 26 (harness) vs 30 (theirs). |
| Controlled spin swap | 304 | 308 | `N + 1` vs `N`. |
| Majorana and controls | 14 | 6 | |
| Reflections | 51 | 82 | `u - 2 = 56`, `w - 2 = 26`. |
| **Total** | **14,629** | **14,788** | |
| Total under their Givens charge (derived) | 14,629 | **15,988** | |

**Qubits.** Ours on `reiher-sa-v1`: 1,080 = control 1 + system 108 + uniform 54 + angles 796 +
outer registers about 63 + inner registers about 34 + phase gradient 16 + a few scratch qubits (the
split is from register widths, not measured per qubit). Two things lower it against their 1,132:
the angle registers are `beta - 1` bits where the data allow (796 vs 848), and the peak is over
phases, not persistent plus largest temporary. The keep draws raise it (the uniform register is 54
wide against their 18 keep-draw qubits plus indices). Li: 1,393 against 1,454. With a
phase-estimation register ours is 1,111 / 1,426.

### 3.4 Totals (derived from the measured `C_step`, `Q_peak` on the rigorous twins)

| | Reiher theirs | Reiher ours | Li theirs | Li ours |
| --- | ---: | ---: | ---: | ---: |
| steps at sigma_PEA 1.0 mHa, their lambda_eff (21.3674 / 43.6538) | 33,564 | 33,564 | 68,572 | 68,572 |
| total Toffolis, their convention (`steps x C_step`) | 342,453,492 (paper 3.42e8) | 336,311,280 | 1,003,139,788 (paper 1.00e9) | 1,014,042,736 |
| same under their Givens charge | 342,453,492 | 364,773,552 | 1,003,139,788 | 1,096,329,136 |
| steps at the certified lambda_eff bound (19.9906 / 43.6098, `score.json`) | | 31,402 | | 68,503 |
| qubits without / with PE register | 1,132 / 1,163 | 1,080 / 1,111 | 1,454 / 1,487 | 1,393 / 1,426 |
| product `lambda_eff x C_step x Q_peak` at 21.3674 / 43.6538 | 2.468e8 | 2.312e8 | 9.285e8 | 8.992e8 |

Theirs: Cost[BE] and qubits quoted (Table V), steps and products derived. Certified claims use the
larger lambda_eff, theirs (spec/SPEC-SA.md section 6); the certificate's bound is lower, and the
harness's own `lambda_eff_ours` (19.9906 / 43.6098) includes the run's rounding error.

### 3.5 Summary

- **The published build reproduces their step within the conventions, not within rounding.** On
  the rigorous twins the measured step is 10,020 (Reiher), 1.8% below their 10,203, and 14,788
  (Li), 1.1% above their 14,629. Under their own Givens charge it is 10,868 (+665, +6.5%) and
  15,988 (+1,359, +9.3%). That like-for-like excess is itemised above: outer PREP +159 / +168,
  inner PREP +414 / +1,014, RPREP +70 / +150, reflections +26 / +31, spin swap +4 / +4, Majorana
  -8 / -8. Most of it is alias-word width and comparator length forced by the rigorous 0.1 mHa
  rounding rule (19 + 19 and 20 + 21 keep bits against their 9 + 9), partly offset by the uniform
  superpositions that are free here (their 152 / 168). The rest (the outer item's looked-up
  fields, the RPREP row level and erasure, about +150 / +250) is this walk's table layout.
- **On the estimated class**, where the keep width is theirs, the published build measures
  9,600 / 1,040 on Reiher: 10,448 under their charge (derived), +245 against 10,203.
- **Qubits** are below theirs, partly by the peak-over-phases convention and partly by `beta - 1`
  angle registers.

## 4. `sa-lowq`: streamed angles

`LOWQ.md` in this directory describes `sa-lowq`: the published step with the angles held
`ceil((N - 1) / C)` at a time, the outer alias garbage measured early and a dense network index.
Measured on the rigorous twins at `C = 2`: 11,605 / 652 (`reiher-sa-v1`) and 18,877 / 832
(`li-sa-v1`).

## 5. `sa-pareto`: lean layout, chunked angles, narrow gadgets

The published step with build-time gadgets that trade Toffolis for qubits (`Params::pareto`,
`pareto.rs`). `sa-low2025` / `sa-low2025-bothspin` still emit byte-identical op streams and lane
maps (`tests::legacy_ops_digest`).

### 5.1 Gadgets

| Gadget | Knob | Toffolis / step (static, `reiher-sa-v1` / `li-sa-v1`) | Qubits |
| --- | --- | ---: | ---: |
| Lean layout. The ragged RPREP iteration skips left-only ANDs: the identity item lands on a real leaf, harmless since `V I V^dagger = I`, and the erasure uses the same leaf map (`SaTables::leaf`); -44 / -102 alone. The inner word drops both `id` flags (`id = [b = B]` formed from `b` around the Majorana, `k_i - 1` Toffolis). The one-body flags `is_ob`, `pos_e` travel with the angle word instead of twice in the outer word (`en_ob`, `en_sq` formed per copy). Needs `SpinSwap`. | `FEMOCO_SA_LEAN` (default on) | -117 / -237 | -6 / -6 |
| Keep comparisons keep their carries, erased by measurement | `FEMOCO_SA_CARRIES` (bit 1 outer, bit 2 inner; 3 both) | -18 / -19 outer, -36 / -40 inner | +18 / +19, +18 / +20 |
| The outer alias read's unused slot is measured right after the swap; the chosen slot is measured at UNPREPARE; the `lt`-dependent phase is cancelled by one fixup controlled by `lt` (`qroam::phase_fixup_ctl`) | `FEMOCO_SA_DROP_ALT` | +42 / +40 | -18 / -20 |
| Angles held `ceil((N-1)/C)` rotations at a time (`shared::angles` schedule: `2C - 1` ragged loads per copy) | `FEMOCO_SA_CHUNKS` | `+4 (C - 1) x 323 / 929` | angle register / C |
| Fewer QROAM blocks, so a read is not the peak once the angles are streamed | `FEMOCO_SA_INNER_A`, `FEMOCO_SA_OUTER_A` | read cost `L / 2^a + (2^a - 1) w` | `2^a w` during the read |

The lean layout alone takes the published build from 10,020 / 1,080 to 9,903 / 1,074
(`reiher-sa-v1`) and from 14,788 / 1,393 to 14,551 / 1,387 (`li-sa-v1`), static
(`tests::pareto_scan`). The carry and keep-width deltas scale with the keep width, so they are
smaller on the estimated specs.

A skipped `lt`-controlled fixup is caught by the harness: the sign flips on 532 of 16,384 lanes.

### 5.2 Measured points

Knobs are `(C, inner_a, outer_a, carries, drop_alt)`, lean on. Measured, `./benchmark.sh`, K = 2^19.

`reiher-sa-est-v1` (keep 9 + 9):

| Knobs | C_step | Q_peak |
| --- | ---: | ---: |
| 1, 4, 2, 3, 0 | 9,459 | 1,050 |
| 1, 4, 2, 0, 1 | 9,525 | 1,016 |
| 3, 4, 2, 0, 1 | 12,101 | 494 |

`li-sa-est-v1` (static, not measured): C = 1 with carries 3 at `inner_a` 5: 13,645 / 1,357; C = 1
with `drop_alt` at `inner_a` 5: 13,709 / 1,321; C = 3 with `drop_alt` at `inner_a` 4:
21,737 / 624.

Rigorous twins (`reiher-sa-v1` rows R1 to R5, `li-sa-v1` rows L1 to L5). "Their charge" is
`C_step + 4 x Givens`. Totals are derived: `ceil(pi lambda_eff / (2 x 1.0 mHa)) x C_step` with
their lambda_eff (33,564 / 68,572 steps) and with the certified bound (31,402 / 68,503 steps).

| Point | Knobs | C_step | Q_peak | Q + PE | their charge | `lambda_eff x C x Q` | total, their lambda_eff | total, certified lambda_eff |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| R1 | 1, 4, 2, 3, 0 | **9,849** | **1,110** | 1,141 | 10,697 | 2.336e8 | 330,571,836 | 309,278,298 |
| R2 | 1, 4, 2, 0, 1 | **9,945** | **1,056** | 1,087 | 10,793 | 2.244e8 | 333,793,980 | 312,292,890 |
| R3 | 2, 4, 2, 0, 1 | 11,233 | 669 | 700 | 12,081 | 1.606e8 | 377,024,412 | 352,738,666 |
| R4 | 4, 3, 2, 0, 1 | 14,473 | 474 | 505 | 15,321 | **1.466e8** | 485,771,772 | 454,481,146 |
| R5 | 8, 2, 1, 0, 1 | 21,595 | 369 | 400 | 22,443 | 1.703e8 | 724,814,580 | 678,126,190 |
| Low et al. (Table V) | | 10,203 | 1,132 | 1,163 | 10,203 | 2.468e8 | 342,453,492 | |
| L1 | 1, 5, 2, 3, 0 | **14,492** | **1,426** | 1,459 | 15,692 | 9.021e8 | 993,745,424 | 992,745,476 |
| L2 | 1, 5, 2, 0, 1 | **14,591** | **1,367** | 1,400 | 15,791 | 8.707e8 | 1,000,534,052 | 999,527,273 |
| L3 | 2, 4, 2, 0, 1 | 18,519 | 852 | 885 | 19,719 | **6.888e8** | 1,269,884,868 | 1,268,607,057 |
| L4 | 4, 3, 2, 0, 1 | 27,767 | 586 | 619 | 28,967 | 7.103e8 | 1,904,038,724 | 1,902,122,801 |
| L5 | 10, 2, 1, 0, 1 | 54,399 | 432 | 465 | 55,599 | 1.026e9 | 3,730,248,228 | 3,726,494,697 |
| Low et al. (Table V) | | 14,629 | 1,454 | 1,487 | 14,629 | 9.285e8 | 1,003,139,788 | |

Implied error bound (sampling, single draw): 0.037 Ha (Reiher), 0.114 Ha (Li) on every twin point.

Reading the twin table:

- **Harness charge.** R1, R2, L1, L2 are below Low et al. on Toffolis per step, qubits, total at
  sigma_PEA = 1.0 mHa with their lambda_eff, and the product. The margins are small on Toffolis
  (-3.5% / -2.5% Reiher, -0.9% / -0.3% Li).
- **Their Givens charge.** No point beats them on Toffolis or total: 10,697 vs 10,203 (+4.8%),
  15,692 vs 14,629 (+7.3%). The remaining like-for-like excess is the keep width of the rigorous
  rule.
- **Product.** Every point beats their `C_step x Q_peak` except L5; the minima are R4 (0.59x
  theirs) and L3 (0.74x). Streaming costs `4 (C - 1)` ragged loads (323 / 929 Toffolis each), so
  the Toffoli penalty grows much faster on Li.
- **Narrow points.** R5 (369 qubits, 400 with a PE register; total 7.2e8 at their lambda_eff) and
  L5 (432 / 465; 3.7e9) are far below every published estimate at or under that width: Berry et
  al. 2019's dirty-ancilla single factorization (378 / 437 qubits, 2.1e13 / 2.0e13 Toffolis),
  qDRIFT (1e16 and above) and the Trotter estimates (5e13). These are not like-for-like
  comparisons (different encodings, lambdas and error conventions).
- The unstreamed (C = 1) qubit margins hold only under the counting convention (their true peak
  is 1,055 / 1,378, derived). The streamed points (C >= 2) are qubit and product gains under either
  convention.

### 5.3 Alternatives that do not pay (static estimates)

- A contiguous network index `r B + b` by an adder: the in-range iteration gets the same saving
  without the adder.
- Moving the sign flag `neg` into the angle word: impossible, the sign of `w_(r,c),b` varies with
  `c` for all 270 / 855 `(r, b)`.
- Overlapping outer fields (`lo` inside `q`): needs a controlled XOR, net 0.
- Clean QROAM transitions between chunks: worse than unary iteration for chunk words of 100 to 500
  bits.
- Dropping the outer keep or the inner alt in this walk: about 13 and 35 Toffolis per qubit (the
  narrow gadgets below do it with gated re-reads instead).

### 5.4 Narrow gadgets (`FEMOCO_SA_NARROW`)

Build-time gadgets for the streamed `sa-pareto` walk that take qubits out of the step's peak. Each
is one letter of `FEMOCO_SA_NARROW` (`pareto::Narrow::parse`: a letter, then an optional number;
an unknown letter panics). Default empty: the circuits above are unchanged. All need the lean
layout. The lane map is `sa-pareto`'s at the same keep split, so the rounding error is the same.

**Where the peak is.** On every streamed point the peak is a chunk transition (an XOR of two
chunks' angle words by a ragged unary iteration over the network index): the angle slots, the
iteration's AND ladder (8 qubits Reiher, 10 Li), and everything else that is live across SELECT
(persistent registers, the outer item and keep, the inner read's keep, sign and alt item, `b`, and
the flags `en_ob`, `en_sq`, spin select, `pass` and the two comparison bits). The Majorana stage
and the inner read sit just below it. So each cut below takes the same qubits out of both leading
stages, and the gadget that touches only the Majorana stage (`j`) matters only once the shared part
is cut. `tests::stream_census` prints the census (every ancilla tagged with the ledger stage that
first touched it).

| Letter | Gadget | Qubits across the peak | Toffolis / step (expected) |
| --- | --- | ---: | ---: |
| `e` | `en_ob`, `en_sq` are made right before the Majorana and erased after it; the spin-select qubit is erased after `F^-s` and made again before `F^s`. None of the three is live across the chunk transitions. | -3 | +2 |
| `g` | Every plain keep comparison still erased with its keep held is erased by the outcome-gated recompute (`common::arith_gated`) | 0 | `-(mu - 1) / 2` per site |
| `c` | Compact outer data `q \| hi` (no `lo`): a one-body item's `q` is `ones \| r`; each copy XORs `is_ob AND q_low` into `b` and out again, `is_ob` the AND of `q`'s `ones` bits | `-k_i` (-5 Reiher, -6 Li) | `2 (q_b - kb - 1 + k_i)` per copy |
| `k<a>` | The outer keep is measured right after the comparison; the comparison is erased by the **outcome-gated re-read** (`narrow.rs`): `lt` is X-measured, and only where the outcome is 1 is `keep` read again (`2^a` blocks, default `a = 2`), the comparison's phase applied with Cliffords on its carries, and the read measured; its outcomes join the outer read's final fixup. Needs `FEMOCO_SA_DROP_ALT=1` and the plain outer comparison. | `-mu_o` | about `(L_o / 2^a + (2^a - 1) mu_o) / 2` |
| `o` | With `k`: `lt` is measured too; UNPREPARE reads `keep \| own ^ alt` once, recomputes `lt`, and cancels `lt . (m_l ^ (m_a ^ m_m) . (own ^ alt))` with one `CZ(lt, g)` | -1 | one full re-read in place of the controlled fixup and the gated re-read |
| `r<a>` | The same gated re-read for the inner keep, in each copy (default `a = 2`). Needs the plain inner comparison. | `-mu_i` | `+(L_i / 2^a + (2^a - 1) mu_i) / 2` per copy |
| `a` | With `r`: the inner read's unchosen item is measured right after the swap; at PREPARE^dagger the item register is measured too and one fixup over `[lt] ++ x` cancels both (the `i` parts by Cliffords), with the read's junk and keep | `-(k_i + 1)` (-6 / -7) | one fixup per copy |
| `q<a>` | The outer item's `q` field is measured after the RPREP read and read again (`2^a` blocks, default 2) before RPREP^dagger in each copy (one in-copy fixup over `[lt] ++ x_o`). Needs `drop_alt`; not with `o`. | `-q_b` (-9) | one read and fixup per copy |
| `p` | The copy's `pass` flag is a classical bit toggled at the end of each copy (the copies are the same ops, so it reads 0 then 1). Needs `e`. | -1 | 0 |
| `j` | The Majorana stage holds `id = [b = B]` without its AND chain (intermediates measured at once, `id` erased by a gated recompute) and one enable at a time. Needs `e`. | -4 at the Majorana stage only | `(k_i - 2) / 2 + 1` per copy |
| `h<n>` | At most `2^n` one-hot qubits in the fixups these gadgets add (default 6) and, when given, in the plain inner erasure's fixup too | | |

Notes:

- The gated re-read runs on half the lanes. Its outcomes (0 where the block did not run) join the
  outer read's single fixup, so releasing the keep costs about half a read, not a full second read
  (`sa-lowq`'s early erase pays +283 / +296 for -39 / -42 on the rigorous twins).
- `drop_alt`'s `lt`-controlled fixup is unchanged with `k` and replaced by the `CZ(lt, g)` with `o`.
- The transition ladder itself (9 index bits, 8 ANDs on Reiher) is the minimum for unary iteration.
- Mutants rejected by the harness (`tests::narrow_mutants_are_rejected`, `narrow::tests`): the
  gated block's `CZ` or `Z` dropped, the block run on the wrong outcome, `g`'s `m_l` part or the
  `CZ(lt, g)` dropped.

These gadgets were measured only on specs that are not shipped (a rigorous ground-energy rounding
rule at one rotation bit fewer), where each measured point traded Toffolis for qubits against the
streamed point it started from. No count on the two tracks is claimed; `tests::narrow_scan` gives
static counts on any spec.

## 6. `sa-toff`: the lever reference (`FEMOCO_SA_TWEAKS`)

`sa-toff` is `sa-low2025` with compile-time levers (`toff.rs`; `Tweaks` in `mod.rs`). Each lever is
a build-time switch so every one can be measured on its own. The lane map (apart from the padding
levers), SELECT's rotations, the Givens count and the four `SpinSwap`s are unchanged, and no
trusted file is involved. Every bundle that is pinned keeps its op stream when a new lever is
added (`tests/sa_digests.rs`).

### 6.1 Syntax

`FEMOCO_SA_TWEAKS` is a string of lever tokens (`Tweaks::parse`):

- `all` (the default) is `imchx`. `none` is every lever off. Every other lever must be named.
- A **letter** turns its lever on. Order does not matter, with the exceptions below.
- A **digit** `G` in 2..9 sets a split one-hot with `G` groups (`v` = 2, `w` = 3) and implies the
  one-hot. The first digit in the string is used.
- **`q<b>`**, **`q<b>_1`**, **`q<b>_1.<c>`**: rank-scheduled delivery with budget `b`, plan mode
  (0 windows, 1 furthest-future) and Majorana budget `c`. The digits after `q` are the budget, not
  a group count. A `_` directly after the budget belongs to `q`, so lever `_` must be written
  before any `q` token.
- **Extension levers** start at the first `.` that is followed by a letter: `.p`, `.g`, `.e`,
  `.h<k>`, written together after one dot (`.eh4`). A `.` followed by a digit belongs to
  `q<b>_1.<c>`. So `q16_1.13.e` sets the Majorana budget 13 and lever `.e`
  (`tests_knee::combined_lever_syntax`).
- Implied levers: `c`, `x` and `d` imply `m`; `x` implies `c`; `L` implies `l`; `u`, `r`, `v`, `w`
  and a digit imply the one-hot; `M` implies `G`. `e` and `n` are read only with `G` or `M`, and
  `o` only with `C`.

### 6.2 Lever index

"Needs" lists the levers that must also be on; the section gives the mechanism and cost.

| Token | `Tweaks` field | Lever | Needs | Section |
| --- | --- | --- | --- | --- |
| `i` | `derive_id` | inner word stores only the sign; `id` derived from `b` | | 6.3 |
| `m` | `measured_undo` | alt swaps undone by measurement | | 6.3 |
| `c` | `compact_outer` | compact outer data (no `lo`) | `m` | 6.3 |
| `x` | `item_outer` | item layout of the outer word; inner lookup over the squares' range | `c` | 6.3 |
| `h` | `hot_erase` | fixups erase their one-hot by measurement | | 6.3 |
| `l` | `keep_ladder` | inner comparator holds its carries | | 6.3 |
| `L` | `outer_ladder` | outer comparator too | | 6.3 |
| `p` | `pad_share` | write-all-words inner read with shared padding words | `2^inner_a = 2^k_i` blocks | 6.4 |
| `g` | `gated` | outcome-gated comparator erasure | | 6.5 |
| `u` | `onehot` | one-hot RPREP | | 6.6 |
| `r` | `in_range` | one-hot written by an in-range iteration | (implies `u`) | 6.6 |
| `E` | `embed` | angle register inside the one-hot | unsplit one-hot; not `q`, `P` | 6.6 |
| `v`, `w`, digit | `hot_groups` | split one-hot, `G` groups | (implies `u`) | 6.7 |
| `z` | `paired_groups` | paired group corrections | split, `G >= 3` | 6.7 |
| `Z` | `shared_triple` | shared triple on the last three groups | `z`, even `G >= 4` | 6.7 |
| `Y` | `triple_sandwich` | the triple's product by a CNOT sandwich | `Z` | 6.7 |
| `f` | `fold` | folded write | `r`, split | 6.8 |
| `Q` | `early_flags` | folded class flags measured early | `f` | 6.8 |
| `C` | `class_inplace` | one-hot in aligned classes, written in place | `r` (split, or one group; with `E` only unsplit) | 6.8 |
| `A` | `class_mixed` | per-row one-body split depths | `C` | 6.8 |
| `o` | `class_pack` | classes packed by length | `C` | 6.8 |
| `.g` | `graft` | last class grafted into spare cells | `C`, split | 6.8 |
| `a` | `measured_unload` | angle register unloaded by measurement | one-hot | 6.9 |
| `U` | `hold_maj` | rotation 0's angle held across the Majorana | `a`, split | 6.9 |
| `.h<k>` | `maj_drop` | `k` bits of the held angle dropped at the Majorana | `U` | 6.9 |
| `s` | `sign_norm` | sign-normalised Householder vectors | one-hot | 6.9 |
| `P` | `pi_edge` | `pi` bit applied at the chain's edge | one-hot; not `E`, `q` | 6.9 |
| `b` | `checkpoint` | index checkpoint | `y`, `i`, `r`, one-hot, `SpinSwap` | 6.10 |
| `B` | `measured_ck` | checkpoint cleared by measurement | `b` | 6.10 |
| `J` | `early_id` | checkpoint's `id'` made before the RPREP write | `b` | 6.10 |
| `y` | `majorana_cut` | narrowed Majorana stage | `i`, `SpinSwap` | 6.11 |
| `N` | `spin_unload` | spin select released between the swaps | `SpinSwap` | 6.11 |
| `d` | `drop_alt` | outer unchosen slot measured early | `m` | 6.12 |
| `k` | `keep_release` | outer keep released, gated re-read | `d`, plain outer comparison | 6.12 |
| `W` | `outer_witness` | outer `lt` released, witness at UNPREPARE | `x`, `d`, `k` | 6.12 |
| `O` | `pos_from_hot` | `pos_e` read from the one-hot | `y`, `d`, one-hot | 6.12 |
| `X` | `excl_select` | outer read by the exclusive multiplexer | | 6.12 |
| `j` | `inner_release` | inner keep released, gated re-read | `m`, plain inner comparison; not `p` | 6.13 |
| `R` | `lt_release` | inner `lt` released and recomputed | `m`, plain inner comparison; not `j` | 6.13 |
| `S` | `sign_phase` | inner signs as a lane phase | `i`, `m`, `y`; not `j` | 6.13 |
| `t` | `sign_pass` | inner signs in the read's pass, control-rooted read | `H` or `G`, `i`, `m`, `y`; replaces `S` | 6.13 |
| `_` | `index_host` | unchosen inner index held in the index register | `m`, and `t` or `S`; not `j`, `D` | 6.13 |
| `H` | `item_hot` | inner read from a paired item one-hot | `x`, `m`; not `j`, `p` | 6.14 |
| `K` | `item_clear` | item fields cleared during the read | `H` | 6.14 |
| `V` | `item_align` | aligned item one-hot | `H` | 6.14 |
| `I` | `item_inplace` | item one-hot expanded in place, measured collapse | `V` | 6.14 |
| `T` | `item_groups4` | four-group item one-hot | `V`, `I` | 6.14 |
| `D` | `stash` | one-body bits stashed in the read's output | `H`, `K` | 6.14 |
| `-` | `item_fold` | folded, `i_0`-refined item one-hot | `H`, `V` | 6.14 |
| `+` | `pad_offset` | donor-padded inner tables, item-constant offset | `H`, `x`, `t` | 6.14 |
| `.d` | `sparse_high` | sparse high keep bit via exact Reiher alias rerounding | `+`, Reiher estimated mu8 | 6.14 |
| `.a` | `align_alias` | exact Li inner alias arrangement with a shared high alt bit over ten folded leaves | `+`, `H`, `V`, `-` | 6.14 |
| `.e` | `erase_pairs` | item one-hot erasure pairs sibling leaves | `H`, two-group item one-hot | 6.14 |
| `.c` | `factor_erase` | measured class and group flags corrected from row-index bits | `C`, four-by-four initial-row classes with one grafted tail row | 6.8 |
| `~` | `hot_keep_release` | inner keep released through the item one-hot | `H`, `m`, `_`, and `S` or `t`; not `R`, `l`, `p`, `D`, `j` | 6.14 |
| `G` | `paired_lookup` | inner read by the paired unary lookup | not `p`, `j` | 6.15 |
| `M` | `slot_host` | slot hosting | (implies `G`) | 6.15 |
| `e` | `field_release` | `hi`, `is_ob` released across the read | `G` | 6.15 |
| `n` | `slot_collapse` | slot one-hot by expansion and measured collapse | `G` | 6.15 |
| `F` | `paired_erase` | inner erasure as a paired phase lookup | `G` | 6.15 |
| `.p` | `pad_runs` | padding runs, pruned slot one-hot | `G`, `n`; not `+` | 6.15 |
| `q<b>` | `rank_hold`, `rank_mode`, `rank_park` | rank-scheduled angle delivery | split one-hot | 6.16 |

### 6.3 Base layout levers: `i`, `m`, `c`, `x`, `h`, `l`, `L`

| Letter | Lever | `reiher-sa-v1` | `li-sa-v1` |
| --- | --- | ---: | ---: |
| `i` | The inner word stores only the sign bit per item; `id = [b = B]` is an AND of `b`'s literals held around the Majorana (word 28 -> 26 / 31 -> 29 bits) | -56 | -118 |
| `m` | Alt swaps undone by measurement: `alt ^= main`, measure `main`, one classically conditioned `CZ(lt, alt_j)` per outcome, the rest into the lookup's fixup (0 Toffolis instead of `w`) | -34 | -38 |
| `c` (with `m`) | Compact outer data `q \| hi \| is_ob \| pos_e`: a one-body `q` is `ones \| r`, `b ^= is_ob AND q_low` gives the RPREP index (`k_i` Toffolis per copy, never undone: its phase is a `CZ(is_ob, q_j)` per outcome) | -25 | -30 |
| `x` (with `m c`) | Item layout: the own item's index is the bucket's (copied, not stored), the alt's is stored; the inner lookup is indexed by `item 2^k_i + i` over the squares' range only (`range.rs`) | -11 | -15 |
| `h` | Every phase fixup erases its one-hot register by measurement plus a small fixup over the low bits (inner erasure 262 -> 222, 397 -> 300 per copy) | -112 | -250 |
| `l` | The inner keep comparator holds its carry ladder through SELECT, so its erasure is free (+18 / +20 live qubits) | -36 | -40 |
| `L` | The outer comparator too (+36 / +39 qubits in all) | -54 | -59 |
| **`imchx`** (`all`) | | **9,784** | **14,339** |
| `imchxL` | | 9,730 | 14,280 |

(Static Toffolis per step on the rigorous twins, `tests::pinned_ledgers_toff`; every lever alone
against `sa-low2025`'s 10,020 / 14,788. `l` and `L` scale with the keep width, so their Toffoli
saving and qubit cost are smaller on the estimated specs.)

Measured, `./benchmark.sh`, K = 2^19:

| | `reiher-sa-est-v1` | `reiher-sa-v1` | `li-sa-v1` |
| --- | ---: | ---: | ---: |
| `all` (`imchx`) | 9,364 / 1,028 | **9,784** / **1,068** | **14,339** / **1,379** |
| `imchxL` | 9,340 / 1,044 | 9,730 / 1,104 | 14,280 / 1,418 |
| `imchxL`, `INNER_A = 5` | 9,312 / 1,044 | | |

`li-sa-est-v1`, static: `imchxL` at `INNER_A = 5` 13,433 / 1,349.

Against Low et al. on the rigorous twins (theirs quoted, ours measured, the rest derived):

| | Reiher theirs | Reiher `imchx` | Reiher `imchxL` | Li theirs | Li `imchx` | Li `imchxL` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Toffolis/step, harness charge | (9,355 derived) | 9,784 | 9,730 | (13,429 derived) | 14,339 | 14,280 |
| Toffolis/step, their charge | 10,203 | 10,632 | 10,578 | 14,629 | 15,539 | 15,480 |
| qubits without / with PE register | 1,132 / 1,163 | 1,068 / 1,099 | 1,104 / 1,135 | 1,454 / 1,487 | 1,379 / 1,412 | 1,418 / 1,451 |
| total at sigma_PEA 1.0 mHa, their lambda_eff | 342,453,492 | 328,390,176 | 326,577,720 | 1,003,139,788 | 983,253,908 | 979,208,160 |
| same, certified lambda_eff (31,402 / 68,503 steps) | | 307,237,168 | 305,541,460 | | 982,264,517 | 978,222,840 |
| same, their Givens charge, their lambda_eff | 342,453,492 | 356,852,448 | 355,039,992 | 1,003,139,788 | 1,065,540,308 | 1,061,494,560 |
| total at 1.6 mHa, their lambda_eff | 214,038,534 | 205,248,752 | 204,115,940 | 626,969,682 | 614,540,862 | 612,012,240 |
| product `C x Q` | 11,549,796 | 10,449,312 | 10,741,920 | 21,270,566 | 19,773,481 | 20,249,040 |

On the harness's Givens charge both twin circuits are below Low et al. on Toffolis per step,
qubits, total and product. At their `2 beta` per Givens every one costs more Toffolis per step and
in total (+4.2% / +3.7% Reiher, +6.2% / +5.8% Li); on the product at their charge only Reiher
`imchx` stays below (10,632 x 1,068 = 11,354,976 against 11,549,796). The excess is the keep width
of the rigorous rule. On the estimated class, `imchxL` at `INNER_A = 5` is 10,160 under their
charge (derived), below 10,203.

Alternatives that do not pay at this level (cost estimates unless stated):

- **Rounding-optimal alias counts to save a keep bit** (rigorous twins). A float model of the
  rounding error with a pairwise local search over each square's inner counts, a per-square optimal
  outer scale and a free `lambda_decl` reduces the error by at most 1%: largest-remainder rounding
  is already near-optimal for the product form. No keep bit can be dropped ((18, 19) stays at
  1.13e-4 Ha on Reiher; (19, 18) gives 1.37e-4 at best; (20, 20) on Li 1.26e-4).
- **Three spin swaps instead of four** (carry the spin frame across the `Reflect`): not buildable.
  The frame after copy 1 holds `s_a`, and whatever register tells copy 2 how to undo it still holds
  `s_a` after the step, where nothing can uncompute it (the inner register has been reflected).
- Merging the RPREP erasure into the inner lookup's fixup (its index would need `lt`, doubling the
  fixup: +30 / +70); a non-power-of-two inner block count; a flat RPREP iteration (visits the 105
  padded Li entries; the ragged one's overhead is about 50); larger QROAM block counts (the counts
  used are already Toffoli-optimal).

### 6.4 `p`: the write-all-words inner read with shared padding (`range.rs`)

Low et al. App. B Eq. (B38) iterates over the squares, writes all `B + 1` inner words and swaps one
out. Here the inner alias table has `2^k_i` buckets (32 on Reiher), and the 4 padding buckets carry
weight (keep 0, all lanes to their alt), so all 32 words are data: the QROAM with `2^k_i` blocks
(`FEMOCO_SA_INNER_A = k_i`) already is Eq. (B38) with `B' = 32`.

- **Construction.** The walk lays out each square's padding so that words repeat: buckets 28 and
  30 go to one owner item, and 29 and 31 to one owner (`PadPlan` d = 1). The pruned network (low
  bits first) then never swaps two equal words: blocks 30 and 31 are never allocated, and the swaps
  (30, 31) and (28, 30) are skipped.
- **Correctness.** The counts are unchanged (the same largest remainders), so the rounding error
  and the class's floor-or-ceiling check are unchanged; only the lane map's padding layout moves
  (`mod.rs::pad_plan_of`).
- **Cost.** `2 w` Toffolis and `2 w` qubits per read. Putting all four padding buckets on one
  owner (d = 0, `3 w`) needs an item holding 1/8 of a table's lanes; 7 of Reiher's 270 squares
  have none (largest-remainder counts at mu = 9), so d = 1.
- **Where it pays.** It needs `2^inner_a = 2^k_i` blocks, which pays only at the estimated class's
  word width (w = 16) on Reiher. Static on `reiher-sa-est-v1`: `imchxL` at `INNER_A = 5`
  9,312 / 1,044 -> 9,248 / 1,044 (-64 per step; the inner read is not at the peak). With the
  one-hot the inner read is at the peak, and the 32 freed temporaries lower it: `imchxlgr` at
  `INNER_A = 5` 9,270 / 714 -> `imchxlgrp` 9,206 / 682. On the rigorous twin (w = 26) and on Li it
  loses to one block level fewer.
- **Measured** (`reiher-sa-est-v1`, K = 2^19): `imchxlgrp`, `INNER_A = 5`: **9,206.005 / 682**
  (10,054.0 under their charge, -149.0 against 10,203).
- **Tests.** `range::tests` (every plan up to 2^7 blocks: the selected word, the swap count, table
  counts over 400 random tables); `tests::toff_pad_share_passes` (two exact specs, d = 0 and
  d = 1, both erase paths; each read saves exactly `saved x w`); `tests::pad_share_off_is_unchanged`.
- Not with `~`.

### 6.5 `g`: outcome-gated comparator erasure (`common::arith_gated`)

Every keep comparison that does not hold its ladder is erased by measuring `lt` in the X basis and
recomputing the lower carries only where the outcome is 1: expected `(mu - 1) / 2` Toffolis instead
of `mu - 1`, no qubits. Alone, `g` applies to the published step's comparisons (`inner.rs`,
`emit`). With `L` it changes nothing, because `L` holds both ladders. `C_step` becomes a sampled
mean. Tests: `tests_c1::gated_erasure_mutants_are_rejected`, `gated_var_bound`.

### 6.6 One-hot RPREP: `u`, `r`, `E`

**`u`.** The network index is written one-hot (`E = R B + N` qubits, one per network), and each
rotation's angle is fanned out by CNOTs into one angle register, instead of reading the whole
angle word (`onehot.rs`). The angle word (796 / 1,051 qubits on the rigorous twins) leaves the
peak. The fan-out writes the stored angle width (`tables::stored_angle`), so it is also correct on
a spec with tapered rotation widths (spec/SPEC-SA.md section 13).

**`r`** (implies `u`). The one-hot is written by an in-range iteration: left-only nodes pass their
control down, and the identity item reaches a real leaf, which is harmless around its identity
Majorana. `E - 1` Toffolis per write.

**`E`** (with the unsplit one-hot). The angle register lives inside the one-hot. For rotation `j`,
`w_j` one-hot qubits whose leaves' angles are independent over GF(2) are turned by CNOTs into the
angle bits (`onehot::Pivot`), the Givens reads them, and they are turned back. No separate angle
register; Cliffords only. Tests: `tests_c1::embed_*`.

Measured on the estimated class (K = 2^19, sampled means):

| spec | tweaks, `INNER_A` | C_step | Q_peak | their charge (derived) |
| --- | --- | ---: | ---: | ---: |
| reiher-sa-est-v1 | `imchxlgr`, 4 | 9,298.003 | 573 | 10,146.0 (-57.0 against 10,203) |
| reiher-sa-est-v1 | `imchxlgrp`, 5 | 9,206.005 | 682 | 10,054.0 (-149.0) |
| li-sa-est-v1 | `imchxgr`, 5 | 13,340.993 | 1,219 | 14,540.993 (-88.007 against 14,629) |

Static on `reiher-sa-est-v1`: `imchxgr` 9,306 / 565; `imchxlgrd` 9,358 / 558; `imchxlgr` at
`INNER_A = 3` 10,122 / 573; `imchxlgrdp` at `INNER_A = 5` 9,266 / 667.

**Anatomy of the peak with the one-hot on** (static, rigorous twins, `tests_combo::anatomy`: the
live ancillas at the first op that reaches `Q_peak`, grouped by the ledger stage that allocated
them; they equal the measured `Q_peak` wherever measured):

| stage that allocated the qubit | `reiher-sa-v1`, `imchxgr` | `li-sa-v1`, `imchxgr` |
| --- | ---: | ---: |
| outer alias QROAM read (keep, own, alt) | 40 | 43 |
| outer keep test, enables | 12 | 12 |
| inner alias QROAM read | **425** (the read's own transient, `2^4` blocks) | 29 |
| inner keep test | | 7 |
| RPREP: the one-hot register | (freed; not live) | **931** |
| SELECT: spin select, Majorana control, angle register | 1 | 1 + 1 + 15 |
| ancillas at the peak | 478 | 1,039 |
| `Q_peak` = base + ancillas + phase gradient | 657 = 163 + 478 + 16 | 1,265 = 211 + 1,039 + 15 |
| peak stage | inner QROAM read (second copy) | SELECT `V` |

Without the one-hot (`imchxL`), both instances peak in the second copy's RPREP angle read, where
the angle word is 804 of Reiher's 925 ancillas and 1,061 of Li's 1,192.

**Why the saving does not transfer to Li.**

- The one-hot is `E` qubits wide. On Reiher (324) it replaces a 796-bit angle word, and the peak
  falls to the next register down, the inner read. On Li (931) it replaces 1,051 bits, saves only
  120 qubits, and becomes the peak itself. Li has 2.9 times as many networks, mostly from `R B`
  (855 against 270); the angle word grows only with `N - 1` (75 against 53).
- **No Clifford-only register can do better** (`tests_combo::onehot_rank`). Every angle bit that a
  register feeds by CNOT fan-out is a linear function of that register, so the register needs at
  least the GF(2) affine rank of the `E x sum_j w_j` matrix of the leaves' angle words. That rank
  is 323 on Reiher and 930 on Li (plain ranks 324 and 931). The one-hot is within one qubit of
  that floor. With the angles loaded Toffoli-free, Li's peak cannot fall below the persistent
  registers plus about 930 qubits, before counting any PREPARE register. Getting under that costs
  Toffolis: the split one-hot (6.7) or rank-scheduled delivery (6.16).
- A second cap sits just below the one-hot on Li: the inner QROAM read at `INNER_A = 5` (937
  ancillas in its `2^5`-block transient on `li-sa-v1`). Any cut to the one-hot therefore also
  needs a narrower inner read.

### 6.7 Split one-hot: `v`, `w`, digit `G`, `z`, `Z`, `Y`

**`v` / `w` / digit `G`** (`onehot::Hot`). The leaves are split into `G` groups that share one
`ceil(E / G)`-qubit one-hot `h[e mod E1]` plus `G - 1` group bits.

- Rotation `j`'s angle is `L_j(h) ^ g D_j(h)`: group 0's fan-out, plus the difference between the
  groups at the same slot, gated by the group bit.
- Each transition costs one Toffoli `(g, s, reg[k])` per group bit and register bit whose
  difference changes; `s` is one scratch qubit holding the parity `(D_j ^ D_j')_k(h)` by CNOTs.
- The register stays loaded across the Majorana, so the correction is not paid twice there.
- Erasure: `h` and the group bits are measured, and one fixup over the same table cancels the
  phases. Its cost depends on the index range, not on the data.
- Cost: one Toffoli per angle bit per chain pass and extra group, 4 passes per step: +3,188
  (Reiher) / +4,208 (Li) per extra group (static). Qubits: the one-hot goes from `E` to
  `ceil(E / G) + G - 1`.

Where it pays: on `reiher-sa-v1` the inner QROAM read caps the peak at 435 for any `G` at
`INNER_A = 3`, so `INNER_A = 2` is needed; on the estimated specs the inner read is small and the
one-hot is the cap.

Static and measured points (K = 64 harness counts from `tests_c1::c1_lanes_pinned` in brackets):

| spec | tweaks | `INNER_A` | C_step | Q_peak | |
| --- | --- | ---: | ---: | ---: | --- |
| li-sa-est-v1 | `imchxgrv` | 4 | 18,145 | 761 | static |
| li-sa-est-v1 | `imchxgrvd` | 4 | **18,205.000** | **745** | measured, K = 2^19 (19,405.0 under their charge) |
| li-sa-est-v1 | `imchxgrwd` | 4 | 22,413 | 591 | static |
| li-sa-v1 | `imchxgr` (unsplit) | 5 | 14,205.5 | 1,265 | static |
| li-sa-v1 | `imchxgrvd` | 4 | **18,685.496** [18,684.2] | **791** [791] | measured, K = 2^19 (19,885.5 under their charge) |
| li-sa-v1 | `imchxgrwd` | 4 | 22,893.5 [22,890.2] | 740 [740] | static |
| li-sa-v1 | `imchxgrwd` | 3 | 24,709.5 | 637 | static |
| reiher-sa-v1 | `imchxgrv` | 4 | 12,899.0 | 657 | static: no gain, Reiher's peak is the inner read |
| reiher-sa-v1 | `imchxgrvd` | 3 | **13,623.020** | **435** | measured, K = 2^19 (14,471.0 under their charge) |

On `li-sa-v1`, `imchxgrvd` at `INNER_A = 4` dominates `sa-lowq` C = 2 (18,877 / 832): -191.5
Toffolis and -41 qubits; `sa-pareto` L3 (18,519 / 852) has fewer Toffolis. On `reiher-sa-v1`,
13,623.020 / 435 dominates `sa-pareto` R4 (14,473 / 474) and `sa-lowq` C = 4 (14,853 / 457). These
are qubit points, not Toffoli points: a split one-hot buys qubits at a fixed Toffoli rate.

The split is a two-level unary register with controlled corrections. No novelty is claimed for it,
and it was not checked against the literature.

**`z`: paired group corrections** (`G >= 3`; `onehot::transition_by`). Two group bits `a`, `c` are
never both 1, so `(a ^ D_c(h)) (c ^ D_a(h)) = a D_a ^ c D_c ^ D_a D_c`: one Toffoli corrects a
register bit for two groups (the pointwise product `D_a D_c` is a function of the one-hot, so a
CNOT fan-out). A split one-hot of `G` groups costs `ceil((G - 1) / 2)` Toffolis per bit and
transition instead of `G - 1`: `G = 3` at the price of `G = 2`. Parities are XORed into the group
bits in place.

The general principle, used by `z`, `Z`, `H`, `X`, `C` and `G`: bits that are never 1 two at a time
(one-hot slots, group bits, gated leaf flags, lane classes) turn products into Cliffords. One
Toffoli `(alpha.g ^ f)(beta.g ^ f')` gives every exclusive bit a coefficient from
`{0, f, f', 1 ^ f ^ f'}`, so `n` exclusive terms cost `n / 2` Toffolis per output bit, and the
measured erasure of such a product is a `CZ`.

**`Z`: shared triple** (with `z`, even `G >= 4`; `onehot::triple`). The odd group bit and the last
pair share their corrections over pairs of register bits: three Toffolis per two bits instead of
four, so `(G - 1) / 2` per bit (`G = 4`: 1.5 instead of 2). Static on `li-sa-est-v1` at G = 4:
-2,116 Toffolis per step.

**`Y`: triple sandwich** (with `Z`; `onehot.rs`). The shared triple's common product
goes into both register bits by a CNOT sandwich (`k2 ^= k1; k1 ^= v1; k2 ^= k1`), with no scratch
qubit: -1 qubit at G = 4.

Tests: `onehot::paired_tests` (exhaustive over every leaf: transitions, measured unload,
`shared_triple_is_exact_for_every_leaf`, `triple_sandwich_is_exact_for_every_leaf`),
`tests_c1::{paired_groups_*, paired_mutants_are_rejected, shared_triple_*,
triple_sandwich_passes_on_exact_specs}`. Group corrections skipped and group bits not written are
rejected by the harness (`tests_c1::onehot_mutants_are_rejected`).

### 6.8 The one-hot's write and layout: `f`, `Q`, `C`, `A`, `o`, `.g`

**`f`: folded write** (with `r` and a split one-hot; `onehot::write_folded`, `onehot::Fold`). Whole
square rows are grouped in classes of `G` rows (one per group) that share slots. The row iteration
sets a class flag and the group bit, and each class costs one iteration over `lo` (`B` leaves)
instead of one per row. Left-over rows and the one-body rows are placed leaf by leaf. Class flags
are X-measured after the write; their phase joins the one-hot's erasure fixup. Same width
`ceil(E / G)`.

**`Q`** (with `f`; `onehot::write_folded_early`). Each folded class flag is X-measured right after
its class's iteration: -2 qubits on Li, 0 Toffolis.

**`C`: classes in place** (with `r`; `onehot::write_classes`, `erase_classes`). The RPREP one-hot
in aligned classes of `G` (virtual) rows written in place: a row one-hot, a Clifford fold into
class flags and group bits, in-place `lo` expansions. It is erased exactly backwards by
measurement, so there is no fixup over the network index. Replaces `f`. Static on the estimated
specs: write 171 -> 124 and erase 43 -> 8 per copy on Reiher (+5 slots); on Li with virtual rows
-232 Toffolis and -2 qubits per step. **At one group** (no digit, with or without `E`): the
unsplit one-hot written in place and erased by the measured collapse (`onehot::write_classes_pre`
takes `E`'s register); RPREP erasure 43 -> 0 (Reiher), 76 -> 0 (Li) per copy.

**`A`** (with `C`; `onehot::ob_depths`). Each one-body row takes its own split depth, chosen jointly
for the fewest slots: on Li G = 4 the first one-body row stays whole and fills the empty group of
the square rows' partial class (247 -> 238 slots, +20 Toffolis).

**`o`: classes packed by length** (with `C`). Square rows are split by their top `lo` bits like the
one-body rows (`onehot::vrows_by`); the virtual rows of each split depth are grouped `G` at a time,
longest first (`onehot::class_plan`); the depths minimise `23 e1 + T_copy` (`onehot::best_pack`).
The identity lane of a split square row is routed as the write routes it, and its slot stands for
the checkpoint's leaf `L`. Li one-hot 247 / 178 / 178 -> 239 / 162 / 135 slots at G = 4 / 6 / 7
(G = 3, 5 are already tight); write plus erase costs `2 V + e1 - rows - 2 C` Toffolis per copy.
When both `o` and `A` are given, `o`'s layout takes precedence. On Reiher the item and inner-read
stages (315 qubits) bind at `G >= 4`, so `o` does not pay there.

Measured, `li-sa-est-v1` (fast engine, K = 524,288):

| bundle (`INNER_A` 3, `OUTER_A` 2) | 8 + 8 | 9 + 9 |
| --- | --- | --- |
| `imchxgrdky6zabCHVIXZJso` | **22,489.735 / 413** | 22,615.434 / 416 |
| `imchxgrdky7zabCHVIXJso` | **24,927.453 / 387** | 25,053.444 / 390 |
| `imchxgrdky4zabCHVIXZJso` | 18,547.330 / 488 | 18,673.539 / 491 |

Below about 387 qubits the inner alias read (383) is Li's limit with these levers.

**`.g`: graft** (with `C` and a split one-hot; `onehot::plan_grafts`). The last class of `C`'s
layout, when it is all sub-rows of a split one-body row, is grafted into the spare cells of the
whole-row classes: each sub-row joins a host class's flag and group bit, its lanes' `lo` is XORed
onto the host slots while the one-hot is live, and its flag is erased by measurement with a
Clifford fixup after the host class expands (recomputed by one Toffoli at the erasure). The
one-hot loses that class's slots.

**`.c`: factored class erasure** (with the four-group `C` and `.g` layout of Li).
After collapsing the slots, the first sixteen rows have four class flags from the high
two row bits and three group flags from the low two. They are gated by the indicator
for these initial rows; row 15 is one-body, while rows 0-14 are square.
X-measure the seven flags. The phase for four outcomes `m0..m3` is the algebraic normal
form `m0 + (m0 xor m1)x + (m0 xor m2)y + (m0 xor m1 xor m2 xor m3)xy`, multiplied
by the initial-row indicator. The constant and linear terms use Z/CZ; the quadratic
term uses one outcome-conditioned CCZ. The group flags use the same form with `m0=0`.
The three grafted subrows of the final one-body row then collapse through the existing
measured splits. This replaces the twelve Toffoli initial-row unfold of `erase_classes` with two CCZs
executed only when their independent outcome parities are set: 22 expected Toffolis
saved per walk step on the Li four-group point, with no change to the peak qubits.

Tests: `tests_c1::{folded_*, early_flags_mutants_are_rejected, class_inplace_*, unsplit_class_*,
class_pack_*, graft_*, factored_*}`; `onehot::tests_factor_erase` (all 17 row values and a
cubic-phase mutant); `tests_rot::{class_slots, pack_options}` (ignored; slot counts).

### 6.9 The angle register: `a`, `U`, `.h<k>`, `s`, `P`

**`a`: measured unload** (`onehot::unload_measured`). The angle register is X-measured; the phase
`(-1)^(m . A(g, h))` is all Clifford on the one-hot (`Z` on slots, `CZ` per group term, `CZ` of the
in-place paired parities). 0 Toffolis instead of a transition to zero. With a split one-hot the
register is then also unloaded across the Majorana and reloaded after it.

**`U`: hold across the Majorana** (with `a` and a split one-hot). The register keeps rotation 0's
angle across the Majorana (no measured unload and reload there: `V`'s first load would pay the
group corrections again). The unload after `V` stays measured.

**`.h<k>`** (with `U`). `k` of rotation 0's register bits (the cheapest to reload) are unloaded by
X measurement (Clifford fixup) before the Majorana and reloaded after it, so the Majorana stage
holds `w - k` angle qubits instead of `w`: a cap on `U`'s held register at the stage where it sets
the peak. Not with `q` (it needs `U` holding rotation 0).

**`s`: sign-normalised Householder vectors** (`signnorm.rs`). Each leaf is delivered as the network
of `+u` or `-u`: `a_j -> 2^(beta-1) - a_j` for `j < N - 2` and `a_(N-2) -> a_(N-2) + 2^(beta-1)`
is exactly `-u` on the same grid. The leaves whose last angle is at least pi are flipped. Squares
apply `1 - 2 n(u)`, and one-body lanes get the same network in both copies, so the operator is
unchanged. On both estimated specs every angle then lies below pi, and the shared angle register
is `beta - 1` qubits: -1 qubit on the SELECT plateau; -12 (Li, G = 5) / -6 (Reiher, G = 3)
Toffolis per step (the last position's top-bit corrections); 0 Toffolis on the unsplit one-hot.

Measured with `s` (fast engine, K = 524,288), C / Q per step:

| spec | bundle (`+ s`), keep, `INNER_A` | without `s` (static) | with `s` |
| --- | --- | --- | --- |
| li-sa-est-v1 | `imchxgrdky5zabCHVIXJ`, 8 + 8, 3 | 20,379.5 / 437 | **20,367.509 / 436** |
| li-sa-est-v1 | same, 9 + 9, 3 | 20,505.5 / 440 | **20,493.518 / 439** |
| li-sa-est-v1 | `imchxgrdky4zabCHVIXZJ`, 8 + 8, 3 | 18,387.5 / 497 | **18,375.692 / 496** |
| li-sa-est-v1 | same, 9 + 9, 3 | 18,513.5 / 500 | **18,501.579 / 499** |
| li-sa-est-v1 | `imchxgr`, 9 + 9, 5 | 13,341.0 / 1,219 | **13,341.001 / 1,218** |
| reiher-sa-est-v1 | `imchxgrdky3zabCHKVIXD`, 8 + 8, 3 | 11,690.5 / 317 | **11,684.539 / 316** |
| reiher-sa-est-v1 | same, 9 + 9, 3 | 11,756.5 / 320 | **11,750.297 / 319** |
| reiher-sa-est-v1 | `imchxgr`, 9 + 9, 4 | 9,306.0 / 565 | **9,305.991 / 564** |

Calibration at the Li G = 4 corner: `FEMOCO_SA_OUTER_A=3` cuts 14 Toffolis (its eight-block outer
read peaks at 457 qubits, below that point's plateau): `imchxgrdky4zabCHVIXZJs` **18,361.459 / 496**
(8 + 8) and **18,491.664 / 499** (9 + 9).

**`P`: `pi` bit at the chain's edge** (with the one-hot; `toff::pi_edge_set`). A rotation whose
angles use the `pi` bit (`theta = 2 pi a / 2^beta`, bit `beta - 1`) and whose modes no later
rotation touches gets its angle without that bit: `G(theta + pi) = G(theta) Z_p Z_q`, and
`Z_p Z_q` commutes with its own Givens and with every later one, so it is applied at the chain's
edge (before `V^dagger`, after `V`), where the angle register is empty. The bit is loaded into a
scratch qubit from the one-hot, `CZ`s go to both modes of both spins, and the scratch is erased by
measurement. Static on `li-sa-est-v1`: -1 qubit, -4 Toffolis. On chain specs `s` dominates it.

Other rotation-side ideas (zero-angle skipping, conditional rotations, sharing between the copies,
cheaper Givens, rank-shared corrections) were evaluated and do not pay on these specs;
`tests_rot::transition_rank` (ignored) prints the rank-sharing measurement.

Tests: `signnorm::tests` (exhaustive at N = 3, beta = 8; random chains to N = 76),
`tests_c1::{sign_norm_*, pi_edge_*, hold_majorana_passes_on_exact_specs,
measured_unload_*, graft_and_cap_letters_parse}`.

### 6.10 Index checkpoint: `b`, `B`, `J`

**`b`** (with `y`, `i`, `r` and the one-hot). `b`, `hi` and `is_ob` are XORed to zero from the
one-hot during `V^dagger` and `V` and rebuilt after (`is_ob` also around the Majorana);
`id' = [b = B] AND NOT is_ob` is held instead. The clearing uses `onehot::transition_by` with the
leaf fields as a one-rotation table, so it inherits `z` and `f`.

**`B`** (with `b`). The checkpoint's clearing pass is an X measurement with a Clifford fixup on the
one-hot (`onehot::unload_measured`) instead of a fan-out transition to zero; the rebuild is
unchanged. 0 qubits, -22 Toffolis per step (static).

**`J`** (with `b`). The checkpoint's `id'` is made before the RPREP write and erased after
RPREP^dagger, so its AND chains are never live with the one-hot: -1 qubit on Li.

Tests: `tests_c1::{early_id_passes_on_exact_specs, measured_unload_*}`.

### 6.11 Majorana stage and spin select: `y`, `N`

**`y`: narrowed Majorana stage** (with `i` and `SpinSwap`). `en_ob`, `en_sq` and `pass` are not
held across the step: each enable is made and erased around its half of the Majorana, `pass` is a
classical bit toggled at the end of each copy (the two copies are the same ops, so it reads 0,
then 1), and `id` keeps no AND chain (it is erased by an X measurement gating a recompute of its
phase). This is the `sa-toff` form of narrow gadgets `e`, `p`, `j` (5.4).

**`N`** (with the `SpinSwap` SELECT). The spin-select qubit is erased by measurement right after
`F^-s` and made again right before `F^s`: -1 qubit through `V` and the Majorana, +2 Toffolis per
step.

### 6.12 Outer PREPARE: `d`, `k`, `W`, `O`, `X`

**`d`: drop the unchosen slot** (implies `m`). The outer alias read's unchosen slot is measured
right after the swap (as `D = own ^ alt`, a function of the bucket) instead of at the end, and the
part of the chosen slot's phase that depends on the keep test is cancelled by one fixup controlled
by it (`qroam::phase_fixup_ctl`, about `E(2^k_o) + 2` Toffolis): `kx + h + 2` fewer qubits through
the whole nested block. Static: -16 qubits for +60 Toffolis per step on Li; -15 for +60 on
`reiher-sa-est-v1`. Tests: `tests_c1::drop_alt_mutants_are_rejected`.

**`k`: outer keep release** (with `d`). The outer keep register is measured right after the keep
comparison instead of being held through the nested block, and the comparison is erased by the
outcome-gated re-read (`narrow::gated_lt_erase`, `2^outer_a` blocks; 5.4), whose outcomes join the
outer read's final fixup. `mu_o` fewer qubits across SELECT (-19 on `reiher-sa-v1`, -9 at 9 + 9)
for about half an outer read (about +92 / +77 Toffolis per step).

**`W`: outer witness** (with `x`, `d`, `k`). The outer keep test `lt` is X-measured right after
the outer swap. At UNPREPARE a witness `[item = x_o]` (an AND of the item index's literals against
the bucket, `k_o - 1` Toffolis) stands in for it: it equals `lt` except on self-aliased buckets
(`alt = own`, keep 0), where every `lt`-controlled term is 0. `Z` on the witness cancels the early
outcome, `k`'s gated re-read erases it, and the self-bucket remainder of both outcomes joins the
outer fixup. -1 qubit everywhere, +8 Toffolis. Tests: `tests_c1::outer_witness_self_buckets`,
`tests_combo::self_buckets`.

**`O`** (with `y`, `d` and the one-hot). The outer word's `pos_e` slot is X-measured right after the
outer swap (its outcome is the one the final outer fixup uses), and each copy's Majorana reads
`pos_e` from a qubit loaded from the one-hot (`onehot::transition_by`) and unloaded by measurement
(a Clifford fixup). -1 qubit everywhere, +2 Toffolis.

**`X`: exclusive multiplexer** (`excl.rs`, `qroam::load_excl`). The outer alias QROAM read selects
its block with the exclusive multiplexer instead of the swap network: `(lambda - 1) w / 2`
Toffolis instead of `(lambda - 1) w`; the low one-hot is written in place and erased by
measurement. Outer read 213 -> 172 Toffolis (static). Tests: `excl::tests` (exhaustive),
`tests_c1::{excl_select_*, excl_bundles_pass, excl_mutants_rejected}`.

### 6.13 Inner PREPARE state: `j`, `R`, `S`, `t`, `_`

**`j`: inner keep release** (with `m`). In each copy the inner keep register is measured right
after the inner comparison, and the comparison is erased at PREPARE^dagger by a gated re-read of
`keep` over the lookup's own index range (`narrow::gated_lt_erase_from`, `2^inner_a` blocks;
`load_from` uses `range::iterate_range`'s node rule with pooled ANDs). `mu_i` fewer qubits across
SELECT (-19 on the rigorous twin, -9 at 9 + 9) for about +1,100 to +1,230 Toffolis per step (the
re-read iterates the whole inner range).

Static (`tests_combo::combo_scan`), `imchxgrd` plus letters:

| spec | letters | `INNER_A` / `OUTER_A` | C_step | Q_peak |
| --- | --- | --- | ---: | ---: |
| reiher-sa-est-v1 | `k` | 4 / 2 | 9,442.5 | 541 |
| reiher-sa-est-v1 | `k4` | 2 / 1 | 21,985.5 | 306 |
| li-sa-est-v1 | `k6` | 3 / 1 | 37,240.5 | 430 |
| reiher-sa-v1 | `k3` | 2 / 2 | 18,854.5 | 362 |
| reiher-sa-v1 | `kj3` | 2 / 1 | 21,171.5 | 343 |
| reiher-sa-v1 | `kj4` | 2 / 1 | 24,359.5 | 317 |
| li-sa-v1 | `k5` | 3 / 1 | 33,304.5 | 495 |

Tests: `narrow::tests::gated_reread_from_is_exact`,
`tests_c1::{recombined_levers_pass_on_exact_specs, keep_release_mutants_are_rejected,
recombine_letters_parse}`. The inner-keep mutants use the wide N = 4 exact spec: on the dyadic
N = 3 / N = 5 exact specs every inner keep is 0 and those mutants would pass.

**`R`** (with `m` and the plain inner comparison). The inner keep test `lt` is X-measured right
after the swap and recomputed from the held keep at PREPARE^dagger, where `Z` on the recomputed bit
(if the first outcome was 1) cancels the first measurement's phase. One comparison per copy for one
qubit through SELECT: -1 qubit, +16 Toffolis.

**`S`: inner signs as a lane phase** (with `i`, `m`, `y`). The square sign is a lane phase, so
`(-1)^(control AND NOT is_ob AND s_chosen)` is applied right after the keep test, as
`CZ(t, s_alt) CZ(t AND lt, s_own ^ s_alt)` with `t = control AND NOT is_ob` (two ANDs, erased by
measurement), and both sign slots are X-measured at once (their contents are functions of the
lookup index and join its fixup). The alt swap then moves `b` alone; the Majorana applies no sign.
-2 qubits (SELECT, write), +2 Toffolis.

**`t`: signs in the read's pass** (with `i`, `m`, `y` and `H` or `G`; in place of `S`). The inner
read's iteration is rooted at the walk control (`itemhot::read_into_ext`), so everything it reads
is 0 on control-0 lanes (as on one-body lanes, whose item one-hot is empty) and every phase it
applies is controlled for free. In the same pass `(-1)^(s_alt(item, i))` is applied as a `CZ` of
the leaf's gated parities (Cliffords), and the word stores `delta = s_own ^ s_alt` (0 where
`keep = 0`) in place of the two flags: `CZ(lt, delta)` after the keep test gives
`(-1)^(lt delta)`, so the chosen item's sign `s_alt ^ lt delta` is applied, and `delta` is
X-measured at once (its content joins the erasure's fixup, also rooted at the control). One word
bit fewer per inner index. With the paired lookup (`G` / `M`, with `F`) the same is done by
`paired::load_paired_ext` and the rooted paired phase erasure `phase_paired_rooted`: Li's lowest
C under 500 qubits moves from 17,927.5 / 493 to 17,855.5 / 492 (9 + 9).

**`_`: unchosen index hosted in the index register** (with `m`, and `t` or `S`; not `j`, `D`).
`i = lt ? chosen : unchosen`, and the chosen index is held (`bsel`, the RPREP one-hot), so
`i ^= lt (chosen ^ unchosen)` puts the unchosen index in the inner index register and frees the
`k_i`-qubit alt slot through RPREP and SELECT. It is undone at PREPARE^dagger (`toff::index_swap`).
`2 k_i` Toffolis per copy (24 per step on Li) for `k_i` qubits off every SELECT-plateau stage.

Tests: `tests_c1::{measured_unload_*, sign_pass_*, paired_sign_pass_*, index_swap_is_exact,
index_host_*}`.

### 6.14 Item one-hot inner read: `H`, `K`, `V`, `I`, `T`, `D`, `-`, `+`, `.e`, `~`

**`H`: paired item read** (with `x`, `m`; `itemhot.rs`). The outer item is written in a `G = 2`
split item one-hot (`ceil(R C / 2) + 1` qubits), present at each copy's start and end and erased
during SELECT. The inner word is read by iterating over `i` only: `u1 = f AND g`, `u0 = f ^ u1`,
one Toffoli `(u0 ^ L1)(u1 ^ L0)` per word bit, the `L0 L1` leftovers cancelled once. Erasure: the
same product as a `CZ` (one Toffoli per `i`). `n_i (w + 1)` Toffolis per read and no QROAM
transient beyond the item one-hot.

**`K`** (with `H`). The outer item index, `hi` and `is_ob` are XORed to zero from the item one-hot
during the inner read and rebuilt after it.

**`V`: aligned item one-hot** (with `H`; `ItemHot::alloc_aligned`). Item `v` sits at slot `v >> 1`
and its group is the item's low bit, so there is no group register and each write iterates over
about `R C / 2` slots: item write 277 -> 142; -453 / -426 Toffolis per step (static, Reiher / Li
est).

**`I`** (with `V`). The aligned item one-hot is written by an in-place expansion of its slot index
rooted at `NOT is_ob` (one AND per split) and erased by the measured collapse (each AND erased by
measurement, its phase a `CZ` of two qubits still present): no phase fixup over the item index. A
level-order collapse rebuilds the cleared index bits. Erase 55 -> 0 per use; -150 Toffolis, -2
qubits.

**`T`: four groups** (with `V`, `I`; `ItemHot::alloc_aligned_groups`). The aligned item one-hot in
four groups on the item's two low bits: about half the slots for two Toffolis per read bit instead
of one.

**`D`: stash** (with `H`, `K`). During the inner read the one-body lanes' low item bits and `pos_e`
are stashed in the read's output register, which is 0 on those lanes. Reiher item stage 321 -> 315
qubits for +12 Toffolis.

**`-`: folded item read** (with `H`, `V`; `itemfold.rs`). The inner read from a folded,
`i_0`-refined item one-hot. The virtual group `(g, i_0) = (1, 1)` moves into `B = ceil(e1 / 3)`
fresh slots, so the read sees three exclusive groups over `e1 + B` slots and iterates over
`i >> 1`. Folded lanes keep both hot bits: every product is a paired product of linear slot
functions, and the cross left-over between the two hot bits costs one Toffoli per output bit
(1.5 Toffolis per word bit per leaf over half the leaves, as `Z`). On Li: read 1,071 -> 876
Toffolis per copy for +48 qubits during the read only. It fits under G = 5's SELECT plateau (read
stage 432 against 433) and G = 4's (432 against 483).

**`+`: donor padding and the item-constant offset** (with `H`, `x`, `t`; `padalias.rs`). Every
padding bucket of a square's inner table aliases to the item's largest inner item (the same
counts), so the padding word `A(item)` is a function of the item alone. The read and its erasure
subtract `A`, skip the then-zero padding rows and add `A` back under the control from the one-hot
(`itemhot::fan_rooted`). The in-pass sign's item offset is left uncorrected: both copies apply it,
so it cancels. -24 Toffolis per step on Reiher, -46 on Li item one-hot points; same Q. Not with
`.p`.

**`.d`: sparse high keep bit** (with `+`, Reiher estimated `mu_i = 8`;
`sparsealias.rs`). The estimated rounding rule allows each inner count to be the exact floor or
ceiling of its ideal. A pinned, exact alias arrangement preserves the parent counts in 268 of
270 square tables; two tables each move one lane from one item to another. The four padding
donors remain as under `+`. At twelve common own bucket indices, all tables' keep high bits are
zero, so the paired item one-hot read skips one Toffoli per index and copy: `−24 C_step` at the
same 313-qubit peak on the `kc-r3re-m8` bundle. The pinned test checks exact rounding and a
two-lane mutant, and `tests_c1::sparse_reiher_exact_and_mutant` checks the complete circuit.

**`.a`: aligned inner alias bit** (with `+`, `H`, `V`, `-`; `padalias.rs`). For Li's
58-item square tables, a deterministic Vose search keeps every item's exact lane count and
the top three donor padding rows, while matching alt bit 5 to the top donor on inner rows
18 through 37. The folded read then removes that bit's support on ten leaves. At the Li
8 + 8, 3 + 3 point, the inner read changes from 845 to 831 Toffolis per copy; static
`C_step` changes from 17,755.5 to 17,727.5 and `Q_peak` remains 471. Other table shapes use
the ordinary `+` arrangement.

**`.e`: paired erasure** (with `H` and a two-group item one-hot;
`itemhot::phase_rooted_paired`). The item one-hot erasure's phase pass pairs sibling leaves of the
inner iteration: one `CCZ` per pair for the group terms instead of one AND per leaf, the quadratic
left-over accumulated in classical bits and applied once. -30 Toffolis per step at the same Q on
the Reiher ladders. Not on `T`'s four-group one-hot.

**`~`: inner keep released through the item one-hot** (with `H`, `m`, `_`, and `S` or `t`; not
`R`, `l`, `p`, `D`, `j`). The inner keep register is X-measured right after the alias swap (its
outcomes join the erasure pass), so it is not held through RPREP and SELECT. At PREPARE^dagger
`lt` is X-measured; only on outcome 1 the keep of `(item, i)` is read again from the rewritten item
one-hot, `(-1)^[draw < keep]` is applied, and the re-read is measured into the erasure pass
(`itemhot::gated_lt_erase_hot`). This is lever `j`'s outcome-gated re-read on the item one-hot.
Two changes keep the gated block inside the PREPARE side's peak:

- the comparator is made in place (`narrow::lt_phase_inplace`: Cuccaro et al.'s majority chain, one
  scratch qubit, `2 (mu - 1)` Toffolis);
- the re-read folds the high group bits into the iteration (`itemhot::read_pooled`): the same
  Toffolis as `read_groups`, with one pair control live per leaf.

Cost: `mu_i` qubits off the SELECT plateau for about half a read per copy (Reiher G = 5 at Q
256 / 253: +623 / +566 Toffolis per step). A shortcut that replaces the re-read by a witness does
not exist here: after `lt` is measured the chosen index still carries `lt`, so some carrier's
erasure always needs `keep(x)`.

Measured with `~` (fast engine, K = 524,288; K = 64 by the reference evaluator):

| spec | bundle, `INNER_A` / `OUTER_A` | 9 + 9 C / Q | 8 + 8 C / Q | against the best point without `~` |
| --- | --- | --- | --- | --- |
| reiher-sa-est-v1 | `imchxgrdky5zabfHKVITXSNOBWs_~`, 2 / 1 | **16,820.173 / 256** | **16,642.580 / 253** | 19,417.332 / 256 and 19,294.377 / 253: -13.4% / -13.7% at equal Q; the same family at G = 6 with `_` (17,993.431 / 256, 17,870.363 / 253): -6.5% / -6.9% |
| reiher-sa-est-v1 | `imchxgrdky4zZabfHKVITXSNOBWs_~`, 2 / 1 | 15,429.471 / 270 | 15,252.726 / 268 | |
| reiher-sa-est-v1 | `imchxgrdky3zabCHKVITXSNOBWs_~`, 2 / 2 | 13,402.508 / 300 | 13,223.304 / 298 | |
| reiher-sa-est-v1 | `imchxgrdky5zabfHKVITXSNOBWUs_~`, 2 / 1 | | 16,584.080 / 256 | |
| li-sa-est-v1 | `imchxgrdky6zabCHVIXZJNOBWsot+_~`, 3 / 2 | 23,090.800 / 396 | 22,903.997 / 394 | |
| li-sa-est-v1 | `imchxgrdky7zabCHVIXJNOBWsot+_~`, 3 / 2 | 25,509.317 / 384 | 25,322.665 / 381 | below the earlier lowest Q (390 / 387) |

`~` composed with `.e` / `.h<k>` and with `q<b>_1.<c>` (the item one-hot erasure pass takes `~`'s
re-read part together with `.e`'s pairing; `.e` on two-group item one-hots only; with `~`, `q`
needs the Majorana budget `.c`, without it `q` is refused for `b <= 12` at G = 5):

| spec | bundle, `INNER_A` / `OUTER_A` | 9 + 9 C / Q | 8 + 8 C / Q | against |
| --- | --- | --- | --- | --- |
| reiher-sa-est-v1 | `imchxgrdky5zabfHKVITXSNOBWs_~q16_1.13`, 2 / 1 | **16,704.888 / 256** | | 19,417.332 / 256: -13.97% |
| reiher-sa-est-v1 | `imchxgrdky5zabfHKVITXSNOBWUs_~q15_1.12`, 2 / 1 | | **16,553.890 / 253** | 19,294.377 / 253: -14.20% |
| reiher-sa-est-v1 | `imchxgrdky4zZabfHVITXSNOBWUs_~q15_1.12`, 2 / 1 | 15,317.819 / 270 | 15,140.515 / 268 | |
| li-sa-est-v1 | `imchxgrdky7zabCHKVIXJOBWsotU+_~.e`, 3 / 2 | 25,367.595 / 377 | 25,180.568 / 374 | lowest Q measured on Li |
| li-sa-est-v1 | `imchxgrdky6zabCHKVIXZJNOBWsotU+_~Y.eh4`, 3 / 2 | 22,986.093 / 395 | 22,798.500 / 393 | dominates 24,761.357 / 400 (-7.2%) |
| li-sa-est-v1 | `imchxgrdky5zabCHKVIXJtNOBWPsU+_~.eh4`, 3 / 2 | 20,883.234 / 419 | 20,696.657 / 417 | dominates 22,515.523 / 420 (-7.3%) |

`tools/server/submit.sh` accepts knob values matching `[A-Za-z0-9._,+-]*` only, so a bundle with
`~` cannot be submitted through it unchanged.

Tests: `itemhot::tests` (exhaustive over every item and inner index, all layouts),
`itemhot::hot_keep_tests` (the gated block, exhaustive over every item, inner index, draw and root
value on the three one-hot layouts; the in-place comparator exhaustive for n = 1..4),
`itemfold::tests` (exhaustive over every item in and out of range, every inner index, root and
seed), `padalias::tests`, `tests_prep::{offset_read_and_erasure_are_exact,
offset_mutants_are_caught}`, and in `tests_c1`: `item_hot_*`, `aligned_item_hot_*`,
`inplace_item_hot_*`, `item_groups4_passes_on_exact_specs`, `stash_passes_on_exact_specs`,
`item_fold_*`, `pad_offset_*`, `erase_pairs_*`, `park_with_erase_pairs_*`, `hot_keep_*` (end to end
on exact specs including the wide N = 4 one, with mutants rejected by the harness; `~` is refused
with `R`, `D`, `j` or without `_`). `tests_c1::hot_keep_self_buckets_keep_zero` (ignored) checks
that every self-aliased inner bucket of both pinned specs keeps 0.

### 6.15 Paired unary lookup inner read: `G`, `M`, `e`, `n`, `F`, `.p`

**`G`: paired unary lookup** (`paired.rs`). The inner alias word is read by a paired unary lookup
instead of clean QROAM or the item one-hot: the low `inner_a` index bits (inner bucket, then low
item bits) are written one-hot, a unary iteration over the rest has its sibling flags paired (one
Toffoli per pair of groups and word bit), and the one-hot is measured. With `G`,
`FEMOCO_SA_INNER_A` is the slot width `s` (`2^s` slots), not a block count. Read stage:
`2^s + w + ladder` qubits. On Li G = 4 at `s = 8`: -358 Toffolis per step against `H` (read 956
per copy, no item one-hot).

| Letter | Lever | Qubits | Toffolis / step (Li G = 4, static) |
| --- | --- | ---: | ---: |
| `M` (implies `G`) | Slot hosting: each low index bit, a parity of the slot one-hot, is XORed to zero and its qubit carries one slot (`Swap`, fresh slot freed); undone before the one-hot leaves | `-s` (-8) | 0 |
| `e` (with `G`) | `hi`, `is_ob` X-measured before the read and reloaded from the item index after it (a small paired lookup, `toff::reload_fields`), `Z` on each reloaded bit whose outcome was 1 | -6 (`h + 1`) | +146 |
| `n` (with `G`) | The slot one-hot by in-place split expansion and measured collapse (Cliffords only; no slot fixup) | 0 | -80 |
| `F` (with `G`) | The inner erasure as a paired phase lookup (`paired::phase_paired`: split expansion of the low index bits, a range iteration over the rest with conditioned `CZ`s per leaf, measured collapse) instead of the QROAM fixup | 0 | -50 |

**`.p`: padding runs** (with `G` and `n`; `padalias::arrange_runs`). Every square inner table's
padding buckets are fed in aligned subcubes, each by one donor item (the same counts), so the
paired read's slot one-hot is a pruned expansion in which each fed subcube is one slot
(`paired::load_paired_pruned`): fewer slots on the read stage. Not with `+` (the two levers arrange
the padding differently; `lane_map` asserts it).

Measured, `li-sa-est-v1`, `INNER_A=8 OUTER_A=3` (fast engine, K = 524,288):

| bundle | keep | C_step / Q_peak |
| --- | --- | --- |
| `imchxgrdky4zabCAXZJSRNOBWMenF` | 8 + 8 | 18,055.421 / 482 |
| same | 9 + 9 | 18,145.488 / 485 |
| `imchxgrdky4zabCAXZJOBWMnF` | 8 + 8 | 17,903.475 / 488 |
| same | 9 + 9 | 17,991.650 / 491 |
| `imchxgrdy4zabCAXZJOBMnF` | 8 + 8 | 17,836.496 / 497 |

On Reiher these levers do not pay: the outer read at `OUTER_A=3` and the 256-slot read stage
exceed the 317-qubit front.

Tests: `paired::tests` (exhaustive gadget tests with hosting and collapse), the paired-lookup
bundle and mutant tests in `tests_c1`, `tests_c1::pad_runs_*`.

### 6.16 Rank-scheduled angle delivery: `q<b>`, `q<b>_1`, `q<b>_1.<c>` (`rankdel.rs`)

Used in place of `a`'s and `U`'s handling of the register.

**Mechanism.** With a split one-hot live in SELECT, the one-hot's slot and group functions are a
free base `B0`. Modulo `B0` the stored words span `D'` dimensions (Reiher G = 3: 210; Li G = 3:
618; Li G = 4: 693), and any one of those directions costs one Toffoli (lever `z`'s pair product
`(g_1 ^ X(h))(g_2 ^ Y(h))`). A direction held live is reused by CNOTs, and one dropped is measured
out with Clifford fixups on the base. Delivery is then a schedule of held subspaces `X_t`
(`dim <= b`, `X_t ⊇` rotation `t`'s words mod `B0`) over the copy's sequence `N-2 .. 0 | 0 .. N-2`,
paying `dim X_t - dim(X_(t-1) ∩ X_t)` per step.

- `q<b>` (mode 0): **windows**. A dynamic programme over contiguous windows, each holding its
  rotations' span; a switch keeps the intersection. Pays only once `b > D' / 2`.
- `q<b>_1` (mode 1): **furthest-future keep** (Belady's rule for subspaces, drop before buy;
  `rankdel::plan_belady`). Before each rotation keep `X_(t-1) ∩ (N_t + S_(t+1..t+j))` for the
  largest `j` that fits, buy the rest of `N_t`. Each step eliminates the pool's images in
  `X_(t-1) / K` by CNOTs and measures the pivots, then places every register bit (bought, or
  gathered by CNOTs, plus a base fix) and swaps the rows onto the register. In the delivery model
  it is within 86 Toffolis per copy of the model's segmented lower bound at every budget from 45
  to 195 on Reiher (`tests_rank::rank_plan_table`).
- `q<b>_1.<c>` (mode 1 only): a second budget at the gap between `V^dagger` and `V`. Before the
  Majorana the held span is cut to at most `c` dimensions by the furthest-future rule, every other
  row is measured out (Clifford fixups, 0 Toffolis), and the plan continues from the cut span
  (`rankdel::belady`). The Majorana stage then holds at most `c` delivery qubits instead of the
  register plus the held span.
- **Any `G >= 2`** (mode 1). A bought direction has `G - 1` group components; they are bought by
  pair products on any two group bits (`ceil(nonzero / 2)` Toffolis), and with `G - 1` odd two
  directions bought at the same step share lever `Z`'s triple on the last three groups
  (`(G - 1) / 2` per direction).

Exactness domain: every one-hot state the RPREP write can leave (`rankdel::reachable` simulates
the write over the valid index values; identity lanes of a split class reach states without a
leaf).

**Reiher** (`reiher-sa-est-v1`, `imchxgrdky3zabCHKVIDXtNBWs` + `q<b>_1`, `INNER_A` / `OUTER_A`
2 / 2 for `b <= 90`, 3 / 3 above; fast engine, K = 524,288). Floor: the segmented floor of
section 7 with `SpinSwap` and reflections at the same Q. Delivery: RPREP write, both chain passes
and the erasure, per copy (static ledger).

| b | 9 + 9: C / Q | floor | C / floor | delivery / copy | floor's delivery term | 8 + 8: C / Q | C / floor |
| ---: | --- | ---: | ---: | ---: | ---: | --- | ---: |
| 15 | 11,638.2 / 318 | 7,425 | 1.57 | 1,715 | 640 | 11,572.4 / 315 | 1.56 |
| 20 | 11,552.5 / 323 | 7,395 | 1.56 | 1,672 | 625 | 11,486.5 / 320 | 1.55 |
| 30 | 11,412.5 / 333 | 7,335 | 1.56 | 1,602 | 595 | 11,346.5 / 330 | 1.54 |
| 45 | 11,202.5 / 348 | 7,245 | 1.55 | 1,497 | 550 | 11,136.5 / 345 | 1.53 |
| 60 | 10,992.3 / 363 | 7,155 | 1.54 | 1,392 | 505 | 10,926.3 / 360 | 1.52 |
| 75 | 10,782.7 / 378 | 7,065 | 1.53 | 1,287 | 460 | 10,716.7 / 375 | 1.51 |
| 90 | 10,572.3 / 393 | 6,975 | 1.52 | 1,182 | 415 | 10,506.4 / 390 | 1.50 |
| 105 | 10,348.5 / 408 | 6,885 | 1.50 | 1,077 | 370 | 10,278.5 / 405 | 1.49 |
| 120 | 10,138.6 / 423 | 6,795 | 1.49 | 972 | 325 | 10,068.5 / 420 | 1.48 |
| 135 | 9,928.6 / 438 | 6,705 | 1.48 | 867 | 280 | 9,858.6 / 435 | 1.47 |
| 150 | 9,718.4 / 453 | 6,663 | 1.46 | 762 | 259 | 9,648.4 / 450 | 1.45 |
| 165 | 9,508.5 / 468 | 6,663 | 1.43 | 657 | 259 | 9,438.6 / 465 | 1.42 |
| 180 | 9,298.4 / 483 | 6,663 | 1.40 | 552 | 259 | 9,228.5 / 480 | 1.39 |
| 195 | 9,088.4 / 498 | 6,663 | 1.36 | 447 | 259 | 9,018.5 / 495 | 1.35 |
| 210 | 8,902.5 / 513 | 6,663 | 1.34 | 354 | 259 | 8,832.5 / 510 | 1.33 |
| 210 with `l` | **8,894.5 / 521** | 6,663 | 1.33 | | | **8,825.6 / 517** | 1.33 |

(`b = 15` is the same circuit in both modes.) Every row is about 14 Toffolis per added qubit below
its neighbour. The `C x Q` optimum stays the product point without `q` (11,676.5 / 316, 3.69e6):
14 Toffolis per qubit is below the roughly 37 per qubit a product gain needs there. Windows (mode
0): 11,416.5 / 408, 10,822.6 / 421, 9,922.5 / 451, 8,902.4 / 516 (9 + 9), dominated by mode 1.

With `.e` and the Majorana budget, every rung of this ladder drops by 30 Toffolis at the same Q
(the two levers act on disjoint stages, the PREPARE item-erasure phase pass and SELECT delivery).
Measured: `imchxgrdky3zabCHKVIDXtNBWs+q16_1.13.e`, 2 / 2, 9 + 9: **11,568.566 / 316**;
`imchxgrdky3zabCHKVIDXtNBWs+Rq17_1.14.e`, 2 / 2, 8 + 8: **11,500.594 / 313**. These are the lowest
Reiher `C x Q` points in this file. On Li, `imchxgrdky7zabCHVIXJsoq22_1.e` (3 / 2, 9 + 9)
measures 24,761.357 / 400.

**Li.** Li's 75 x 14 word bits per pass have affine rank 929 over 930 reachable leaves, so a held
dimension saves 3 buys per copy (Reiher: 7): 6 Toffolis per qubit and step at G = 3, 9 at G = 4,
12 at G = 5, 15 at G = 6. G = 3 needs 311 slots and cannot fit under 500 qubits (its SELECT
plateau is at least 531), so below 500 the lever runs on G = 4 .. 7.

Mode 0 at G = 3 (`imchxgrdky3zabCHVIXJtRNOBWs`, 9 + 9, 3 / 2, measured): no `q` 16,493.2 / 555;
`q320` 15,869.4 / 855, `q400` 14,885.5 / 939, `q480` 14,205.4 / 1,024, `q520` 13,785.5 / 1,066,
`q640` 13,529.5 / 1,167.

Mode 1 (`li-sa-est-v1`, 9 + 9; fast engine, K = 524,288):

| Q | C | C / floor | bundle (`INNER_A` / `OUTER_A`) |
| ---: | ---: | ---: | --- |
| 400 | 24,825.2 | 2.22 | `imchxgrdky7zabCHVIXJsoq22_1` (3 / 2) |
| 430 | 22,367.6 | 2.01 | `imchxgrdky6zabCHVIXZJsoq26_1` (3 / 2) |
| 450 | 20,161.6 | 1.82 | `imchxgrdky5zabCHVIXJtRNOBWsq27_1` (3 / 2) |
| 480 | 19,801.5 | 1.80 | `imchxgrdky5zabCHVIXJtRNOBWsq57_1` (3 / 2) |
| 499 | 17,867.5 | 1.63 | `imchxgrdky4zabCAXZJBMnFsYUq21_1` (8 / 3) |
| 540 | 17,499.6 | 1.61 | `imchxgrdky4zabCAXZJBMnFsYUq62_1` (8 / 3) |
| 555 | 16,217.6 | 1.49 | `imchxgrdky3zabCAXJSRNOBWMenFs` (8 / 3; G = 3 with the paired lookup, no `q`) |
| 600 | 15,809.4 | 1.47 | `imchxgrdky3zabCAXJBMnFsUq50_1` (8 / 3) |
| 800 | 14,609.5 | 1.41 | `...q250_1` |
| 1,000 | 13,425.4 | 1.35 | `...q450_1` |
| 1,168 | **13,089.4** | 1.34 | `...q618_1` (the whole span; dominates the unsplit 13,341.0 / 1,218) |

8 + 8: 22,197.4 / 430, 19,997.6 / 450, 19,637.4 / 480, 17,753.7 / 499, 13,001.5 / 1,165.

**The linear-factor bound** (`rankdel::lf_bound`, printed by `tests_rank::rank_plan_table`): when
every Toffoli's inputs are parities of the one-hot's qubits, a product's group-difference matrix
over the full slots has rows in `span{x, y, 1}`, so `#CCX(I) >= (r(I) - 1 - (G - 1) b) / 2` per
segment, `r(I)` the row rank of the segment's words. On Li G = 4 it gives 2,220 Toffolis per copy
at b = 21 (union-span model: 2,037; construction: 3,112). Under 500 qubits on Li the distance to
the floor is this representation cost: the pair products' `(G - 1) / 2` per direction.

Tests: `rankdel::tests` (exhaustive over every leaf, the empty one-hot and lonely group bits of
random tables, G = 2 .. 7, both modes, budgets from one rotation's rank to the whole span, outcome
seeds, with mutants), `tests_c1::{rank_delivery_*, rank_park_*, rank_delivery_wide_*}` (bundles end
to end through the harness on the exact specs, mutants rejected in both modes),
`tests_rank::{dump_rank_layout, toffoli_census, rank_plan_table}` (ignored; analysis).

### 6.17 Composition rules

- `emit` refuses `sa-toff` levers together with `sa-pareto`.
- `f` and `C` are alternatives for the one-hot write (`C` replaces `f`); `o` takes precedence over
  `A`.
- `S` and `t` are alternatives; `H` and `G` are alternative inner reads, and neither combines with
  `p` or `j`.
- `q` replaces `E` and `P`.
- "Plain comparison" in the index means the comparator does not hold its ladder (`l` for the
  inner one, `L` for the outer one).
- `+` and `.p` cannot be combined (different padding layouts).
- `.h<k>` cannot be combined with `q` (it needs `U` holding rotation 0).
- `_` cannot be combined with `D` or `j`, and must be written before any `q` token.
- `~` needs `_` and cannot be combined with `R`, `l`, `p`, `D`, `j`.
- `.e` needs a two-group item one-hot (not `T`).
- `q` on Li below 500 qubits needs `G >= 4` in mode 1; with `~` it needs the Majorana budget `.c`.
- The compositions that give the fronts in section 8: `t` + `q<b>_1` on Li's paired-lookup bundles
  (G = 3, 4); `+` + `q<b>_1` on Li's G = 5 item one-hot, and `t` + `+` on G = 6 / 7; `+` +
  `q<b>_1` on Reiher's product ladder; `s` on the low-Q `T` / `U` bundles; `C E t R N O B W +` on
  Reiher's unsplit one-hot; and `INNER_A = 9` on Li's G = 3 ladder from `b` about 250, where the
  wider inner QROAM's transient (about 745 qubits) fits under the SELECT plateau (-110 to -140
  Toffolis at the same Q).

## 7. Lower bounds

`tests_floor.rs` (ignored tests; no op stream involved) proves lower bounds on `C_step` as a
function of the qubit budget for any step of this lane-map family.

- **Rotations.** At least `2 (2N - 3)` chain Givens per step (a dense Householder per copy needs
  an increasing and a decreasing chain of rotations): 7,748 Toffolis on Li, 5,880 on Reiher.
- **Delivery (union-span bound).** Each `CCX` adds at most one function to the span of everything
  ever computed, and both halves of a copy must present every stored angle word. So
  `T_copy >= (r + 1 - d0) + (r - L)`, with `r` the words' affine GF(2) rank (929 Li, 321 Reiher),
  `d0` the outer-class functions (90, 62) and `L <= Q - 2N - beta`.
- **Li, Q <= 499:** C >= 10,622 (9 + 9) from these two terms, and 10,977 with `SpinSwap` and the
  reflections. C < 10,000 needs Q >= 811 / 988. This holds for delivery of the stored words; a
  sign-variant delivery has the weaker proven floor 9,034 / 9,389 (variant-proof rank 532).
- `tests_floor::segmented_floor` evaluates the bound at a circuit's own Q and keep widths; it is
  the "floor" column of sections 6.16 and 8.
- **PREPARE** (`tests_prep::prep_floor`). The inner read meets the row-space bound to 0.5% (item
  one-hot) and 3% (paired lookup), and the erasure meets the degree-2 floor `2 sqrt(L) - 2` to
  3-6%. The union-span floor charges PREPARE nothing, and cannot. PREPARE (3,404.5 Toffolis on Li,
  2,223.5 on Reiher at the fronts of section 8) is half of the gap to the bound.

```bash
cargo test --release --lib sa_low::tests_floor::floor_bound -- --ignored --nocapture   # SA_SPECS, SA_MU
cargo test --release --lib sa_low::tests_prep::prep_floor -- --ignored --nocapture
```

## 8. Measured fronts on the two tracks

All rows: `sa-toff`, estimated class, measured at K = 524,288 lanes on the fast engine after the
K = 64 check with the reference evaluator, every run passing. `C_step` values are sampled means.
C x Q in brackets where it is the point's purpose.

### 8.1 Products and low-C corners

| spec | bundle, `INNER_A` / `OUTER_A` | 9 + 9 C / Q | 8 + 8 C / Q |
| --- | --- | --- | --- |
| li-sa-est-v1 | `imchxgrdky5zabCHKVIXJtRNOBWPs+-_`, 3 / 2 (best Li C x Q) | **19,965.500 / 427 (8.525e6)** | **19,891.463 / 424 (8.434e6)** |
| li-sa-est-v1 | `imchxgrdky4zabCAHVIXZJtRNOBWsY+-_`, 3 / 2 | **17,987.408 / 477 (8.580e6)** | **17,913.426 / 474 (8.491e6)** |
| li-sa-est-v1 | `imchxgrdky5zabCHVIXJtRNOBWPs+-`, 3 / 2 | 19,937.383 / 433 | 19,863.404 / 430 |
| li-sa-est-v1 | `imchxgrdky4zabCAHVIXZJtRNOBWsY+-`, 3 / 2 | 17,963.427 / 483 | 17,889.569 / 480 |
| li-sa-est-v1 | `imchxgrdky4zabCAXZJtRNOBWMenFsY`, 8 / 3 | 18,059.385 / 484 (8.741e6) | 17,969.443 / 481 (8.643e6) |
| li-sa-est-v1 | `imchxgrdky4zabCAXZJSRNOBWMenFsY`, 8 / 3 | 18,133.572 / 485 (8.795e6) | 18,043.497 / 482 (8.697e6) |
| li-sa-est-v1 | `imchxgrdky5zabCHVIXJtRNOBWs`, 3 / 2 | 20,373.232 / 433 (8.822e6) | 20,245.419 / 430 (8.706e6) |
| li-sa-est-v1 | `imchxgrdky5zabCHVIXJtRNOBWPs+`, 3 / 2 | 20,327.454 / 433 | 20,197.442 / 430 |
| li-sa-est-v1 | `imchxgrdky4zabCAXZJBMnFsYUt`, 8 / 3 (lowest C < 500, 9 + 9) | **17,855.487 / 492** | 17,767.585 / 489 |
| li-sa-est-v1 | `imchxgrdy4zabCAXZJBMnFsYUt`, 8 / 3 (lowest C < 500, 8 + 8) | (501) | **17,708.512 / 497** |
| li-sa-est-v1 | `imchxgrdky4zabCAXZJBMnFsYU`, 8 / 3 | 17,927.449 / 493 | 17,839.612 / 490 |
| li-sa-est-v1 | `imchxgrdy4zabCAXZJBMnFsYU`, 8 / 3 | (502) | 17,780.480 / 498 |
| li-sa-est-v1 | `imchxgrEC`, 5 / 2 (lowest C at any Q without `q`) | **13,189.020 / 1,210** | **13,116.495 / 1,206** |
| reiher-sa-est-v1 | `imchxgrdky3zabCHKVIDXtNBWs+`, 2 / 2 (best Reiher C x Q without `q` and `.e`) | **11,652.678 / 316 (3.682e6)** | **11,586.476 / 313 (3.627e6)** |
| reiher-sa-est-v1 | `imchxgrdky3zabCHKVIDXtNBWs`, 2 / 2 | 11,676.491 / 316 (3.690e6) | 11,610.477 / 313 (3.634e6) |
| reiher-sa-est-v1 | `imchxlgrdkyECHKVIXt+`, 3 / 3 (lowest C) | **8,791.359 / 532** | **8,722.413 / 528** |
| reiher-sa-est-v1 | `imchxgrdkyECHKVIXt+`, 3 / 3 | 8,799.538 / 524 | 8,729.535 / 521 |
| reiher-sa-est-v1 | `imchxlgrdkyEHKVIXt+`, 3 / 3 | 8,877.495 / 532 | 8,808.535 / 528 |
| reiher-sa-est-v1 | `imchxlgrdkyEHKVIXt`, 3 / 3 | 8,901.546 / 532 | 8,832.386 / 528 |

The Li product point moved from the paired lookup at G = 4 (18,059.385 / 484, bound by the paired
lookup's read at slot width 8) to G = 5 once `-` let G = 5's 49 qubits of PREPARE headroom pay for
a cheaper read, and `_` took the redundant index copy off the SELECT plateau. On Reiher neither
`-` nor `_` moves the product point: the item one-hot stages bind there.

Earlier steps of the same ladders, kept because they isolate one lever group each (`OUTER_A = 2`,
keep 8 + 8; K = 4,096 lanes, `eval_circuit --samples 4096`):

| spec | tweaks, `INNER_A` | static C / Q | K = 4,096 C / Q | C x Q |
| --- | --- | --- | --- | ---: |
| reiher-sa-est-v1 | `imchxgrdky3z`, 3 | 13,434.5 / 325 | 13,434.78 / 325 | 4.366e6 |
| reiher-sa-est-v1 | `imchxgrdky3zf`, 3 | 13,122.5 / 325 | 13,122.17 / 325 | 4.265e6 |
| reiher-sa-est-v1 | `imchxgrdky3zfab`, 3 | 13,167.5 / 312 | 13,166.43 / 312 | 4.108e6 |
| reiher-sa-est-v1 | `imchxgrdky3zfabSRNOBW`, 3 | | 13,174.69 / 306 | 4.032e6 |
| reiher-sa-est-v1 | `imchxgrdky3zabCHKVIXD`, 3 | 11,690.5 / 317 | | 3.706e6 |
| li-sa-est-v1 | `imchxgrdky3z`, 4 | 18,246.5 / 575 | 18,246.31 / 575 | 1.049e7 |
| li-sa-est-v1 | `imchxgrdky3zf`, 4 | 17,126.5 / 575 | 17,125.39 / 575 | 9.847e6 |
| li-sa-est-v1 | `imchxgrdky3zfab`, 4 | 17,167.5 / 564 | 17,167.09 / 564 | 9.682e6 |
| li-sa-est-v1 | `imchxgrdky5zfabH`, 3 | 21,231.5 / 440 | | 9.342e6 |
| li-sa-est-v1 | `imchxgrdky5zfabHSRNOBW`, 3 | | 21,236.60 / 435 | 9.238e6 |
| li-sa-est-v1 | `imchxgrdky5zabCHVIXJ`, 3 | 20,379.5 / 437 | | 8.906e6 |

### 8.2 Best validated circuit per qubit budget, against the proven floor

Floor: `tests_floor::segmented_floor` at the circuit's own Q and keep, with `SpinSwap` and
reflections.

| corner | 9 + 9 C / Q (C / floor) | 8 + 8 C / Q (C / floor) | bundle, `INNER_A` / `OUTER_A` |
| --- | --- | --- | --- |
| Li closest to the floor | **12,879.514 / 1,166 (1.317)** | **12,827.411 / 1,163 (1.312)** | `imchxgrdky3zabCAXJBMnFsUtq618_1`, 9 / 3 |
| Li at Q 499 | **17,777.409 / 499 (1.620)** | **17,663.555 / 499 (1.610)** | `imchxgrdky4zabCAXZJBMnFsYUtq23_1` (8 + 8: `q26_1`), 8 / 3 |
| Li at Q 480 | 19,755.5 / 480 (1.794) | **18,907.471 / 480 (1.718)** | 9 + 9 `...5zabCHVIXJtRNOBWs+q57_1` 3 / 2; 8 + 8 `imchxgrdky4zabCAXZJtRNOBWMenFsY` 7 / 3 |
| Li at Q 450 | **20,115.470 / 450 (1.817)** | 19,949.5 / 450 (1.803) | `imchxgrdky5zabCHVIXJtRNOBWs+q27_1` (8 + 8 `q30_1`), 3 / 2 |
| Li at Q 430 | **22,165.359 / 430 (1.995)** | 20,197.4 / 430 (1.819) | 9 + 9 `imchxgrdky6zabCHVIXZJsot+q28_1`; 8 + 8 `...5zabCHVIXJtRNOBWs+`, 3 / 2 |
| Li at Q 413 | 24,383.5 / 413 (2.187) | **22,245.590 / 413 (1.997)** | 9 + 9 `...7zabCHVIXJsot+q37_1`; 8 + 8 `imchxgrdky6zabCHVIXZJsot+q14_1`, 3 / 2 |
| Reiher closest to the floor | 8,791.359 / 532 (1.319) | 8,722.413 / 528 (1.310) | `imchxlgrdkyECHKVIXt+`, 3 / 3 |
| Reiher at Q 520 | **8,827.527 / 520 (1.325)** | **8,755.543 / 517 (1.315)** | `imchxgrdkyECHKVIXtRNOBW+`, 3 / 3 |
| Reiher at Q 453 | **9,694.564 / 453 (1.455)** | **9,600.690 / 453 (1.442)** | `imchxgrdky3zabCHKVIDXtNBWs+q150_1` 3 / 3 (8 + 8 `q153_1` 2 / 2) |
| Reiher at Q 285-288 | 14,429.7 / 288 (1.897) | **14,305.525 / 285 (1.877)** | `imchxgrdky4zZabCHKVITXSRNOBWs`, 2 / 2 |
| Reiher at Q 256 / 253 without `~` | **19,417.332 / 256** | **19,294.377 / 253** | `imchxgrdky7zabfHKVITXSRNOBWUs`, 2 / 1 |
| Reiher at Q 256 / 253 with `~` | **16,704.888 / 256** | **16,553.890 / 253** | section 6.14 |

Where the gap to the floor sits at the closest points (stage ledgers):

- **Reiher at 520.** Of the 2,164.5 gap, PREPARE is 1,949.5, delivery's one-hot write over the
  floor's term is +136, and the rotations are +56.
- **Li at 1,166.** Of the 3,098.5 gap, PREPARE is 2,681.5, delivery is +355 and the rotations are
  +52.

So at the low-C ends the remaining distance to the floor is PREPARE, which the union-span floor
does not charge. Under 500 qubits on Li the distance is the group representation of section 6.16.

## 9. Measured circuits and their digests

Every measured circuit of the two tracks is pinned by digest. `tests/sa_circuits/list.rs` holds
one `circuit!` line per circuit: the test name, the spec id, the family, a parameter constructor,
and the pinned `[ops count, ops sha256, lanemap sha256, family sha256]`. `tests/sa_digests.rs`
rebuilds each one from its `Params` (what `build_circuit` compiles from the circuit's `FEMOCO_*`
knobs) and checks that it writes exactly the pinned `ops.bin`, `lanemap.bin` and
`family.out.json`. A change to this directory that moves any measured circuit fails there; the
measured counts then no longer describe the code. `tests/equiv_pinned.rs` runs the same list under
the reference and the fast lane engine.

Knob to pin-constructor correspondence (`FEMOCO_WALK_SPEC` is the `circuit!` line's spec id):

| Constructor in `list.rs` | Build |
| --- | --- |
| `low(true)` / `low(false)` | `FEMOCO_WALK_ARCH=sa-low2025` / `sa-low2025-bothspin` |
| `toff(tw)` | `FEMOCO_WALK_ARCH=sa-toff FEMOCO_SA_TWEAKS=tw` |
| `toff_a(tw, ia)` | the same with `FEMOCO_SA_INNER_A=ia` |
| `toff_mu(tw, mu_o, mu_i)` | `sa-toff` with `FEMOCO_SA_TWEAKS=tw FEMOCO_SA_MU_O=mu_o FEMOCO_SA_MU_I=mu_i` |
| `toff_mua(tw, (mu_o, mu_i), ia, oa)` | the same with `FEMOCO_SA_INNER_A=ia FEMOCO_SA_OUTER_A=oa` |
| `toff_lw(tw, mu, ia, oa)`, `toff_rp(tw, mu, ia, oa)` | `FEMOCO_WALK_ARCH=sa-toff FEMOCO_SA_TWEAKS=tw FEMOCO_SA_MU_O=mu FEMOCO_SA_MU_I=mu FEMOCO_SA_INNER_A=ia FEMOCO_SA_OUTER_A=oa` |
| `pareto(c, ia, oa, carries, drop_alt)` | `FEMOCO_WALK_ARCH=sa-pareto FEMOCO_SA_CHUNKS=c FEMOCO_SA_INNER_A=ia FEMOCO_SA_OUTER_A=oa FEMOCO_SA_CARRIES=carries FEMOCO_SA_DROP_ALT=drop_alt` (lean on) |

Knobs that a constructor does not set take `Params::for_spec`'s defaults (section 2). Pin names
ending in `_m8` / `_m9` use keep 8 + 8 / 9 + 9.

`tools/repro/` rebuilds pinned circuits and refuses an `ops.bin` whose SHA-256 differs from the
pin. On the rigorous twins, the `sa-low2025`, `sa-toff`, `sa-pareto` and `sa-lowq` circuits that
were re-measured with independent seeds reproduced their counts exactly.

## 10. Tests and static scans

```bash
cargo test --release --lib sa_low                                     # unit and end-to-end tests
cargo test --release --test sa_digests                                # every pinned circuit
cargo test --release --lib sa_low::tests::pinned_ledgers -- --ignored --nocapture       # per-stage counts
SA_TWEAKS=none,i,m,mc,x,h,l,L,imchx \
  cargo test --release --lib sa_low::tests::pinned_ledgers_toff -- --ignored --nocapture
FEMOCO_SA_SCAN="chunks,inner_a,outer_a,lean,carries,drop_alt;..." \
  cargo test --release --lib sa_low::tests::pareto_scan -- --ignored --nocapture        # sa-pareto, static
SA_MU=9,9 SA_SPECS=reiher-sa-est-v1 SA_TWEAKS=imchxgrdky3zabCHKVIDXtNBWsq150_1 SA_INNER_A=3 SA_OUTER_A=3 \
  cargo test --release --lib sa_low::tests_combo::combo_scan -- --ignored --nocapture   # sa-toff, static
SA_SPECS=li-sa-est-v1 SA_TWEAKS=imchxgr,imchxgrvd SA_INNER_A=4 \
  cargo test --release --lib sa_low::tests_combo -- --ignored --nocapture               # anatomy, scan, rank
SA_MU=9,9 cargo test --release --lib sa_low::tests_rank::rank_plan_table -- --ignored --nocapture
```

- `tests_combo::combo_scan` gives static `C_step` / `Q_peak` for any spec, bundle, keep split and
  block counts (`SA_SPECS`, `SA_TWEAKS`, `SA_MU`, `SA_INNER_A`, `SA_OUTER_A`).
  `tests_combo::anatomy` shows what is live at the peak; `SA_PEAKSTAGE` restricts it to named
  stages and `SA_PEAKOPS` prints the ops at the peak and what each live qubit was first touched by.
- `tests_c1::c1_lanes_pinned` runs K = 64 lanes of a bundle through the harness's `evaluate`.
- Every lever has an end-to-end test through `score::evaluate` on small exact specs (N = 3, 4, 5
  and padded or wide variants built for the lever), a check that the ledger equals the harness's
  count, and mutants (a dropped fixup, a wrong outcome, a skipped correction) that the harness must
  reject. The mutants are selected through `onehot::FAULT` and are compiled for tests only.
