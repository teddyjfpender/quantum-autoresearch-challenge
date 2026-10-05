# Architecture taxonomy (version 1.3.0)

The unit of search in this benchmark is an **architecture**, not a tuned parameter. Every
submission declares a family (`family.out.json`, spec/DESIGN.md section 10): one value on each
axis below. The harness checks each declared value against facts it measured from the circuit
(`CircuitFacts`, `src/facts.rs`) and records it as **Verified**, **DeclaredOnly** or
**Contradicted**. A contradicted axis rejects the run.

The machine-readable file is [`taxonomy.json`](taxonomy.json); the checker is `src/taxonomy.rs`.
This page explains what each check proves and, just as important, what it cannot show.

**Scope.** This repository ships specs of one encoding, `sos-sa`, so a passing submission declares
`encoding = sos-sa`, `lane_map = sa-nested-alias-v1`, `select = givens-sa-nested` and
`rotation = phase-gradient-givens`. The taxonomy file also defines the values of three other
encodings (`sparse`, `df`, `thc`) whose lane maps and loaders exist in the trusted code. No spec
for them ships, so no run here can verify them; they are listed below in brief because the file
and the checker carry them.

## The family rule

- A family is the tuple of axis values. Two submissions are in the same family if and only if
  their `axes` maps are equal (`taxonomy::family_key`).
- The name, the parent, seeds and every parameter are **not** part of the family: `u`, `k`,
  `mu`, table contents, the QROAM block size, angle bits, how segments are split, qubit layout.
  Changing them is calibration inside a family.
- A new value is added to an axis only when the walk step is built differently, with a
  citation or a written definition that says what differs. Renaming is not a new value.
- A *new family* should count as one only if at least one axis that distinguishes it from every
  existing family is **Verified**, not just declared. A family whose only distinguishing axes are
  DeclaredOnly is a claim, not evidence. `taxonomy::distinguishing_axes` lists the axes on which
  two families differ.

## How a verdict is reached

For every axis, in this order:

1. The family's `taxonomy_version` must have this file's MAJOR number, or every axis is
   contradicted.
2. A missing axis, an unknown axis or an unknown value is contradicted.
3. A **reserved** value (listed so the map shows the gap, but with no spec or harness support)
   is contradicted with its TODO.
4. A compatibility **rule** whose `if` matches the family and whose `then` excludes the value
   contradicts it.
5. The value's **necessary** predicate, if false on the facts, contradicts it. The rejection
   names the predicate and the observed values.
6. The value's **sufficient** predicate, if true, verifies it.
7. Otherwise the value is **DeclaredOnly**.

Predicates are three-valued. A fact the harness does not provide (a missing field, a null, a
segment that was never labelled) is *unknown*; unknown never contradicts and never verifies.
Missing entries inside a map of op counts read as 0.

**Signature strengths.** `exact`: one predicate is necessary and sufficient. `necessary`:
failure refutes the claim, success says nothing. `sufficient`: success proves the claim,
failure says nothing. `necessary+sufficient`: two different predicates. `declared`: no
predicate over the facts separates the value from its siblings. The checker evaluates only
necessary and sufficient predicates. Heuristics are written down below and are never evaluated.

**Segment labels are hints.** `Segment` ops (op kind 40) are emitted by the submitter.
Predicates over `segment_*` facts can therefore refute a claim that the submitter's own labels
contradict, but they prove nothing about ops that are unlabelled or labelled wrongly. That is why
no segment predicate is sufficient on labels alone. The `clifford-mask` sufficient predicate also
requires `segments_validated`: the harness checks that the hints form one prepare, select and
unprepare, with every system-targeting op in select and nothing before the first hint.

## Axes and values

`reserved` values are listed so the family map shows the gap.

### encoding: which LCU of FeMoco's Hamiltonian

| Value | Status | Signature | What it proves |
| --- | --- | --- | --- |
| `sos-sa` | active | exact: `encoding == "sos-sa"` | The harness loaded a spec whose `spec.json` says `sos-sa` and validated every sampled lane against it: Low et al. 2025's DFTHC + BLISS sum-of-squares operator, which equals the Hamiltonian only on the electron sector (spec/SPEC-SA.md). Low et al. 2025 (Phys. Rev. X 15, 041016) Secs. II and IV. |
| `sparse` | active, no shipped spec | exact: `encoding == "sparse"` | Same for a sparse spec. Lee et al. 2021 App. A.1; Berry et al. 2019 Sec. 5. |
| `df` | active, no shipped spec | exact: `encoding == "df"` | Same for a double-factorization spec. von Burg et al. 2021 Supp. Sec. VII; Lee et al. 2021 App. C. |
| `thc` | active, no shipped spec | exact: `encoding == "thc"` | Same for a tensor-hypercontraction spec. Lee et al. 2021 Secs. II.C-D and IV.A. |
| `bliss` | reserved | declared | No stand-alone BLISS spec exists (the `sos-sa` specs carry Low et al.'s BLISS shift with its sector stated). Loaiza and Izmaylov 2023, arXiv:2304.13772. |

### lane_map: how uniform values map to terms

| Value | Status | Signature | Notes |
| --- | --- | --- | --- |
| `sa-nested-alias-v1` | active | exact: `lane_map_family` | Spectrum amplification: outer alias over the generators' items with a free spin bit, per-item inner alias of one shared width with a free spin bit; lanes cross the `Reflect` and the reference is `M(b)^dagger M(a)` (spec/SPEC-SA.md section 5). Low et al. 2025 Eqs. (7), (10), Fig. 2. |
| `alias-v1` | active | exact | Coherent alias sampling over a spec's flat term list, Babbush et al. 2018 Sec. III.D. Ruled out for `sos-sa` by rule. |
| `sparse-sym-alias-v1` | active, no shipped spec | exact | Unique sparse entries plus symmetry and spin expansion bits. Lee et al. 2021 App. A.1. |
| `df-pair-alias-v1` | active, no shipped spec | exact | A flat `(l, k1, k2)` pair form of double factorization defined by the harness. It is not a published construction. |
| `df-nested-alias-v1` | active, no shipped spec | exact | Nested double factorization: outer alias over leaves, per-item inner alias of one shared width; lanes cross the `Reflect`. von Burg 2021 Supp. VII.C.4 Eqs. (77)-(80); Lee 2021 App. C pp. 51-54. |
| `thc-pair-alias-v1` | active, no shipped spec | exact | Alias over THC's unordered pairs plus spin bits and a swap bit that the block encoding flips. Lee et al. 2021 Sec. III.B-C, Eqs. (36)-(42), Fig. 6. |
| `symmetry-class-alias` | reserved | declared | Alias over classes of equal-magnitude terms plus member bits (an unpublished construction; reference `qa-candidates` in `taxonomy.json`). No parser exists. |

The lane-map family comes from the harness parsing `lanemap.bin`, so these checks are exact.
They say nothing about how the circuit reads the tables.

### lookup: how classical data reaches quantum registers

| Value | Signature | Why it is weak |
| --- | --- | --- |
| `none` | declared | Ruled out for every alias lane map by rule (alias sampling reads keep/alt from a table, Babbush 2018 Sec. III.D step 2). |
| `unary-iteration` | declared | Babbush et al. 2018 Sec. III.C, Fig. 10: T-count 4L-4, i.e. L-1 ANDs. The table size L is not a fact, and QROAM with k = 1 is the same circuit. |
| `qroam-clean` | declared | Berry et al. 2019 Sec. 3.2 and App. B (Theorem 2): cost ceil(d/k) + M(k-1). Controlled swaps are ordinary CCX/CX ops. |
| `qroam-dirty` | declared | Berry et al. 2019 App. A (Theorem 1); Low et al. 2018 Fig. 1d. Whether a borrowed qubit was dirty needs per-qubit history. |
| `select-swap` | declared | Low, Kliuchnikov and Schaeffer 2018 Sec. 2, Fig. 1c. |
| `monomial-lookup` | necessary: `executed_hmr > 0` | Gidney 2025 Sec. 3.1 gives the cost 2^n - n - 1, credited there to [Bab+18]. App. A.4 describes the construction: power products of the two address halves, multi-target Toffolis, then X-basis measurement of the ancillas. The paper says "power product", not "monomial". The check fails only if nothing was measured. |

Heuristic, not evaluated: the executed Toffolis spent on lookup, set against the table size,
separate unary iteration (about d), QROAM (about d/k + Mk) and monomial lookup (about 2^n - n).
Evaluating that would need per-segment executed counts and the table size, which are not facts.

### select: how the chosen term reaches the system

| Value | Signature | What it proves / cannot show |
| --- | --- | --- |
| `givens-sa-nested` | exact: `encoding == sos-sa`, `op_counts.GIVENS > 0`, `op_counts.REFLECT == 1` and `nested_validated` | The harness checked the nested structure (two inner copies identical op for op around exactly one `Reflect` on exactly the inner uniform register, which nothing else touches; spec/DESIGN.md section 16.3) and validated paired and diagonal lanes through the `Reflect` against `M(b)^dagger M(a)`: each sum-of-squares generator's LCU applied with Givens networks, the reflection, then its adjoint (the identical copy reads a pass flag). Low et al. 2025 Eq. (10), Figs. 1-2. It cannot show that the second copy is the adjoint of the first on unsampled lanes; the operator is certified by the lanes, not by this signature. |
| `unary-pauli` | necessary: segment 1 has CCX + CCZ > 0 | Unary iteration over indices needs ANDs inside SELECT. Babbush 2018 Sec. III.A. It cannot be told apart from `selected-majorana`. Sparse only, by rule. |
| `selected-majorana` | necessary: same | Babbush 2018 Sec. III.B, Fig. 9. Heuristic: the number of ops on system qubits scales with the Majoranas (O(n) per slot), not with the terms. Sparse only, by rule. |
| `clifford-mask` | necessary: segment 1 has no CCX/CCZ/Givens and at least one system op. sufficient: the same **and** `segments_validated` | SELECT is Clifford, driven by data loaded earlier (an unpublished construction; reference `qa-candidates`). Its sufficient signature relies on `segments_validated`, which checks the hint structure. Hints are still the submitter's labels, so this proves the structure, not where each Toffoli logically belongs. Sparse only, by rule. |
| `givens-network` | exact: `encoding` is `df` or `thc`, and `op_counts.GIVENS > 0` | The only basis-rotation primitive is the harness's Givens op. von Burg 2021 Supp. VII.C.1 (Lemma 8); Lee 2021 App. C step 4 and Sec. III.C steps 2-5. |
| `givens-nested` | exact: `encoding == df`, `op_counts.GIVENS > 0`, `op_counts.REFLECT == 1` and `nested_validated` | The same nested structure as `givens-sa-nested`, for double factorization. von Burg 2021 Supp. VII.C Eq. (54) and VII.C.3-4; Lee 2021 App. C. `givens-sa-nested` is a separate value rather than a widened `givens-nested`. |

A necessary condition on segment 1 is unknown (never a rejection) when the circuit has no
segment 1 label.

### uncompute: how temporaries are erased

| Value | Signature | Proves |
| --- | --- | --- |
| `unitary` | exact: `op_counts.HMR == 0` and `op_counts.R == 0` | No measurement or reset exists, so every ancilla returned to 0 by gates. This is the "reverse of the computation circuit" alternative in Gidney 2018 Fig. 3. |
| `measurement-based` | necessary: `executed_hmr > 0` and `condition_bits > 0`; sufficient: `measurement_uncompute` | The harness found an Hmr followed by a conditioned CZ/Z on the former controls. Gidney 2018 Fig. 3; Babbush 2018 Fig. 4; Berry 2019 App. C (lookups). It cannot say what fraction of temporaries was erased this way. |

### reuse: how scratch is organized across prepare, select, unprepare

| Value | Signature |
| --- | --- |
| `serial` | declared |
| `shared-workspace` | declared (Berry 2019 Sec. 3.2: forward-QROAM ancillae "can be erased after the QROAM and reused") |

Both are honest labels only. Telling them apart needs a per-qubit record of which segment
wrote each ancilla during one allocation. Even that is subtle, because serial designs also
write lookup outputs in prepare and erase them in unprepare. No such fact exists.

### rotation: arbitrary-angle rotations

| Value | Signature | Proves |
| --- | --- | --- |
| `phase-gradient-givens` | exact: `op_counts.GIVENS > 0` | The Givens op is by definition a register-angle phase-gradient rotation charged at the formula of spec/DESIGN.md section 15. Lee 2021 App. C step 4(d); von Burg 2021 Supp. VII.C.1 (the phase gradient technique, their ref. [34] = Gidney 2018). The angle bits are a parameter, not a family. |
| `none` | exact: `op_counts.GIVENS == 0` | The op set has no other arbitrary-angle gate. A phase-gradient register is not a basis state, so it cannot live on a lane. Ruled out for `sos-sa` by rule. |
| `nonorthogonal-thc` | exact: `encoding == "thc"` and `op_counts.GIVENS > 0` | The same Givens primitive and charge, rotating onto one non-orthogonal THC factor at a time. Lee et al. 2021 Sec. II.D and III.C step 3. |

## Compatibility rules

| Rule | If | Then |
| --- | --- | --- |
| sa-lane-maps | encoding sos-sa | lane_map in {sa-nested-alias-v1} |
| sa-nested-select / sa-select | lane_map sa-nested-alias-v1 / encoding sos-sa | select in {givens-sa-nested} |
| sa-rotation | encoding sos-sa | rotation in {phase-gradient-givens} |
| *-needs-lookup | any alias lane map (one rule per family, `sa-nested-needs-lookup` included) | lookup not none |
| monomial-measures | lookup monomial-lookup | uncompute measurement-based |
| sparse-lane-maps | encoding sparse | lane_map in {alias-v1, sparse-sym-alias-v1} |
| sparse-select | encoding sparse | select in {unary-pauli, selected-majorana, clifford-mask} |
| sparse-rotation / df-rotation | encoding sparse / df | rotation none / phase-gradient-givens |
| df-lane-maps | encoding df | lane_map in {df-pair-alias-v1, df-nested-alias-v1} |
| df-pair-select | lane_map df-pair-alias-v1 | select in {givens-network} (a flat lane map admits no `Reflect`) |
| df-nested-select | lane_map df-nested-alias-v1 | select in {givens-nested} |
| df-select | encoding df | select in {givens-network, givens-nested} |
| thc-lane-maps | encoding thc | lane_map in {thc-pair-alias-v1} |
| thc-select | encoding thc | select in {givens-network} |
| thc-rotation | encoding thc | rotation in {nonorthogonal-thc} |

## What the checks add up to

For an `sos-sa` submission the harness can verify **encoding, lane_map, select and rotation**
(exact signatures), and **uncompute** when its predicate holds. **lookup** and **reuse** are
DeclaredOnly: no fact separates their values.

The rules fix encoding, lane_map, select and rotation for every `sos-sa` run, so on the two
tracks families can differ only in lookup, uncompute and reuse, and of those only uncompute can be
verified. The five architectures shipped in `src/walk/sa_low` declare the same axis tuple
(`qroam-clean`, `measurement-based`, `serial`), so they are one family under different names: their
differences are calibration in the sense of the family rule.

## Changing the taxonomy

- MAJOR: a value is removed or a signature's meaning changes.
- MINOR: a value, axis or rule is added, or a reserved value becomes active.
- PATCH: wording only.

Version history. 1.1.0 and 1.2.0 (MINOR) added the nested double-factorization and THC values and
their rules, and widened `givens-network`'s encoding clause to include `thc`. 1.3.0 (MINOR) added
`sos-sa`, `sa-nested-alias-v1` and `givens-sa-nested` (exact signatures), the rules
`sa-lane-maps`, `sa-nested-select`, `sa-select`, `sa-rotation` and `sa-nested-needs-lookup`, and
the reference `low2025`. No existing value, signature or rule changed in any of these, and every
family declaring a 1.x version is accepted (same MAJOR) with unchanged verdicts.

`load_taxonomy` rejects files that are inconsistent: a strength that does not match its
predicates, an unknown fact path, a reserved value without a TODO, a rule that names an unknown
value, a citation of an unknown reference, or a duplicate id. It cannot check that a citation
says what it claims. The citations here were checked against the papers' text at the locations
given.
