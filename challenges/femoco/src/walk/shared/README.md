# Shared walk components

Contestant code (`src/walk/shared/`): lookups, small arithmetic, unary iteration and
Givens-network conjugation, meant to be reused by any walk architecture. Nothing here depends on a
lane-map family. Every function emits ops through the `Builder`, leaves no garbage, and erases
every AND by X-basis measurement plus a conditioned `CZ` (Gidney 2018, arXiv:1709.06648, Fig. 3),
so only the computing ANDs cost Toffolis.

The spectrum-amplified walks in `src/walk/sa_low/` use `lookup.rs`, `arith.rs` and `unary.rs`.
`angles.rs` was written for double-factorized networks; `sa_low/pareto.rs` follows its chunk
schedule with its own loads, and no shipped walk calls `angles::conjugate`.

Unit tests (`cargo test --release --lib shared`) run each component on a one-lane basis-state
simulator (`testsim.rs`, test-only) with random measurement outcomes and check the value, the
cleanup and the phase.

## `lookup.rs`: reading classical tables

A table is `data: &dyn Fn(u64) -> Word` (a `Word` is a little-endian bit vector in `u64`
limbs); `index` is a little-endian register; entries at `x >= limit` read as 0.

| function | effect | Toffolis | extra qubits |
| --- | --- | --- | --- |
| `xor_lookup(b, index, limit, out, data)` | `out ^= data(x)` by unary iteration (Babbush et al. 2018, Sec. III.A/C) | about `limit` | the iteration's ANDs |
| `qroam_xor(b, index, limit, out, data, Qroam)` | `out ^= data(x)` by clean QROAM | `ceil(limit / lambda) + 2 (lambda - 1) w + E(ceil(limit / lambda))` | `lambda w` during the read |
| `qroam_load(b, index, limit, w, data, Qroam)` | fresh register `= data(x)` (the position-0 block is the output) | `ceil(limit / lambda) + (lambda - 1) w + E(limit)` | `(lambda - 1) w` during the read |
| `erase_lookup(b, index, limit, out, data, h)` | `out` holding `data(x)` back to 0 and freed | `E(limit) = 2 (2^h - 1) + ceil(limit / 2^h)` | `2^h` |
| `measure(b, out)` + `phase_fixup(b, index, limit, &bits, data, h)` | the same, split: one measurement, then any number of fixups whose phases multiply | `E(limit)` per fixup | `2^h` |
| `load_const` / `unload_const` | classical constant in a fresh register | 0 | `w` |

`Qroam { a, m, junk_h }` sets `lambda = m 2^a` blocks. With `m > 1` the index is first divided
by the constant `m` in place (`arith::divmod_const`, about `q_w (3 |m| + 2)` Toffolis each way),
so the block count need not be a power of two: this lets a read fill exactly the qubits another
stage already needs. The blocks are written by unary iteration over the high index, block `x`
is swapped to position 0 (Low, Kliuchnikov and Schaeffer 2018, Fig. 1c; Berry et al. 2019
App. B), copied out, the swaps are undone, and the blocks, which then hold the data of every
index sharing the lane's high part, are erased by measurement with a fixup over the high index
only (Berry et al. 2019 App. C). `qroam_load` skips the copy and the undo and pays a fixup over
the whole index instead; it also emits about `limit (lambda - 1) w / 2` classical ops, so use it
only for small tables (angle words, not the alias table).

Two fixups compose: a register holding `d1(x) ^ d2(y)` is erased by one measurement and fixups
over `x` with `d1` and over `y` with `d2`.

## `arith.rs`: small reversible arithmetic

| function | effect | Toffolis |
| --- | --- | --- |
| `add_into(b, x, y)` | `y += x mod 2^|y|` (`|x| <= |y|`) | `|y| - 1` |
| `sub_from(b, x, y)` | `y -= x` | `|y| - 1` |
| `ctrl_add`, `ctrl_sub` | `y += c x`, `y -= c x` | `|x| + |y| - 1` |
| `less_than(b, a, b)` / `unless_than` | fresh `[a < b]` / its erasure | `n` / `n - 1` |
| `cswap`, `cswap_reg` | controlled swap of two qubits / two registers | 1 per qubit pair |
| `div_small(b, r, d, q_w)` / `div_small_undo` | `r -> r mod d` in place, fresh `q = r div d` (restoring division over `|d| + 1`-bit windows) | `q_w (3 |d| + 2)` / `q_w (3 |d| + 1)` |
| `divmod_const(b, x, m)` / `_undo` | the same for a constant divisor | as above |
| `select_index(b, c, x, y)` / `unselect_index` | fresh `p = c ? x : y` / its erasure | `|p|` / 0 (the erasure's phase is Clifford) |

## `angles.rs`: Givens-network conjugation with looked-up angles

`conjugate(b, table, index, plan, sys, on_flags, middle)` emits `V_n middle V_n^dagger` for the
network `n` held in `index`, where every network in `table` is the same chain of mode pairs
with different angles. `sys(p)` maps a table mode to a system qubit (`2p` for spin 0 under the
double-factorized layout).

- **The inverse costs no second load.** `Z_q G_{pq}(theta) Z_q = G_{pq}(-theta)` (the mode
  parity flips `a_q`), and a chain's modes are two-coloured, so
  `V^dagger = Z_S V_rev(theta) Z_S` with `Z_S` the parity of one colour class: `V^dagger` uses
  the same angle registers in reverse order, bracketed by Clifford `Z`s on system qubits.
- **Loads.** `LoadPlan { chunk, first, erase_h }`: `chunk` rotations are held at once; the
  chunk sequence is `C-1 .. 0` (for `V^dagger`) then `1 .. C-1` (for `V`), `2C - 1` loads, the
  first by `qroam_load` with `first` blocks and the rest by XOR transitions, then one erasure.
  With `chunk = N - 1` a conjugation is one lookup of `E` entries plus `E(E, erase_h)`.
- **Flags.** `table.flags` holds per-network bits loaded with the first chunk; `on_flags`
  receives their qubits right after the load (for a double-factorized table each network's sign
  goes there, so a pair's sign is the XOR of its two networks' flags: a `CZ` per factor).

`NetworkTable::from_df(spec)` builds the table from a double-factorized spec (one-body networks
`0..N`, then each leaf's eigenvectors) with the sign flag; the spec loader for that encoding is
part of the harness, but no such spec ships. `spin_layer(b, beta, n, set)` is the `F^v` layer of
`pi/2` Givens that maps spin-0 modes to spin 1.

## `unary.rs`: unary iteration

`iterate` walks a little-endian index register over `0..limit` with one computing AND per node
and calls a closure with each leaf's control qubit (Babbush et al. 2018, Sec. III.A). `erase_and`
erases one AND by measurement and a conditioned `CZ`.
