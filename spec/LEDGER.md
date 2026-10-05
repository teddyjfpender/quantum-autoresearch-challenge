# The results ledger

Each challenge has one `results.tsv`: tab-separated, a header row, no quoting, one row per
validated circuit, in time order. It is the only source of the leaderboard and of the site feed.
Rows are appended by the judge workflow's bot and are never edited by hand.

## Columns

| Column | Meaning |
| --- | --- |
| `unix_time` | When the circuit was first recorded (seconds since the epoch, UTC). |
| `track` | The track, e.g. `reiher` or `li`. |
| `spec` | The pinned specification the circuit was validated against. |
| `architecture` | The declared architecture, an id from `architectures.json`. |
| `toffoli` | Toffoli gates per walk step, averaged over the sampled lanes, 3 decimals. |
| `qubits` | Peak logical qubits. |
| `score` | The challenge score, lower is better. For FeMoco: `lambda_eff x toffoli x qubits`. |
| `lambda_eff` | The normalisation the evaluator used in the score, `score / (toffoli x qubits)`. |
| `samples` | Lanes sampled in the full validation. |
| `engine` | The evaluation engines that passed the circuit, e.g. `reference-4096+sliced-524288`. |
| `seed` | The 32-byte lane seed of the full validation, hex. |
| `ops_sha256`, `lanemap_sha256`, `family_sha256` | SHA-256 of the circuit's three files. |
| `family_name` | The taxonomy family the circuit declares to the evaluator. |
| `verifier_sha256` | Digest of the trusted evaluator that passed the circuit. |
| `commit` | The commit the circuit builds from. |
| `pr` | The pull request number, empty for historical rows. |
| `author`, `model`, `harness` | Who submitted, and the model and harness they declared. |
| `submission` | `<track>/<id>` of the submission directory, or `pin:<name>` for historical rows. |
| `kind` | `submission`, or `historical` for circuits found before the challenge opened. |
| `standing` | Why the row was recorded: `new-architecture`, `architecture-elite`, `front`. |
| `status` | `OK`. Only validated circuits are recorded. |
| `note` | The submission's title. |
| `mac` | Chained HMAC-SHA256 of the row under the ledger key. |

## How a row is authenticated

- **The circuit is named by content.** The three SHA-256 digests identify the exact op stream,
  lane map and family file. Rebuilding at `commit` with the submission's build knobs must
  reproduce them byte for byte.
- **The judge is named by content.** `verifier_sha256` is one digest over every trusted file of
  the challenge (`python3 challenge.py verifier-digest <challenge>`). Rows passed by different
  versions of the evaluator are distinguishable.
- **The lanes could not be chosen by the submitter.** `seed` is
  `HMAC-SHA256(ledger key, "qac-lane-seed-v1|ops|lanemap|family")`. It is unknown until the
  judge runs and different for every circuit, so a submitter cannot search for a circuit whose
  sampled lanes miss a fault. It is published, so anyone can repeat the exact run:
  `eval_circuit --server-seed <seed>`.
- **The row could not be forged.** `mac` is
  `HMAC-SHA256(ledger key, "qac-ledger-row-v1|previous mac|row cells")`, chained from an
  all-zero value. Adding, changing, reordering or removing a row breaks the chain. Only the
  judge workflow holds the key; `python3 challenge.py verify-ledger` checks the chain.

Anyone can check a row without the key by rebuilding the circuit and re-running the evaluator
with the published seed. The key is needed only to check that the row was written by the judge.

## Historical rows

Circuits found before the challenge opened are in the ledger with `kind = historical`. Each is
pinned by a byte-identity test in the challenge's test suite (`submission = pin:<name>`), was
re-validated by the same pipeline with a ledger-key seed, and carries the date it was first
measured. They are the starting board and count exactly as submissions do.

## Reading it

- **Leaderboard by architecture:** the lowest `score` per `(track, architecture)`.
- **Trade-off front:** per track, the rows no other row beats on both `toffoli` and `qubits`.
- **Improvement over time:** the running minimum of `score` in `unix_time` order, per track or
  per architecture.

`data/site/<challenge>/leaderboard.json` holds exactly these three views, rebuilt from the
ledger on every recorded row ([WEBSITE.md](WEBSITE.md)).
