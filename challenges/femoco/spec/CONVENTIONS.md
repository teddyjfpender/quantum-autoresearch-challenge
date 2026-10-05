# Conventions for frontier claims

This document fixes one accuracy convention and one qubit convention for any claim that a circuit
here sits on, beats or meets a published FeMoco resource estimate. The views below are reported
beside the benchmark's score and never replace it. The code is the `conventions` module of
`src/score/report.rs`, which writes `metrics.conventions` and, for the spectrum-amplified specs,
`metrics.spectral_amplification` in `score.json`. The ground-energy certificates are
`specs/<id>/certificate.json`. `tests/sa_report.rs` re-implements the formulas and checks that
each certificate is bound to its spec.

This is circuit resource research: compiled, checked walk steps and their counts. Nothing here is
a claim of quantum advantage.

## 1. The views

Every view uses the run's measured `C_step` (Toffolis per controlled walk step) and peak qubits
`Q_peak`, and a step count `I = ceil(pi lambda_view / (2 eps_PEA))`. Totals are `I x C_step`.

| View | `score.json` key under `metrics` | lambda | eps_PEA | What it is |
| --- | --- | --- | ---: | --- |
| (a) controlled cost comparison | `conventions.worst_case.controlled` | worst-case lambda | 1.6 mHa | The whole 1.6 mHa to phase estimation. Equals `total_toffoli_lit`. Not any paper's convention. |
| (b) chemical-accuracy resource estimate | `conventions.worst_case.chemical_accuracy` | worst-case lambda | 1.0 mHa | Lee et al. 2021's split: 1.0 mHa to phase estimation, 0.6 mHa reserved for the approximation of the Hamiltonian, judged by an **estimate** (below). |
| (c) the same two at lambda_eff | `conventions.lambda_eff.controlled`, `.chemical_accuracy` | lambda_eff | 1.6 / 1.0 mHa | The phase slope at a certified bound on the ground energy (section 3). |

For an `sos-sa` spec the worst-case lambda is the walk normalization `Lambda`, the run's
`lambda_decl`. `metrics.spectral_amplification` repeats view (c) for three values of lambda_eff:
the certificate's (`ours`), the paper's (`published`) and the larger of the two (`used`), which is
the one the score uses (section 3).

Source of (b): Lee et al. 2021 (arXiv:2011.03494v3), p. 14, after Eq. (25): "We will take
ε_pea ≤ 0.001 Hartree and ε_thc ≤ 0.0006 Hartree", with ε = 0.0016 Hartree "(chemical accuracy)",
and p. 20, Eq. (45): "I = ⌈πλ/(2ε_PEA)⌉". Low et al. 2025 (Phys. Rev. X 15, 041016;
arXiv:2502.15882) use the same split (Table V caption: "In line with previous work we take σPEA =
1.0mHa").

**The approximation budget is judged by an estimate, not a bound.** View (b) shows, next to its
total, the spec's approximation-error estimate: the CCSD(T) correlation-energy error that Low et
al. publish for the DFTHC Hamiltonian (Table V, `eps_corr`), 0.1254 mHa (Reiher) and 0.1362 mHa
(Li), copied from the certificate with `kind: estimate` and not recomputed here, with
`within_budget_by_estimate` against 0.6 mHa. The rigorous statements the specs also carry are
copied beside it: the DFTHC fit bound (854.8 Ha Reiher, 284.6 Ha Li) and the rotation-angle
quantization bound (0.053 Ha and 0.368 Ha). They are orders of magnitude above the budget. Nothing
here bounds the approximation error at chemical accuracy.

**Rules.** Compare totals only within one view. A lambda_eff total is never compared with a
worst-case one. The step cost `C_step` omits the one Toffoli per step that Lee et al. charge for
the unary iteration over the phase register (p. 13: "The unary iteration procedure introduces a
trivial additional cost of one Toffoli per step"): a relative under-count of `1 / C_step`, about
1e-4 for a step of ten thousand Toffolis.

## 2. Qubits

**Convention:** `qubits_lee = Q_peak + 2 ceil(log2(I + 1)) - 1`, with `I` the view's step count.

Lee et al. 2021 count the phase-estimation control register and the unary iteration over it in
every logical-qubit total (arXiv:2011.03494v3). For THC, p. 20, item 1: "The control register for
the phase estimation needs ⌈log(I + 1)⌉ qubits. The unary iteration to control on this register
needs another ⌈log(I + 1)⌉ − 1 qubits." The total, p. 21, Eq. (46), begins
"2⌈log(I + 1)⌉ + N + 2nM + i + ...". Their sparse and double-factorization totals (Eqs. (A18) and
(C40)) count the same two items. Their phase-gradient register is also counted; `Q_peak` already
includes it whenever a `Givens` is used.

Low et al. 2025 itemize the block encoding only. The journal's Tables I and V print 1,132 qubits
(Reiher) and 1,454 (Li); the itemization their published cost script produces for those numbers
consists of block-encoding registers and the system qubits, with no phase-estimation control
register. (The first arXiv version printed 1,137 and 1,459, from an earlier version of the same
cost model.) The convention therefore adds `2 ceil(log2(I + 1)) - 1` to their printed counts when
comparing; such a figure is derived, not published.

`Q_peak` includes the walk's control qubit. Lee et al. control only the reflection, from the unary
iteration's output, so adding the full register may count one qubit twice on our side: conservative
by at most one qubit.

`metrics.spectral_amplification` reports `qubits_without_pe_register` (`Q_peak`) beside each
view's `qubits_lee`.

## 3. lambda_eff

A qubitized walk has eigenphases `± arccos(E / lambda)` for each eigenvalue `E` of the encoded
operator `A` (Lee et al. p. 13: "a quantum walk W which has eigenvalues proportional to
e^{±i arccos(E_n/λ)}"). Phase estimation to precision `delta` in phase gives energy precision
`delta sqrt(lambda^2 - E^2)`, so the steps needed near the ground energy scale with

`lambda_eff = sqrt((lambda + E0')(lambda - E0'))`,

Low et al. 2025 Eq. (11), "λeff = sqrt(Egap (2Λ − Egap))", in this walk's normalization
(`E_gap = Lambda + E0'`). `E0'` is the lowest eigenvalue of `A`; the worst case is `E0' = 0`,
lambda_eff = lambda. On `[-lambda, 0]` lambda_eff increases with `E0'`, so an **upper** bound on
the ground energy gives an **upper** bound on lambda_eff: a conservative total.

**E0'.** `A = H_spec - identity + offset` up to the run's rounding error, where `offset` is the
multiple of the identity the lane-map family adds (section 4.2). With `E_up` a certified upper
bound on the lowest eigenvalue of `H_spec` (section 4):

`E0' <= E_up - identity + offset + error_factor x rounding_error`.

The view is reported only when that bound lies in `(-lambda, 0]`, and only for lane-map families
the certificate covers; otherwise `lambda_eff.available` is false with the reason.

**For the `sos-sa` specs** the ground state sits near the bottom of the walk's spectrum, so
lambda_eff is far below `Lambda`: with `E_gap <= E_up - E_SOS`, the certificates give lambda_eff
at most 19.99 (Reiher) and 43.61 (Li), against `Lambda` 58.34 and 179.73. These values hold at
`lambda_decl = Lambda` with no rounding error; a run's own bound adds its offset and
`2 x rounding_error` and is slightly larger.

**Two bounds, and the one a claim uses.** Low et al. publish lambda_eff 21.3674 (Reiher) and
43.6538 (Li) in Table V. Their `E_gap` is the Hartree-Fock energy of their DFTHC Hamiltonian
minus their `E_SOS`: also a variational bound on the same operator's sector ground energy,
evaluated on their float32 factors. On Reiher the certificate's determinant is 0.523 Ha lower
than the energy their gap implies, so its gap is smaller; on Li the two nearly coincide (5.3708
against 5.3820). **A certified claim uses the larger of the two.** The score does the same:
`score = lambda_eff_used x C_step x Q_peak` with `lambda_eff_used = max(ours, published)`, where
"ours" is the bound above at the run's `lambda_decl` and rounding error. If the certificate gives
no bound for a run, the score uses the worst-case `Lambda`.

Low et al.'s worst-case views use their block-encoding normalization Λ (Table V: 58.3440 and
179.7296), the worst-case slope constant of their walk (Eq. (11) at `E_gap = Λ`). Their published
totals are lambda_eff totals; the views here keep the two apart.

## 4. Ground-energy certificates (`specs/<id>/certificate.json`)

`E_up` is the energy of a pinned Slater determinant under the spec's own operator `H_spec`
(identity included; the operator the walk block-encodes, read from the committed payload), plus a
stated floating-point allowance.

- **Why it is an upper bound.** Any normalized state's energy is at least the lowest eigenvalue of
  `H_spec` in its symmetry sector (variational principle). The determinant has the instance's
  electron count (Reiher 54, Li 113) and the smallest `|S_z|` (27 + 27 and 57 + 56 electrons).
  `H_spec` conserves N and S_z and is spin-free, and the S_z = 0 (1/2) sector holds a component of
  every integer (half-integer) spin multiplet, so `E_up` bounds the lowest N-electron eigenvalue
  of `H_spec` over all spins. It does not bound the ground energy of the exact (unapproximated)
  Hamiltonian, and it says nothing about the overlap of any initial state.
- **The state.** Unrestricted Hartree-Fock on `H_spec` itself, from ten starting points (the
  native orbitals, the core orbitals with a broken-symmetry twist, and eight seeded random
  rotations); the lowest is kept. The
  occupied orbitals are stored as base64 float64. The state is the determinant spanned by those
  columns, whose 1-RDM is exactly `C (C^T C)^-1 C^T`, so the bound does not depend on the SCF
  having converged or on the orbitals being exactly orthonormal. `<S^2>` is recorded (about 3.0 on
  Reiher and 14.4 on Li: broken-symmetry determinants), for information only.
- **"Exact Hartree-Fock energy".** `E_det` is the exact energy of that determinant, evaluated in
  float64. It is not claimed to be the global Hartree-Fock minimum; a lower determinant would give
  a tighter (smaller) lambda_eff.
- **Floating point.** The generator and an independent checker evaluate `E_det` by different
  derivations and must agree to 1e-6 Ha. `E_up = E_det + 1 mHa`, rounded up to 1e-6 Ha. The 1 mHa
  margin is a stated allowance, above a crude a-priori summation bound recorded per spec
  (`summation_bound`, 1.8e-5 Ha on Reiher and 6.8e-6 Ha on Li). It is not an interval-arithmetic
  proof.

### 4.1 Two derivations

The certificates were generated and checked by two scripts that are not part of this repository;
each certificate records the generator's hash. The generator rewrites `H_spec` in spin-free form,
`H = c + sum h_pq E_pq + 1/2 sum g_pqrs E_pq E_rs`, and evaluates the determinant with Coulomb and
exchange contractions. The checker has its own payload reader and evaluates the definition term by
term with Wick's theorem. On the two `sos-sa` operators they agree to 1.6e-11 Ha (Reiher) and
3.2e-12 Ha (Li). Both were also checked against a dense Fock-space evaluation of a small random
`sos-sa` payload.

### 4.2 Walk offsets

The encoded `A` can differ from `H_spec - identity` by a multiple of the identity that the lane
map adds. The certificate records it per lane-map family (`walk_offsets`), with the factor by
which the run's rounding error bounds the deviation. The shipped certificates have one entry:

| Family | Offset | Error factor | Why |
| --- | --- | ---: | --- |
| `sa-nested-alias-v1` | `(Lambda - lambda_flat) + (lambda_decl - Lambda)`: -39.982374 (Reiher) and -141.127207 (Li) at `lambda_decl = Lambda` | 2 | `A = lambda_decl sum_alpha p_alpha (2 O_alpha^dagger O_alpha / lambda_alpha^2 - 1) = sum_t c^_t M_t + lambda_decl - sum_t c^_t`, where `c^_t` are the lane map's ordered-product weights. With the spec's `c_t`, `A = H_spec - identity + (lambda_decl - lambda_flat) + sum_t (c^_t - c_t)(M_t - 1)`, and `\|\|M_t - 1\|\| <= 2` |

`lambda_flat` is the sum of the spec's ordered-product coefficients (`spec.json`). A run with any
other lane-map family gets no lambda_eff view. spec/SPEC-SA.md section 5 derives the counts.

### 4.3 The certificates

| Spec | E_up (Ha) | identity (Ha) | offset at `lambda_decl = Lambda` | E_up - identity + offset | Lambda | E_SOS (Ha) | E_gap upper | lambda_eff upper | <S^2> |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `reiher-sa-est-v1`, `reiher-sa-v1` | -13481.963620 | -13467.133441 | -39.982374 | -54.812 | 58.343975 | -13485.495042 | 3.531422 | 19.990083 | 3.01 |
| `li-sa-est-v1`, `li-sa-v1` | -1118.355082 | -1085.123547 | -141.127207 | -174.359 | 179.729577 | -1123.725917 | 5.370835 | 43.609057 | 14.40 |

An estimated-class spec and its rigorous twin state the same operator (the same `sa.bin`), so
their certificates hold the same state and the same bound; only the spec id and the `spec.json`
hash differ.

`E_gap upper = E_up - E_SOS`, and `E0' = E_gap - Lambda`, which is the fifth column. Each
certificate also copies the published `E_gap`, lambda_eff and `Lambda` (`spectral_amplification.
published`) and states the rule of section 3 (`lambda_eff_certified`: 21.3674 and 43.6538).

`certificate.json` records the SHA-256 of `spec.json` and of the payload, so a changed spec needs
a new certificate; `tests/sa_report.rs` fails otherwise. `src/score/report.rs` compiles the four
certificates in.

## 5. The published point under each view

Low et al. 2025's FeMoco results, derived from their published block-encoding cost (Table V:
Cost[BE] = 10,203 Toffolis for Reiher and 14,629 for Li, with 1,132 and 1,454 qubits) by
`ceil(pi lambda_eff / (2 eps)) x Cost[BE]`. These rows are derived from published numbers, not
measured here. The register column is `2 ceil(log2(I + 1)) - 1` at the 1.0 mHa step count
(section 2).

| | lambda_eff | steps at 1.0 mHa | total at their Cost[BE] | steps at 1.6 mHa | total | qubits + PE register |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reiher, published lambda_eff | 21.3674 | 33,564 | 3.4245e8 (paper: 3.42e8) | 20,978 | 2.1404e8 | 1132 + 31 |
| Reiher, the certificate's bound | 19.9901 | 31,401 | 3.2038e8 | 19,626 | 2.0024e8 | 1132 + 29 |
| Li, published lambda_eff | 43.6538 | 68,572 | 1.0031e9 (paper: 1.00e9) | 42,858 | 6.2697e8 | 1454 + 33 |
| Li, the certificate's bound | 43.6091 | 68,501 | 1.0021e9 | 42,814 | 6.2633e8 | 1454 + 33 |

The 1.0 mHa column at the published lambda_eff reproduces the paper's own totals, as it should: it
is their convention. A claim called certified uses the published rows (section 3).

**Comparing a circuit from this repository with these rows.** State four things with every claim:

1. the view (1.0 or 1.6 mHa to phase estimation, and which lambda_eff);
2. the qubit convention (with or without the phase-estimation register, on both sides);
3. the Givens charge. Low et al. charge `2 beta` Toffolis per Givens and this harness charges
   `2 (beta - 2)`. `metrics.spectral_amplification.low2025_givens_charge` gives a run's step cost
   under their charge (`C_step + 4 x executed Givens per step`); compare with their Cost[BE] using
   that figure, or say which charge the comparison uses;
4. the rounding class of the spec (spec/DESIGN.md section 13). A result under the estimated class
   is within Low et al.'s estimated truncation budget, not within a rigorous bound.

Between views (a) and (b) no ordering changes: both scale every total by the same step ratio, and
`qubits_lee` moves by at most 2.
