# Explicit correction and certified sparse compression

The corrected-Hamiltonian experiment succeeds as an **operator approximation**,
but sparse correction is expensive in the resource model below. It does not yet
provide an implemented, fully certified ground-energy algorithm.

The target, exact input files, electron-number sectors and interval-arithmetic
trust boundary are those of the [energy audit](README.md). This experiment changes
the approximating Hamiltonian; it does not establish the ground-energy error of
the original published factors. No judged payload, circuit or ledger is changed.

## Results

The [Reiher](../../rigorous/corrected-reiher.json) and
[Li](../../rigorous/corrected-li.json) reports were recomputed from all target
integrals and factors with 128-bit Arb. Bounds below are rounded upward.

| Quantity | Reiher | Li |
| --- | ---: | ---: |
| Rotation bits | 26 | 28 |
| Full packed residual table entries | 1,103,355 | 4,282,201 |
| Nonzero entries after baseline dyadic quantization | 1,103,355 | 4,282,183 |
| Nonzero entries after budgeted sparse compression | 1,101,579 | 4,252,735 |
| Entries removed, relative to full table (display) | 0.1610% | 0.6882% |
| Coefficient fractional bits | 34 | 36 |
| Rotation + preprocessing error | <0.0510 mHa | <0.0450 mHa |
| Baseline correction quantization error | <0.0161 mHa | <0.0156 mHa |
| Compressed correction error (dropping + quantization) | <0.2160 mHa | <0.2155 mHa |
| **Explicit corrected baseline Hamiltonian error** | **<0.0671 mHa** | **<0.0605 mHa** |
| **Explicit compressed Hamiltonian error** | **<0.2669 mHa** | **<0.2604 mHa** |
| Compressed correction alias-distribution error | <0.00670 mHa | <0.00860 mHa |
| Conditional total, including reserved encoding and QPE errors | <1.484 mHa | <1.479 mHa |

The last row adds actual rotation, correction and alias bounds, 0.2 mHa for the
base's nested coefficient encoding, 0.01 mHa reserved for outer combination,
and 1 mHa for a conditional QPE plan. It is **budget feasibility**, not completed
implementation evidence. The outer combination, physical synthesis, state
preparation/selection and QPE failure accounting are outstanding. The remaining
0.116/0.121 mHa margin is available for further systematic implementation errors;
no synthesis algorithm is asserted to meet it here.

## Why this proves ground-energy proximity for the new approximation

With the audit's notation, on the specified electron-number sector,

```
H_fit - H_target = R,        R = (1/2) sum_ab D_ab A_a A_b.
H_corrected = H_beta - R_kept.
```

Here `H_beta` is a newly proposed and independently validated finite-rotation
DFTHC Hamiltonian. Thus

```
||H_corrected - H_target||
    <= ||H_beta - H_fit|| + ||R - R_kept||.
```

The min-max principle gives the same upper bound on
`|E0(H_corrected)-E0(H_target)|` within that sector. **No ground-state guess, gap
estimate or cancellation assumption enters this result.** It bypasses the need
for a ground-energy-specific proof for the old factors by changing the operator.
It does not give the numerical ground energy or prepare its eigenstate.

## Correction representation and compression

For packed pairs `a=(p,q)`, use `k_a=1` on the diagonal and 2 otherwise, so
`||A_a|| <= k_a`. Define Hermitian contractions

```
B_aa = A_a^2/k_a^2,
B_ab = (A_a A_b + A_b A_a)/(2 k_a k_b),  a>b.
c_aa = D_aa k_a^2/2,        c_ab = D_ab k_a k_b.
R = sum_(a>=b) c_ab B_ab,   ||B_ab|| <= 1.
```

The baseline keeps every coefficient, rounded to a signed dyadic integer with
34/36 fractional bits. Compression sorts the proposed `|c_ab|` and drops the
smallest entries under a 0.2 mHa allocation. Both scenarios reserve 0.05 mHa for
dyadic quantization. All retained and dropped coefficients are then independently
revisited with Arb: `sum |c_ab-c_hat_ab|` certifies the operator error. A floating
sort or eigensolver never decides acceptance. A failed interval check produces no
accepted report. This is one sparse method, not an optimum over all representations
or certificates. A loose entrywise bound can restrict compression even when the
actual operator would admit more.

The generated NPZ artifacts contain packed-triangle positions and signed dyadic
integers for `R_kept`, plus verified alias tables. **The correction to add to the
base is their negative.** Positions enumerate `a=0..M-1`, `b=0..a`,
`M=N(N+1)/2`. The report binds the contents and base payload by SHA-256. Large
reproducible artifacts are generated locally, not stored in the source repository.

## A concrete logical correction block encoding

In interleaved Jordan-Wigner order, `A_pp/k_pp` is the average of two negative Z
Paulis; duplicate each to get four uniform choices. For `p>q`, `A_pq/k_pq` is the
average of four signed Pauli strings (XX and YY strings for both spins). The helper
`pair_paulis` returns their exact masks and signs. Consequently `B_ab` is the
average over 16 symmetrized Pauli products. Commuting products are Hermitian
Paulis. An anticommuting product has zero Hermitian part; SELECT implements X on
an extra signal ancilla in that case, whose zero-to-zero block is zero. This
avoids silently encoding a non-Hermitian ordered product. Exact small Fock-space
tests verify every pair, branch, matrix element and anticommutation case.

An integer Walker alias table prepares the correction row distribution. Pad to
`K=2^ceil(log2(term_count))` buckets. With `mu=24/22`, apportion `K*2^mu` draws by
flooring each desired count and assigning the remainder to the largest weight.
The checker verifies the exact distribution error and reconstructs every output
histogram count from the alias table. Two Hadamard registers suffice for its
uniform input. Within each row, the two Pauli-choice registers are exactly uniform.
The correction normalization is `sum |c_hat|`, about 213.689/71.160 Ha. It is an
LCU normalization and upper bound, **not a measured operator norm**.

## Resource results and their limits

The base circuits are actually emitted and passed through the existing layout
compiler. Each new lane map passes its exact 0.1 mHa coefficient 1-norm test.
The reports are [Reiher base](../../rigorous/corrected-base-reiher.json) and
[Li base](../../rigorous/corrected-base-li.json). Counts below use Low's logical
`2*beta` Givens charge; the reports also give the harness's `2*(beta-2)` charge.
Expected counts weight the builders' independent measurement corrections. Qubits
include the phase gradient, but exclude QPE registers. These new circuit instances
have not been given new full symbolic-equivalence certificates.

| Base circuit role | Reiher Toffolis | Reiher qubits | Li Toffolis | Li qubits |
| --- | ---: | ---: | ---: | ---: |
| Low-layout reference at new precision | 15,108 | 1,620 | 23,788 | 2,381 |
| Fewest-Toffoli source | 14,574.5 | 582 | 22,442.5 | 1,214 |
| Best-product source | 19,421.5 | 366 | 34,048.5 | 591 |
| Fewest-qubits source | 27,720.5 | 297 | 48,207.5 | 425 |

The original small Reiher rank caches fail at 26 bits: one rotation no longer fits.
The experiment raises their hold budget to 27 and their parked budgets to 24/22.
These are recorded runtime recalibrations, not changes to historical knobs. No
claim is made that these four points remain optimal at the new precision.

Correction resources below are **analytical constructive upper bounds**, not
emitted or symbolically verified full circuits. Each row is for one
`P^dagger SELECT P` correction block. It excludes the base block, their outer
combination, a walk reflection/control, QPE, and physical fault-tolerance overhead.
All qubit columns include the system register, so they must not simply be added
to the base's peak qubits.

| Correction design | Reiher baseline T / Q | Reiher compressed T / Q | Li baseline T / Q | Li compressed T / Q |
| --- | ---: | ---: | ---: | ---: |
| Unbatched lookup | 12,850,670 / 975 | 12,843,566 / 975 | 50,778,258 / 1,284 | 50,660,466 / 1,284 |
| Smallest T bound in tested block grid | 138,598 / 12,542 | 138,574 / 12,542 | 274,362 / 24,540 | 274,134 / 24,540 |
| Smallest TQ bound in tested block grid | 207,038 / 3,710 | 206,986 / 3,710 | 712,646 / 4,188 | 711,726 / 4,188 |

`T` denotes Toffoli count in this table, not the number of single-qubit T gates.
These conservative constructions do not prove a lower bound on the best possible
cost. Comparing their cost to an optimized compiled base identifies a bottleneck
in this design, not a universal penalty for rigorous accuracy.

For reproducibility the model uses a fully unitary XOR lookup. For `L` table rows,
word width `w` and power-of-two batch size `b`, allocate `b*w` work bits, build all
batch words by unary iteration, select a word by swaps, XOR it to the output, then
reverse the swaps and iteration. A sufficient Toffoli bound is

```
lookup(L,w,b) <= 2 ceil(L/b) + 2(b-1)w.
```

No measurement-based erasure saving is assumed. The construction follows the
select-swap approach discussed by
[Berry et al., appendices B-C](https://arxiv.org/abs/1902.02134), with conservative
unitary uncomputation. For the correction block, count two alias-table lookups
`(K, k+mu+1)`, two row-data lookups `(term_count, 2d+1)`, and four Pauli-dictionary
lookups `(4M, 4N+1)`, where `k=log2(K)`, `d=ceil(log2(M))`.

The remaining bound is `8(mu+1)+2k+16N+16`: reversible comparator compute/undo,
alias Fredkin banks, anticommutation parity compute/undo, controlled Pauli masks
and phase flags. Space is overallocated as `10N+3k+4(mu+1)+2d+20`, plus the largest
lookup workspace `b*w+ceil(log2(L))`; lookups reuse that workspace. Batch sizes
`1..4096` in powers of two are tried independently for all three tables. This
explicit model, including the deliberately conservative arithmetic allowance,
is stored alongside every result.

An indefinite residual correction does not inherit the old sum-of-squares
spectral-amplification advantage automatically. The conditional QPE plans use
ordinary qubitization with `Lambda_base+Lambda_correction`; they make no claim to
reproduce Low's optimized phase-estimation or state-preparation costs. Their
query counts cannot be multiplied by the correction-only table to claim a full
algorithm cost.

## Interpretation and publication claims

The hypothesis was that sparse residual correction could retain the compact
DFTHC cost advantage. Its cheapest test was the number of entries retained under
the certified budget. The result is unfavorable: over 99.3% remain, and the modeled
cost hardly changes after compression. Further tuning of this sparse cutoff is
unlikely to resolve its dominant lookup cost. Structured residual factorization,
direct integral decompositions, or tighter sector-specific compression bounds
are more substantive next experiments.

This does not invalidate circuit improvements for a specified approximate
Hamiltonian. Those can support a circuit-methods paper under clearly matched
assumptions. The audit and correction experiment add a reproducible distinction
between implementation accuracy and accuracy to the target Hamiltonian. They
weaken any unsupported claim of a cheaper, fully guaranteed chemical-energy
calculation; they do not settle novelty or guarantee publication.

The relevant bound is needed because the reported energy has separate errors:
Hamiltonian approximation, its encoding and synthesis, and energy estimation.
Circuit equivalence only controls implementation of the chosen operator. A small
fit loss or accurate energy on selected trial states does not by itself bound the
target ground energy. Here the uniform operator inequality supplies that missing
link for the **new corrected approximations**; implementation obligations remain.

## Reproduce

From the repository root, after the energy audit's pinned downloads:

```sh
python3 -m pip install -r challenges/femoco/tools/chemistry/requirements.txt
OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/corrected.py \
  --instance reiher --data /tmp/femoco-chemistry --artifacts /tmp/femoco-corrected \
  --out /tmp/corrected-reiher.json
OPENBLAS_NUM_THREADS=1 python3 challenges/femoco/tools/chemistry/corrected.py \
  --instance li --data /tmp/femoco-chemistry --artifacts /tmp/femoco-corrected \
  --out /tmp/corrected-li.json
CARGO_PROFILE_RELEASE_LTO=false cargo +1.93.0 build --release \
  --manifest-path challenges/femoco/Cargo.toml --bin cost_corrected_base
challenges/femoco/target/release/cost_corrected_base \
  /tmp/femoco-corrected/reiher-corrected-base.bin reiher /tmp/corrected-base-reiher.json
challenges/femoco/target/release/cost_corrected_base \
  /tmp/femoco-corrected/li-corrected-base.bin li /tmp/corrected-base-li.json
python3 -m unittest discover -s tests -p 'test_corrected.py' -v
```

The default root Rust toolchain may be older; the command explicitly uses the
challenge's pinned 1.93.0. LTO was disabled for the local resource build, not in the
repository's release profile. NumPy/BLAS proposes eigenpairs, so binary hashes can
vary across numerical-library builds; every regenerated proposal is validated
against the same target with intervals. Tests cover exact fermion/Pauli identities,
all small alias draws, a non-dyadic residual, rejection of unresolved intervals,
budget arithmetic, source digests and linkage to the emitted base payloads.
