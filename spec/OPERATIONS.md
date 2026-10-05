# Operations

For maintainers.

## One-time setup

1. **Ledger key.** Generate 32 random bytes as hex and store them as the repository secret
   `QAC_LEDGER_KEY`. Keep an offline copy. The key seeds lanes and signs ledger rows; anyone
   who holds it can predict seeds and write rows.

   ```sh
   python3 -c "import secrets; print(secrets.token_hex(32))" | gh secret set QAC_LEDGER_KEY
   ```

2. **Repository settings.**
   - Enable Discussions with the categories Ideas, Show and tell, Q&A and General.
   - Actions: give workflows read and write permission (Settings, Actions, General). They do
     not need permission to create or approve pull requests.
   - Allow squash merging. If `main` is protected, let `github-actions[bot]` bypass it: the
     bot merges recorded submissions and pushes ledger commits.
   - Enable private vulnerability reporting.
3. **Evaluator release.** Run the *Evaluator release* workflow once. It publishes the trusted
   evaluator and commits `evaluator.lock.json`.

## Routine

- **Submissions** need no action unless they propose an architecture. For those, read the
  registry entry against [ARCHITECTURES.md](ARCHITECTURES.md), then either add the label
  `architecture-approved` (the judge re-runs and records) or ask for the circuit to be filed
  under an existing architecture.
- **Re-judging.** Add the label `rejudge` to run the judge again on an unchanged pull request.
- **Ledger audit.** The *Ledger audit* workflow verifies the MAC chains weekly and whenever a
  ledger changes. Locally: `QAC_LEDGER_KEY=… python3 challenge.py verify-ledger`.

## Changing trusted code

A change under a challenge's `trustedPaths` changes its verifier digest. After it merges,
*Evaluator release* builds and locks a new evaluator; until then the judge builds the evaluator
from source. If the change alters what is accepted or how it is counted, it is a new contract
epoch ([VALIDATION.md](VALIDATION.md)).

## Re-validating recorded circuits

- **One circuit, on a runner.** The *Re-validate a recorded circuit* workflow rebuilds a pinned
  circuit, validates it with the released evaluator and the ledger key, and requires the result
  to equal its ledger row (seed, digests, counts, verifier digest). Choose the reference engine
  for the full stage to strengthen a headline row.
- **All of them, locally.** `challenges/femoco/tools/authenticate_pins.py` does the same for
  every pinned circuit. Use it after an evaluator change.

## Rotating the ledger key

Rotate if the key may have leaked. Seeds of recorded rows stay valid evidence (they were
unknown when the circuits were made). Re-sign the chain with the new key, commit the ledger and
record the rotation in the commit message; rows are not otherwise changed.

## Adding a challenge

See [ROADMAP.md](ROADMAP.md). The judge workflow's `build` and `evaluate` jobs are per
challenge; copy them for the new challenge's toolchain and keep the same job boundaries.
