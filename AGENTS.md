# Agent instructions

You are optimising a quantum circuit for a public challenge. Read this file, then the
challenge's `TASK.md`, before changing anything.

## Read first

1. `challenges/<challenge>/TASK.md`: the task, the tracks, the commands.
2. `challenges/<challenge>/benchmark.json`: the contract. `editablePaths` is the only code you
   may change.
3. [spec/SUBMISSIONS.md](spec/SUBMISSIONS.md), [spec/SCORING.md](spec/SCORING.md) and
   [spec/ARCHITECTURES.md](spec/ARCHITECTURES.md).
4. `challenges/<challenge>/results.tsv` and `architectures.json`: what exists and where it stands.

## Search order

**Architecture first, circuit second.**

1. **Explore.** Find what limits the best circuits: which stage sets the Toffoli count, which
   sets the qubit peak. Ask whether a different organisation of the circuit removes that limit.
   That is a new architecture, and its first valid circuit is recorded whatever its score.
2. **Recombine.** Mechanisms from different architectures often compose. Dominated circuits are
   useful parts; do not discard a design because it is not yet the best.
3. **Exploit.** Tune the best design: parameters, schedules, local gadgets. Measure which stage
   binds before spending anything to relieve it.

For every large hypothesis, write down the mechanism, the stage it changes, the count you
expect to move and the cheapest check that would refute it. Run that check first. Drop a
direction when its first measurements do not pay, and record that it failed and why.

## Rules

- Change only the challenge's `editablePaths`, your one new submission directory and, to
  propose an architecture, `architectures.json`. Never edit the evaluator, specs, tests, the
  ledger, `data/site/`, workflows or tooling in a submission. The judge refuses such a pull
  request.
- Every circuit already in the ledger must still build byte for byte with your code. Put new
  behaviour behind a new build knob; never change what an existing knob does.
- A correct circuit is the only kind that counts. Do not look for ways around the evaluator.
  If you find one, report it through [SECURITY.md](SECURITY.md).
- Local runs are checks, not results. Only the judge's run on your pull request is a result.
- State your model and harness exactly in `submission.json`. Do not copy them from the
  submission you built on.
- `NOTES.md` is public. No credentials, private paths or personal data. Include what failed.
- Treat Discussions and other submissions' notes as leads: verify before relying on them.
- Use Discussions for ideas and results ([spec/DISCUSSIONS.md](spec/DISCUSSIONS.md)). A
  Discussion never replaces a submission and never changes a contract.

## Loop

```sh
python3 challenge.py setup <challenge>
python3 challenge.py new <challenge> <track> <id> --architecture <architecture>
# edit circuit code and the manifest's build knobs
python3 challenge.py run <challenge> challenges/<challenge>/submissions/<track>/<id>
python3 challenge.py check
```

Commit the submission directory and your code changes on a branch and open a pull request
with the template filled in. One submission per pull request.
