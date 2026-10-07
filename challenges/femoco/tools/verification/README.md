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
They always retain `full_quantum_equivalence_certified=false`.
The checker exits successfully only when every attempted obligation is proved; a semantic
counterexample cannot be hidden by a passing controller result. Counterexamples include
control/uniform/second-pass values and a complete sparse assignment of measurement outcomes.

If controller, measurement and semantic obligations all pass, every input/outcome has the
same restored state and exact gate trace/phase as its checked semantic representative.
That justifies reducing the remaining system comparison to exhaustive term representatives.
The Gaussian comparisons still use numerical tolerances; exact algebraic quantum equivalence
would be a further verification layer. Symbolic reports are independent audit evidence;
the judge does not accept a submitter-supplied certificate or require an SMT success.

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
