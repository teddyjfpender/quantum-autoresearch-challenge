# Signed correction and a combined logical walk

The signed double factorization reduces the correction's LCU normalization by
**6.60x for Reiher and 3.16x for Li** relative to the explicit sparse correction.
The base and correction share one emitted hierarchical program. Its layout, every
lookup record and its coefficient errors are checked exactly for both full
instances; that the layout block-encodes the Hamiltonian is an identity argued
below, not something the checker derives. The resource bounds cover this whole
controlled logical walk and its QPE query count. They are **far above Low et
al.'s published estimate**: 28.7x / 37.8x in total Toffolis, and 43x / 69x on the
challenge's own score for one step, Toffolis x qubits.

These circuits are not the challenge's circuits. The challenge tracks use the
published factors at the authors' own error standard; this experiment changes the
Hamiltonian to meet a proven bound, and its costs are not comparable with any
row of the ledger.

This is maintainer research tooling, not a judged circuit submission. The
combined program is executable at the logical primitive level; it has not been
flattened into the evaluator's gate stream. Primitive implementations, physical
synthesis, ground-state preparation and a complete QPE implementation remain
separate obligations. Both reports retain `chemical_accuracy_certified: false`.

## Measured construction and certified errors

The target is the exact decimal FCIDUMP Hamiltonian on the fixed electron-number
sector, with the same core policy and input hashes as the [energy audit](README.md).
The [numerical Reiher](../../rigorous/signed-reiher.json) and
[numerical Li](../../rigorous/signed-li.json) certificates use 128-bit Arb.
The [combined Reiher](../../rigorous/combined-reiher.json) and
[combined Li](../../rigorous/combined-li.json) reports bind the emitted program,
rotation words, numerical certificate and checker source by SHA-256.

| Quantity | Reiher | Li |
| --- | ---: | ---: |
| Previous sparse correction entries | 1,101,579 | 4,252,735 |
| Signed correction factors | 1,386 | 2,153 |
| Full residual matrix dimension | 1,485 | 2,926 |
| Previous correction normalization, Ha | 213.6890 | 71.1598 |
| Signed correction normalization, Ha | 32.3961 | 22.5048 |
| Previous combined normalization, Ha | 272.0330 | 250.8893 |
| New combined normalization, Ha | 90.7401 | 202.2343 |
| Base rotation/preprocessing bound, mHa | <0.050967 | <0.044918 |
| Signed factorization residual bound, mHa | <0.078630 | <0.079019 |
| Correction rotation/eigenpair bound, mHa | <0.031760 | <0.026297 |
| **Explicit combined Hamiltonian bound, mHa** | **<0.161357** | **<0.150233** |
| Actual combined coefficient-encoding bound, mHa | <0.015704 | <0.028465 |
| Conditional QPE allocation, mHa | 1 | 1 |
| **Total budget, mHa** | **<1.177060** | **<1.178698** |

Error bounds in this table are rounded upward; normalizations are display values.
Factor counts and entry counts describe different objects: each signed factor
has N eigenvalues and N rotation networks. This is not a claim of a 795x/1975x
circuit or storage reduction. The improvement comes from the second
factorization, smaller normalization, shared preparation and the lookup backend.

The operator bound also bounds the ground-energy difference on the stated sector
by the min-max principle, without assuming a ground state or gap. This bounds
the distance of the **new corrected** Hamiltonian from the target. It neither bounds
ground-energy accuracy of the original published factors nor computes a ground
energy. The remaining approximately 0.423/0.421 mHa is unspent systematic-error
margin, not a certificate for a synthesis scheme that has not been implemented.

## Representation and interval certificate

Using the audit's packed-pair operators, the residual is

```text
H_fit - H_target = R = (1/2) sum_ab D_ab A_a A_b,
||A_a|| <= k_a,     k_pp = 1, k_pq = 2 (p != q).
```

The proposal diagonalizes the real symmetric D, retains signed factors in
descending absolute eigenvalue order, and searches for an entrywise residual
below 0.08 mHa. The search stops within eight factors; it does not claim the
minimum feasible rank. For rows l_t and signs s_t in {-1,+1}, the acceptance test
recomputes, with intervals, every entry of

```text
Delta = D - sum_t s_t l_t l_t^T,
epsilon_fit = (1/2) sum_ab |Delta_ab| k_a k_b <= 0.1 mHa.
```

Map each packed l_t to a symmetric spatial matrix L_t and diagonalize it again.
For the proposed eigenvalues e_tj, encode every eigenvector by a 26-bit Givens
chain. The certificate reconstructs those actual integer-angle vectors u_tj
with Arb and checks Lhat_t = sum_j e_tj u_tj u_tj^T directly. It does not assume
the rounded vectors are mutually orthogonal or that the eigensolver was exact.
The centered spin-free generator Ghat_t has norm at most S_t = sum_j |e_tj|.
An independently computed matrix nuclear-norm upper bound delta_t on the
generator error gives

```text
||(G_t^2 - Ghat_t^2)/2|| <= S_t delta_t + delta_t^2/2.
H_new = H_base,beta - (1/2) sum_t s_t Ghat_t^2.
||H_new-H_target|| <= epsilon_base + epsilon_fit + epsilon_rot.
```

The base uses the preceding experiment's independently validated one-body
eigenpairs and scalar at 26/28 bits. Li's correction angles are padded exactly
from 26 to the walk's common 28-bit width. NumPy supplies proposals only; failure
of either interval allocation rejects the candidate. Saved proposals receive
the same checks.

Low-rank truncation alone failed the initial screening: for Reiher, retaining
512/742 residual eigenvectors left approximately 4.25/0.459 Ha entrywise bounds;
for Li, 512/1463 left approximately 12.4/0.00329 Ha. These figures come from
exploratory float64 runs that are not in the repository; they are context, not
results, and nothing here depends on them. Keeping the first-factor generators in the entry basis also
gave poor square normalization (about 2787/1783 Ha before truncation). The
successful mechanism is the **second factorization**, not a very low first rank.

## The combined implementation

`combined.py` emits `*-combined-program.json` and `*-combined-networks.npy`.
The JSON contains actual integer lookup words, signed row records, inner and
outer alias tables, conjugation instructions, erasures and controlled
reflections. The NPY contains the integer-angle networks. The program has
1,710/2,514 rows and 75,168/164,559 networks for Reiher/Li.

For a normalized orbital, Et(u) = -(Z(u,up)+Z(u,down))/2. An inner SELECT uses
the signed rotated Z, or the identity for a base scalar coefficient. All its
branches are Hermitian involutions, including negative coefficients. For a
square row with generator G and S = sum |w|, the projected SELECT is B = G/S.
Conjugating the inner uniform reflection gives

```text
<+| V (2|+><+|-I) V |+> = 2 B^2 - I.
mass = S^2/4,
signed mass * (2 B^2-I) + signed mass * I = signed G^2/2.
```

The correction's outer sign is **minus** the residual factor sign. That sign
affects both the SELECT phase and its scalar offset. Original DFTHC squares and
correction squares use the same preparation. One-body rows disable the inner
reflection and the second SELECT, and use the outer spin draw. A final reflection
on all uniform registers makes the qubitized walk. Global control disables the
SELECT phases, row signs and reflections; the conjugations cancel to identity.
The report records the complete scalar offset used in E = offset+Lambda*cos(theta).

The compiler uses 24 alias keep bits. Exact rational apportionment and integer
histograms determine every realized probability. If p_t is the realized outer
probability and delta_t is the inner probability 1-norm error, the bound is

```text
epsilon_encoding <= sum_t |Lambda*p_t - mass_t|
                    + sum_(square t) 4*Lambda*p_t*delta_t.
```

The second term follows from ||B||, ||Bhat|| <= 1, including noncommuting terms.
Each summand is rounded upward to a rational with denominator 2^128 before
summation; this avoids enormous denominator products without losing rigor.

### What is checked, and what is assumed

`combined.py` contains both the compiler and the checker, and they share the code
that reads the certified inputs into rows (`assemble`). The checker is therefore
not an independent verification of the walk, and the reports say so
(`independent_verification: false`). What it establishes exactly, for both full
instances:

- **Layout.** The program body is the one layout the compiler emits, with every
  lookup and comparison uncomputed and the two SELECT conditions, the inner
  reflection, the row sign and the walk reflection in place.
- **Lookup records.** Every emitted own and alternate record names the row index,
  kind and sign, or the network, coefficient sign and identity flag, of the
  target rows: 111,488 / 325,888 alias buckets over 1,710 / 2,514 rows.
- **Coefficients.** The histograms are those of the emitted tables, and the
  encoding error above is rational arithmetic on them.
- **Row signs against the numerical certificate.** The scalar offset computed from
  the rows must equal the offset `signed_df.py` certified plus the base squares'
  masses, so a sign error in `assemble` is rejected.
- **Bindings.** Certified rotation precision, angle words and input hashes.

What is assumed: the identity `<+|V(2|+><+|-I)V|+> = 2B^2-I` for the layout, which
is the argument above and is not derived row by row; and the logical primitives
(QROAM and erasure with phase fixup, comparisons, Fredkin, Givens, spin swaps,
SELECT, reflections). Nothing is lowered to gates, and the Reiher gate-word
certificate of the rigorous tracks does not apply to these programs.

The identity is exercised, not proved, by an exact-rational small Fock-space
execution (`execute_projected`) on two-orbital instances with noncommuting
projectors, a negative square, mixed coefficient signs, an identity term and a
one-body row; the result equals a separately built fermionic Hamiltonian. That
execution reads every lookup word and, from the body, the condition of each
SELECT, of the inner reflection and of the row sign. It assumes the primitives,
the uniform preparation and the final walk reflection.

The tests hold one mutant per check of the checker, named by the message that
must reject it, recompute both encoding errors by counting every draw of the
emitted words, and recompute the resource bounds from the formulas below.

**Trusted base:** Python, FLINT/Arb, the Hamiltonian inequalities, the parsers,
the identity and primitives above. This is not a machine-checked theorem.

## Complete logical cost and comparison with Low

The resource pass expands every stage of the emitted controlled walk. For a
table of length L, width w and block size b, the clean lookup plus erasure is
charged at ceil(L/b)+(b-1)w+2E(L), where
E(L)=min_h(2*2^h+ceil(L/2^h)). It counts one outer lookup pair, two inner pairs,
two angle pairs, four spin swaps, 4(N-1) Givens at 2*beta Toffolis each,
comparisons, controlled signs and both controlled reflections. Space includes
the system, live table outputs, phase-gradient register, controls and reused
lookup workspace. QPE registers and state-preparation workspace are separate.
The report sweeps power-of-two lookup blocks and records a space/time frontier.

These are **upper bounds from a per-primitive expansion**, not measured flattened
gate counts. The lookup and erasure costs follow Berry et al., Appendices B and
C; a Givens rotation is charged 2*beta Toffolis. The qubit bound takes the largest
single lookup workspace, which assumes each lookup's junk register is measured
out before the next lookup; holding them together would need more (about 13,757
for Reiher at the minimum-Toffoli setting).

| Cost at minimum-Toffoli setting | Reiher | Li |
| --- | ---: | ---: |
| Previous sparse correction block only, upper bound | 138,574 | 274,134 |
| **New complete controlled walk, upper bound** | **68,863** | **119,339** |
| New logical qubits, upper bound | 12,825 | 19,375 |
| Rotation contribution | 11,024 | 16,800 |
| Angle lookup contribution | 41,310 | 75,160 |
| Minimum-product alternative: Toffolis / qubits | 108,703 / 4,557 | 217,559 / 6,775 |

The previous correction model used unitary lookup erasure and excluded the base
and walk reflection. The new bound is already smaller while including those
stages, but this is **not a matched-backend ablation or measured speedup**.
It combines a different representation, shared preparation and clean QROAM.

Using Low et al.'s Table V convention Q=ceil(pi*lambda/(2*sigma)), sigma=1 mHa:

| Full-query comparison | Reiher | Li |
| --- | ---: | ---: |
| Low effective normalization, Ha | 21.3674 | 43.6538 |
| Low published Toffolis per step | 10,203 | 14,629 |
| Our ordinary-walk queries | 142,535 | 317,669 |
| Low queries from printed parameters | 33,564 | 68,572 |
| Our query-weighted Toffoli upper bound | 9,815,387,705 | 37,910,300,791 |
| Low total from printed parameters | 342,453,492 | 1,003,139,788 |
| **Ratio of our bound to Low's estimate** | **28.66x** | **37.79x** |
| Break-even step cost at our normalization | 2,402.59 | 3,157.81 |
| Break-even normalization at our step cost, Ha (continuous) | 3.166 | 5.351 |

This is a resource comparison under the same query formula, **not a matched
chemical-accuracy guarantee**. Low uses spectral amplification and an effective
normalization; this implementation uses ordinary qubitization and its actual LCU
normalization. We cannot transfer Low's effective normalization to the indefinite
signed correction. A valid spectral-amplification construction and its energy
bounds would have to be established for this new Hamiltonian.

Toffolis alone understate the gap, because this construction also needs about
eleven to thirteen times the qubits. On the challenge's score:

| Toffolis x qubits | Reiher | Li |
| --- | ---: | ---: |
| Low published logical qubits | 1,132 | 1,454 |
| Our logical qubits at the minimum-Toffoli setting, upper bound | 12,825 | 19,375 |
| Low published step, Toffolis x qubits | 11,549,796 | 21,270,566 |
| Our step at the minimum-product setting, upper bound | 495,359,571 | 1,473,962,225 |
| **Ratio for one step** | **42.89x** | **69.30x** |
| **Ratio including the query counts above** | **182.1x** | **321.0x** |

The concrete cost reduction still needed is therefore about 28.7x/37.8x in the
query-weighted Toffoli estimate, and about 182x/321x in Toffolis x qubits, if
everything else is held fixed. Even the rotation
charge alone exceeds the current-normalization break-even step cost. Lookup
tuning alone cannot meet that benchmark within this rotation schedule and cost
model. Reducing normalization, changing rotation organization/representation, or
recovering useful spectral amplification is necessary for a competitive design.
This is not a lower bound on other possible implementations.

Separately, the audit's conservative **99% absolute-error** textbook-QPE plan
uses 25/26 phase bits and 33,554,431/67,108,863 controlled walks, for
2,310,658,781,953 / 8,008,704,601,557 Toffolis at these step bounds. It assumes an
exact eigenstate, controlled walk and QFT. It must not be compared directly with
Low's standard-deviation cost as though the success guarantees were identical.
Ground-state preparation/selection, physical synthesis (including preparation
of the phase-gradient state), QFT synthesis and failure accounting remain open.

## Reproduce

From the repository root, use the dependencies and downloader in the energy
audit, then run:

```sh
python3 -m pip install -r challenges/femoco/tools/chemistry/requirements.txt
python3 challenges/femoco/tools/chemistry/fetch.py --data /tmp/femoco-chemistry
for instance in reiher li; do
  OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/signed_df.py \
    --instance "$instance" --data /tmp/femoco-chemistry \
    --artifacts /tmp/femoco-signed \
    --out "challenges/femoco/rigorous/signed-$instance.json"
  OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/combined.py \
    --instance "$instance" --artifacts /tmp/femoco-signed \
    --out "challenges/femoco/rigorous/combined-$instance.json"
  OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/combined.py \
    --instance "$instance" --artifacts /tmp/femoco-signed --verify-only \
    --out "/tmp/verified-$instance.json"
done
python3 -m unittest discover -s tests -v
```

The whole pipeline takes a few minutes and a few GB of RAM on a laptop.

**Checking the committed reports.** The committed certificates name their inputs
by SHA-256 (`artifacts` in each combined report): the base payload, the signed
factors, the emitted program and the rotation words. Those four files per
instance are too large for the repository and are published as assets of the
[`femoco-research-signed-walk-v1` release](https://github.com/teddyjfpender/quantum-autoresearch-challenge/releases/tag/femoco-research-signed-walk-v1). Download them into a
directory and run the `--verify-only` command above with `--artifacts` pointing
at it: it refuses any file whose hash differs, re-runs every check and writes a
report that must equal the committed one in every field except the recorded
library versions.

**Regenerating from Zenodo** gives a new, equally valid certificate, not the same
bytes: a different BLAS proposes slightly different eigenvectors, so the factor
files and their hashes differ. Acceptance never depends on the solver, only on
the recomputed interval residual. Two independent runs (the original and the one
committed here) agree on every figure in this document.

References: Low et al., [PRX 15, 041016](https://doi.org/10.1103/pb2g-j9cw),
especially Table V; Berry et al., [arXiv:1902.02134](https://arxiv.org/abs/1902.02134),
Appendices B/C for clean QROAM and erasure; the [preceding correction experiment](CORRECTED.md).
