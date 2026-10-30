# Lean controller certificates

An opt-in, executable first step toward proof-carrying FeMoco optimisation. Lean proves
soundness of an exact Boolean-polynomial controller interpreter and checks complete
`FEMOOPS1` binary programs in its supported signed-basis subset. No full-size Reiher or Li
walk circuit has a Lean certificate from this package yet.

This is maintainer verification tooling, not a scored circuit submission or a new track.
It does not alter the evaluator, emitted circuits, ledger, precision rules or acceptance
standard. The existing [symbolic operator checks](../tools/verification/README.md) remain
separate evidence.

## What is proved

| Component | Statement |
| --- | --- |
| [Boolean.lean](FeMoco/Boolean.lean) | XOR, AND, NOT, monomial multiplication and equality checking are sound for every Boolean assignment. Nonlinear measurement products are retained. |
| [Controller.lean](FeMoco/Controller.lean) | Symbolic interpretation equals concrete signed-basis execution for every input/outcome assignment. Acceptance implies equality of every declared wire and the sign, and cleanliness of every reset. |
| Controller certificate | Proper physical operands, correctness and static resource bounds refer to the same program. Executed conditional counts never exceed the static bounds. |
| [Binary.lean](FeMoco/Binary.lean) | A strict executable decoder connects literal bytes to that typed program. Generated certificates include a kernel-checked decoder equality, rather than trusting a Python lowering or a hash alone. |
| [Examples.lean](FeMoco/Examples.lean) | Measurement-based AND erasure restores both inputs, the ancilla and an arbitrary incoming sign for every independent measurement outcome. Phase and reset mutants are refused. |

The polynomial checker is sound without assuming its arguments are canonical. Failure is
an unproved sufficient condition, not necessarily inequivalence. This first implementation
uses lists, not CUDD or a scalable proof-producing BDD. It is intended for bounded gadgets
and controller fragments that are themselves complete programs under a declared interface.
A timeout or an oversized expression cannot certify a program.

## Run

Install the toolchain in `lean-toolchain` (Lean **v4.28.0**). There are no third-party Lake
packages, Mathlib downloads or Python dependencies.

```sh
cd challenges/femoco/lean
lake --wfail build
python3 scripts/verify.py --selfcheck
python3 scripts/verify.py /absolute/path/to/program.femoops \
  --wires 3 --inputs 0,1 --out /tmp/controller-report.json
```

`--wires` is the complete static wire interface. Every declared input bit is arbitrary;
all other wires start at zero. The postcondition is the identity on **all** wires, the
incoming sign is arbitrary and restored, and every reset must have a zero target when
reached. This is intentionally a fixed identity-controller contract, not a way to choose
an arbitrary convenient postcondition.

The command builds the current package, checks the pinned compiler version, and generates
Lean source containing only literal bytes and validated numeric interface parameters.
Lean computes the decoding and verifies the certificate with kernel reduction (`decide`).
A separate `leanchecker` process replays the generated artifact; transitive axiom audits
allow only `propext`, `Quot.sound` and `Classical.choice`. `sorryAx`, custom axioms and
`Lean.ofReduceBool` (including native-evaluation-based proof shortcuts) are refused.
The runner accepts binary files, not submitted Lean source or compiled artifacts.

The binary is read again after replay to reject concurrent changes. A failed attempt
removes any report at the requested output path before verification, so stale success is
not retained. Reports record the exact binary digest and keep quantum-equivalence and
energy-accuracy flags false. Reports are local evidence, not authenticated ledger rows.
The reported counts are static upper bounds, not a sampled mean or a physical runtime.

The integration suite constructs exact binary streams in the existing Rust wire format,
including the positive and negated AND-erasure patterns used by
[`shared/unary.rs`](../src/walk/shared/unary.rs), reused classical measurement storage,
missing/wrong corrections, dirty resets, a phase defect behind 32 simultaneous outcomes,
malformed headers/lengths/padding, illegal wire
aliasing, unbound conditions, and unsupported gates:

```sh
# From the repository root, with Lean on PATH:
FEMOCO_LEAN_REQUIRED=1 python3 -m unittest discover -s tests -p test_lean_controller.py -v
```

The dedicated read-only CI workflow installs a checksum-pinned official Lean distribution,
builds with warnings as errors, replays the kernel and runs these positive/adversarial
checks. Ordinary Python tooling runs skip the integration class if Lean is absent;
the dedicated CI job requires it.

## Binary subset and measurement scope

The on-disk layout is the existing `FEMOOPS1` header plus a little-endian count and 56-byte
records. The Lean decoder checks the entire file length, byte range, padding, operand
shape, bounded wire indices and gate operand distinctness. It supports `NEG`, `X`, `Z`,
`CX`, `CZ`, `CCX`, `CCZ`, `HMR`, and clean `R` obligations. Ordinary gates may be conditioned
on a previously stored measurement bit. Each HMR is assigned a fresh variable, even when
its classical destination is reused. Inputs, incoming sign and outcomes use disjoint
variable namespaces. Classical outcome records are retained; the identity postcondition
concerns quantum controller wires and the sign, not erasure of those records.

Conditional HMR, conditional reset, condition-stack operations, classical bit manipulation,
register declarations, swaps, S/Sdg, Givens, spin swaps, reflection and all other opcodes
are currently refused. The decoder does not silently strip metadata or ignore gates.
Its explicit caps are 4,096 interface wires and 100,000 operations; these are refusal
limits, not a scalability claim. The polynomial representation can become expensive well
before those limits. Compilation and replay each have a configurable process timeout.

The signed-basis HMR model updates the sign by `outcome AND target` and clears the target.
Outcomes are universally quantified free Boolean variables. This does **not** assert their
physical probability distribution, establish quantum-channel equivalence, or prove
measurement-record decoupling on arbitrary entangled states. The binary parser is a new
formal specification of the supported subset; its correspondence to the Rust evaluator
is reviewed and tested, not a formal refinement theorem about Rust.

## Applying this to the full challenge

The first full-circuit target is the existing `reiher-fewest-toffoli-rigorous` candidate in
[the promotion catalogue](../rigorous/promotions.json), whose existing symbolic evidence
is described in [the rigorous verification report](../rigorous/README.md). Its full Lean
proof remains outstanding. The implementation order is:

1. Add a proof-producing DAG/BDD representation, sharing work across gates and partitions.
   Have the existing search tools emit witnesses, while keeping acceptance in a small
   proved checker. Include completeness and disjointness of partition coverage.
2. Extend the binary/interpreter refinement to the actual controller instruction set,
   three-bit phase arithmetic, conditional measurements and reflection interfaces. Bind
   every proof to the complete emitted operations and pinned payload; no unchecked
   exporter interpretation should enter the theorem.
3. Prove the system-word identities: CAR/Jordan-Wigner signs for arbitrary separated modes,
   Givens identities, both Majorana spin transports, and reflection composition. Attach
   complex-valued semantics to the measurement controller and prove record decoupling.
4. Prove the alias preimage counts, normalization and scalar identity correction against
   the fixed rounded Hamiltonian. An exact circuit for the wrong Hamiltonian is insufficient.
5. Only then propose a separate formal track with protected specifications, independent
   kernel replay and a precise resource convention. Keep static support/upper bounds
   distinct from the existing mean-Toffoli/peak-live leaderboard.

Original chemistry approximation, finite rotation synthesis, ground-state preparation or
selection, and QPE accuracy/confidence remain separate obligations. Kernel checking does
not tighten a loose mathematical error bound by itself. No result here certifies chemical
accuracy or a complete ground-energy algorithm.

## Source and attribution

The architecture is informed by Yukon's
[ecdsa.fail Lean challenge](https://www.yukon.org/ecdsafail-lean) and
[Layr-Labs/ecdsafail-lean](https://github.com/Layr-Labs/ecdsafail-lean/tree/72d11defaf275d2429601cf9b1d9f97734f46721):
fixed proof contracts, same-program resource bounds, independent replay and axiom auditing.
This package is an independent implementation; it does not vendor their secp256k1 library.
Their challenge's main signed-basis certificate also has an explicit
[quantum-semantics scope boundary](https://github.com/Layr-Labs/ecdsafail-lean/blob/72d11defaf275d2429601cf9b1d9f97734f46721/docs/PROOF_SCOPE.md).
Their retained-record quantum-semantics modules are useful candidates for the later
measurement bridge, rather than an already proved FeMoco result.
