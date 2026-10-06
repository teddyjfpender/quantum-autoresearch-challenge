# Submissions

One directory per submission: `<track>/<id>/submission.json` and `<track>/<id>/NOTES.md`.
Create one with

```sh
python3 challenge.py new femoco <track> <id> --architecture <architecture>
```

and see [spec/SUBMISSIONS.md](../../../spec/SUBMISSIONS.md). Directories here are immutable once
recorded.

Every row of [`results.tsv`](../results.tsv) has a directory here, including the circuits found
before the challenge opened. `submission.json` holds the build knobs that reproduce the circuit
byte for byte; `tools/verify_submissions.py` rebuilds each one from its manifest alone and
checks the digest against the ledger. The starting board's notes are generated from the build
parameters and the validation run.
