# Deterministic coverage and symbolic obligations

The rigorous tracks use the existing stored `reiher-sa-v1` and `li-sa-v1` Hamiltonians and
their exact **coefficient** 1-norm rounding rule, `sum |c - c_hat| <= 1/10000 Ha`.
This does not add a rigorous bound for the original DFTHC approximation or the finite-
precision rotation networks. Estimated tracks and their historical rows remain separate.
The contract epoch is `femoco-sa-rigorous-v2`.

## Evidence layers

| Layer | Domain | Evidence | Limit |
| --- | --- | --- | --- |
| Coefficient rounding | All declared coefficients | Exact rational arithmetic in the evaluator | Relative to the stored Hamiltonian |
| Sampled regression | 4,096 reference + 524,288 sliced lanes in the judge | Existing term, sign, phase, cleanup and control-off checks | Statistical; resources are sample means |
| `--coverage exhaustive` | Every control, uniform and second-pass inner value | Complete finite input enumeration, capped at 16,777,216 lanes | Measurement outcomes sampled; Gaussian comparisons numerical |
| `--coverage terms` | Every reachable term pair plus alias comparator boundaries on both passes | Deterministic and independent of the lane seed | Representatives do not prove constancy inside an alias cell |
| Symbolic controller | Every control, uniform, second-pass and independent HMR assignment | UNSAT miter for restoration, every reset, final ancillas and the inner register at Reflect | Does not compare the quantum operator to the spec |
| Symbolic measurement reduction | Same full domain | System-event parameters and scalar phase match the zero-HMR execution | Sufficient trace equality; some equivalent rewrites can fail it |
| Symbolic semantic reduction | Every pair of inputs naming the same terms | Independent exact alias decoder plus UNSAT trace/phase miter | Must be combined with digest-matched term checks |
| Exact symbolic quantum proof | Every control, uniform, second-pass and independent HMR assignment | Canonical Boolean functions plus exact integer-angle/Pauli identities against the pinned payload | Supported untapered spin-swap chain words; unsupported words remain unproved |

`terms` chooses a preimage of each nonzero-count alias item, all semantically relevant spin
values, both controls, and every ordered first/second item pair. It omits identity-spin
duplicates and a square's unused outer-spin bit from term representatives. The boundary
layer visits each bucket at sigma `0`, `keep-1`, `keep`, and `2^mu-1` wherever valid, including
padding and keep=0. Inner boundaries are tested diagonally and in both orientations against
an anchor; outer boundaries are also visited. This is not the cross product of every outer
and inner boundary. All cases must pass; hitting the cap rejects without partial success.

Coverage has its own lane set and never changes the sampled resource tallies or statistical
bounds. Its report is `metrics.validation`; neither deterministic mode sets `certified=true`.
Ordinary runs without these flags retain the old output shape.

## Reproduction

From `challenges/femoco`, build the trusted evaluator without contestant code:

```sh
cargo build --release --locked --no-default-features --bin eval_circuit
target/release/eval_circuit --root /path/to/circuit --samples 4096 --engine reference \
  --coverage terms --export-symbolic /path/to/circuit/symbolic-input.json
python3 -m pip install -r tools/verification/requirements.txt
python3 tools/verification/symbolic.py /path/to/circuit/symbolic-input.json \
  --out /path/to/circuit/symbolic-report.json --timeout-ms 120000
```

The symbolic input comes from the trusted parser/compiler after rounding and sampled
validation pass. It contains the lowered program, resolved registers, layout, reflection
layout and alias tables. The input SHA-256 and ops, lane-map, family and spec digests bind
the report to the circuit. The SMT interpreter uses Boolean expressions and an exact
three-bit phase accumulator. Every HMR has an independent free Boolean outcome, including
conditional measurements. System wires never feed back to the controller: the compiler
enforces this restriction. A quantum gate is an opaque ordered event whose enable/angle
parameters are compared exactly, without trigonometry or floating-point arithmetic.

`unsat` proves only the named obligation. `sat` supplies a counterexample to that obligation;
a trace-miter counterexample can mean the circuit uses an equivalent different trace.
`unknown`, timeout, unsupported opcode and checker failure never count as a proof. Full
FeMoco programs contain millions of gates; monolithic SMT runs can be expensive or
inconclusive. Reports retain the solver version, checker hash and each obligation's status.
The SMT-only reports always retain `full_quantum_equivalence_certified=false`.
The checker exits successfully only when every attempted obligation is proved; a semantic
counterexample cannot be hidden by a passing controller result. Counterexamples include
control/uniform/second-pass values and a complete sparse assignment of measurement outcomes.

If controller, measurement and semantic obligations all pass, every input/outcome has the
same restored state and exact gate trace/phase as its checked semantic representative.
That justifies reducing the remaining system comparison to exhaustive term representatives.
The Gaussian comparisons still use numerical tolerances; exact algebraic quantum equivalence
would be a further verification layer. Symbolic reports are independent audit evidence;
the judge does not accept a submitter-supplied certificate or require an SMT success.

## Exact quantum proof

`exact.py` supplies that further layer for the supported circuit structure. It reads the
evaluator export and independently parses `sa.bin`, requiring the payload hash to match the
export. Re-export with the current evaluator: the new `rotation_widths` field must explicitly
be `null`. Tapered rotation schedules and unsupported system opcodes are rejected.

```sh
target/release/eval_circuit --root /path/to/circuit --samples 4096 --engine reference \
  --export-symbolic /path/to/circuit/symbolic-input.json
python3 tools/verification/exact.py /path/to/circuit/symbolic-input.json \
  --payload specs/reiher-sa-v1/sa.bin --partition-bits 8 --jobs 4 \
  --out /path/to/circuit/exact-report.json
```

This checker uses CUDD through pinned `dd==0.6.0`, with a separate Boolean decision diagram
manager per worker. All controller wires, classical bits and the scalar phase are exact
Boolean functions; the phase is in Z/8. Each HMR occurrence gets its own independent free
outcome. Every reset, the inner register at reflection, final ancillas, control and uniform
register are checked. Conditional HMR branches preserve the interpreter's measurement and
phase semantics. Measurement conditions depend only on classical history, so their branch
normalizations carry no information about the quantum input.

The selected outer and inner items are independently decoded from the alias tables, using
all sigma bits. The reference uses the payload's integer angles and coefficient signs to
construct each `M_a` and `M_b` adjoint. It checks each system sandwich, including its spin
selection, pivot Pauli and scalar sign, using these exact identities:

- Conjugating a fermionic Givens rotation by Z on exactly one endpoint negates its angle.
- `G(theta + pi) = Z_p Z_q G(theta)` and rotations on the same pair add their angles.
- Moving Z across X adds a minus sign; angles are reduced modulo `2^beta`.
- The spin-swap layer and its adjoint transport the pivot Majorana to the selected spin.
- A chain's vector sign change preserves a square and negates a single Majorana. Any
  resulting one-body sign is explicitly included in the global phase obligation.

On branches with no active pivot (control off or a square identity item), the entire
system word must reduce to identity, including any padding rotations. Every residual
scalar sign is retained. No sine, cosine, numerical tolerance, sampled lane or assumed
builder annotation enters the proof. The two independently variable inner inputs establish
all matrix elements of the nested reflection composition in the evaluator's contract.

`--partition-bits 8` splits the domain into all 256 assignments of the high outer bucket
bits. The remaining inputs and **all** measurement outcomes stay symbolic in each part.
All parts must pass. Checkpoints and `--partition N` diagnostic runs always retain
`full_quantum_equivalence_certified=false`; only completion of the entire partition domain
sets it to true. Unknown words, a failed obligation, process failure or resource exhaustion
never certify equivalence. A failed gate-word comparison is an unproved sufficient condition,
not necessarily a counterexample to quantum equivalence. Boolean failures include a witness
to the stated obligation; omitted variables in a BDD witness are don't-cares.

The proof's trusted base is the evaluator parser/compiler and export, the independent payload
decoder and small algebraic checker, CUDD's Boolean canonicalization, and the stated gate
identities. It is not a Lean/Coq proof or a proof of CUDD's implementation. It certifies the
declared logical operations and pinned finite-precision Hamiltonian, not a new bound on the
original chemistry approximation or physical rotation synthesis. Reports bind the input,
checker source, circuit, lane map, family and payload by SHA-256. They remain local audit
evidence; they neither write the ledger nor replace judge validation.

## Rigorous promotions

[`../../rigorous/promotions.json`](../../rigorous/promotions.json) lists eight candidates:
the Low-layout reference and low-Toffoli, low-product and low-qubit frontier choices for each
molecule. Source manifests remain immutable. Builds use 19/19 alias keep bits (Reiher) and
20/21 (Li), and recheck the exact coefficient rule. `.d` (Reiher) and `.b` (Li) are disabled
because they require the estimated mu8 spec. Other mechanisms are retained.
The [local validation evidence](../../rigorous/README.md) records the rebuilt resource frontier,
artifact digests, deterministic coverage and the full-sized symbolic attempt's status.

```sh
python3 tools/repro/promote_rigorous.py --out-root /tmp/rigorous-runs
python3 tools/repro/promote_rigorous.py --out-root /tmp/rigorous-proof \
  --id reiher-fewest-toffoli-rigorous --symbolic
```

The runner writes isolated circuits, scores, logs and reports. Its bulk runtime-parameter
builder links `walk`; the evaluator is separately built without `walk` and copied before
the feature build. This is local maintainer tooling. The normal sandboxed `build_circuit`
and judge are the route to authenticated results. Neither this runner nor this PR writes
`results.tsv`. New staging tracks retain pending baselines until fresh circuit submissions
are recorded. No estimated result is relabelled into the rigorous leaderboard.
