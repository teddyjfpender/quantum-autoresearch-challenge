# Validated FeMoco energy bounds

The newer [signed correction and combined-walk experiment](SIGNED.md) reduces
correction normalization by 6.60x/3.16x, verifies the emitted hierarchical logical
walk, and reports its complete logical step and QPE costs. It fits the conditional
error budget but remains 28.7x/37.8x above Low's published cost under the paper's
query convention. Its primitive contracts and full-algorithm limitations are explicit.

The follow-up [explicit correction experiment](CORRECTED.md) constructs new
Hamiltonians with certified target errors below 0.267/0.261 mHa, rebuilds the
higher-precision base circuits and measures sparse compression's limited benefit.
Its correction resource counts are analytical bounds; it is not a complete
ground-energy algorithm.

This maintainer tool recomputes bounds from the original integrals and published
DFTHC factors. It is a separate layer from circuit equivalence and the challenge's
coefficient-rounding tracks. **Neither shipped instance is certified to chemical
accuracy.** The current factors provably cannot satisfy a 1.6 mHa *operator-norm*
criterion, even after an arbitrary scalar correction. Their ground-energy error
remains undecided.

## Results

The committed [Reiher report](../../rigorous/energy-reiher.json) and
[Li report](../../rigorous/energy-li.json) use 128-bit Arb ball arithmetic. All
decisions use outward rational endpoints, never rounded display values.

| Quantity | Reiher | Li |
| --- | ---: | ---: |
| Certified DFTHC fit norm upper bound | 213.690 Ha | 71.160 Ha |
| Certified fit norm lower bound, valid after **every** scalar shift | >47.500 mHa | >21.340 mHa |
| Current rotation + preprocessing upper bound | <52.935 mHa | <368.441 mHa |
| Proposed 26-bit rotation + preprocessing upper bound | <0.0510 mHa | <0.1794 mHa |
| Proposed 28-bit rotation + preprocessing upper bound | <0.0129 mHa | <0.0450 mHa |
| Stored-Hamiltonian Slater energy (display only, Ha) | -13481.964620369854 | -1118.3560829297053 |

Upper-bound rows are conservatively rounded upward; lower-bound rows downward.
The fit upper bounds are about four times smaller than the old float64-evaluated
`2 sum |D|` estimates. They still do not fit a chemical-accuracy budget. **A large
upper bound is not evidence of a large ground-energy error.** The determinant
witnesses establish a lower bound on an operator norm, not on ground-energy error.

Higher-precision rotations are *validated numerical proposals*, not new circuit
specs or measured resource results. They use newly proposed one-body eigenpairs
and angles; their residuals are rechecked with intervals. Existing circuits and
payloads are unchanged. Resource comparisons must be rerun if these proposals are
adopted. Rotation precision alone cannot remove the factorization obstruction.

The Slater energies have interval radii below 2e-32 Ha. They replace a discretionary
1 mHa numerical allowance **in this audit** with validated Gram-matrix inversion,
trigonometry and contractions. These are variational upper bounds for the stored
Hamiltonian; the signed evaluator and its historical certificates are unchanged.

## Reproduce

From the repository root:

```sh
python3 -m pip install -r challenges/femoco/tools/chemistry/requirements.txt
python3 challenges/femoco/tools/chemistry/fetch.py --data /tmp/femoco-chemistry
OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/certify.py \
  --instance reiher --data /tmp/femoco-chemistry --out /tmp/energy-reiher.json
OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/certify.py \
  --instance li --data /tmp/femoco-chemistry --out /tmp/energy-li.json
python3 -m unittest discover -s tests -p test_chemistry.py -v
```

Add `--require-chemical-accuracy` to make either run exit **2** after writing the
negative result. Without that flag, exit 0 means the audit completed, not that the
chemical-accuracy claim passed. Invalid inputs/arithmetic errors produce no new
report; a previous output is removed before recomputation. `--precision 192`
recomputes with narrower balls. This is a deterministic scientific calculation,
not a sampled circuit test.

The downloader retrieves the 78.8 MB integrals archive and range-fetches only the
two selected factor members from the 14.8 GB factors archive. It verifies member
names, sizes, CRCs and SHA-256 hashes. It never executes the authors' scripts.
The factor reader permits only the particular NumPy/JAX array reconstruction
objects in these hash-pinned files, substituting NumPy arrays for JAX objects.

## Target and exact conventions

`sources.json` pins Zenodo record 17066718, the exact factor members and FCIDUMPs.
The target Hamiltonian uses the **exact decimal coefficients in the FCIDUMP**,
enclosed directly by Arb. The published float32 factor values and stored float64
payload values are exact dyadic inputs. Input format precision is part of the
target definition; no claim is made about errors relative to the unknown integrals
before the FCIDUMP was written.

The sector is fixed electron number (54 or 113), including all spin sectors. Li's
core energy is set to zero, matching the challenge: restoring its FCIDUMP core
(-21021.2140122 Ha) to both sides changes no error. An experimental chemistry or
reaction-energy guarantee needs its own model/basis/difference error budget.

Let `E_pq = sum_spin a†_p,spin a_q,spin`, `Et_pq = E_pq - delta_pq`, and
`L_rc = sum_b w_rcb u_rb u_rb^T`, with exactly normalized published vectors.
Writing `Hs` for the published symmetry-shift matrix,

```
D_pqrs = sum_rc L_rc,pq L_rc,rs - g_pqrs
         - (delta_pq Hs_rs + Hs_pq delta_rs)/2.
R = (1/2) sum_pqrs D_pqrs Et_pq Et_rs.
```

The code independently constructs the original one-body shift and scalar from
the FCIDUMP. Expansion gives, *before rotation rounding*,

```
H_fit - H_target = R + (N_hat - eta) (H1_I + Hs.Et/2).
```

The second term vanishes on the stated sector. The tests check this identity
against exact rational creation/annihilation matrices, including the nonzero
off-sector term. No small float residual is treated as an exact BLISS identity.

## Certified bounds and obstruction

For a packed spatial pair, define `A_pp = Et_pp` and
`A_pq = E_pq + E_qp` for `p > q`. Their norms are at most 1 and 2, respectively.
With `k_pp=1`, `k_pq=2`,

```
||R|| <= (1/2) sum_ab |D_ab| k_a k_b.
```

All tensor entries, products and sums are enclosed. The pair multiplicities and
the factor 1/2 are essential. Both the norm inequality and determinant formula
are tested against a separately constructed small Fock-space Hamiltonian.

For a computational Slater determinant with occupations `n_p,s`, Wick's theorem
gives

```
<R> = (1/2) sum_pq D_ppqq (n_p-1)(n_q-1)
      + (1/2) sum_s,pq D_pqqp n_p,s (1-n_q,s).
```

The reports give explicit occupied orbital lists and rigorous intervals for four
such determinants. If their expectations include values `d_low` and `d_high`,
then **for every real scalar `s`**,

```
||R - s I|| >= (d_high - d_low)/2.
```

The checker uses the high state's lower interval endpoint and the low state's
upper endpoint. This proves the 47.5/21.34 mHa obstructions without any spectral
solver or ground-state assumption. It refutes the proposed uniform operator-norm
budget for these factors, even with a fitted constant correction. It does not
refute a low-energy or ground-energy-specific certificate.

For rotations, `||Et(u)-Et(v)|| <= 2 sqrt(1-(u.v)^2)`. Each squared generator has
norm at most `S=|wB|+sum|w|`; its half-square changes by at most `S*delta`.
The one-body residual is compared directly with the algebraically constructed
shifted matrix, so eigenpair error is included. A centered spin-free one-body
operator has norm at most its matrix nuclear norm, bounded by `sqrt(N)*Frobenius`
and by a separate entrywise bound. Scalar discrepancies are added, never dropped.

The Slater certificate uses `P=C(C^T C)^(-1)C^T` rather than assuming the supplied
columns are orthonormal. For each square its expectation is `mean^2 + variance`,
where `variance=sum_spin Tr(P L^2 - P L P L)`. Certified inversion and contractions
enclose the exact expectation of the stored operator. The SOS lower bound then
gives an upper bound on its ground-energy gap and effective normalization.

## End-to-end accounting and what remains

The report adds the certified factorization and rotation/preprocessing bounds,
**0.2 mHa** for any accepted rigorous coefficient map (the nested walk costs
`2 * rounding_error`, including its identity correction), and an illustrative
**1 mHa** phase-estimation budget. Systematic errors are added, not combined in
quadrature. `energy_budget_met` is false for both shipped instances.

An optional resource *plan* derived in the report uses ordinary textbook QPE on
an exact qubitized walk with the stated normalization. The geometric-series tail
bound is `P(circular bin error >= L) <= 1/[2(L-1)]`. The energy map
`E=offset+Lambda*cos(theta)` is Lambda-Lipschitz. Choosing the register so
`2*pi*Lambda*L/2^m <= 1 mHa` and `L=51` gives failure probability at most 1%.
This deliberately conservative plan has 25/26 phase bits and 33,554,431/67,108,863
controlled walk uses. It is **not** an implementation or a replacement for Low's
optimized spectral-amplification cost model; it demonstrates why a 99% absolute
error claim cannot inherit a standard-deviation cost unchanged.

The QPE statement is conditional on an exact eigenstate, controlled walk and QFT.
The full claim remains false even for a hypothetical passing energy budget until
ground-state preparation/selection and implementation failure/error bounds are
established. The quantum-equivalence evidence from PR #7 is a separate obligation.

To actually finish a 1.6 mHa guarantee, one of these substantive results is needed:

1. **Ground-energy-specific certificates for these factors.** For example, prove
   target and approximate ground-energy brackets `[L_t,U_t]`, `[L_f,U_f]` with
   `max(U_f-L_t,U_t-L_f)` inside the factorization allocation. Variational states
   certify upper bounds; checkable SOS/validated SDP dual certificates can certify
   lower bounds. Classical CCSD(T) agreement alone cannot supply these brackets.
2. **A different approximation.** Refit/increase rank under a certifiable error
   objective, or retain a less compressed representation, then rerun this audit,
   circuit generation, exact equivalence and resource comparison. Any unchanged
   factors retain the obstruction proved above, regardless of rotation precision.

The checker does not accept externally asserted `certified: true` values, optimistic
energy estimates or a solver's success status as proof. No ground-energy-specific
certificate or chemical-accuracy circuit is claimed by this PR.

## Trust boundary and references

This is a numerical certificate using validated arithmetic, not a Lean/Coq proof.
The trusted base includes Python, FLINT/Arb, the formulas above, the input parsers,
and the pinned scientific data. NumPy's eigensolver only proposes candidates; its
roundoff is absorbed by the interval residual checks. Report hashes bind inputs
and checker code, and the CI tests reject stale checker hashes.

- Low et al., *Fast Quantum Simulation of Electronic Structure by Spectral
  Amplification*, [PRX 15, 041016](https://doi.org/10.1103/pb2g-j9cw).
- Published factors, integrals and scripts: [Zenodo 17066718](https://zenodo.org/records/17066718).
- [Arb real balls](https://python-flint.readthedocs.io/en/latest/arb.html) and
  [validated matrices](https://python-flint.readthedocs.io/en/latest/arb_mat.html).
- The challenge's [SA specification](../../spec/SPEC-SA.md).
