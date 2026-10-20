# Circuit-to-encoded-operator equivalence

`encoded_operator.py` checks the leading estimated-track circuits against the **rounded
operator specified by their binary alias maps and pinned integer-angle payloads**. It
reuses `exact.py`'s Boolean controller and gate-word normalization. The historical checker
and its rigorous-promotion certificate are unchanged. This is local maintainer evidence;
it neither authenticates a leaderboard row nor changes acceptance or precision rules.

## Separate statements

| Statement | Evidence | Scope |
| --- | --- | --- |
| The walk is a reflection | Existing Rust reflection checker | Does not identify its Hamiltonian |
| All-input lane equivalence | New checker; every address partition must pass | Every control, uniform value, independent second-pass value and independent HMR record; ideal gate model |
| The certificate describes the emitted files | Independent binary decoder and lowering comparison | Every raw op, register, layout, alias threshold/target and reflection register |
| Which coefficients are encoded | Independent rational expansion of all ordered pairs | Exact dyadic coefficients, normalization and identity shift |
| Rounded operator versus stored payload | Exact coefficient L1 plus explicit identity shift | Conservative norm upper bound; not the actual ground-energy error |
| Stored payload versus chemistry target | Existing interval chemistry audit, separately | Factorization, preprocessing, finite rotation precision and sector conventions |
| Chemical accuracy / end-to-end energy accuracy | **Unresolved** | Neither a reflection nor an operator certificate closes the full error budget |

The report's `encoded_operator_equivalence_certified` is true only after **all** partitions
pass. A diagnostic `--partition` run never sets it and exits 2 even if its part passes.
`certifies_energy_error` and `chemical_accuracy_certified` always remain false.
Failed gate-word matching is an unproved sufficient condition, not automatically a
counterexample to operator equivalence. Boolean restoration/phase failures do have witnesses.
Unsupported operations, malformed artifacts, process failure and resource exhaustion cannot
produce a certificate. Progress reports start with the full-domain flag false.

## Reproduce

From `challenges/femoco`, build the trusted evaluator without contestant code, and use the
existing submission manifests to build the circuits. `tools/repro/audit_encoded.py`
automates this, verifies the three emitted file hashes against the signed ledger records,
runs a 4,096-lane sliced screen, exports the symbolic input and runs the new checker:

```sh
python3 -m pip install -r tools/verification/requirements.txt
python3 tools/repro/audit_encoded.py --out-root /tmp/femoco-operator --jobs 4
```

The selected submissions are the minimum Toffolis-times-qubits rows of each estimated track
in the current ledger; `--submission reiher/outer-hi-reconstruct-r1` or
`--submission li/composed-alias-m8` pins a selection explicitly. Reports retain the selected
row, manifest hash, source commit, source cleanliness and evaluator hash. The hashes are
compared with the ledger; this runner does not itself authenticate the ledger's MACs.

For an already-built circuit with an evaluator export:

```sh
python3 tools/verification/encoded_operator.py /tmp/circuit/symbolic-input.json \
  --payload specs/reiher-sa-est-v1/sa.bin --circuit /tmp/circuit \
  --partition-bits 6 --jobs 4 --network-form sign-normalized \
  --out /tmp/circuit/operator-report.json
```

Six partition bits enumerate all 64 values of the high outer-bucket bits; all other bits
remain symbolic. This is a complete partition of the domain, not 64 sampled addresses.
Finer partitioning trades repeated interpreter work for smaller BDDs. Workers have separate
CUDD managers. The lowered program is packed into fixed-width records before forking, to
avoid duplicating GB of Python dictionary pages. Round-trip tests require identical
interpreter input. The evaluator streams the export one operation at a time, preserving
the original v1 JSON bytes; the fixture regeneration test checks this.

## Independent artifact interpretation

`artifacts.py` reads `FEMOOPS1` and `FEMOLMAP` itself. It checks lengths, padding, operand
shapes, system-wire restrictions, register declarations and every lowered operation against
the export. Thus an export with correct hashes but edited operations or alias thresholds
is rejected. The binary program must have two byte-identical inner copies, exactly one
intervening reflection, and the actual reflection register must be exactly the inner uniform
register. Inner wires cannot be used outside the copies. The Boolean interpreter then proves
restoration of the inner register before reflection, all input registers at the end, every
reset, all ancilla cleanup and the scalar phase including every measurement result.

This removes the unchecked exporter/lowering correspondence from the old checker's trust
base. The independent decoder, both interpreters and their agreement test now belong to the
trust base. It does not turn two implementations into a proof-assistant theorem.

## Exact outcome-phase arithmetic

The controller operations generate only even Z/8 phase increments (0, 2, 4 or 6).
The low nonzero bit is a BDD; the high bit is a square-free polynomial over independent
measurement variables, with lane-only BDD coefficients. Adding 2 adds the exact carry
`old_low AND mask` to the high polynomial and XORs the low bit with the mask; adding 4
XORs the mask into the high polynomial. Adding 6 performs both operations. This is the
same arithmetic as the original three-bit accumulator, with a different representation.

Shannon expansion over GF(2), `P = P(0) XOR m*(P(1) XOR P(0))`, derives every coefficient
exactly. The multilinear representation is unique by induction on the number of outcome
bits: evaluate at m=0 and m=1 to recover the two smaller polynomials. Two phase functions
therefore agree for every outcome precisely when every coefficient of their difference
vanishes. The checker tests that statement as a BDD identity over all lane bits. Products
of outcomes and outcome-dependent carries are retained; linearity is not assumed. The
expected system-word phase is decomposed by the same exact expansion, so a controller phase
may cancel a system-word phase depending on measurements.

This avoids the exponential BDD for a sum of independent HMR signs. The phase engine uses
at most 65,536 monomials per polynomial and recursion depth 128; exceeding either refuses
the proof. It never drops terms or samples outcomes. Tests compare randomized nonlinear
phase programs with both the original BDD adder and an exhaustive integer-modulo-8 oracle,
including every outcome assignment, carries, deliberate mismatches and budget refusal.

## Gate identities, spin transport and vector orientation

Let `K_pq = a_q† a_p - a_p† a_q` and `G_pq(theta) = exp(theta K_pq)`. The canonical
anticommutation relations imply that `K` is zero on the zero/two-particle endpoint sectors
and squares to -1 on the single-particle sector. Consequently, for **any** `p < q`, including
the occupied modes between them:

* `Z_p G(theta) Z_p = G(-theta)`, likewise for `Z_q`; other occupation parities commute.
* `G(theta+pi) = Z_p Z_q G(theta)` and rotations of the same generator add angles.
* `Z_p X_p = -X_p Z_p`.

These are the exact identities used by `exact.normalize`, with angles modulo `2^beta` and
an exact Z/8 scalar phase. A symbolic equality of normalized gate words therefore implies
equality of their system operators. The normal form is sufficient, not complete.

The spin layer is `F = product_p G_(2p,2p+1)(pi/2)`. The same CAR identity gives
`F a_(2p)† F† = a_(2p+1)†` and `F a_(2p+1)† F† = -a_(2p)†`.
Both Majorana flavors of an even-spin mode are thus transported to the corresponding
odd-spin mode. This also transports a whole even-spin chain and its pivot. Requiring entry
`F^-s` and exit `F^s` with the correct selected spin proves the required conjugation; on
inactive branches the gates between them must reduce to identity. Replacing the transport
of a Majorana by a bare qubit X on the other spin omits its Jordan-Wigner prefix and is wrong.

The leaders use the exact sign freedom of a chain vector. For a chain of `N-1` rotations,

`u = (cos(theta_0), sin(theta_0) cos(theta_1), ...,
      product_(j<N-2) sin(theta_j) cos(theta_(N-2)), product_j sin(theta_j))`.

For `j < N-2`, replace `theta_j` by `pi-theta_j`; replace the last angle by `theta+pi`.
Every displayed component changes sign: earlier sine factors stay the same, each earlier
cosine changes sign, and both last trigonometric factors change sign. This proves `u -> -u`
for arbitrary length, including zero components. Both substitutions preserve the integer
angle grid exactly. It follows that `Z(-u)=Z(u)`, and each one-body Majorana changes sign.
The one-body network is fixed by the shared outer item, so its two signs cancel in
`M_b† M_a`, regardless of either inner value or either spin bit. The reference transformation
depends on the payload alone, never on builder levers or a supplied correctness annotation.

`tests/test_fermion_oracle.py` evaluates independent exact Jordan-Wigner creation/annihilation
operators in `Q[z]/(z^4+1)`, `z=exp(i*pi/4)`. It checks the normalization identities on
nonadjacent modes with every spectator occupation, spin transport of both Majorana flavors,
and multi-rotation vector flips. Partial flips and omitted Jordan-Wigner signs are rejected.
No floating-point tolerance or Gaussian simulator enters these tests. They are finite
regressions supporting the general derivation above, not a formal proof of that derivation.
Real Rust-generated three-orbital sign-normalized fixtures additionally exercise the BDD
reference selection against the original, unmodified payload.

## Reflection composition and exact coefficients

For a fixed outer item, proved restoration before reflection and full-domain lane equality
give the kernel `M_b† M_a` for every pair of inner values. In the ideal model the inner
reflection is exactly `2|+><+| - I`. Its off-diagonal matrix elements are `2/D` and its
diagonal elements are `2/D - 1`. Summing this kernel under uniform preparation yields

`A = lambda_decl sum_alpha p_alpha (2 B_alpha† B_alpha - I)`,
`B_alpha = sum_x q_alpha,x M_alpha,x`.

Conditional HMR records are independent symbolic variables; their phases and cleanup are
checked. An X-basis measurement of a computed Boolean ancilla contributes the usual scalar
normalization and a diagonal sign. Conditions depend only on classical measurement history;
after the checked correction the logical operator is independent of the record. The ideal
measurement/erasure and uniform-preparation interpretation is part of the gate model.

The binary alias decoder counts **all preimages** by integer interval lengths, not by sampling
representatives or comparator boundaries. The coefficient audit then expands every ordered
pair, including both one-body spins, both square spins, the duplicated identity item's spin,
and every diagonal pair. Every payload binary64 number is an exact dyadic rational.
Coefficient signs and rotation words are in the system-word proof; coefficient magnitudes
are in this separate arithmetic audit.

Write `H_enc = E_SOS + lambda_decl + A` and `H_payload = identity + sum_(x!=y) c_t M_t`.
If `delta_t = c_hat_t - c_t`, the independent expansion checks the exact identity

`identity_shift = 2(lambda_decl - Lambda_payload) - sum_t delta_t`,
`H_enc - H_payload = identity_shift I + sum_t delta_t M_t`.

Each `M_t` is unitary, so
`||H_enc - H_payload|| <= |identity_shift| + sum_t |delta_t|`.
The audit retains that shift rather than silently assuming the chemistry offset is unchanged.
Its coefficient L1 must agree exactly with `score.json`'s `rounding_error_exact` in the
reproduction runner. A changed coefficient magnitude can preserve every SELECT gate and
still change this error; regression tests cover that case as well as normalization changes.

## Chemistry obligations remain separate

The [interval chemistry audit](../chemistry/README.md) supplies the current evidence between
the stored approximation and the original, fixed-electron-sector FCIDUMP target. Its
committed validated calculations give operator-fit lower bounds, after any scalar shift,
above 47.500 mHa (Reiher) and 21.340 mHa (Li). Those obstruct a uniform 1.6 mHa operator-norm
argument for the uncorrected published fit. They do **not** lower-bound ground-energy error.
Current rotation/preprocessing upper bounds are below 52.935 and 368.441 mHa respectively;
higher precision proposals and the corrected-Hamiltonian experiments are separate operators.

An encoded-operator certificate supplies no new bound for any of those chemistry differences.
The coefficient error computed here must also be added, with its identity shift. An
end-to-end claim additionally needs ground-state preparation/selection, a justified QPE
confidence/error allocation, physical synthesis and implementation error bounds. The deep
statistical sample bounds in SAMPLING.md remain useful regression evidence, but they cannot
replace these systematic-error obligations.

The trust base is Python and its exact integers/rationals, CUDD/dd, the binary parsers, the
CAR identities and composition argument above, the pinned spec transcription in `exact.py`,
and the ideal definitions of Givens, spin-swap, uniform preparation and reflection. A
machine-checked derivation of these statements and their physical implementation remains
open. All reports distinguish this computational proof scope from chemical accuracy.
