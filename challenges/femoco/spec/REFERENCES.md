# References

Primary sources for the FeMoco challenge.

## The instances

- M. Reiher, N. Wiebe, K. M. Svore, D. Wecker, M. Troyer, "Elucidating reaction mechanisms on
  quantum computers", PNAS 114, 7555 (2017); arXiv:1605.03590. The 54-orbital active space (`reiher`).
- Z. Li, J. Li, N. S. Dattani, C. J. Umrigar, G. K.-L. Chan, "The electronic complexity of the
  ground-state of the FeMo cofactor of nitrogenase as relevant to quantum simulations",
  J. Chem. Phys. 150, 024302 (2019). The 76-orbital active space (`li`).

## The encoding and the acceptance standard

- G. H. Low, R. King, D. W. Berry, Q. Han, A. E. DePrince III, A. F. White, R. Babbush,
  R. D. Somma, N. C. Rubin, "Fast Quantum Simulation of Electronic Structure by Spectral
  Amplification", Phys. Rev. X 15, 041016 (2025); arXiv:2502.15882. The spectrum-amplified
  sum-of-squares walk, its data, its cost model and the published points in
  [`../targets.json`](../targets.json).

## Constructions used by the shipped circuits

- J. Lee, D. W. Berry, C. Gidney, W. J. Huggins, J. R. McClean, N. Wiebe, R. Babbush, "Even
  more efficient quantum computations of chemistry through tensor hypercontraction", PRX
  Quantum 2, 030305 (2021); arXiv:2011.03494. Givens-rotation costing by phase-gradient addition; alias sampling
  conventions.
- D. W. Berry, C. Gidney, M. Motta, J. R. McClean, R. Babbush, "Qubitization of arbitrary basis
  quantum chemistry leveraging sparsity and low rank factorization", Quantum 3, 208 (2019); arXiv:1902.02134.
  QROAM and measurement-based uncomputation of lookups.
- R. Babbush, C. Gidney, D. W. Berry, N. Wiebe, J. McClean, A. Paler, A. Fowler, H. Neven,
  "Encoding electronic spectra in quantum circuits with linear T complexity", Phys. Rev. X 8,
  041015 (2018). Unary iteration and alias sampling.
- A. Caesura, C. L. Cortes, W. Pol, S. Sim, M. Steudtner, G.-L. R. Anselmetti, M. Degroote,
  N. Moll, R. Santagati, M. Streif, C. S. Tautermann, "Faster quantum chemistry simulations on
  a quantum computer with improved tensor factorization and active volume compilation",
  arXiv:2501.06165.
  One-hot data loading for rotations (their App. D), which the one-hot architectures build on.
- L. A. Belady, "A study of replacement algorithms for a virtual-storage computer", IBM
  Systems Journal 5, 78 (1966). The furthest-future replacement rule used by rank-scheduled
  delivery.

## Context

- The benchmark design (two-process build and evaluation, Fiat-Shamir sampled validation, an
  op-stream format) follows the ecdsa.fail point-addition challenge; see [`../../../NOTICE`](../../../NOTICE).
- A 2026 classical study (arXiv:2601.04621) reports reaching chemical accuracy on FeMoco. This
  challenge compares logical resource estimates of a quantum subroutine and makes no claim of
  quantum advantage.
