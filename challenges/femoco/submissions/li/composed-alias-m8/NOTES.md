# Composed sparse keep and aligned alternate Li alias

## Goal and starting point

This candidate builds on the recorded Li `kl-joint-alias-m8` circuit. It retains that circuit's spectrum-amplified walk, four-group split one-hot angle delivery, paired item one-hot inner read, folded index, measured erasure, and eight-bit outer and inner alias draws. The goal is to reduce the two inner alias reads per walk step while leaving the reversible SELECT and all recorded operation streams unchanged. The new table witnesses are selected only by default-off extension letters `.i`, `.j`, `.k`, and `.l`; the submitted build enables all four after the recorded `.f` extension.

## Mechanism and correctness

The `.i` and `.j` witnesses rearrange the 285 Li square inner alias tables without changing any parent item count. They align alternate-index bits 0 and 1 with the common donor on shared folded leaves, in addition to the parent's aligned alternate bit 5. The `.k` witness changes permitted floor-or-ceiling count choices to concentrate keep bit 0 on ten shared folded leaves rather than 29. It retains the high keep-bit support on nine leaves and the alternate-bit-5 alignment. The final `.l` witness composes the sparse keep and low alternate-bit alignments. A second count search reduced its exact 1-norm error at the same static circuit cost; the final witness uses those improved counts in 280 tables and validated fallback counts in five. All 285 table totals remain 16,384 lanes, with the donor padding condition preserved. The resulting paired folded read skips nonlinear products for the newly constant bits. The read, erasure, comparison, rotations, and Majorana gadgets are unchanged.

The witnesses are embedded in the builder source. Their parser checks shape, index range, padding, and table count. The estimated-class test checks every table's exact floor-or-ceiling counts; separate focused tests check the sparse support and reject a deliberately altered `.k` count. The recorded-circuit digest suite checks that the new default-off paths do not alter any of the 266 recorded circuits. These are table-layout changes within the declared `onehot-split` architecture.

## Experiments and measurements

The recorded parent has judge measurement 17,651.583 Toffolis per step at 471 peak qubits, a product of 8,313,895.593. With the leader's exact build settings (inner and outer block exponents 3, alias precision 8+8), its static count is 17,651.5. The `.j` exact-count checkpoint measured 17,617.454 × 471 on a full local sliced evaluation, with unchanged exact 1-norm rounding error 0.5603668293 Ha. The separate `.k` keep-rerounding candidate measured 17,609.5 × 471 statically. The composed `.l` circuit measures **17,571.5 × 471 statically**, 80 Toffolis per step below the parent. Its inner alias read is the stage that changes; peak qubits remain 471.

The final witness's trusted 4,096-lane reference evaluation passed at **17,571.498 × 471 = 8,276,175.558**. Its trusted 524,288-lane sliced evaluation passed at **17,571.485 × 471 = 8,276,169.435**, with zero fallback batches and zero guard fallbacks. Against the recorded parent score this local full measurement is about **0.454% lower**. These local lanes are public; only the pull-request judge establishes a standing.

The exact 1-norm rounding error rises from the parent's 0.5603668293 Ha to **0.868133 Ha** in the final witness. Before the second count search it was 0.988755 Ha, so the search recovered about 12.2% of that error without changing static C or Q. The estimated-class acceptance rule explicitly permits either adjacent integer count for each item and reports, but does not bound, this 1-norm. This circuit passes that rule; the per-step challenge score excludes the rounding error. The auxiliary convention that multiplies by a rounding-sensitive effective lambda therefore need not improve with the per-step score. This candidate makes a narrower claim about the challenge's specified per-step Toffoli-qubit objective, not a better certified end-to-end chemistry resource estimate.

Other screens found no useful score cut from changing the Majorana angle-drop setting around the recorded `.h4` value. Sparse keep bit 1 could not be cleared under the fixed counts and support constraints. An attempted additional alternate bit-2 alignment was not feasible on several exact-count tables under its first shared-leaf mask; it is outside this candidate. Four-group class-slot packing is already 235 slots against a mathematical 233-slot floor, so that path has at most two qubits of direct headroom.

## Validation and attribution

The focused estimated-class/support/mutant tests passed. All **266 recorded-circuit digests passed** after the source changes. Both trusted local evaluations above passed, and `python3 challenge.py check` reports `Contract OK`.

Developed by GPT-6 in Codex under Teddy Pender's direction. The parent circuit and earlier levers retain their recorded attribution. The optimization search and tradeoffs are tracked in [Discussion 4](https://github.com/teddyjfpender/quantum-autoresearch-challenge/discussions/4).
