# `sa-lowq`: Low et al.'s spectrum-amplified step with fewer qubits

The target is Low et al. 2025 (Phys. Rev. X 15, 041016, doi:10.1103/pb2g-j9cw), whose step is
built as published in `README.md` here (`sa-low2025`). About 75% of their qubits are the
rotation-angle register: all `N - 1` angles of the lane's Givens network are held at once
(848 / 1,125 bits in their count, 796 / 1,051 in ours). `sa-lowq` keeps their construction and
lane map. It changes only how the angles and the outer alias garbage are held.

```bash
FEMOCO_WALK_ARCH=sa-lowq ./benchmark.sh                                  # C = 2 (default)
FEMOCO_WALK_ARCH=sa-lowq FEMOCO_SA_CHUNKS=4 FEMOCO_SA_INNER_A=3 ./benchmark.sh
cargo test --release --lib sa_low::tests::lowq_scan -- --ignored --nocapture   # static frontier
```

Family `sa-lowq-stream`, parent `sa-low2025-spinswap`, with the same axes (streaming the angles
changes no taxonomy axis value). It uses `SpinSwap` / `SpinSwapDg` (spec/SPEC-SA.md section 11),
like `sa-low2025`, and involves no trusted code.

All counts in this file were measured on the rigorous twins `reiher-sa-v1` / `li-sa-v1` (keep bits
19 + 19 and 20 + 21). `sa-lowq` builds on the two tracks `reiher-sa-est-v1` / `li-sa-est-v1`
unchanged; no count on them is stated here. The conventions (measured, static, derived, "their
charge", qubit counting) are those of `README.md` section 1.

## The three changes (build-time knobs)

1. **Streamed angles** (`FEMOCO_SA_CHUNKS=C`, default 2). The angles are held
   `g = ceil((N - 1) / C)` at a time in `g` slot registers. Rotation `j` uses slot `j mod g`. One
   conjugation `V M V^dagger` visits the chunks in the order `C-1, ..., 0` (for `V^dagger`, as
   `Z_S V_rev Z_S`) and then `1, ..., C-1` (for `V`). Chunk 0 serves both halves around the
   Majorana. The first chunk is one unary-iteration read. Each change of chunk is one XOR
   transition, a read of `chunk_c(v) XOR chunk_c'(v)` into the same slots. After `V` the slots are
   measured out with one phase fixup. Cost: `2 (C - 1)` extra table reads per inner copy, so
   `4 (C - 1) E` Toffolis per step, where `E` is the number of networks. The slots are about
   `1/C` of the register. `C = 1` emits the published stream byte for byte: the SHA-256 of all
   four `sa-low2025` / `-bothspin` op streams is unchanged (`tests::pinned_ledgers` prints it).
   Held one rotation at a time (their "stream one rotation" idea), a network would need
   `2 (N - 1) - 1` reads of `E` entries per copy, about 70,000 Toffolis per step on Reiher.
   Chunks are the useful form of that idea.
2. **Outer garbage out early** (`FEMOCO_SA_OUTER_ERASE`, default on). After the outer alias swap
   the keep value (`mu_o` qubits) and the unchosen item (`D` qubits) are dead until the unprepare.
   They are measured right away. At the end the outer table is read a second time, and the chosen
   item register is measured as well. The three measurements leave the phase
   `mk.keep(i) ^ f(i) ^ lt g(i)`, with `f = ma.own ^ mm.alt` and
   `g = (ma ^ mm).(own ^ alt)` (derivation in `mod.rs`, `unprepare_from_reread`). Conditioned `Z`
   and `CZ(lt, .)` gates on the second read cancel it with no Toffolis. `lt` is then erased by the
   usual comparator. The two reads' junk phases share one fixup (`qroam::erase_with`). The
   trade is +283 (Reiher) / +296 (Li) Toffolis per step for -39 / -42 qubits in every phase of
   the inner copies (rigorous twins; both figures scale with the keep width).
3. **Dense network index** (`FEMOCO_SA_DENSE`, default on). The outer item carries the network
   base `r B` (square) or `R B + r` (one-body). Each copy adds the inner item `b` to it (`add_into`,
   then `sub_from`), so every angle read is one contiguous unary iteration over exactly
   `E = R B + N` values (324 Reiher, 931 Li) instead of a ragged `(hi, lo)` iteration
   (345 / 981 Toffolis). The identity item `b = B` points at the next network, which is harmless
   because it applies only its sign and `V V^dagger = I`. The unit test covers this case: `spec5`
   has `B < 2^k_i`.

When the angles are streamed, the transient of the inner alias QROAM read (`2^inner_a` blocks of
the inner word, 28 / 31 bits on the rigorous twins) sets the peak together with the slots.
`sa-lowq` therefore caps `inner_a` at 4 once `C > 1` (Li's published 5 would leave the peak at
1,243 on `li-sa-v1`). `FEMOCO_SA_INNER_A` overrides the cap.

Tests (`src/walk/sa_low/tests.rs`):

- `streamed_walk_passes_on_an_exact_spec` runs `score::evaluate` on an exact five-orbital spec for
  `C` in 1..4, both spin choices, and every erase/dense combination. On each it checks the
  harness's `C_step` against the ledger.
- `streamed_mutants_are_rejected`: a Givens reading the wrong slot, and an outer-erase circuit
  whose conditioned `CZ`s or `Z`s are dropped, are all rejected with a flipped sign or a wrong
  operator.

## Measured (rigorous twins, `./benchmark.sh`, K = 2^19; every lane passed)

`C_step` and `Q_peak` are measured. `Q_peak` includes the 16 / 15 phase-gradient qubits and no
phase-estimation register. The static ledgers (`tests::lowq_scan`) predicted every row exactly.
Rounding errors are 8.778e-5 and 9.768e-5 Ha. Implied error bounds (single draw) are 0.037 and
0.114 Ha.

| Circuit | Spec | Knobs | `C_step` | `Q_peak` | `lambda_eff_used` x C x Q |
| --- | --- | --- | ---: | ---: | ---: |
| lowq-c1 | reiher-sa-v1 | C=1, erase, dense | 10,297 | 1,041 | 2.2904e8 |
| **lowq-c2** | reiher-sa-v1 | C=2, erase, dense, inner_a 4 | **11,605** | **652** | **1.6168e8** |
| lowq-c4 | reiher-sa-v1 | C=4, erase, dense, inner_a 3 | 14,853 | 457 | 1.4504e8 |
| (padded index, no early erase) | reiher-sa-v1 | C=2, `OUTER_ERASE=0 DENSE=0`, inner_a 4 | 11,400 | 691 | 1.6832e8 |
| lowq-c1 | li-sa-v1 | C=1, erase, dense | 14,993 | 1,349 | 8.8292e8 |
| **lowq-c2** | li-sa-v1 | C=2, erase, dense, inner_a 4 | **18,877** | **832** | **6.8561e8** |
| lowq-c3 | li-sa-v1 | C=3, erase, dense, inner_a 3 | 24,397 | 650 | 6.9226e8 |

`lambda_eff_used` is 21.3674 (Reiher) and 43.6538 (Li), the paper's values. They are larger than
the certified bounds, 19.9906 and 43.6098 (`score.json` `lambda_eff_ours`: a UHF determinant of
`H_spec` plus the run's rounding error), so every product here uses theirs.

### Against Low et al. (their Cost[BE] and qubits quoted from Table V; everything else derived)

Totals are `steps x C_step` at sigma_PEA = 1.0 mHa, with `steps = ceil(pi lambda_eff / 0.002)`:
33,564 / 68,572 at their lambda_eff and 31,402 / 68,503 at the certified one. Qubits "+PE" add
the Lee-convention register `2 ceil(log2(I + 1)) - 1` (31 / 33 at their step counts). "Their
charge" is `C_step + 4 x Givens`, their `2 beta` per Givens, which is the like-for-like Toffoli
count.

| | C_step (their charge) | Q_peak / +PE | total, their lambda_eff | total, certified lambda_eff | C x Q |
| --- | ---: | ---: | ---: | ---: | ---: |
| Low et al., Reiher | 10,203 | 1,132 / 1,163 | 342,453,492 | (n/a) | 1.1550e7 |
| lowq-c1 Reiher | 10,297 (11,145) | 1,041 / 1,072 | 345,608,508 | 323,346,394 | 1.0719e7 |
| lowq-c2 Reiher | 11,605 (12,453) | 652 / 683 | 389,510,220 | 364,420,210 | 7.5665e6 |
| lowq-c4 Reiher | 14,853 (15,701) | 457 / 488 | 498,526,092 | 466,413,906 | 6.7878e6 |
| Low et al., Li | 14,629 | 1,454 / 1,487 | 1,003,139,788 | (n/a) | 2.1271e7 |
| lowq-c1 Li | 14,993 (16,193) | 1,349 / 1,382 | 1,028,099,996 | 1,027,065,479 | 2.0226e7 |
| lowq-c2 Li | 18,877 (20,077) | 832 / 865 | 1,294,433,644 | 1,293,131,131 | 1.5706e7 |
| lowq-c3 Li | 24,397 (25,597) | 650 / 683 | 1,672,951,084 | 1,671,267,691 | 1.5858e7 |

Reading the table:

- **Qubits are below theirs on both instances.** lowq-c2 needs 652 against 1,132 (-42%) on Reiher
  and 832 against 1,454 (-43%) on Li. lowq-c4 / c3 need 457 (-60%) and 650 (-55%). About 77 / 76
  of each gap is qubit-counting convention: `Q_peak` is the true peak, theirs is the persistent
  registers plus the largest temporary. The rest is circuit. Streaming alone takes 389 qubits off
  the published Reiher build, 1,080 -> 691 (both measured, same keep bits). On Li it takes 517
  off, 1,393 -> 876 (876 is static only).
- **Toffolis per step and totals are above theirs on both instances, under any charge.** Streaming
  costs `4 (C - 1) E` Toffolis per step, `E = 324 / 931` networks, and there is no way around it:
  a conjugation needs every angle twice, in mirrored order. lowq-c2 is +13.7% (Reiher) and +29.0%
  (Li) above their Cost[BE] under the harness's charge, and +22% / +37% under theirs. Totals scale
  the same way. At the certified lambda_eff (7% fewer steps on Reiher) lowq-c2's total is still
  6.4% above theirs; lowq-c1 Reiher is 5.6% below it, but only because of the different
  lambda_eff and the Givens charge, not because the circuit is cheaper.
- **The product `C x Q` is below theirs on both.** lowq-c2 is -34.5% (Reiher) and -26.2% (Li).
  lowq-c4 Reiher is -41.2%. lowq-c3 Li (-25.4%) is no better than c2 on Li, because every Li
  transition costs 931 Toffolis. Under their Givens charge the products are still lower:
  12,453 x 652 = 8.12e6 against 1.155e7, and 20,077 x 832 = 1.67e7 against 2.127e7. Qubit
  convention is part of this too.
- lowq-c1 (no streaming, garbage erase and dense index only) is the small-increase point:
  +0.9% / +2.5% Toffolis (harness charge) for -8% / -7% qubits (about 5 of those points are
  convention). The product gain is small: -7.2% / -4.9%.

Every Toffoli count above uses the harness's `2 (beta - 2)` Givens charge unless it says "their
charge".

## Design points (static unless marked measured)

"Static" means the ledger plus the harness's static pass, which matched the harness on all 7
measured `sa-lowq` runs.

| Design point | Result (rigorous twins) | Kept as a default? |
| --- | --- | --- |
| C = 1 through the streamed code path | op-stream SHA-256 identical to `sa-low2025` / `-bothspin` on both specs | check only |
| C = 2..6 with inner_a `k-1`, `k-2`, `k-3`, no erase | e.g. Reiher C=3 inner_a 4: 12,780 / 690 (peak = inner QROAM transient); Li C=2 inner_a 5: 18,712 / 1,286 (transient) | the inner read's `2^inner_a` blocks bound the peak, so inner_a must drop with C |
| Outer garbage erase | +283 / +296 Toffolis, -39 / -42 qubits (C=1: 10,303 / 1,041; 15,084 / 1,351) | yes |
| Dense network index | -6 (Reiher C=1) to -279 (Li C=2) Toffolis, -2 qubits on Li | yes |
| Reiher C=3 inner_a 3 | 13,545 / 517 (7.00e6) | no: C=4 has the lower product (6.79e6) |
| Reiher C=5, 6 | 16,161 / 442 and 17,469 / 442: the inner_a 3 transient floors the peak at 442 | no |
| Li C=4, 5 inner_a 3 | 28,133 / 566 and 31,869 / 510 (products 1.59e7, 1.63e7) | no: C=2 has the lower product |
| inner_a 2 anywhere | +1,936 (Reiher) / +3,568 (Li) Toffolis for at most -15 qubits | no |

## Alternatives that were not built, and why

- **Fewer rotation bits via an error split.** The spec pins the angles at 16 / 15 bits, and those
  quantized networks are the operator (spec/SPEC-SA.md section 2.2). Dropping a bit needs a new
  pinned spec generated from their factors, with its own certificate. No proof exists that the
  extra error stays inside their budget: their split (Eq. (D1), `sigma(b) = 2^(c - b)` with fitted
  `c`) is an estimate. The only rigorous statement the spec carries, `angle_quantization`, is
  already 0.0525 Ha (Reiher) and 0.368 Ha (Li) at the current bits. That is 60x and 440x the whole
  0.83 mHa truncation budget, and it doubles at one bit fewer.
- **Angles in `[0, pi/2]` (one more bit per rotation).** On the chain `(j, j+1)`,
  `G(pi - psi) = Z_j G(psi) Z_{j+1}`. The right-hand `Z`s pass through to `M` and cancel. The
  left-hand ones collect into a per-network `Z` string on up to `N - 1` modes, which needs one
  flag qubit per mode it can touch. That is the same count as the bits it saves: the sign
  information is `N - 1` bits per network either way. This was checked on the data
  (`tests::angle_diag`: every angle but the last rotation's lies in `[0, pi)`, which the published
  layout already exploits with `beta - 1`-bit registers). `sa-toff` lever `s` (`README.md` section
  6.9) removes the last rotation's exception by a global sign of the Householder vector.
- **Dirty-qubit QROAM.** The wide reads are the angle chunks (400-500 bits) and the inner alias
  word. For the chunks, `lambda w` blocks are never cheaper than one unary iteration at these
  widths (`lambda = sqrt(E / w) ~ 1`). For the inner read, the only live non-system qubits during
  the read are the uniform register and a few outer flags, about 80, far fewer than the 420+ a
  dirty read would borrow. The harness does not let a swap network borrow system qubits (two
  system wires in one gate are rejected).
- **Inner alias garbage** (keep plus alt, about 27 qubits during SELECT on the rigorous twins).
  Freeing it early by a full second inner read costs 963 / 1,534 Toffolis per copy, and those
  qubits do not bound the peak during the inner read itself. The outcome-gated re-read of
  `README.md` sections 5.4 and 6.13 does it at about half that cost.
