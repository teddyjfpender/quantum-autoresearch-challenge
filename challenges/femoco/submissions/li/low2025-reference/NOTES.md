# Low et al. 2025 construction on the Li active space

## Goal and starting point

The Li track's board starts from circuits of the one-hot architectures. It has no circuit of
the construction the acceptance standard comes from: the spectrum-amplified walk step of Low et
al. 2025, with one QROAM lookup loading a network's whole angle word. This submission adds that
reference point, so every other circuit on the track can be read against it. It builds on no
other submission.

## Mechanism

Nothing new. The builder is the shipped `sa-low2025` (`src/walk/sa_low/mod.rs`,
`src/walk/sa_low/inner.rs`), unchanged, with every knob at its default on `li-sa-est-v1`:

- nested alias-sampling PREPARE over outer items and inner indices, 9 + 9 keep bits;
- a clean-ancilla QROAM read of the selected network's angle word, held through the rotations;
- phase-gradient Givens rotations with a controlled spin swap, so each network runs on one spin;
- measurement-based uncomputation of the lookups.

No file under `src/walk/` changes in this pull request.

## Architecture

`qroam-word`: the selected network stays in binary and its angles are fetched by one table
lookup into a wide data register. There is no unary copy of the index.

## Experiments

```sh
python3 challenge.py run femoco challenges/femoco/submissions/li/low2025-reference -- --samples 4096
```

Local result on the reference engine, 4,096 lanes: 13,906.000 Toffolis per step at 1,347 peak qubits, `eval OK`. The count is deterministic for
this builder (no measurement-controlled Toffolis), so the judge's figure should equal it.

For comparison, Low et al. report 14,629 Toffolis per step at 1,454 qubits for this instance
under their own cost model, which charges 4 Toffolis more per Givens rotation than this
evaluator and counts qubits by a register rule (`targets.json`). The two are not like for like
on qubits.

## Result and next steps

This is the baseline of the track, not an improvement: its qubit peak is set by the angle word
and its Toffoli count by the two lookups per SELECT copy. The one-hot architectures on the
board remove the angle word from the peak; the streamed architecture trades Toffolis for it.

## Attribution

Model: Claude Opus 5.5. Harness: Claude Code. Human direction: Teddy Pender. The construction is
Low, King, Berry, Han, DePrince, White, Babbush, Somma and Rubin, Phys. Rev. X 15, 041016 (2025).
