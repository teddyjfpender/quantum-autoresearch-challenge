# The spectrum-amplified sum-of-squares specs (`sos-sa`)

This document defines the `sos-sa` encoding, the lane map `sa-nested-alias-v1`, the ground-energy
certificates, the scoring view for spectral amplification, and the two rounding classes under which
a lane map is accepted.

Four specs ship under `specs/`. They state two operators:

| spec id | instance | `sa.bin` | rounding class | role |
| --- | --- | --- | --- | --- |
| `reiher-sa-est-v1` | Reiher | shared with `reiher-sa-v1` | estimated (section 14) | challenge track |
| `li-sa-est-v1` | Li | shared with `li-sa-v1` | estimated (section 14) | challenge track |
| `reiher-sa-v1` | Reiher | | rigorous, 0.1 mHa 1-norm (section 5.3) | twin, used as a test fixture |
| `li-sa-v1` | Li | | rigorous, 0.1 mHa 1-norm (section 5.3) | twin, used as a test fixture |

Sections 1 to 11 apply to all four. Section 14 is the acceptance standard of the two challenge
tracks. Sections 12 and 13 describe two rules the trusted code implements that no shipped spec
declares.

Code: `src/spec/sa.rs` (loader), `src/spec/rounding.rs` (rounding classes),
`src/lanemap/sa_nested.rs` (lane map), `src/score.rs` and `src/score/report.rs` (acceptance and
`score.json`).

Source: G. H. Low, R. King, D. W. Berry, Q. Han, A. E. DePrince III, A. F. White, R. Babbush,
R. D. Somma, N. C. Rubin, "Fast Quantum Simulation of Electronic Structure by Spectral
Amplification", Phys. Rev. X 15, 041016 (2025), doi:10.1103/pb2g-j9cw (arXiv:2502.15882; the
journal version supersedes v1). Data and scripts: Zenodo 10.5281/zenodo.17066718 (CC-BY-4.0),
called "the record" below.

## 1. Headline

| | Reiher | Li | Where |
| --- | ---: | ---: | --- |
| DFTHC (R, B, C) | (10, 27, 27) | (15, 57, 19) | the record's `final_rbc_systems.csv` |
| Lambda, published | 58.3440 | 179.7296 | Table V |
| Lambda, the factor pickle (float32) | 58.3440056 | 179.7296448 | pickle `values['lambda_DFTHC']` |
| **Lambda, this spec (exact)** | **58.343974747** | **179.729577124** | `spec.json` `lambda` |
| E_gap, published (HF-adjusted) | 4.0535 | 5.3820 | Table V |
| **E_gap upper bound, this certificate** | **3.531422** | **5.370835** | `certificate.json` |
| lambda_eff, published | 21.3674 | 43.6538 | Table V |
| **lambda_eff upper bound, this certificate** | **19.990083** | **43.609057** | `certificate.json` |
| lambda_eff used for a certified claim | 21.3674 (theirs) | 43.6538 (theirs) | the larger of the two, section 6 |
| Rotation bits | 16 | 15 | their cost script (section 2.2) |
| Generators / ordered-product terms | 378 / 802,116 | 437 / 3,736,654 | section 3 |

The spec reproduces the authors' Lambda to their printed precision. It differs from the float32
value stored in their pickle by -3.08e-5 (Reiher) and -6.77e-5 (Li). A float32 re-run of their own
functions on the same inputs gives 58.343956 and 179.729599, so the difference comes from float32
evaluation, not from a different operator. This spec evaluates exactly (rationals) from the float32
factors, which are exact binary rationals, and float64 one-body eigenpairs.

The certificate's lambda_eff is lower than the published one (by 6.4% Reiher, 0.1% Li), because the
certificate's UHF determinant of the operator has a lower energy than the authors' RHF one
(section 6). Both are upper bounds. A certified claim uses the larger one, the published value.

## 2. Inputs and the operator

### 2.1 Inputs

- **Factors.** The published point's file in the record's `spinfree.zip` is the one whose pickled
  values equal the CSV row (lambda_walk, error_h2, the SOS gap, to all printed digits):
  - Reiher: `spinfree/best_solutions/FeMoCo54o54e_10_27_27_ln25_ep1000_lrf10_tg3464_te10_ndiff0_ln25_seed3000000.pickle`
  - Li: `spinfree/FeMoCo76o113e/scan_gap_seed2000000/FeMoCo76o113e_15_57_19_ln100_ep1000_lrf10_tg4666_te10_ndiff35.pickle`

  Both pickles hold float32 arrays.
- **Integrals.** The record's FCIDUMPs equal the active-space integrals of Reiher et al. 2017
  (54 electrons in 54 orbitals) and Li et al. 2019 (113 electrons in 76 orbitals) entry for entry:
  the maximum difference is 0.0 over all 8,503,056 (Reiher) and 33,362,176 (Li) nonzero
  two-electron entries. The Li FCIDUMP carries a core energy (-21021.2140122) that the Li specs
  omit: they use a core energy of 0, which shifts energies by a constant and changes no gap.
- **The authors' scripts** in the record define the conventions reproduced here (`df_thc.py`,
  `df_thc_sga.py`, `read_utils.py`, `spin_free_block_encoding_costs.py`,
  `get_walk_counts_different_bits_adjustments_for_paper_revision.py`). The pair-index convention
  is a row-major lower-triangle index.

### 2.2 Parameters

- `(R, B, C)` and the electron count (54, 113), the sector the BLISS shift is fitted for.
- **Rotation bits 16 (Reiher) and 15 (Li).** The record's resource script, run unchanged, prints
  `FeMoco54 ... Coeff bits: 9.00 Rot bits: 16.00` and `FeMoco76 ... Coeff bits: 9.00 Rot bits: 15.00`,
  and with those bits `FeMoco54 10203 1132 3.42e+08` and `FeMoco76 14629 1454 1.00e+09`, the
  journal's Cost[BE], qubits and totals. The rotation bits define the spec: the quantized networks
  are the operator.
- The authors' 9 keep ("coefficient") bits are **not** a spec parameter. A lane map chooses its own
  keep bits and must meet the rounding rule of its spec: the estimated class of section 14 on
  `*-sa-est-v1`, the 0.1 mHa 1-norm rule of section 5.3 on `*-sa-v1`.

### 2.3 The operator

A Givens network is the chain of `N - 1` rotations on the adjacent spatial orbitals `(j, j + 1)`,
`j = 0 .. N - 2`, each with an angle `2 pi a / 2^beta` for a `beta`-bit integer `a` (`beta` = the
rotation bits). The network `V_u` maps spatial orbital 0 to the unit vector `u`, quantized to those
angles. With `Et(u) = sum_s n(u, s) - 1 = -(Z(u,0) + Z(u,1))/2` and `Z(u, s) = V_u Z_s V_u^dagger`:

`H_spec = sos_const + sum_r e_r Et(u_r) + sum_{r,c} (1/2) (wB_rc + sum_b w_rcb Et(u_rb))^2`

- `w_rcb = W[r, b, c]` and `wB_rc = H2_I[r, c]` are float32 values of that pickle. `u_rb` is its
  `U[r, b]`, normalized (their `Compute_H2_sqrt`), then quantized.
- `e_r, u_r` are the float64 eigenpairs of their shifted one-body matrix `h1'`
  (`df_thc_sga.Compute_H1_shifted_by_Hsym_lower`), built from the integrals exactly as they build
  it: `H1_DF + (N - eta)/2 Hsym - sum_rc H2_I_rc L_rc + H1_I`, with `H1_DF = h0 + einsum('pprs') -
  einsum('pqqs')/2` and `L_rc = sum_b w_rcb u_rb u_rb^T`.
- `sos_const = ecore_DF + (N - eta) H1_I - (1/2) sum H2_I^2`, their `-H0`.

**What it equals.** On the `eta`-electron sector, `H_spec = H + (1/2) sum_pqrs D_pqrs Et_pq Et_rs`
up to the rotation rounding. Here `H` is the instance's Hamiltonian, `Et_pq = E_pq - delta_pq`, and
`D` is the DFTHC fit error of the BLISS-shifted two-body tensor. Off the sector a BLISS term
`(N^ - eta)(alpha + (Hs/2).E)` is added. The identity was checked in spin-free form (before
rotation rounding) to 5.8e-15 (Reiher) and 1.2e-14 (Li) per one-body entry, and to 5e-13 and
2e-12 Ha in the constant, and densely in Fock space on random 3-orbital instances (on-sector
residual below 1.5e-14). So the spec is the authors' Hamiltonian, and **it agrees with FeMoco's
only on the electron sector**.

## 3. The sum of squares

`H_spec = E_SOS + sum_alpha O_alpha^dagger O_alpha` with `E_SOS = sos_const - sum_r |e_r|` (the paper's
Eqs. (26)-(30)). Majorana operators use the interleaved mode order: spin orbital `2p + s` for
spatial orbital `p` and spin `s`, with `gamma_{2m + x}`, `x` in {0, 1}, the two Majoranas of mode
`m`. The generators are:

- **One-body `(r, s)`**, `r < N`, `s` in {0, 1}: `O = sqrt|e_r| a(u_r, s)` for `e_r >= 0` and
  `sqrt|e_r| a^dagger(u_r, s)` otherwise. The LCU is `sqrt|e_r| (M_0 + M_1)/2` with
  `M_0 = gamma(u_r, s, 0)` and `M_1 = +i gamma(u_r, s, 1)` (`e_r >= 0`) or `-i gamma(u_r, s, 1)`.
  These are rotated single Majoranas, `gamma(u, s, x) = sum_p u_p gamma_{2(2p+s)+x}`. `lambda^2 = |e_r|`.
- **Square `(r, c)`**: `O = (wB + sum_b w_b Et(u_b)) / sqrt 2`. The LCU runs over
  `M_(b,s) = sign(w_b) i gamma(u_b, s, 0) gamma(u_b, s, 1) = -sign(w_b) Z(u_b, s)` with weight
  `|w_b|/2`, and `M_B = sign(wB) I` with weight `|wB|`. `lambda^2 = S^2/2` with
  `S = |wB| + sum_b |w_b|`.

`Lambda = (1/2) sum_alpha lambda_alpha^2 = sum_r |e_r| + (1/4) sum_rc S_rc^2` is the paper's Eq. (33)
and their `lambda_H1 + lambda_H2`.

**Ordered-product terms.** `O^dagger O = lambda^2 sum_{x,y} q_x q_y M_y^dagger M_x`. The terms
with `x = y` are the identity. The spec's terms `t = (alpha, x, y)` with `x != y` have
`c_t = lambda_alpha^2 q_x q_y`: `|e_r|/4` (one-body), `|w_b w_b'|/8` (two spin items of a square),
and `|wB w_b|/4` (identity and spin item). So `H_spec = identity + sum_t c_t M_t` with

- `identity = sos_const + sum_rc (wB_rc^2/2 + sum_b w_rcb^2/4)`, and
- `lambda_flat = sum_t c_t = 2 Lambda - (identity - E_SOS)`.

All four (`lambda`, `identity`, `E_SOS`, `lambda_flat`) are exact rationals, recomputed by the Rust
loader from the payload and compared with `spec.json`. `terms = 4N + RC (2B+1) 2B`.

## 4. Payload (`sa.bin`, `FEMOSAS1` v1, committed)

Little-endian: `magic, u32 version = 1, u32 N, R, B, C, beta, electrons, f64 sos_const, f64 e[N],
u32 e_angles[N][N-1], f64 wB[R][C], f64 w[R][C][B], u32 angles[R][B][N-1]`. Angles of one network
are the rotations `(j, j+1)` for `j = 0 .. N-2`, in order. Sizes: 129,644 bytes (Reiher) and
412,192 bytes (Li). The loader refuses a payload whose SHA-256 differs from `spec.json` or
`specs/INDEX.json`, or whose recomputed exact values differ from `spec.json`.

`reiher-sa-est-v1` and `reiher-sa-v1` hold the same `sa.bin` byte for byte, as do `li-sa-est-v1`
and `li-sa-v1`. The twins differ only in the `rounding_class` block of `spec.json` (section 14.1).

## 5. Lane map `sa-nested-alias-v1` (`src/lanemap/sa_nested.rs`)

### 5.1 Layout

The uniform register is laid out low bits first: outer alias index `k_o`, outer keep `mu_o`, the
outer spin bit `s1`, then the inner register (inner alias index `k_i`, inner keep `mu_i`, the inner
spin bit `s0`). So `u_o = k_o + mu_o + 1`, `w = k_i + mu_i + 1` and `u = u_o + w <= 63`.

- The **outer alias** (an alias table on bits `0..k_o+mu_o`) picks an outer item: one-body
  eigenvector `r < N`, whose generator is `(r, s1)`, or square `(r, c)` at `N + rC + c`, which
  ignores `s1`. This is the paper's Fig. 2: PREP on `x_o` over `N + RC` items plus a Hadamard'd
  spin qubit `sigma_1` used by the one-body generators only.
- The **inner alias** (one table per outer item, all of one width) picks `x` in {0, 1} (one-body),
  or `j` in `0..=B` for a square, where `j = B` is the identity and `s0` is the spin of `j < B`.
- An alias table of width `(k, mu)` has `2^k` buckets, each with a `keep` threshold below `2^mu`
  and an `alt` item. A lane with bucket `i` and keep value `v` names item `i` when `v < keep[i]`
  and item `alt[i]` otherwise. The number of lanes that name an item is its **count**; the counts of
  a table sum to `T = 2^(k + mu)`.
- Payload after the family header: `u32 k_o, mu_o, k_i, mu_i`, `lambda_decl` (`Exact::to_bytes`),
  `keep_o[2^k_o], alt_o[2^k_o]`, then `N + RC` inner tables `keep[2^k_i], alt[2^k_i]` (u32
  little-endian). The parse-time checks are, per table: every alt is an item, every padding bucket
  (index at or above the item count) has `keep = 0`, and every keep is below `2^mu`; plus one
  shared inner width, the exact payload length, and a positive dyadic `lambda_decl` with exponent
  at most 512.
- **`lambda_decl` byte layout** (`Exact::to_bytes` / `Exact::from_bytes`, `src/spec/exact.rs`).
  Starting at payload byte 16: `u8 sign` (1 = negative, 0 = non-negative; the reader treats any
  value other than 1 as non-negative), `u32 n_len` (little-endian), `n_len` bytes of the
  numerator's magnitude (little-endian), `u32 d_len`, `d_len` bytes of the denominator
  (little-endian). Each part is at most 128 bytes (`MAX_PART_BYTES`), and a zero denominator is
  rejected. The fraction need not be in lowest terms. The keep and alt tables start right after the
  last denominator byte.

### 5.2 Lanes and the reference

The circuit must have the nested shape: `C`, one `Reflect` on exactly the inner register, then
`C'` identical to `C` op for op, with nothing else touching that register. A lane is a value `s`
of the uniform register before the `Reflect` together with a sampled inner value after it: the
inner register holds `a` before and `b` after. Half the sampled lanes are diagonal (`b = a`), half
paired. The reference a control-1 lane must apply is

`R(s, b) = M_alpha(b)^dagger M_alpha(a)`

with the second pass on the left. The circuit therefore block-encodes

`A = lambda_decl sum_alpha p^_alpha (2 sum_{x,y} q^_x q^_y M_y^dagger M_x - 1)`,

the paper's Eq. (10) `BE[H_SA / Lambda - I] = PREP^dagger SEL^dagger REF_BE SEL PREP`, scaled by
`lambda_decl`. Where `M` is not Hermitian (the one-body `+-i gamma`), the identical second copy
applies `M^dagger` by reading a pass flag that the first copy leaves: an `S` or `Sdg` on the
selected lane, plus `CZ(sel, pass)`. The paper's Fig. 2 does the same with `Maj^dagger`.

### 5.3 Counts and the rounding error

Outer lanes of a generator: `n_r` for one-body `(r, s1)` (one value of `s1`), and `2 n_rc` for a
square (both `s1`). Inner lanes: `2 m_x` for one-body `x` and for the identity item (both `s0`),
and `m_b` for `(b, s0)`. The encoded weight of term `t = (alpha, x, y)` is
`c^_t = 2 lambda_decl n_alpha m_x m_y / 2^(u_o + 2w)`. `rounding_error = sum_t |c_t - c^_t|` is
computed exactly in dyadic integers. Every `x = y` product and every `-1` is a multiple of the
identity. With counts summing to their register sizes,
`A = H_spec - identity + (lambda_decl - lambda_flat) + sum_t (c^_t - c_t)(M_t - 1)`.

**The rigorous rule.** On a spec of the rigorous class (`reiher-sa-v1`, `li-sa-v1`) a lane map is
accepted only if `rounding_error <= 1/10000` Ha (0.1 mHa), decided exactly
(`score::max_rounding_error`). On a spec of the estimated class the same exact number is computed
and reported, but acceptance is the rule of section 14.

`tests/sa_dense.rs` checks the identity above densely: the exact map gives `A` to 1.2e-13, and a
rounded map stays within `2 x rounding_error` entrywise. `tests/sa_nested.rs` checks the formula
against a brute-force enumeration of every lane triple.

### 5.4 Measured widths under the rigorous rule (`tests/sa_spec.rs`, release, float64 largest-remainder counts)

| operator | outer (k, mu) | inner (k, mu) | u | rounding error (Ha) |
| --- | --- | --- | ---: | ---: |
| reiher | (9, 23) | (5, 24) | 63 | 3.30e-6 |
| reiher | (9, 22) | (5, 25) | 63 | 4.39e-6 |
| reiher | (9, 18) | (5, 20) | 54 | 8.44e-5 (smallest passing total keep bits, 38) |
| reiher | (9, 9) | (5, 9) | 34 | 7.97e-2 (the authors' 9 keep bits) |
| li | (9, 22) | (6, 24) | 63 | 1.71e-5 |
| li | (9, 23) | (6, 23) | 63 | 1.98e-5 |
| li | (9, 20) | (6, 22) | 59 | 6.97e-5 (smallest passing total keep bits, 42) |
| li | (9, 9) | (6, 9) | 35 | 2.83e-1 (the authors' 9 keep bits) |

**The authors' precision does not pass the rigorous rule.** At their 9 keep bits the lane map's
1-norm rounding error is 80 mHa (Reiher) and 283 mHa (Li) with these counts. They judged their bit
counts by a CCSD(T) energy change against a 0.587 mHa target per component (`target_error = 0.83`,
split by sqrt 2, plus fitted constants). That is an estimate, not a 1-norm bound. Under the
rigorous rule a walk needs about 18-20 keep bits per alias level where they cost 9, which changes
the QROM output widths and the inequality tests. The `*-sa-est-v1` specs adopt the authors'
estimate instead (section 14); the two are never the same standing.

### 5.5 Constructor and builder API

- `sa_nested::build(spec, (k_o, mu_o), (k_i, mu_i))`: outer weights `2|e_r|` (both spins) and
  `S_rc^2/2`; inner weights `(1, 1)` and `(|w_rc0|, ..., |w_rc,B-1|, |wB_rc|)`; float64
  largest-remainder counts, Walker tables, and `lambda_decl` = the spec's `Lambda` exactly. The
  harness trusts only the exact checks it runs on the result.
- `SaNestedMap::{outer_bits, inner_width, outer_item(s), decode_outer(s) -> Generator,
  inner_item(o, a) -> (item, s0)}` and the public `outer`/`inner` tables give a circuit its data.
- `SaSpec::{one_body_op(r, s, x), square_op(r, c, j, s), e_nets, nets, e, w, wb}` give the per-pass
  operators and networks. `spec::sa::{adjoint, product}` build references.
- The Givens tracker runs at the spec's `beta` (`sim::givens_tracker`), charging `2(beta - 2)` per
  Givens and adding `beta` phase-gradient qubits.
- `tests/sa_nested.rs` holds a complete hand-built circuit (unary iteration, naive) that passes with
  exact counts on a 3-orbital spec. Six mutants are each rejected: no adjoint in the second pass,
  a wrong square sign, a wrong identity sign, a wrong inner term, a wrong spin, and an unrestored
  inner register.

## 6. Certificates and the scoring view

### 6.1 Certificates (`specs/<id>/certificate.json`; conventions in `spec/CONVENTIONS.md`)

`E_up` is a UHF determinant energy of `H_spec` (the spec's own operator, identity included) in the
27+27 (Reiher) and 57+56 (Li) sectors, plus a 1 mHa allowance for floating-point evaluation. Two
independent evaluations of the determinant energy (Coulomb and exchange contractions of the
spin-free form of section 2.3, and Wick's theorem term by term on the definition) agree to 1.6e-11
(Reiher) and 3.2e-12 Ha (Li).

| operator | E_det | E_up | E_SOS | E_gap upper = E_up - E_SOS | lambda_eff upper | `<S^2>` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| reiher | -13481.964620 | -13481.963620 | -13485.495042 | 3.531422 | 19.990083 | 3.01 |
| li | -1118.356083 | -1118.355082 | -1123.725917 | 5.370835 | 43.609057 | 14.40 |

`lambda_eff = sqrt(E_gap (2 Lambda - E_gap))` (the paper's Eq. (11)) increases with `E_gap` on
`[0, Lambda]`, so an upper bound on the sector ground energy bounds `lambda_eff` above.

Each of the four specs has its own `certificate.json`, bound to it by the spec id and the SHA-256
of its `spec.json`. The `*-sa-est-v1` certificates carry the same numbers as their twins', since
the operator is the same.

**Against the published values.** The authors' E_gap is the DFTHC Hamiltonian's Hartree-Fock energy
minus their E_SOS (the record's HF-adjusted CSV column). With this spec's E_SOS their gap implies
`E_HF = -13481.4415` (Reiher). The certificate's UHF determinant is 0.523 Ha lower, so its gap is
smaller. On Li the two nearly coincide (5.3708 against 5.3820). Both are variational bounds on the
same operator's sector ground energy (theirs on their float32 evaluation of it). **Rule: a
certified claim uses the larger lambda_eff**, 21.3674 and 43.6538, the published values.

**Walk offset** (`walk_offsets["sa-nested-alias-v1"]`): `(Lambda - lambda_flat)` plus the run's
`(lambda_decl - Lambda)`, with error factor 2 (section 5.3): -39.982374 (Reiher) and -141.127207
(Li). The lambda_eff view then yields exactly the certificate's lambda_eff when
`lambda_decl = Lambda` and the rounding error is 0, and a larger (conservative) value otherwise.

### 6.2 Scoring view (`score.json` `metrics.spectral_amplification`)

For an `sos-sa` run:

- **`score = lambda_eff_used x C_step x Q_peak`**, with `lambda_eff_used = max(lambda_eff_ours,
  lambda_eff_published)`. `lambda_eff_ours` is the certificate's bound at the run's `lambda_decl`
  with an allowance of `2 x rounding_error`, the exact 1-norm, in both rounding classes. When the
  certificate gives no bound for the run, the score falls back to the worst-case `Lambda`.
  `metrics.lambda` stays `lambda_decl` (equal or close to Lambda), and so does the `lambda` column
  of a locally generated `results.tsv`, so for these rows `score != lambda x toffoli x qubits`.
- For each of `ours`, `published` and `used`, the block has totals at Low et al.'s convention
  (`sigma_PEA = 1.0 mHa`: `ceil(pi lambda_eff / 0.002)` steps x `C_step`) and at this repository's
  controlled comparison (1.6 mHa: `/ 0.0032`), each with `qubits_lee` (plus `2 ceil(log2(I+1)) - 1`
  for the phase-estimation register) beside `qubits_without_pe_register` (`Q_peak`).
- `low2025_givens_charge` (derived): `C_step + 4 x (Givens per step)`. Their cost script charges
  `4 (N-1) beta` per SELECT, which is `beta` per Majorana block and `2 beta` per Givens. This harness
  charges `2 (beta - 2)` (Lee et al. 2021 App. C), so the same circuit costs 4 fewer Toffolis per
  Givens here. Compare with their Cost[BE] under their charge, or say which charge a comparison
  uses.

The worst-case and `lambda_eff` totals of `metrics.conventions` keep their meaning
(`spec/CONVENTIONS.md`). For `sos-sa` their "worst case" is `Lambda`, i.e. `E_gap = Lambda`.

### 6.3 The published numbers under both conventions (derived from the published Cost[BE], not measured here)

| | lambda_eff | steps at 1.0 mHa | total at their Cost[BE] | steps at 1.6 mHa | total | qubits + PE register |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reiher, published | 21.3674 | 33,564 | 3.4245e8 (paper: 3.42e8) | 20,978 | 2.1404e8 | 1132 + 31 |
| Reiher, certificate bound | 19.9901 | 31,401 | 3.2038e8 | 19,626 | 2.0024e8 | 1132 + 29 |
| Li, published | 43.6538 | 68,572 | 1.0031e9 (paper: 1.00e9) | 42,858 | 6.2697e8 | 1454 + 33 |
| Li, certificate bound | 43.6091 | 68,501 | 1.0021e9 | 42,814 | 6.2633e8 | 1454 + 33 |

Formula: `ceil(pi lambda_eff / (2 eps)) x Cost[BE]`, Cost[BE] = 10,203 and 14,629 (Table V).
The register is `2 ceil(log2(I + 1)) - 1`. Their printed qubit counts appear to exclude it.

## 7. Reproduction of the paper, and findings

### 7.1 Reproduced

- Lambda to the printed 4 decimals, both instances.
- `error_h2`, the Frobenius norm of the fit error, is 0.2382644 (pickle 0.2382531) and 0.0825173
  (pickle 0.0825174).
- The record's cost script, run unchanged, prints the journal's Cost[BE], qubits and totals
  (section 2.2).

### 7.2 Approximation statements (`spec.json` `errors`)

| name | kind | Reiher | Li |
| --- | --- | ---: | ---: |
| sector | rigorous (identity) | H_spec = H + fit on the eta-electron sector | same |
| dfthc_fit_frobenius | estimate | 0.2383 | 0.0825 |
| dfthc_fit_bound | rigorous, loose | 854.8 Ha | 284.6 Ha |
| angle_quantization | rigorous, loose | 0.0525 Ha | 0.368 Ha |
| one_body_eigendecomposition | rigorous | 2.0e-14 Ha | 2.4e-14 Ha |
| ccsd_t (their eps_corr) | estimate, published | 0.1254 mEh | 0.1362 mEh |

Nothing bounds the approximation of FeMoco's Hamiltonian at chemical accuracy. The rigorous
statements are orders of magnitude above the budget, and the 0.6 mHa approximation budget is judged
by a CCSD(T) estimate (`spec/CONVENTIONS.md`).

### 7.3 Finding: the DFTHC error is strongly state dependent (measured)

`E_spec - E_H` (`H` the instance's Hamiltonian) evaluated on four UHF determinants, in Ha. The
determinants are the SA certificate's own and three minimized on other factorizations of the same
Hamiltonian (sparse, THC and DF; those specs are not part of this repository):

| determinant | Reiher (10, 27, 27) | Li (15, 57, 19) |
| --- | ---: | ---: |
| its own UHF (the SA certificate's) | -0.1049 | -0.0703 |
| UHF of the sparse form | -0.0647 | -0.0685 |
| UHF of the THC form | -0.0657 | -0.0674 |
| UHF of the DF form | -0.0645 | -0.0676 |

On the same four determinants the absolute errors of a THC and a DF factorization of the Reiher
Hamiltonian vary by at most 1.1 mHa. Reiher's DFTHC error varies by **40 mHa**, and its own UHF
minimum exploits the fit error. The record's CSV shows the RHF-level shift is +0.227 Ha (Reiher)
and +0.308 Ha (Li), against -0.065 to -0.105 at UHF. That is a variation of about 0.3 Ha across
low-lying determinants, where the authors' eps_corr (0.13 mEh) measures the change in CCSD(T)
correlation energy at one RHF reference. This does not show that the ground-energy error exceeds
chemical accuracy. It shows that eps_corr does not measure it, and that the DFTHC operator is a
less uniform approximation of H than THC or DF on these states. It is a caveat on every comparison
with this operator.

## 8. Checks

**Provenance of the committed payloads.** The files under `specs/` were generated and checked
before being committed, by a generator and an independent checker that are not part of this
repository. `spec.json` and `certificate.json` keep provenance fields that name those scripts and
source manifests; they are records, not paths in this tree. What was checked:

- the record's members against their size, CRC-32 and pinned SHA-256;
- the operator identity of section 2.3;
- with an independent reader: exact `lambda`, `identity`, `E_SOS` and `lambda_flat` (the last one
  directly as a sum of term weights); the authors' Lambda rebuilt independently and equal at
  printed precision; the eigenvalues against an independent `h1'` (1e-10); every network within the
  a-priori `(N-1) pi / 2^beta` of the vector it rounds (worst 2.0e-4 against 2.5e-3 Reiher, 5.3e-4
  against 7.2e-3 Li); `sos_const`; and the fit norm against its pickled value (1e-4);
- dense proofs on 3 random 3-orbital instances: the generators' LCUs, built from Givens unitaries,
  give `H_spec` (7e-14); the ordered products give `identity + sum c_t M_t` and `lambda_flat`; the
  Chebyshev block encoding gives `H_spec - E_SOS - Lambda` (7e-14); `H_spec - E_SOS >= 0`; the
  authors' construction equals `H` plus the fit on the sector (1.4e-14) and differs off it;
- the certificates' determinant energies by two independent evaluations (section 6.1), both
  checked against a dense Fock-space evaluation of a random `sos-sa` payload.

**The committed files are pinned** by the SHA-256 values in `specs/INDEX.json` (`spec_json_sha256`,
`payload_sha256`) and in each `spec.json` (`payload.sha256`). The loader refuses a mismatch and
recomputes the exact values from the payload (section 4).

**Tests in this repository** (`cargo test`):

- `tests/sa_spec.rs`: both pinned operators load with the published Lambda; a tampered payload is
  refused; and, ignored in release, the full lane maps and the smallest-width scan of section 5.4.
- `tests/sa_nested.rs`: the hand-built circuit, six mutants, brute-force rounding error.
- `tests/sa_dense.rs`: dense lane semantics.
- `tests/sa_report.rs`: the scoring view and certificate binding.
- `tests/sa_spin_swap.rs`: section 11.
- `tests/sa_estimated.rs`: the estimated class (section 14.8).

## 9. Interfaces

- **Spec ids** `reiher-sa-est-v1`, `li-sa-est-v1` (challenge tracks) and `reiher-sa-v1`, `li-sa-v1`
  (twins); **encoding** `"sos-sa"`; **lane map** `sa-nested-alias-v1`; **taxonomy 1.3.0** axes
  `encoding = sos-sa`, `lane_map = sa-nested-alias-v1`, `select = givens-sa-nested`,
  `rotation = phase-gradient-givens`, and any lookup except `none`
  (`taxonomy/taxonomy.json`).
- **Walk dispatch.** `src/walk/mod.rs` builds for the spec named by `FEMOCO_WALK_SPEC` (default
  `reiher-sa-est-v1`) with the architecture named by `FEMOCO_WALK_ARCH` (default `sa-low2025`),
  matching on the pair (architecture, `spec.encoding()`).
- **Circuit contract.** Declare `u = map.uniform_bits()`. The inner register is
  `b.inner_register(map.outer_bits(), map.inner_width())`. Emit the inner block with
  `b.nested_inner(reg, |b| ...)`, identical op for op in both passes. Read a pass flag for the
  one-body adjoint. Control-0 lanes must be the identity. Givens angles at `spec.beta` bits.
- **Costs the harness charges that differ from Low et al.'s.** `2(beta - 2)` per Givens against
  their `2 beta` (section 6.2). The controlled spin swap is the `SpinSwap` op of section 11, at
  `N + 1` Toffolis against their `N`. The outer and inner reflections are charged `u - 2` and
  `w - 2`, against their `walk_reflection_cost` and `t2_block_encoding_reflection_cost`.
- **Keep bits.** On `*-sa-est-v1`: tables that resolve at least 9 bits in Low et al.'s sense
  (section 14.4); the authors' 9 + 9 keep bits pass. On `*-sa-v1`: at least 38 (Reiher) and 42 (Li)
  total keep bits (section 5.4).
- **Score.** `score.json` `metrics.spectral_amplification` (section 6.2) and, on `*-sa-est-v1`,
  `metrics.rounding_class` (section 14.5).

## 10. Limits

- The SDP lower bounds of the paper's Table II are not recomputed. The gaps here are variational
  upper bounds only.
- Measured results live in the challenge ledger (`results.tsv` at the challenge root, written by the judge). The evaluator's own run log is local (`run/results.tsv`).

## 11. `SpinSwap` / `SpinSwapDg` (op kinds 32 / 33)

Low et al.'s SELECT picks the spin sector of a generator with a controlled swap of the two spin
registers (Fig. 2; their cost script charges `N` Toffolis per swap, `2N` per SELECT). This harness
lets no ordinary gate touch two system qubits, so without a dedicated op the only ways to pick a
sector are a layer of `N` Givens reading `pi / 2` from a register (`2 (beta - 2)` Toffolis each) or
running every network on both spins (`2 (N - 1)` more Givens per SELECT). Both cost about 6,000
(Reiher) / 7,900 (Li) Toffolis per step more than their swap. `SpinSwap` is that primitive, defined
for `sos-sa` specs only.

- **Ops.** Kind 32, `SPIN_SWAP` (`F`), and kind 33, `SPIN_SWAP_DG` (`F^dagger`); operands:
  `q_target` = the control qubit (non-system, required); `c_condition` allowed; everything else
  banned. `Builder::spin_swap(c)`, `Builder::spin_swap_dg(c)`.
- **Semantics.** On a lane where the op executes and `c` is 1, the system register is acted on by
  `F = prod_{p < N} G_{2p, 2p+1}(pi / 2)`, the Givens rotation at angle `pi / 2` between the two
  spin orbitals of every spatial orbital. The simulator runs exactly the Givens path with the angle
  register replaced by the constant `2^(beta - 2)` (`TrackerFactory::quarter_turn`;
  `3 x 2^(beta - 2)`, i.e. `-pi / 2`, for `SpinSwapDg`); nothing new is tracked. `F` maps each
  spin-0 mode to its spin-1 partner (up to one common sign), so `F^s V M V^dagger F^-s` is the
  spin-`s` rotated operator. `F^dagger` equals `Z_S1 F Z_S1` (`Z_S1` a `Z` on every spin-1 qubit),
  so it could be written with Cliffords, but those two Pauli frames per use push the Gaussian
  tracker past its 512-factor limit on FeMoco; a separate op kind avoids that and changes no charge
  (`F^dagger` is the same block swap times other parity `Z`s).
- **Charge: `N + 1` Toffolis per executed op** of either kind (`score::spin_swap_toffoli`),
  whatever the value of `c`, like any gate. Why: `F` is the fermionic swap of the two spin blocks
  times one parity `Z` per pair (`tests/sa_spin_swap.rs`, dense, N = 2, 3). In the spin-blocked
  Jordan-Wigner order that swap is the qubit swap of the two blocks times `(-1)^(N_up N_down)`
  (same test), so the controlled version is `N` controlled swaps (one Toffoli each) plus one `CCZ`
  on the two block parities, which CNOT ladders compute in place and undo. The Jordan-Wigner order
  is a bookkeeping choice here: every other charge in the harness is order-independent (a `Givens`
  is charged `2 (beta - 2)` for any mode pair; controlled Majorana strings are Clifford in any
  order), so a circuit costs the same in the blocked order. In the interleaved order the same swap
  would be `N` controlled fermionic swaps of adjacent qubits, 2 Toffolis each; the blocked order
  is the cheaper honest implementation, and it is the one Low et al.'s count assumes (their `N`
  per swap; the `+1` for the fermionic sign is added here).
- **Scope.** `score::evaluate` compiles with `compile_sa` only when the spec's encoding is
  `sos-sa`; `compile` and `compile_nested` reject ops 32 and 33 with "SpinSwap is defined for
  sos-sa specs only". `SpinSwap` counts as a system op in the circuit facts. `score.json` of a run
  that executes it gains `metrics.executed.spin_swap` and `metrics.spin_swap_toffoli`. It needs no
  phase-gradient register.
- **Tests.** `tests/sa_spin_swap.rs`: the scope and shape rules; the walk with `SpinSwap` passes,
  and the same stream with each `SpinSwap` rewritten as `N` register-read Givens passes too, with
  exactly the expected Toffoli difference; dropping the closing swap is rejected; the two dense
  identities above.
- **Code.** `src/circuit.rs` (kinds 32, 33 and their operand shape), `src/circuit/builder.rs`,
  `src/sim/compile.rs` (`compile_sa`), `src/sim/lanes.rs` (execution), `src/sim/tracker.rs` and
  `src/sim/gaussian.rs` (`quarter_turn`), `src/score.rs` (the charge), `src/score/report.rs`.

## 12. `sos-ground-cs-v1`: a rigorous ground-energy rounding bound

A second rigorous way to judge a lane map's rounding, applied **only** to an `sos-sa` spec whose
`spec.json` carries a `rounding` block. **No shipped spec declares it**: the four specs here are
judged by the 1-norm rule (section 5.3) or the estimated class (section 14). The trusted code that
implements the rule remains (`src/lanemap/sa_nested/ground.rs`, `src/spec/sa.rs`, `src/score.rs`),
and this section is its reference.

### 12.1 The bound

The lane map's tables make the circuit encode (section 5.2)
`A = lambda_decl sum_alpha p^_alpha (2 O^_alpha^dagger O^_alpha - 1)`, `O^_alpha = sum_x q^_x M_x`,
so `A + lambda_decl = H~' = sum_alpha O~_alpha^dagger O~_alpha` with
`O~_alpha = sqrt(2 lambda_decl p^_alpha) sum_x q^_x M_x`. The spec's operator is
`H' = H_spec - E_SOS = sum_alpha O_alpha^dagger O_alpha` (section 3). The rounded operator is still
a sum of squares. Write `Delta_alpha = O~_alpha - O_alpha`.

**Theorem.** Split the generators into sets `A` and `B`. Let
`D_A = sum_{alpha in A} ||Delta_alpha||^2`,
`L_B = sum_{alpha in B} ||Delta_alpha|| (||O~_alpha|| + ||O_alpha||)`, and `G >= E0(H')`, where `E0`
is the lowest eigenvalue on the certificate's sector (N electrons, any spin). Then

`|E0(H~') - E0(H')| <= 2 sqrt(G D_A) + D_A + L_B`.

*Proof sketch.* For `alpha` in `A` and any `t > 0`,
`(sqrt t O - Delta / sqrt t)^dagger (sqrt t O - Delta / sqrt t) >= 0` gives
`O~^dagger O~ <= (1 + t) O^dagger O + (1 + 1/t) Delta^dagger Delta`, and the same with `O` and `O~`
exchanged. For `alpha` in `B` the triangle inequality gives
`+-(O~^dagger O~ - O^dagger O) <= ||Delta|| (||O~|| + ||O||)`. Every `O^dagger O` is positive
semidefinite, so summing gives `H~' <= (1 + t) H' + (1 + 1/t) D_A + L_B` and the same with the two
operators exchanged. Both operators conserve the electron number (the rule requires the two inner
items of a one-body table to have equal counts, so a one-body `O~` stays a multiple of `a(u)` or
`a^dagger(u)`), so the inequalities hold on the sector and, by min-max, for its lowest eigenvalue.
With `0 <= E0(H') <= G` both directions are at most `t G + (1 + 1/t) D_A + L_B`, smallest at
`t = sqrt(D_A / G)`.

- It bounds the ground energy, which is what phase estimation of the ground state reads. It is not
  an operator-norm bound on `H~ - H_spec`, which is what the 1-norm rule gives; that is why it is a
  separate rule and not a relaxation of the 1-norm rule.
- It is rigorous given the certificate's `E_up` (section 6.1); `G = E_up - E_SOS`.
- Arithmetic is exact: `||Delta_alpha|| <= sum_x mult_x |q~_x - a_x|` (`mult` 2 for a square's spin
  item), square roots bracketed by integer square roots at `2^-128`, and the decision
  `D_A + L_B <= budget` and `4 G D_A <= (budget - D_A - L_B)^2`. The partition into `A` and `B` is
  chosen by a float scan; any partition is valid.

### 12.2 Where it enters the harness

- `spec.json` `rounding`: `rule = "sos-ground-cs-v1"`, `budget.exact`. The loader (`src/spec/sa.rs`,
  `ground_rule`) accepts a budget in `(0, 1/4000]` Ha only and reads `E_up` from the spec's own
  `certificate.json`. A spec may not declare both this rule and an estimated rounding class.
- `SaSpec::rounding` is `Rounding::OneNorm` (no block: all four shipped specs) or
  `Rounding::Ground { budget, e_up }`.
- `LaneMap::ground_bound` is a defaulted trait method (`None`); only `sa-nested-alias-v1` answers
  it.
- `score::check_lanemap`, only when the spec declares the rule: computes the bound, rejects above
  the budget, and otherwise returns the bound as `rounding_error` (the lambda_eff view then adds
  2 x the bound). The 1-norm is also computed, for the report only.
- `score.json` `metrics.rounding_rule` (such runs only): the rule, class, bound, budget, `G`,
  `D_A`, `L_B`, the generators in `B`, and the 1-norm.

### 12.3 The budget cap

The cap of 1/4000 Ha (0.25 mHa) comes from fitting a worst-case coefficient error inside Low et
al.'s total. Their App. D, Eq. (D1): `sqrt(sigma_PEA^2 + sigma_trunc^2) + eps_th <= 1.6 mHa` with
`sigma_PEA = 1.0`, `eps_th = 0.3` and `sigma_trunc^2 = sigma_coeff^2 + sigma_rot^2`. Keeping their
phase-estimation and rotation terms and replacing the coefficient estimate by a worst-case bound
that adds linearly, as a bias:

`eps_th + eps_coeff + sqrt(sigma_PEA^2 + sigma_rot(b)^2) <= 1.6 mHa`, so
`eps_coeff <= 1.3 - sqrt(1 + sigma_rot(b)^2)`.

At the authors' rotation bits (`sigma_rot` = 0.3078 / 0.2939 mHa, their fit, an estimate) the
allowance is 0.25371 (Reiher) and 0.25771 mHa (Li) (derived). Under this rule only the coefficient
rounding becomes worst-case; the rotation and factorization terms stay judged by the authors'
estimates.

## 13. Tapered rotation widths

An `sos-sa` spec may declare a **width schedule** in a `rotation_widths` block of `spec.json`:
rotation `j` of every Givens network (chain position `j`, modes `(j, j + 1)`) is stored with only
`w_j` bits, and a `Givens` at that position costs `2 (w_j - 2)` Toffolis instead of
`2 (beta - 2)`. **No shipped spec declares a schedule**, so every `Givens` on the four specs here is
charged `2 (beta - 2)`. The trusted code remains and this section is its reference.

- **Schema and loader** (`src/spec/sa.rs`, `rotation_widths`). `widths`: `N - 1` integers,
  `3 <= w_j <= beta`, whose largest is `beta` (so the phase-gradient register, `beta` qubits, is as
  wide as the widest rotation). `sa.bin` is unchanged (angles as `beta`-bit integers). The loader
  proves that every network's angle at position `j` has its low `beta - w_j` bits zero, and
  otherwise refuses the spec. `SaSpec::widths` is `Some(schedule)` only for such a spec.
- **Tracker** (`src/sim/gaussian.rs`, `tapered_factory`). A `Givens` on system modes
  `(2 j + s, 2 j + 2 + s)` (spin `s`, chain position `j`) reads its angle register at scale `w_j`:
  `theta = 2 pi (a mod 2^w_j) / 2^w_j`. Every other `Givens` keeps `beta`, including `SpinSwap`'s
  quarter turns. The lane is checked against the reference as before, so a register that does not
  hold the top `w_j` bits of the spec's angle fails validation.
- **Charge.** `TrackerFactory::givens_charge(p, q)` is `Some(2 (w - 2))` for a tapered tracker
  only; `score::evaluate` sums it per executed `Givens` for `C_step` in that case and uses
  `2 (beta - 2) x executed Givens` otherwise. `score.json` gains
  `metrics.rotation_widths.givens_toffoli_per_step` for tapered runs only. A `w`-bit rotation adds
  into the top `w` qubits of the phase-gradient register, at `w - 2` Toffolis per addition (Lee et
  al. 2021, App. C).
- **Why the charge cannot be gamed.** A single `Givens` at scale `w` reaches only multiples of
  `2 pi / 2^w`, so a finer spec angle cannot be matched at a coarser position, and composing two or
  more costs at least `4 (w_min - 2)` Toffolis, above one `2 (beta - 2)` Givens whenever
  `2 w_min - 2 >= beta`.
- **Tests.** `src/sim/gaussian.rs` (`taper_tests`): a tapered position rotates exactly as the plain
  tracker does at `a << (beta - w)`; bits above the width are ignored; non-chain pairs and quarter
  turns keep `beta`; the charge is `2 (w - 2)`; untapered factories report no per-position charge.

## 14. The estimated rounding class (Low et al. 2025, App. D)

This is the acceptance standard of the two challenge tracks, `reiher-sa-est-v1` and `li-sa-est-v1`.

**Standing of every result under it: "estimated rounding error (Low et al. class)", an estimate
adopted from the authors, not a bound.** A run accepted here says "within Low et al.'s estimated
truncation budget". It never says "within 0.1 mHa".

Code: `src/spec/rounding.rs` (the class, its constants, the bit rule),
`LaneMap::rounding_estimate` in `src/lanemap/sa_nested.rs` (the exact structural check),
`score::check_lanemap` in `src/score.rs` (the decision), `src/score/report.rs` (the report).

### 14.1 Scope: which specs, and how a spec declares the class

- A spec is of the **rigorous** class unless its `spec.json` carries a `rounding_class` block.
- `reiher-sa-est-v1` and `li-sa-est-v1` carry the block, with `class = "estimated-low2025"`. Each
  states the same operator as its twin (`reiher-sa-v1`, `li-sa-v1`): the same `sa.bin`, `Lambda`,
  `identity`, `E_SOS`, `lambda_flat` and published `lambda_eff`. Only the rounding class differs.
- The class cannot be declared freely. The loader refuses a `rounding_class` block
  - whose `class` is not `"estimated-low2025"`;
  - on a spec id that is not in the harness's pinned table (`rounding::pinned`);
  - whose constants (`coeff_const_fits`, `rot_const_fits`, `sigma_trunc_budget_mHa`) differ in any
    way from the harness's pin for that id;
  - on a spec whose encoding is not `sos-sa`;
  - on a spec that also declares a `rounding` rule (section 12).
- The class and its constants therefore come from the trusted spec and the harness, never from the
  submitted lane map.
- `src/spec/rounding.rs` also recognises further spec ids, with a variant bit rule, that are not
  shipped in this repository; nothing here depends on them.

### 14.2 The acceptance rule

A lane map submitted against `reiher-sa-est-v1` or `li-sa-est-v1` is accepted when all of the
following hold. Items 1 to 3 are exact checks; item 4 is the estimate.

1. **Shape.** It parses as `sa-nested-alias-v1` against the spec (section 5.1) and `u <= 63`.
2. **`lambda_decl = Lambda`, exactly.** Any other value is refused ("lambda_decl ... must equal the
   spec's Lambda ... exactly").
3. **Every count is the floor or the ceiling of its ideal value.** For every alias table (the
   outer one and all `N + RC` inner ones), with `T = 2^(k + mu)` lanes and weights `w_i`, the count
   `n_i` of every item satisfies `|n_i sum_j w_j - T w_i| < sum_j w_j`, decided in exact rationals.
   That is `|n_i - T w_i / sum w| < 1`: the count is `floor` or `ceil` of the ideal, and equals it
   when the ideal is an integer. The weights are fixed by the spec:
   - outer table: `2 |e_r|` for one-body item `r`, and `S_rc^2 / 2` for square `(r, c)`,
     `S_rc = |wB_rc| + sum_b |w_rcb|`;
   - inner table of a one-body item: `(1, 1)`, so both items hold exactly half the lanes;
   - inner table of a square: `(|w_rc0|, ..., |w_rc,B-1|, |wB_rc|)`.

   This is the support of the authors' randomized rounding (their `M_b = floor(M |w_b| / |w|_1)`
   plus at most one leftover bin per item). A table of zero total weight is refused.
4. **The bit rule (the estimate).** The authors model the standard deviation of the CCSD(T)
   correlation-energy change under unbiased randomized rounding to `b` bits as
   `sigma(b) = 2^(const - b)` mHa, and take the smallest integer `b` with
   `sigma(b) <= split = 0.83 / sqrt 2` mHa, i.e.

   `b_required = ceil(log2(1 / split) + const)`.

   This is their cost script's even split of the truncation budget `sigma_trunc <= 0.83` mHa between
   coefficients and rotations. The lane map is accepted when
   - `b_coeff >= b_required(const_coeff)`, where `b_coeff` is the resolution of the alias tables
     (section 14.4), and
   - `beta >= b_required(const_rot)`, where `beta` is the spec's rotation bits. The lane map has no
     influence on this; it holds for both specs by construction (16 and 15 are the authors' own
     values).

Nothing else about rounding is required. In particular the exact 1-norm `rounding_error` of
section 5.3 is computed and reported but is **not** required to be at most 0.1 mHa.

The checks run in this order: `u <= 63`; the exact 1-norm; items 2 and 3 (first failure reported);
then item 4. The circuit itself is then validated lane by lane exactly as on any other spec: the
class changes only how the lane map's rounding is judged.

### 14.3 Pinned constants

`const` is the mean of the authors' three fitted values per molecule (their cost script
`get_walk_counts_different_bits_adjustments_for_paper_revision.py` in the record, lines 112-143;
App. D, Figs. 6-7: fits over 3 solutions x 64 randomized roundings). `target_error = 0.83` mHa is
their truncation budget (App. D: "leaves a budget of sigma_trunc <= 0.830 mHa").

| | `reiher-sa-est-v1` | `li-sa-est-v1` |
| --- | ---: | ---: |
| `coeff_const_fits` | 7.2, 7.9, 7.9 | 8.6, 8.3, 7.5 |
| `const_coeff` (mean) | 7.667 | 8.133 |
| `rot_const_fits` | 14.0, 14.3, 14.6 | 13.3, 13.3, 13.1 |
| `const_rot` (mean) | 14.3 | 13.233 |
| `sigma_trunc_budget_mHa` | 0.83 | 0.83 |
| `split` = budget / sqrt 2 (mHa) | 0.587 | 0.587 |
| **required `b_coeff`** | **9** | **9** |
| **required rotation bits** | **16** | **15** |
| the spec's rotation bits `beta` | 16 | 15 |
| `sigma_coeff` at 9 bits (mHa) | 0.3969 | 0.5484 |
| `sigma_rot` at `beta` (mHa) | 0.3078 | 0.2939 |
| `sigma_trunc` at (9, `beta`) (mHa) | 0.502 | 0.622 |

The required bits reproduce the script's own output, "Coeff bits: 9.00 Rot bits: 16.00" (FeMoco54)
and "9.00 / 15.00" (FeMoco76). The sigma rows are derived from `sigma(b) = 2^(const - b)`;
`sigma_trunc = sqrt(sigma_coeff^2 + sigma_rot^2)`.

### 14.4 How the keep bits are judged

The authors' `b` coefficient bits mean `M = L 2^(b - 1)` bins over `L` items. An alias table of
width `(k, mu)` has `T = 2^(k + mu)` lanes. Its resolution in their sense is

`b_equiv(L, k + mu) = 1 + max{ j : L 2^j <= 2^(k + mu) }`, and `0` if `L > 2^(k + mu)`:

the largest `b` whose `L 2^(b - 1)` bins are no finer than the table's lanes. The harness uses

- `b_equiv_outer = b_equiv(N + RC, k_o + mu_o)`, with `N + RC` = 324 (Reiher) and 361 (Li);
- `b_equiv_inner = b_equiv(B + 1, k_i + mu_i)`, with `B + 1` = 28 (Reiher) and 58 (Li). The
  one-body inner tables round nothing (two equal items), so the squares' tables set this value;
- `b_coeff = min(b_equiv_outer, b_equiv_inner)`.

So what is judged is the lane count `k + mu` of each level against its item count, not `mu` alone.
With the index widths the items need (`k_o = 9`, `k_i = 5` Reiher / `6` Li):

| keep bits `mu_o + mu_i` | `b_equiv` outer / inner | `b_coeff` | decision |
| --- | --- | ---: | --- |
| 9 + 9 (the authors' keep width) | 10 / 10 | 10 | accepted |
| 8 + 8 | 9 / 9 | 9 | accepted (the boundary) |
| 7 + 9 | 8 / 10 | 8 | refused: "below the 9 Low et al.'s rule needs" |
| 9 + 7 | 10 / 8 | 8 | refused |

The table holds for both instances. At 9 + 9 the tables are one bit finer than the authors' bins
(a 9-bit keep over `2^k >= L` buckets against `L 2^8` bins), so the reported `sigma_coeff` is half
the value at 9 bits.

Two points where this class is stricter than the authors' procedure or differs from it:

- Their fit covers the inner (coefficient) tables only; they treat the outer weights as prepared
  at machine precision. This class applies the same resolution rule to the outer table too.
- Their fit is of unbiased randomized rounding. A deterministic rounding, such as the
  largest-remainder counts of `sa_nested::build`, is one realization with the same support, not a
  random draw. The class accepts any floor or ceiling rounding.

### 14.5 What `score.json` and `results.tsv` report

Every accepted run on an estimated-class spec is labelled in three places: the spec id in
`metrics.spec`, the block `metrics.rounding_class`, and the note column of the locally written
`results.tsv`, which `eval_circuit` prefixes with "estimated rounding error (Low et al. class)".
Runs on a rigorous spec have no `rounding_class` block.

`metrics.rounding_class` holds:

| field | content |
| --- | --- |
| `class`, `label`, `kind` | `"estimated-low2025"`, `"estimated rounding error (Low et al. class)"`, `"estimate, not a bound"` |
| `rule`, `rigorous_rule`, `structural_check`, `b_equiv_rule` | the rules of this section, in words |
| `rigorous_one_norm_Ha` | the exact 1-norm rounding error of section 5.3 (equal to `metrics.rounding_error`) |
| `rigorous_rule_met` | whether that 1-norm is at most 0.1 mHa (`false` at the authors' keep bits) |
| `tables_checked` | tables checked for floor or ceiling counts: `1 + N + RC`, all of them |
| `outer_bits_k_mu`, `inner_bits_k_mu` | the lane map's widths |
| `b_equiv_outer`, `b_equiv_inner`, `b_coeff`, `b_coeff_required` | section 14.4 |
| `beta`, `beta_required` | the spec's rotation bits and the rule's requirement |
| `sigma_coeff_mHa`, `sigma_rot_mHa`, `sigma_trunc_mHa` | `sigma` at `b_coeff` and at `beta`, and their quadrature sum |
| `budget_mHa`, `split_mHa` | 0.83 and 0.83 / sqrt 2 |
| `constants` | the three fits and their means for coefficients and rotations, and their source |
| `accepted` | `true` |

Effects elsewhere in `score.json`:

- `metrics.rounding_error` and `rounding_error_exact` remain the exact 1-norm, not an estimate.
- **The lambda_eff bound stays rigorous.** `lambda_eff_ours` (section 6.2) still carries an
  allowance of twice the exact 1-norm, which is large under this class. With `lambda_decl = Lambda`
  the bound is `sqrt(E (2 Lambda - E))` at `E = E_gap_upper + 2 x rounding_error`; its value
  depends on the lane map's tables, and the score uses the larger of it and the published
  lambda_eff (section 6.2). More keep bits lower the 1-norm and with it this bound, at the cost of
  wider tables.

### 14.6 What the estimate does not cover

- **It is fitted, not proved.** `sigma(b)` is a fit to the standard deviation of the CCSD(T)
  correlation-energy change. The mean change (`eps_trunc`) is assumed to be 0.
- **It does not include the Hartree-Fock energy change.** The record holds the
  randomized-rounding statistics behind the paper's Fig. 6 (64 roundings per bit count for the
  three FeMoco54 solutions, `spinfree/best_solutions/rounded/`). For the published (10, 27, 27)
  file at 9 coefficient bits (derived from that file): the standard deviation of the
  correlation-energy change is 0.485 mHa, which the fit models; the standard deviation of the
  Hartree-Fock energy change is **19.0 mHa**, and at 16 rotation bits it is 5.2 mHa. The authors'
  error model counts only the first. That is sound only if the mean-field shift of the rounded
  Hamiltonian is removed classically. It can be, because the rounded operator is known exactly, but
  a phase-estimation run without that correction is off by about 20 mHa, not 0.5. This caveat
  applies to every result in this class.
- **It says nothing about the operator norm.** At the authors' keep bits the exact 1-norm is
  7.97e-2 Ha (Reiher) and 2.83e-1 Ha (Li), about 800 and 2,800 times the rigorous rule's limit.
- **The constants are means over three solutions** per molecule, not properties of the one
  solution each spec holds.
- The approximation of FeMoco's Hamiltonian by the DFTHC operator is a separate question
  (sections 7.2 and 7.3) that this class does not address.

### 14.7 Contrast with the rigorous rule of the twins

`reiher-sa-v1` and `li-sa-v1` state the same operators and accept a lane map only if its exact
1-norm rounding error `sum_t |c_t - c^_t|` is at most 0.1 mHa. That is a proved statement: the
operator the circuit block-encodes differs from the spec's by at most twice that in operator norm
(section 5.3), on every state, with no fitted constant and no assumption about a classical
correction. It costs about 18 to 20 keep bits per alias level (38 and 42 in total) where the
estimated class accepts 8 or 9, and `lambda_decl` may differ from `Lambda` as long as the 1-norm
holds. The estimated class replaces the proved statement by the authors' own procedure so that a
circuit can be compared with their Table V costs at their precision. The two classes are different
standings under different spec ids: a result on a `*-sa-est-v1` spec must not be reported as
meeting the 0.1 mHa rule, and results of the two classes must not be ranked together.

### 14.8 Tests (`tests/sa_estimated.rs`, `src/spec/rounding.rs`)

- Every spec in `specs/INDEX.json` has the class its id is pinned to, and the twins state the same
  operator (same payload hash and exact values).
- The class is refused off its pin: altered constants, an unknown class name, the block on a
  rigorous spec id or a new id, and the block on a non-`sos-sa` spec.
- The rigorous twins still reject the authors' 9 + 9 keep bits ("exceeds 0.1 mHa").
- The bit rule: 9 + 9 gives `b_equiv` (10, 10) and passes; 8 + 8 passes; one bit fewer on either
  level fails; `sigma_coeff` at 9 + 9 is half the authors' value at 9 bits.
- A lane map that is not a rounding is refused: two lanes moved from an item to its alias ("not
  the floor or the ceiling"), and `lambda_decl` off `Lambda` by one unit of a fine dyadic. The
  largest-remainder counts of `sa_nested::build` pass.
- End to end on FeMoco: a walk at 9 + 9 passes both specs and its `score.json` carries the block
  above with `rigorous_rule_met = false`; a run on a rigorous spec gains no block; a run at too
  few keep bits is rejected with "estimated rounding class".
- `rounding.rs` unit tests: the required bits come out as 9 / 16 and 9 / 15, `sigma_trunc` as
  0.502 / 0.622, and `b_equiv` on the item counts above.
