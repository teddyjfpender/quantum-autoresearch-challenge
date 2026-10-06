# FeMoco walk-step challenge: design contract

This document is the contract between the trusted harness and a submission. Sections 1 to 11 are
cited by number from code comments; their numbers and topics are fixed. The spectrum-amplified
specs have their own contract, spec/SPEC-SA.md, and the accuracy and qubit conventions for claims
are in spec/CONVENTIONS.md.

## 1. What the challenge is

The benchmark scores one step of the controlled qubitized walk used in phase estimation of the
FeMoco Hamiltonian, at real size, and validates it by simulating the submitted op stream on test
inputs seeded by a Fiat-Shamir hash of that op stream. The design follows ecdsa.fail, which does
the same for one point addition of Shor's algorithm on secp256k1.

- **Instance.** FeMoco, the iron-molybdenum cofactor of nitrogenase, in the two active spaces the
  resource-estimation literature uses: `reiher` (Reiher et al. 2017, 54 orbitals, 108 spin
  orbitals, 54 electrons) and `li` (Li et al. 2019, 76 orbitals, 152 spin orbitals, 113
  electrons).
- **The primitive.** One controlled walk step: a controlled block encoding of the encoded operator
  plus the reflection. An energy estimate repeats it about `pi * lambda / (2 * epsilon)` times, so
  its cost is the cost of the algorithm.
- **Encoding spec.** Which operator the walk encodes is fixed by a pinned spec, identical for
  everyone, like the curve in ecdsa.fail. There is one acceptance standard: the spectrum-amplified
  sum-of-squares encoding `sos-sa` of Low et al. 2025 (Phys. Rev. X 15, 041016), with their
  estimated rounding class (section 13).
- **Two tracks.** `reiher-sa-est-v1` and `li-sa-est-v1`, one per instance. Their rigorous twins
  `reiher-sa-v1` and `li-sa-v1` state the same operator (the same `sa.bin`) under the exact
  0.1 mHa rounding rule of section 6; they ship as test fixtures and are not tracks.
- **Score.** The challenge score is `C_step x Q_peak`, Toffolis times qubits, lower is better
  (the repository's `spec/SCORING.md`). `C_step` is the average executed Toffoli count per walk
  step (section 8) and `Q_peak` the peak number of live qubits. The ledger computes it from those
  two measured cells. `eval_circuit` also writes a normalization-weighted figure,
  `lambda_eff_used x C_step x Q_peak`, to the `score` field of `score.json`; it is report-only
  and is not the challenge score. `lambda_eff_used` is the spectrum-amplified walk's effective normalization (section 12): the
  larger of the bound from the spec's ground-energy certificate and the value Low et al. publish.
  The number of walk steps is proportional to it, which is why it is in the score.
- **Unit of search: architecture.** Every submission declares a family tuple from
  `taxonomy/taxonomy.json`. The harness verifies every axis it can from the circuit and records
  which axes are verified and which are only declared (section 10).

What a passing run certifies: this circuit implements the spec's encoding on every
Fiat-Shamir-sampled lane, at this Toffoli count and width, and the lane map it declares is
accepted under the spec's rounding class. On the two tracks that class is an estimate, not a
bound (section 13). What a run does not certify: the ground energy, the overlap of any initial
state with the ground state, or correctness on unsampled lanes (section 9).

## 2. Repository layout

TRUSTED means a submission may not edit it. `benchmark.json` lists `src/walk` as the only editable
path; everything else is trusted.

| Path | Trust | What it is |
| --- | --- | --- |
| `benchmark.json`, `rust-toolchain`, `Cargo.toml`, `Cargo.lock` | trusted | benchmark manifest, pinned toolchain, locked dependencies |
| `setup.sh`, `benchmark.sh` | trusted | toolchain setup and the run pipeline (section 4) |
| `spec/` | trusted | this contract, spec/SPEC-SA.md, spec/CONVENTIONS.md, spec/FAST-EVALUATOR.md |
| `specs/` | trusted | the pinned specs, their payloads and certificates; `specs/INDEX.json` (section 7) |
| `taxonomy/` | trusted | `taxonomy.json` and its explanation `TAXONOMY.md` (section 10) |
| `src/circuit.rs`, `src/circuit/` | trusted | op format, per-op shape validation, `ops.bin` I/O, the `Builder` (sections 4, 5) |
| `src/spec/` | trusted | spec types, the loader, exact rationals, rounding classes (sections 7, 13) |
| `src/lanemap/` | trusted | lane-map families (section 6) |
| `src/fiat_shamir.rs` | trusted | the seed stream (sections 9, 14) |
| `src/sim/` | trusted | static checks, liveness, the bit-sliced lane simulator, the fermionic Gaussian tracker, per-lane validation |
| `src/fastsim/`, `src/equiv.rs` | trusted | the optional fast lane engine and its equivalence harness (spec/FAST-EVALUATOR.md) |
| `src/facts.rs`, `src/taxonomy.rs`, `src/taxonomy/` | trusted | `CircuitFacts` and the taxonomy checker (section 10) |
| `src/score.rs`, `src/score/` | trusted | evaluation, scoring, `score.json` and `results.tsv` writers (sections 8, 11) |
| `src/bin/` | trusted | `build_circuit`, `eval_circuit`, `eval_diff`, `femoco_serve` |
| `src/walk/` | **editable** | the submission: `mod.rs` (dispatch), `sa_low/` (the shipped architectures), `common/`, `shared/` (gadget libraries) |
| `tests/` | trusted | harness, spec, taxonomy, equivalence and adversarial tests |
| `tools/` | trusted | `heavy.sh` (one heavy job at a time), `fastsim/equivalence.sh`, `repro/`, `server/` |

`src/lib.rs` declares every module. `walk` is compiled only with the default-on `walk` cargo
feature; the trusted `eval_circuit` is built without it (section 4).

The harness still contains the trusted spec loaders and lane maps of three other encodings
(`sparse`, `df`, `thc`). No spec or walk for them ships in this repository, and this document
mentions them only where the shared code needs it.

## 3. Conventions

- **Spin orbitals** are interleaved: system qubit `2p` is spatial orbital `p` spin alpha, `2p+1`
  spin beta (OpenFermion's convention).
- **Jordan-Wigner Majoranas** on `n = 2N` spin orbitals (`N` spatial): `gamma_{2j} = Z_0 ... Z_{j-1}
  X_j`, `gamma_{2j+1} = Z_0 ... Z_{j-1} Y_j`.
- **Coefficients** are exact rationals. Float64 inputs are taken as exact binary rationals, and the
  harness compares coefficients and counts in rational arithmetic, never in floating point.
- **Energy units** are Hartree. `epsilon = 0.0016` Ha is used only for the literature-convention
  total `total_toffoli_lit = ceil(pi * lambda / (2 * epsilon)) * C_step` (Lee et al. 2021 and prior
  FeMoco work), reported and not scored.

## 4. Run pipeline

`./benchmark.sh` runs two stages in separate processes. All of its command-line arguments are
forwarded to `eval_circuit`.

1. **Clean slate and build.** It deletes stale `ops.bin`, `lanemap.bin`, `family.out.json` and
   `score.json`, then builds both binaries in release mode, locked and offline: `eval_circuit`
   with `--no-default-features`, so `src/walk` is not compiled into it at all, and `build_circuit`
   in its own target directory (`target/untrusted`), so the two feature sets share no artifacts.
2. **Untrusted stage.** `build_circuit` links `src/walk` and runs sandboxed: bubblewrap on Linux
   (read-only filesystem, no network, all capabilities dropped, unprivileged uid) or `sandbox-exec`
   on macOS (writes denied outside the scratch directory, no network). Its only writable path is a
   throwaway scratch directory, which is its working directory. It runs in its own process group,
   which is killed when it exits. If neither sandbox is available the script prints a warning and
   runs it unconfined; that fallback is for local development only. `build_circuit` loads the spec
   named by `walk::spec_id()` from `$FEMOCO_ROOT/specs/`, calls `walk::build`, checks that the lane
   map's `uniform_bits()` equals the width the builder was given, and writes `ops.bin`,
   `lanemap.bin` and `family.out.json` into the scratch directory.
3. **Fail closed.** Only regular, non-empty files are copied out of the scratch directory. If
   `build_circuit` exited non-zero or any of the three files is missing, the script calls
   `eval_circuit --fail REASON`, which records a `FAIL` row and evaluates nothing.
4. **Trusted stage.** `eval_circuit` re-reads the three files, validates the walk on
   Fiat-Shamir-sampled lanes, counts, writes `score.json` and appends one row to `results.tsv`.

`eval_circuit` flags:

| Flag | Meaning |
| --- | --- |
| `--samples K` | number of sampled lanes (default `2^19`, section 9) |
| `--note TEXT` | free text for the `results.tsv` note column |
| `--root DIR` | directory holding `specs/`, `taxonomy/` and the three input files (default `.`); `score.json` and `results.tsv` are written there |
| `--engine reference\|sliced` | the lane engine. `reference` is the default and the oracle. `sliced` is the fast engine of spec/FAST-EVALUATOR.md. The choice is printed on stdout and not recorded in `score.json` |
| `--fail REASON` | record a `FAIL` row with that reason and evaluate nothing (used by `benchmark.sh`) |
| `--dump-verdicts FILE` | write one byte per sampled lane, in sample order: pass, a failure category index, or unchecked. Used by `tools/fastsim/equivalence.sh` |
| `--server-seed HEX` | 32 bytes chosen by a local artifact server after upload (section 14) |
| `--freeze FILE`, `--audit-round N`, `--audit-seed HEX`, `--audit-signature HEX` | the fresh-seed audit, all four together (section 14) |

Flags may be written `--flag value` or `--flag=value`. Exit status: 0 for a passing run, 1 for a
rejected one (a `FAIL` row is written and no `score.json` exists), 2 for a usage error or a
refused audit (nothing is recorded). A stale `score.json` is removed before anything else.

**Choosing what to build.** The walk reads two variables at compile time: `FEMOCO_WALK_SPEC` (the
track; default `reiher-sa-est-v1`) and `FEMOCO_WALK_ARCH` (the architecture; default `sa-low2025`).
For example `FEMOCO_WALK_SPEC=li-sa-est-v1 ./benchmark.sh`. The shipped architectures are
`sa-low2025`, `sa-low2025-bothspin`, `sa-lowq`, `sa-toff` and `sa-pareto` (`src/walk/mod.rs`).

**Walk API** (`src/walk/mod.rs`, called only by `build_circuit`):

```rust
pub fn spec_id() -> &'static str;                                         // the pinned spec this walk implements
pub fn build(spec: &dyn EncodingSpec, b: &mut Builder) -> Box<dyn LaneMap>; // emits ops through the builder
pub fn family() -> Family;                                                // declared taxonomy tuple, name, parent
```

**Qubit layout**, fixed by the harness: qubit 0 is the walk control; qubits `1..=2N` are the
system register (system qubit `j` is qubit `1 + j`, interleaved as in section 3); then the `u`
uniform-register qubits (uniform bit `i` is qubit `1 + 2N + i`, little-endian). `build` must call
`b.declare_uniform(u)` before anything else, with `u <= 63` equal to the lane map's
`uniform_bits()`. All further qubits are ancillas from `b.alloc()` and must be returned to `|0>`
with `b.free` (emits `R`) or `b.hmr` (X-basis measurement; the circuit must cancel the phase it
leaves). `eval_circuit` re-checks everything the builder enforces, since the builder runs on the
untrusted side.

## 5. Op stream (`ops.bin`)

Magic `FEMOOPS1`, a `u64` op count, then 56 bytes per op in ecdsa.fail's layout, little-endian:
`u32 kind, u32 pad, u64 q_control2, u64 q_control1, u64 q_target, u64 c_target, u64 c_condition,
u64 r_target`. An absent operand is `u64::MAX`. The reader rejects unknown kinds, nonzero padding,
operands that do not fit in 32 bits, and more than `2 x 10^9` ops. Kinds 0 to 17 keep ecdsa.fail's
numbering and meaning (`Neg, Register, AppendToRegister, BitInvert, BitStore0, BitStore1, X, Z, CX,
CZ, Swap, R, Hmr, CCX, CCZ, PushCondition, PopCondition, DebugPrint`). Additions:

| Kind | Name | Meaning |
| ---: | --- | --- |
| 20 | `S` | phase `i` on `|1>` of `q_target` |
| 21 | `Sdg` | phase `-i` on `|1>` of `q_target` |
| 30 | `Givens` | Fermionic Givens rotation `G_{p,q}(2 pi a / 2^beta)` between system modes `p = q_control1 < q = q_target`, where `a` is the value of register `r_target` and `beta` the spec's rotation bits. Accepted only when the spec has a rotation tracker; charged by formula (sections 8 and 15) |
| 31 | `Reflect` | `2\|+><+\| - I` on the inner uniform register. Operand `r_target` only, a register holding exactly that register's qubits; never conditioned. Accepted only when the lane map is nested (section 16) |
| 32 / 33 | `SpinSwap` / `SpinSwapDg` | `sos-sa` specs only. Where the non-system qubit `q_target` is 1, the layer `F = prod_p G_{2p, 2p+1}(pi / 2)` on the system register (or `F^dagger`), which maps every spin-alpha mode to its spin-beta partner. May be conditioned. Charged by formula (section 8); see spec/SPEC-SA.md section 11 |
| 40 | `Segment` | a hint with no effect on simulation; `r_target` is the segment code: 0 prepare, 1 select, 2 unprepare, 3 inner-begin, 4 inner-end. Read by the structural and taxonomy checks |

Every op is checked for operand shape (each operand of a kind is required, allowed or banned) and
no qubit may appear twice in one op.

**No Hadamard exists.** The uniform register's Hadamard layers are implicit: the harness simulates
the block encoding on basis values of the uniform register (lanes), so the circuit between the
implicit Hadamard layers must be a permutation-and-phase circuit on non-system wires.

**System-wire rules** (`src/sim/compile.rs`). A system qubit may only be the target of `X`, `Z`,
`S`, `Sdg`, or of `CX`/`CCX`/`CZ`/`CCZ` in which exactly one qubit is a system qubit (for `CZ` and
`CCZ` it may sit in any position), or be acted on by `Givens` and `SpinSwap` through the tracker. A
system qubit cannot be measured or reset, and registers hold non-system qubits only. So no
non-system wire ever depends on the system state, and a lane's system action is exactly the
operator the simulator tracks.

**`R` is strict.** `R` frees an ancilla and carries no condition of its own; freeing a qubit that
is `1` on any sampled lane rejects the run. `Hmr` frees with an X-basis measurement into the
classical bit `c_target`; a `1` on the qubit leaves phase `-1` on lanes whose outcome is 1. There
is no Z-basis measurement, so classical condition bits cannot depend on a lane's data.

**`Hmr` under a false condition** (`src/sim/lanes.rs`). An `Hmr` executes on the lanes where both
the condition stack and its own `c_condition` (if any) are 1. On the other lanes it does nothing:
the qubit and the classical bit keep their values, no phase is applied, and the lane is not counted
as an executed measurement. The batch's outcome bits are drawn for every lane either way, so the
outcome stream does not depend on conditions. A non-executing `Hmr` does not reject a lane whose
qubit holds 1; that lane fails only if the qubit is still 1 when an `R` frees it or at the end of
the stream. For liveness such an `Hmr` does not free the qubit (section 8).

Static caps: at most `2^20` qubits, `2^22` classical bits and `2^20` registers.

## 6. Lanes, lane maps and the encoded operator

A lane is `(c, s)`: control bit `c` and uniform value `s in [0, 2^u)`. For each sampled lane the
harness runs the whole op stream with the system register carried as an operator and requires:

- the control and uniform registers return to `(c, s)` (for `c = 1`, to `(1, uniform_after(s))`,
  which is `s` for every lane-map family a shipped spec uses); every freed ancilla is `|0>` when
  freed; every live non-system qubit is `|0>` at the end; the lane carries no leftover phase;
- for `c = 0`, the system operator is the identity with phase `+1`;
- for `c = 1`, the system operator equals the reference operator the lane map assigns to the lane,
  including its sign.

**How operators are compared.** For Pauli-only circuits the simulator carries each system qubit as
`x`/`z` bit planes with an exact phase in `Z/8`, and compares with the reference's Jordan-Wigner
frame exactly, phase included. When the spec has rotations (`Givens`, `SpinSwap`), a fermionic
Gaussian tracker carries the system action and compares it with the spec's rotated reference
(`SystemOp::Rotated`: an ordered product of Majorana monomials, each conjugated by a quantized
Givens network). Section 15 describes the tracker.

**The lane map** is declared by the submission as structured data from a harness-supported family
(`lanemap.bin`: magic `FEMOLMAP`, `u16` family-name length, the name, then the family's payload),
so the harness can compute its term counts exactly without simulating the circuit. The trait
(`src/lanemap/mod.rs`):

```rust
pub trait LaneMap: Send + Sync {
    fn family(&self) -> &str;
    fn uniform_bits(&self) -> u32;                                    // u
    fn lambda_decl(&self) -> Exact;                                   // declared dyadic normalization
    fn reference_op(&self, spec: &dyn EncodingSpec, s: u64) -> SystemOp;
    fn uniform_after(&self, s: u64) -> u64;                           // default: s
    fn rounding_error(&self, spec: &dyn EncodingSpec) -> Result<Exact, String>;
    fn rounding_estimate(&self, spec: &dyn EncodingSpec, params: &EstimatedParams)
        -> Result<Estimate, String>;                                  // default: refuse (section 13)
    fn alias_bits(&self) -> Option<(u32, u32)>;                       // (k, mu), facts only
    fn to_bytes(&self) -> Vec<u8>;                                    // lanemap.bin
    fn inner_bits(&self) -> Option<(u32, u32)>;                       // nested families: (first bit, width)
    fn reference_nested(&self, spec: &dyn EncodingSpec, s: u64, after: u64) -> SystemOp;
    // ...
}
pub fn parse(bytes: &[u8], spec: &dyn EncodingSpec) -> Result<Box<dyn LaneMap>, String>;
```

A family that overrides `uniform_after` must make it an involution with
`reference_op(uniform_after(s)) = reference_op(s)^dagger`, so that the block encoding
`sum_s |uniform_after(s)><s| (x) M_s` stays Hermitian. Control-0 lanes always return `s`.

**Alias sampling.** The building block is the alias table: `k` index bits, `mu` keep bits, tables
`keep[2^k]` (u32, `< 2^mu`) and `alt[2^k]` (u32 item ids). With `i = s mod 2^k` and `r = s >> k`,
a value maps to item `i` if `r < keep[i]`, else to `alt[i]`.

**Nested lane maps and `sa-nested-alias-v1`.** The only family an `sos-sa` spec accepts is
`sa-nested-alias-v1` (spec/SPEC-SA.md section 5): an outer alias over the generators' items with
a free spin bit, and per outer item an inner alias of one shared width with a free spin bit. The
inner alias occupies a contiguous inner register inside the uniform register (`inner_bits`). A
nested circuit has the shape "inner block, one `Reflect` on the inner register, the same inner
block again":

- **Structure** (`src/sim/nested.rs`, syntactic). In `Segment` hints the shape is
  `... 3 C 4 R 3 C' 4 ...`, where `C` and `C'` are identical op for op, `R` is exactly one
  `Reflect` on exactly the inner register and the only op between the copies, and nothing outside
  the two copies names an inner uniform qubit.
- **Lanes cross the `Reflect`.** Per lane the harness samples a second-pass inner value (section
  9). The `Reflect` is run as "the inner register holds `a`; make it `b`". Half the lanes are
  diagonal (`b = a`), half paired.
- **Reference.** A control-1 lane must apply `M(b)^dagger M(a)`, the second pass on the left. The
  step then block-encodes `lambda_decl sum_alpha p_alpha (2 O_alpha^dagger O_alpha /
  lambda_alpha^2 - 1)`, Low et al. 2025's `BE[H_SA / Lambda - I]` (their Eq. (10)).

Section 16 gives the full statement. The structural check certifies the architecture (one block
encoding, a reflection, the same block encoding). It cannot show that the two copies act identically, since identical ops can read state
the first copy left behind. The operator is certified by the paired and diagonal lanes.

**Rounding.** From the lane map's exact counts `n_t` and its declared dyadic normalization
`lambda_decl`, the encoded coefficients are `c^_t = lambda_decl n_t / 2^u`. The harness computes
the 1-norm rounding error `sum_t |c_t - c^_t|` against the spec's coefficients exactly, in
rationals. How it is judged depends on the spec's rounding class, which comes from the trusted
spec and never from the lane map:

- **rigorous** (every spec without a `rounding_class` block, including `reiher-sa-v1` and
  `li-sa-v1`): the 1-norm must be at most 0.1 mHa, exactly `1/10000` Ha;
- **estimated** (`reiher-sa-est-v1`, `li-sa-est-v1`): the 1-norm is still computed and reported,
  and acceptance is Low et al.'s procedure (section 13).

A lane map with `u > 63` is rejected.

Other families in the trusted code (`alias-v1`, a flat alias over a spec's terms, used with small
synthetic specs by the harness's own tests; `sparse-sym-alias-v1`, `df-pair-alias-v1`,
`df-nested-alias-v1`, `thc-pair-alias-v1`) belong to encodings with no shipped spec.

## 7. Specs

`specs/<id>/spec.json` (metadata) plus a binary payload; `specs/INDEX.json` lists every spec with
the SHA-256 of both. A spec id must match `[a-z0-9-]+` (at most 64 characters): the id reaches the
loader from the untrusted `family.out.json` and must name a directory directly under `specs/`.

The four shipped specs:

| Spec | Instance | Rounding class | Role |
| --- | --- | --- | --- |
| `reiher-sa-est-v1` | reiher | estimated | track |
| `li-sa-est-v1` | li | estimated | track |
| `reiher-sa-v1` | reiher | rigorous | test fixture, same `sa.bin` as its twin |
| `li-sa-v1` | li | rigorous | test fixture, same `sa.bin` as its twin |

All four have encoding `sos-sa`: Low et al. 2025's DFTHC + BLISS sum-of-squares Hamiltonian
`H_spec = E_SOS + sum_alpha O_alpha^dagger O_alpha`, from the authors' published factors (Zenodo
record 17066718), with rotation networks at their 16 (Reiher) and 15 (Li) bits. `lambda` is the
walk normalization `Lambda` (their Eq. (33)): 58.343974747 and 179.729577124, against their
printed 58.3440 and 179.7296. The operator equals the DFTHC Hamiltonian only on the 54- (113-)
electron sector; off it a BLISS term acts. Payloads are `sa.bin` (format `FEMOSAS1`, 0.13 and
0.41 MB). spec/SPEC-SA.md is the full contract. The scripts that generated the specs and the
source data are not part of this repository; `spec.json` records each source and its hash.

`spec.json` fields common to every spec: `id`, `instance`, `encoding`, `spatial_orbitals`,
`spin_orbitals`, `terms`, `lambda` and `identity` (exact rationals as strings, plus floats),
`parameters` and their source, `published` (the authors' numbers, for comparison), `errors`,
`payload` (file, format, version, sha256) and `generator`. The estimated-class specs add
`rounding_class` and `twin`.

**Error statements are labelled.** Every entry of `errors` carries a `kind`, `rigorous` or
`estimate`, and how it was computed. The rigorous statements are loose: for the DFTHC fit the
rigorous bound is hundreds of Hartree, against a published CCSD(T) estimate of about 0.13 mHa
(spec/CONVENTIONS.md section 1). Nothing in a spec bounds the approximation error at chemical
accuracy.

**Loader** (`src/spec/mod.rs`):

```rust
pub trait EncodingSpec: Send + Sync {
    fn id(&self) -> &str;
    fn encoding(&self) -> &str;
    fn spatial_orbitals(&self) -> usize;
    fn system_qubits(&self) -> usize;           // 2N
    fn lambda(&self) -> Exact;
    fn identity(&self) -> Exact;                // identity coefficient (classical offset)
    fn payload_sha256(&self) -> [u8; 32];
    fn flat_terms(&self) -> Option<Vec<(Exact, SystemOp)>>;   // None unless small enough to flatten
    fn as_any(&self) -> &dyn std::any::Any;
    fn rounding_class(&self) -> RoundingClass;  // default Rigorous
}
/// Loads `<root>/specs/<id>/spec.json` and dispatches on its `encoding` field.
pub fn load(root: &Path, id: &str) -> Result<Box<dyn EncodingSpec>, String>;
```

`Exact` is an exact rational. `SystemOp` is the system action a spec assigns to one term: a
`Monomial` (`i^phase` times a product of Jordan-Wigner Majoranas with strictly increasing indices,
Hermitian) or a `Rotated` product (section 6).

## 8. Counting

- `C_step` is the average over sampled lanes of executed `CCX + CCZ` (ops under a false condition
  do not execute), plus:
  - `2 (beta - 2)` per executed `Givens`: two single-qubit rotations, each `beta - 2` Toffolis by
    addition into a `beta`-qubit phase-gradient register (Lee et al. 2021 App. C; section 15);
  - `N + 1` per executed `SpinSwap` or `SpinSwapDg` on `2N` system qubits: `N` controlled swaps
    and one `CCZ` on the two block parities;
  - the walk reflection, `max(u - 2, 0)`: an AND ladder on the uniform register with
    measurement-based uncomputation, charged by formula;
  - for nested lane maps, `max(w - 2, 0)` per executed `Reflect` on the `w`-qubit inner register
    (section 16.1).
- Because classical bits come only from X-basis measurements (section 5), the average of executed
  Toffolis over lanes is an expectation over measurement outcomes only.
- Low et al. 2025 charge `2 beta` per Givens where this harness charges `2 (beta - 2)`.
  `score.json` reports the derived figure `C_step + 4 x (Givens per step)` for comparisons under
  their charge (section 12).
- `Q_peak` is the peak number of live qubits: control, system, uniform, live ancillas, plus the
  `beta`-qubit phase-gradient register whenever any `Givens` is used.
  - **When a qubit becomes live** (`src/sim/liveness.rs`). Control, system and uniform qubits are
    live throughout. An ancilla becomes live at the first op that names it as `q_target`,
    `q_control1` or `q_control2`, with these exceptions: `Register`, `AppendToRegister`,
    `DebugPrint` and `Segment` never make a qubit live (declaring a register is not a use), and an
    unconditioned `R` or `Hmr` at condition depth 0 on a qubit that is not live leaves it not
    live. A `Givens` makes every qubit of its angle register live, even if no gate has touched
    them.
  - **When it stops being live.** Only an `R`, or an `Hmr` with no `c_condition`, at condition
    depth 0 (outside every `PushCondition` block) ends liveness. An `Hmr` with its own condition,
    or any `Hmr` or `R` inside a condition block, counts as a use: the qubit stays live until a
    later unconditional free at depth 0, or to the end of the stream.
  - The peak is taken over the whole op stream and is the same for every lane. Reusing a freed id
    and allocating a fresh one count the same.
- Cliffords, measurements, resets, op depth and Toffoli depth are counted and reported, not
  scored.

## 9. Validation and sampling

- **Fiat-Shamir seed.** `SHAKE256("femoco-walk-fiat-shamir-v1" || sha256(spec payload) ||
  sha256(lanemap.bin) || sha256(family.out.json) || sha256(ops.bin))`. Every input is committed
  to, so changing any byte redraws every lane.
- **Seed stream**, read in this order:
  1. 32 bytes, the measurement key. Each batch's `Hmr` outcomes come from
     `SHAKE256("femoco-walk-hmr-v1" || key || u64 batch index)`, so results do not depend on
     thread scheduling.
  2. One little-endian `u64` `w` per lane: `c = w & 1`, `s = (w >> 1) mod 2^u`.
  3. Nested lane maps only: one more `u64` `x` per lane. If `x & 1 = 1` the lane is diagonal (its
     second-pass inner value `b` equals its inner value `a`); otherwise `b = (x >> 1) mod 2^w`.
     The lanes of step 2 are unchanged.
- **Sample count.** `K = 2^19` by default; `--samples K` overrides. Every sampled lane must pass.
  `score.json` records `K` and `f_bound = ln(1/delta) / K` with `delta = 1e-6`: with confidence
  `1 - delta`, at most a fraction `f_bound` of lanes are wrong (`2.6e-5` at the default `K`).
- **Implied error bound.** Only control-1 lanes test the encoded operator, and about half the
  lanes have control 0, so the bound counts control-1 lanes only. A circuit wrong on every
  control-1 lane of a fraction `g` of uniform values has only `g / 2` of its lanes wrong but is
  `2 lambda g` from the declared operator. For a flat lane map `implied_error_bound = 2 lambda
  ln(1/delta) / K_1`, with `K_1` the sampled control-1 lanes (`control_one_samples`). For a nested
  lane map, which every `sos-sa` run has, it is `2 lambda (2 f_p1 + f_d1)`, with `f_p1` and
  `f_d1` the same expression over the control-1 paired and diagonal lanes, at confidence
  `1 - 2 delta` (section 16.4). It is reported, not charged, and it is far above chemical accuracy at any
  practical `K`: the sampled check is ecdsa.fail's standard (every drawn lane passes), not an
  energy-error statement.
- **Known limit.** The implied bound holds for the encoded operator only if the block encoding is
  exactly Hermitian on every lane. Certifying that for unsampled lanes would need a symbolic
  involution check of SELECT, which the harness does not have.
- **Grinding.** The seed commits to every input, but any byte of `family.out.json` or `ops.bin`
  is a free nonce, so a submitter who knows which lanes are wrong can search offline for a draw
  that misses them. This is inherent to Fiat-Shamir with a cheap verifier. `score.json` therefore
  also reports `f_bound_grinding = ln(2^40 / delta) / K` (`7.9e-5` at the default `K`) and the
  matching `implied_error_bound_grinding`, which keep confidence `1 - delta` against `2^40`
  re-draws (`GRINDING_LOG2` in `src/score.rs`). Section 14 describes the audit mode that removes
  the nonce.
- **Per-lane checks.** Those of section 6. A failing run reports the first failure category that
  occurred, from: `dirty-ancilla-freed`, `tracker`, `registers-not-restored`,
  `dirty-ancilla-at-end`, `phase-garbage`, `control-0-not-identity`, `not-a-pauli`, `wrong-phase`,
  `wrong-term`, `inner-register-at-reflect`.
- **Mutant tests** (`tests/harness_mutants.rs`). A circuit wrong on one term's sign, one wrong
  term, a dirty ancilla (freed or at the end), phase garbage, an ignored control, a lane map whose
  counts disagree with the circuit, a lane map over the rounding threshold, a declared family axis
  the circuit contradicts, and a circuit that would pass only for a fixed seed are each rejected
  with a specific message. The `tests/adversarial_*.rs` files hold further adversarial cases.

## 10. Taxonomy, families and `CircuitFacts`

`taxonomy/taxonomy.json` defines versioned axes, their values, compatibility rules and, for each
value, a signature: a predicate over `CircuitFacts` that the harness can check, or `declared` if
none exists. The axes are `encoding`, `lane_map`, `lookup`, `select`, `uncompute`, `reuse` and
`rotation`. A new name, seed or parameter value is not a new family. `taxonomy/TAXONOMY.md` explains
what each check proves and what it cannot show.

`CircuitFacts` (`src/facts.rs`), computed during evaluation:

- `spec_id`, `encoding`, `lane_map_family`, `u`, and `k`, `mu` when the lane map has them;
- `op_counts` by kind, overall, and `segment_op_counts` per `Segment` code (ops before any hint
  are under code 255);
- `executed_ccx`, `executed_ccz`, `executed_hmr`, `executed_givens`, averaged over lanes;
- `measurement_uncompute`: some `Hmr` is followed, within its segment, by a conditioned `CZ`/`Z`
  on the measured qubit's former controls;
- `condition_bits` (distinct classical condition bits) and `max_condition_depth`;
- `segment_ancilla_peak` per segment; `prepare_unprepare_inverse` (the unprepare segment is the
  op-by-op inverse of the prepare segment);
- `segment_system_ops`: per segment, the ops that act on a system qubit in any operand, by kind.
  Every segment has an entry, possibly empty. A `SpinSwap` counts as a system op;
- `depth`, `toffoli_depth`, `q_peak` (before any phase-gradient register);
- `segments_validated`: the hints form one walk step (codes 0, 1, 2 in that order, each once, with
  3/4 only inside 1 and paired), every op acting on a system qubit lies in 1, 3 or 4, and nothing
  but register bookkeeping precedes the first hint. Hints are the submitter's labels: this checks
  their structure, not that each op belongs where it is. It is cleared when inner hints (3/4)
  appear without a nested lane map, because ops inside them are counted under 3/4 and could hide
  SELECT's Toffolis from the `select` signatures;
- `nested_validated` (the lane map is nested and the structure of section 6 was accepted) and
  `inner_uniform_bits`.

`taxonomy::check(taxonomy, family, facts) -> Vec<AxisVerdict>` returns, per axis, `Verified`,
`Contradicted(reason)` or `DeclaredOnly`. A `Contradicted` axis rejects the run.

`family.out.json` is written by `build_circuit` from `walk::family()` and `walk::spec_id()`. Its
`spec` field is covered by the seed's family digest, and the evaluator rejects the run if it does
not match the loaded spec. The default build writes:

```json
{ "taxonomy_version": "1.3.0", "name": "sa-low2025-spinswap", "parent": null,
  "axes": { "encoding": "sos-sa", "lane_map": "sa-nested-alias-v1", "lookup": "qroam-clean",
            "select": "givens-sa-nested", "uncompute": "measurement-based", "reuse": "serial",
            "rotation": "phase-gradient-givens" },
  "spec": "reiher-sa-est-v1" }
```

## 11. `score.json` and `results.tsv`

`score.json` is written only for a passing run:

```json
{ "score": 0.0,
  "metrics": {
    "toffoli": 0.0, "qubits": 0, "lambda": 0.0, "lambda_exact": "p/q",
    "spec": "reiher-sa-est-v1", "instance": "reiher", "encoding": "sos-sa",
    "lane_map": "sa-nested-alias-v1",
    "total_toffoli_lit": 0.0, "epsilon": 0.0016,
    "rounding_error": 0.0, "rounding_error_exact": "p/q",
    "samples": 524288, "control_one_samples": 0,
    "f_bound": 0.0, "implied_error_bound": 0.0,
    "grinding_draws_log2": 40, "f_bound_grinding": 0.0, "implied_error_bound_grinding": 0.0,
    "reflection_toffoli": 0,
    "executed": { "ccx": 0.0, "ccz": 0.0, "hmr": 0.0, "givens": 0.0 },
    "cliffords": 0.0, "measurements": 0.0, "resets": 0.0, "depth": 0, "toffoli_depth": 0,
    "family": { "name": "", "parent": null, "taxonomy_version": "1.3.0", "axes": { } },
    "verified_axes": [], "declared_axes": [],
    "digests": { "ops": "", "lanemap": "", "family": "", "spec": "" },
    "eval_seconds": 0.0,
    "conventions": { },
    "spectral_amplification": { },
    "nested": { },
    "rounding_class": { } } }
```

(The values above are placeholders showing the keys.)

- The top-level `score` is the evaluator's normalization-weighted figure, not the challenge
  score. The challenge score is `metrics.toffoli x metrics.qubits`, which the ledger computes.
- `metrics.lambda` is `lambda_decl`. For an `sos-sa` run the evaluator's `score` uses
  `lambda_eff_used` instead (section 12), so it is not `lambda x toffoli x qubits` for these runs.
  `total_toffoli_lit` uses `lambda_decl`.
- `metrics.conventions` holds the report-only views of spec/CONVENTIONS.md.
- `metrics.spectral_amplification` is present on `sos-sa` runs (section 12);
  `executed.spin_swap` and `spin_swap_toffoli` appear when a `SpinSwap` executed.
- `metrics.nested` is present on nested runs: `inner_uniform_bits`, `inner_reflect_toffoli`, the
  paired and diagonal sample counts, their control-1 counts, `f_paired`, `f_diagonal` and the
  formula of the implied bound.
- `metrics.rounding_class` is present only under the estimated class (section 13).
- `metrics.audit` or `metrics.server_seed` is present only on the runs of section 14.
- The lane engine is not recorded.

This `results.tsv` is the evaluator's local run log, written in the `--root` directory. `benchmark.sh` runs the evaluator in `run/` (git-ignored), so the log is `run/results.tsv` and is never committed. It is not the challenge ledger, `results.tsv` at the challenge root, which has its own columns and is appended to only by the judge ([../../../spec/LEDGER.md](../../../spec/LEDGER.md)).
Columns: `unix_time, commit, spec, family_name, family_axes, lambda, toffoli, qubits, score,
total_toffoli_lit, samples, status, note`. `family_axes` is `axis=value` pairs joined by `;`, sorted
by axis name. `commit` is the short git commit of the root, or `nogit`. A failed run appends a row
with status `FAIL` and the rejection reason in the note; it is never dropped. Untrusted text
(family name, spec id, rejection reasons) is cleaned before it is written: control characters
become spaces and `"` becomes `'`, so a field cannot break the row or open a CSV quote.

## 12. The spectrum-amplified specs (`sos-sa`)

spec/SPEC-SA.md is the contract. This section lists what the harness does for these specs.

- **Lane map.** `sa-nested-alias-v1` (section 6), with reference `M(b)^dagger M(a)`.
- **Tracker.** `sim::givens_tracker` returns the fermionic Gaussian tracker at the spec's `beta`.
  It charges `2 (beta - 2)` per `Givens` and adds `beta` phase-gradient qubits (section 8).
- **`SpinSwap` / `SpinSwapDg`** (op kinds 32 / 33; spec/SPEC-SA.md section 11). Low et al.'s
  controlled swap of the spin sectors, run through the same tracker and charged `N + 1` Toffolis
  per execution. Accepted for `sos-sa` specs only.
- **The evaluator's weighted figure** (report-only; the challenge score is `C_step x Q_peak`).
  `score.json` holds `lambda_eff_used x C_step x Q_peak` with `lambda_eff_used = max(ours,
  published)`. "Ours" is the bound from the spec's certificate at the run's `lambda_decl` and
  `2 x rounding_error` (spec/CONVENTIONS.md sections 3 and 4); "published" is the paper's
  `lambda_eff`, 21.3674 (Reiher) and 43.6538 (Li). Both are upper bounds, and the larger is the
  one a certified claim may use. If the certificate gives no bound for the run, the score falls
  back to the worst-case `Lambda`.
- **Certificates.** `specs/<id>/certificate.json`, compiled into `src/score/report.rs`. A UHF
  determinant of `H_spec` gives `E_gap <= 3.531422` (Reiher) and `5.370835` (Li), so
  `lambda_eff <= 19.990083` and `43.609057` at `lambda_decl = Lambda` with no rounding error.
- **`metrics.spectral_amplification`.** `Lambda`, `lambda_eff_ours`, `lambda_eff_published`,
  `lambda_eff_used`; for each of the three, totals at `sigma_PEA = 1.0 mHa` (Low et al.'s
  convention) and at 1.6 mHa, with the phase-estimation register counted beside
  `qubits_without_pe_register`; and `low2025_givens_charge` (derived): `C_step + 4 x (executed
  Givens per step)`, the step cost under their `2 beta` per Givens.
- **Taxonomy.** `encoding = sos-sa`, `lane_map = sa-nested-alias-v1` and
  `select = givens-sa-nested` (all exact signatures), with rules tying them together and to
  `phase-gradient-givens` and a lookup (taxonomy 1.3.0).
- **Walk.** `src/walk/sa_low` holds the five architectures of section 4; its README lists every
  lever.

The trusted code also contains two further `sos-sa` options that no shipped spec declares: a
rigorous ground-energy rounding rule selected by a `rounding` block in `spec.json`, and per-position
rotation widths. A spec without those blocks, which is every shipped spec, is unaffected by them.

## 13. The estimated rounding class (`*-sa-est-v1`)

The acceptance standard of the two tracks. spec/SPEC-SA.md section 14 is the full statement.

- **Specs.** `reiher-sa-est-v1` and `li-sa-est-v1` have the same `sa.bin`, `lambda`, identity,
  `E_SOS` and `lambda_flat` as their rigorous twins, plus a `rounding_class` block
  (`class: "estimated-low2025"`). Their certificates are the twins' with the spec id and
  `spec.json` hash rebound.
- **Class** (`src/spec/rounding.rs`). `EncodingSpec::rounding_class()` defaults to `Rigorous`.
  The loader accepts a `rounding_class` block only for spec ids pinned in the harness, only on
  `sos-sa`, and only with the constants the harness pins (Low et al.'s fitted `const` values and
  their 0.83 mHa truncation budget). Anything else is refused at load.
- **The rule.** Low et al. 2025 (App. D and their published cost script) model the standard
  deviation of the CCSD(T) correlation-energy change under unbiased randomized rounding to `b`
  bits as `sigma(b) = 2^(const - b)` mHa and take `b = ceil(log2(1 / split) + const)` with
  `split = 0.83 / sqrt(2)`. This is an **estimate, not a bound**.
- **Acceptance** (`score::check_lanemap`, `LaneMap::rounding_estimate`, implemented by
  `sa-nested-alias-v1` only):
  - exact: `lambda_decl = Lambda`, and every count of every alias table is the floor or the
    ceiling of its ideal value, which is the support of their randomized rounding;
  - the estimate: every table's resolution `b_equiv = 1 + max{j : L 2^j <= 2^(k + mu)}` for `L`
    items is at least the required coefficient bits, 9 on both instances. Their fit covers the
    inner tables; this class applies the same rule to the outer table too, which is stricter;
  - the spec's rotation bits meet the same rule (16 and 15, the authors' own values).

  Every sampled lane is still validated exactly against the rounded operator the lane map
  declares.
- **Labels.** `score.json` gains `metrics.rounding_class`, which also reports the exact 1-norm and
  whether the rigorous 0.1 mHa rule would have been met; the `results.tsv` note is prefixed
  `estimated rounding error (Low et al. class)`; and the spec id differs from the rigorous twin's.
  A result under this class says "within Low et al.'s estimated truncation budget", never "within
  0.1 mHa", and is never ranked together with a rigorous-class result.
- **Walk.** `sa_low::Params::for_spec` defaults to 9 + 9 keep bits on an estimated-class spec.
- **Tests.** `tests/sa_estimated.rs` pins the class: rigorous specs still reject the authors'
  9 + 9 keep bits, the class cannot be declared elsewhere or with other constants, fewer bits
  fail, and a lane map that is not a floor-or-ceiling rounding fails.

## 14. Fresh-seed audit and server seed

These are evaluator modes. The judge uses the server seed, derived from the ledger key
(`spec/LEDGER.md` at the repository root); the fresh-seed audit and `tools/server` are local tooling.

Both modes answer the grinding weakness of section 9 by mixing a value into the seed that the
submitter could not know when the circuit was fixed. Both use the audit seed stream

    SHAKE256("femoco-walk-fiat-shamir-audit-v1" || sha256(spec payload) || sha256(lanemap.bin)
             || sha256(family.out.json) || sha256(ops.bin) || u64le(round) || randomness)

which is then read exactly as in section 9. The separate domain string means no audit seed equals
an ordinary one. Without these flags nothing about a run changes (`tests/seed_stream.rs` pins the
ordinary stream against independently computed values).

**Fresh-seed audit** (`--freeze FILE --audit-round N --audit-seed HEX --audit-signature HEX`, all
four together). The circuits to audit are first recorded in a freeze file, which is committed to
git: format `femoco-audit-freeze-v1`, the four digests of each frozen circuit, the sample count
`K`, and the round of the public randomness beacon drand quicknet (chain hash
`52db9ba70e0cc0f6eaf7803dd07447a1f5477735fd3f661792ba94600c84e971`, one round every 3 s) whose
value will seed the audit. `round` and `randomness` are that round's number and its 32-byte
randomness. `eval_circuit` stays offline and refuses (exit 2, no `score.json`, no `results.tsv`
row) unless all of these hold:

- `randomness = SHA-256(signature)`;
- the freeze file is tracked by git and unmodified, with the expected format and chain;
- `N` is the round the freeze file names, and the file's round time is quicknet's time for `N`;
- the freeze file's last commit time is earlier than the round's time;
- `--samples`, if given, equals the freeze file's `K`;
- the circuit's four digests equal those of one frozen circuit.

It then draws the lanes from the audit stream at the frozen `K` and adds `metrics.audit` to
`score.json` (the circuit's name, the freeze file's path, hash and commit, the beacon values and
what was checked, the first 32 bytes of the seed stream, `K`, `K_1`, `f_bound` and
`implied_error_bound`). The `results.tsv` note starts with `audit <name> quicknet#<round>`. Because
the seed was fixed by a beacon value published after the freeze, it could not be re-drawn, and the
single-draw `f_bound` applies instead of the grinding figure.

Limits. `eval_circuit` does not verify the beacon's BLS signature; it only ties the randomness to
the signature. The git commit time is set by the committer, so the claim also rests on the freeze
commit having been published before the round. The bound is per freeze file: auditing the same
circuit `T` times is `T` draws. The audit does not make the Hermiticity limit of section 9 go
away, and it re-measures the counts without proving anything new about them. The tool that writes
freeze files is not shipped in this repository; the flags are documented here because the
evaluator implements them.

**Server seed** (`--server-seed HEX`, 32 bytes). For a local artifact server that picks 32 random
bytes after a submission is uploaded. It uses the same audit stream with `round = 0` and the
bytes as `randomness`, and records them in `score.json` as `metrics.server_seed` (protocol
`server-post-upload-v1`). It is not a public beacon and not a formal proof, and it cannot be
combined with the beacon audit flags.

## 15. Givens rotations and the fermionic-Gaussian tracker

### 15.1 The `Givens` op (kind 30)

- **Semantics.** `Givens` with `q_control1 = p`, `q_target = q` (system qubits, `p < q`, not
  necessarily adjacent) and angle register `r_target` holding `a` applies
  `G_{p,q}(theta) = exp(theta (a+_q a_p - a+_p a_q))` with `theta = 2 pi a / 2^beta`, where `beta`
  is the spec's rotation bits. Then `G a+_p G^dagger = cos(theta) a+_p + sin(theta) a+_q` and
  `G a+_q G^dagger = cos(theta) a+_q - sin(theta) a+_p`. `G` is real and number-conserving, and it
  fixes the vacuum exactly (`G |vac> = |vac>`), which fixes its sign. In Majoranas both the even
  and the odd components rotate by `theta` in the `(p, q)` plane. `tests/df_gaussian.rs` checks
  these statements against dense matrices on three modes.
- **Angle encoding.** The register is read little-endian as an unsigned integer and taken mod
  `2^beta`. A narrower register is allowed (it holds smaller values), and the charge is the same.
  A negative angle is `2^beta - a`.
- **Any two modes.** With interleaved spins a same-spin rotation is between modes `p` and `p + 2`.
  The Jordan-Wigner string between two modes is Clifford, so the Toffoli charge does not depend on
  `q - p`.
- **Networks in a spec.** A spec's `Network` is a list `(p, q, a)` applied in order (the first
  entry acts first), at one `beta`. On spin `s` a spatial-orbital network acts on spin orbitals
  `2p + s`. The quantized network defines the spec's operator, not the float vector it was
  derived from.
- **Charged cost: `2 (beta - 2)` Toffolis per executed `Givens`.** Source: Lee et al. 2021
  App. C, p. 49: "The Z rotations as per their Eq. (68) need to be performed 4N times. Those have
  complexity ℶ − 2 each". Also p. 53: "Apply the controlled rotations to rotate the basis with
  cost N(ℶ − 2)". There `N` counts spin orbitals and one basis rotation is `N/2` Givens rotations
  (p. 11: "As shown in Eq. (51) of [10], only N/2 Givens rotations are needed"), so each Givens is
  two rotations at `ℶ − 2` Toffolis each. The rotations add a table-loaded angle into a
  phase-gradient register; von Burg et al. 2021 (arXiv:2007.14460v2) p. 66: "using the phase
  gradient technique [34] eliminates this error with a worst-case cost of one Toffoli gate per R_b
  rotation".
- **Qubits.** When any `Givens` is used, `beta` phase-gradient qubits count toward `Q_peak`
  (Lee et al. p. 55: "The phase gradient state which needs ℶ qubits"). The angle register's qubits
  are ordinary ancillas and count as live from the first `Givens` that reads them (section 8).
- **Where it is enforced.** `sim::givens_tracker(spec)` returns the tracker at the spec's `beta`
  (`3 <= beta <= 32`), which charges `2 (beta - 2)` and adds `beta` qubits. A spec without
  rotations has no tracker, and `Givens` is then rejected.

### 15.2 The tracker (`src/sim/gaussian.rs`)

The simulator hands the tracker, at each `Givens`, the lane's Pauli frame since the previous
hand-off. The lane operator is `O = w^phase F_{m+1} G_m F_m ... G_1 F_1` with `w = exp(i pi / 4)`.
Moving every frame to the left gives `O = w^phase X P`:

- `P = G_m ... G_1` is number-conserving, so `P |vac> = |vac>` exactly. Its action on Majorana
  vectors is `U (+) U`, and the real `n x n` mode rotation `U` is tracked (two rows per `Givens`).
- `X` is the product of the frames' Majoranas, each rotated by every later `Givens`. These are
  tracked as vectors in the even or odd block.
- A frame becomes `w^k gamma_{m_1} ... gamma_{m_d}` exactly. An `S^2` is a `Z`; an odd power of `S`
  at a `Givens` is rejected, because it is not a Majorana product.

At the end of the lane the tracker forms `Omega = O_ref^-1 O = w^k Y P`, where `Y` is a product of
Majorana vectors (the reference's reversed, then `X`), and checks two things:

1. **Conjugation.** `max |Ad(Y)(U (+) U) e_b - e_b|` over all `2n` basis vectors must be at most
   `AD_TOL = 1e-11`. This shows `Omega` is a scalar to that tolerance. Conjugation alone cannot
   tell `U` from `-U`, hence step 2.
2. **Sign and phase, exactly.** `<vac| Omega |vac> = w^k <vac| Y |vac>`, because `P |vac> =
   |vac>`. The right-hand side is a Pfaffian of vacuum contractions (Wick's theorem). It must be
   within `PHASE_TOL = 1e-6` of 1. The alternatives `w^j`, `j != 0`, are at least 0.765 away, so
   the sign and phase are decided exactly.

Rejections: "the rotated operator differs from the reference: max |Ad(O_ref^-1 O) - I| = ...",
"sign flipped: the operator is -1 times the reference", and "phase off by w^j from the
reference".

**Floating point.** What a pass proves about the continuous part is `|Ad(Omega) - I|_max <=
1e-11`, not exact equality. Deviations below that can only come from sub-unit combinations of
angle errors, such as commutators of rotations, and are not ruled out. Their effect is bounded:
`Ad(Omega)` rotates `n` planes by `phi_j`, and `|Omega - (+-1)| <= sum |phi_j| / 2 <= (pi/2)
sqrt(n) |Ad - I|_F / sqrt(8)` with `|Ad - I|_F <= 2n x 1e-11`. That is at most 1.3e-8 per lane at
`n = 108` and 2.1e-8 at `n = 152`; multiplied by `lambda_decl` it bounds the effect over all
lanes. The sign check then fixes the scalar.

Measured on double-factorization rotation networks of the two instances (a test that is not part
of this repository; the figures are kept because the tolerance was set from them): the largest
residual of a correct lane was 4.0e-15 (Reiher) and 7.3e-15 (Li), and the largest `|overlap - 1|`
was 4.4e-15 and 6.7e-15. One angle unit off at one rotation gave a residual of at least 2.6e-5 at
`beta = 16` and 1.5e-6 at `beta = 20`. So the tolerance sits more than three orders of magnitude
above the honest residual and five below the smallest single-unit error at those precisions.

**Limits.** At most `MAX_FACTORS = 512` Majorana factors per lane; more is rejected as
unsupported, which bounds the Pfaffian's cost. Only real (orthogonal) mode rotations are supported.

## 16. Nested composition: `Reflect` and nested lane maps

A nested walk step applies an inner block encoding, reflects the inner register, and applies the
same inner block again. The shipped nested family is `sa-nested-alias-v1` (spec/SPEC-SA.md section
5). `df-nested-alias-v1` is the other nested family in the trusted code, with no shipped spec.

### 16.1 The `Reflect` op (kind 31)

- **Operands.** Only `r_target`, a register declared before use. Every other operand, the
  condition included, must be absent, and it may not sit inside a `PushCondition` block. It
  therefore executes on every lane and is not controlled.
- **Semantics.** `R = 2|+><+|^{(x) w} - I` on the register's `w` qubits, which must be exactly the
  inner uniform register the lane map declares (`LaneMap::inner_bits`), each qubit once, in any
  order. In the implicit-Hadamard picture of section 5 this is the reflection about `|0...0>` of
  those qubits between Hadamard layers.
- **Charged cost: `max(w - 2, 0)` Toffolis per executed `Reflect`**, the formula section 8 charges
  for the walk's own reflection: a `(w-1)`-controlled `Z` as an AND ladder of `w - 2` Toffolis
  with measurement-based uncomputation. Lee et al. 2021 App. C charge the same (`n` qubits cost
  `n - 2`), p. 52: "Reflect about the zero state on nΞ +1 qubits (the qubits where the state
  preparation is being performed and the rotated ancilla) with cost nΞ − 1." Their inner
  reflection is also controlled on two flags, which costs two more; the harness's `Reflect` has no
  control and is not charged for one. von Burg et al. 2021 call this reflection's cost "negligible
  and thus ignored" (Supp. Sec. VII.C.3); the harness charges it anyway.
- **Where it is valid.** Only with a nested lane map. Anywhere else it is rejected at compile time
  ("Reflect needs a nested lane map").

### 16.2 Nested lanes and the reference

The uniform register splits into outer bits `[0, u_o)` and the inner register `[u_o, u_o + w)`. A
sampled nested lane is `(c, s, b)`: the control bit and uniform value of every lane, whose inner
part is `a`, plus a second-pass inner value `b` (16.4). The simulator runs the op stream on basis
values as always. At the `Reflect` it requires the inner register to hold `a` (otherwise the lane
fails with `inner-register-at-reflect`: the first inner copy must restore it), then replaces it
with `b`.

**Why this is exact.** `R = (2 / 2^w) sum_{a,b} |b><a| - I`, and `|b><a| = X^{a^b} Pi_a`. So the
whole step `C` (outer prepare, first copy, `Reflect`, second copy, unprepare) is exactly
`C = (2/2^w) sum_{a,b} C_{b,a} - sum_a C_{a,a}`, where `C_{b,a}` is `C` with the `Reflect` replaced
by "the inner register holds `a`; make it `b`". Each `C_{b,a}` is a permutation-and-phase circuit,
which is what a lane simulates. Taking matrix elements between the implicit `|+>` states of the
whole uniform register, the encoded operator is

`A = lambda_decl * E_s [ 2 E_b R(s, b) - R(s, a) ]`,

with `R(s, b)` the lane's reference for second-pass value `b` (`LaneMap::reference_nested(spec, s,
after)`, where `after` is `s` with its inner part replaced by `b`), `E_s` uniform over the `2^u`
values `s` and `E_b` uniform over the `2^w` inner values.

**Per lane.** Every sampled lane must pass all the checks of section 6, with the uniform register
ending as `after`, and with the reference:

- control 1: `R(s, b)`, the product of the two passes' inner terms with the second pass on the
  left. For `sa-nested-alias-v1` it is `M(b)^dagger M(a)`;
- control 0: the identity.

The tracker of section 15 compares products of rotated operators, sign included. A paired lane
(`b != a` in general) checks the two passes' inner terms together. A diagonal lane (`b = a`) checks
the second pass against the first on the same inner value. Paired lanes alone could not catch a
circuit that misbehaves only when `b = a`, because they draw `b = a` with probability `2^-w`; that
is why half the lanes are diagonal.

### 16.3 The structural check

`src/sim/nested.rs`, run before any lane. The op stream holds exactly two `Segment 3` and two
`Segment 4` hints, as `3 C 4 R 3 C' 4`, where:

- `C` and `C'` are identical op for op (every field of the 56-byte record), with no `Segment`,
  `Register`, `AppendToRegister` or `Reflect` inside, and with the condition stack balanced within
  each copy;
- `R` is exactly one `Reflect`, the only op between the copies, at condition depth 0, on a
  register equal to the inner uniform register;
- no op outside the two copies names an inner uniform qubit, except that one `Reflect`. A `Givens`
  outside them may not read a register that holds one.

`Builder::nested_inner(reg, inner)` emits this shape from one closure.

The check is syntactic. Identical ops do not imply identical operators, because a copy can read
state that it, or the other copy, left in an ancilla; a second copy that applies the adjoint by
reading a pass flag relies on exactly that. So the check is not a premise of soundness: the paired
and diagonal lanes certify the operator without it. What it certifies is the architecture ("one
block encoding, a reflection, the same block encoding"), and it is what makes the nested `select`
values of the taxonomy verifiable (`nested_validated`, section 10).

### 16.4 Sampling and the bound

The control bit and uniform value are drawn exactly as for a flat lane map, so the prefix of the
seed stream is unchanged (section 9). Then one more little-endian `u64` `x` per lane: if
`x & 1 = 1` the lane is diagonal (`b = a`); otherwise it is paired and `b = (x >> 1) mod 2^w`.

Every sampled lane passed and the counts are exact (16.5). Given that, with confidence
`1 - 2 delta`, at most a fraction `f_p1 = ln(1/delta) / K_p1` of the control-1 paired tuples
`(s, b)` are wrong and at most `f_d1 = ln(1/delta) / K_d1` of the control-1 diagonal lanes are,
where `K_p1` and `K_d1` are the sampled control-1 paired and diagonal lanes (each about `K / 4`).
A wrong entry moves `A` by at most 2 in norm, and the paired term carries weight 2, so

`||A_circuit - A_declared|| <= 2 lambda_decl (2 f_p1 + f_d1)`,

reported as `implied_error_bound` on nested runs; `implied_error_bound_grinding` uses
`ln(2^40 / delta)` in both fractions. Only control-1 lanes are counted because control-0 lanes test
the identity, not `A`: counting all lanes would understate the bound by about a factor of 2.
`score.json` `metrics.nested` reports `f_paired` and `f_diagonal` over all paired and diagonal
lanes, and the control-1 counts the bound uses.

What it cannot certify:

- Lanes that were not drawn, as for a flat lane map.
- That the inner block is Hermitian and squares to the identity on unsampled lanes. The walk
  operator is a reflection only if the block encoding is self-inverse on every lane; a block that
  is not, on unsampled lanes, changes the walk without changing any sampled lane (the limit of
  section 9).
- `Reflect` reflects the inner uniform register only, not other ancillas. A circuit that leaves
  garbage in an ancilla after the first copy is not reflected about "its prepared state". The
  paired lanes see the garbage instead, because the second copy runs with a different inner value
  and must still clean up: the circuit fails there, or it is correct by the decomposition of 16.2.

### 16.5 Rounding error of a nested map

The outer map (which outer item `s mod 2^u_o` names) and each inner map (which inner item an inner
value names, given the outer item) are alias tables with exact integer counts, `n_o` and `m_j`. A
nested family's inner tables all share one width, so one inner register serves every outer item.
The encoded operator then follows analytically from 16.2: the ordered product of inner items
`j1 != j2` of outer item `o` has weight

`c^_t = 2 lambda_decl n_o m_j1 m_j2 / 2^(u_o + 2w)`,

and the `j1 = j2` products together with the `-1` of each item form a known multiple of the
identity: an energy offset, not an error (spec/CONVENTIONS.md section 4.2). The harness computes
`rounding_error = sum_t |c_t - c^_t|` over the spec's terms exactly, in dyadic integers, and judges
it by the spec's rounding class (section 6). For `sa-nested-alias-v1` the terms are the ordered
products `t = (alpha, x, y)`, `x != y`, of each generator's LCU; the spin bits change how many
lanes an item owns, and spec/SPEC-SA.md section 5 gives the counts. At parse a nested lane map is
checked for well-formed tables: every `alt` names an existing item, every bucket at or past the
item count has `keep = 0`, and every `keep` is below `2^mu`.
