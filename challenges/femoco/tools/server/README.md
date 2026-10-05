# tools/server: a local validation service and its client tools

`femoco_serve` (src/bin/femoco_serve.rs) accepts **artifacts, not source code**, for the pinned
`sos-sa` specs, and runs the trusted `eval_circuit` binary on them. A fast screen uses 4,096
sampled lanes; a passing artifact then enters a single-worker queue for a 524,288-lane
confirmation. The confirmation seed is drawn from the server's OS after the upload is immutable
(`eval_circuit --server-seed`) and is stored with the submission. A screen is provisional; only
a full pass is `accepted`. Neither pass proves every lane or chemical accuracy. The worker
limits Rayon to four threads; a full Reiher or Li run can still take minutes.

Build the trusted binaries without contestant code linked into the evaluator:

```sh
cargo build --release --locked --no-default-features --bin eval_circuit --bin femoco_serve
```

Build a candidate with `build_circuit` (or another generator that writes `ops.bin`,
`lanemap.bin` and `family.out.json`) and add an `architecture.json` with the three registered
descriptor axes, for example:

```json
{"delivery":"onehot","erasure":"ladder+gated","outer_slot":"held"}
```

The accepted values are `word|onehot|onehot-split|streamed`, `plain|gated|ladder|ladder+gated`
and `held|dropped`. These cells are **declared**, not certified by the circuit facts. They
organise an archive and retain alternatives; they are not novelty awards. The trusted evaluator
still rejects a contradicted taxonomy axis and reports which axes it verified.

```sh
./target/release/femoco_serve pack ops.bin lanemap.bin family.out.json architecture.json submission.fmc
./target/release/femoco_serve serve "$PWD" /absolute/path/to/state \
  "$PWD/target/release/eval_circuit" 127.0.0.1:8753
curl --data-binary @submission.fmc http://127.0.0.1:8753/submissions
curl http://127.0.0.1:8753/submissions/UPLOAD_SHA256
curl 'http://127.0.0.1:8753/frontier?spec=reiher-sa-est-v1'
```

The submission endpoint returns an id immediately. Poll for `screened`, `confirming`, `accepted`
or `rejected`. The frontier retains **every accepted artifact**, even when its scalar score
loses globally. It marks the global best and the Pareto status in (lambda x Toffolis per step,
peak qubits), both globally and within each declared cell. Comparisons never cross spec ids. The
response says `architecture_verified: false`. Each record binds the bundle hash, the trusted
evaluator binary's hash, the spec digest and the confirmation seed. The bundle is a fixed
four-part `FEMOSUB1` stream made by `pack`, with bounded part sizes. Submitting the same circuit
bytes under a second descriptor is rejected. Small no-op changes can still evade that identity
check, so a declared cell is never treated as verified novelty.

This is a **loopback-only service for local use**. It does not execute uploaded source, provide
accounts or TLS, or confer verified architectural novelty. A public deployment would still need
authentication, an external archive, worker isolation and a public beacon or an equivalent
auditable seed protocol. The local server seed prevents submitter-side re-rolling of the
confirmation lanes but is not an independently attested beacon.

## Client tools

Untrusted client tooling. None of it changes trusted code, `results.tsv` or `score.json`.

| file | what it does |
| --- | --- |
| `submit.sh` | Builds one candidate from this checkout with only the given `FEMOCO_*` knobs, then runs a K = 64 local check with the trusted `eval_circuit`. It packs the candidate, POSTs it and prints `id <sha256>`. `--no-post` stops after packing; `--expect-ops SHA` refuses a build whose `ops.bin` differs from a pinned digest. See its header for flags. |
| `poll.sh` | Waits for accepted / rejected / failed. Prints the verdict, the cell, the screen and confirmation counts, and the /frontier flags. |
| `frontier.py` | Prints the accepted archive per spec and declared cell. |
| `cell.py` | Suggests the declared cell from the knobs. It is a suggestion only: cells are declared, not verified. |

```sh
tools/server/submit.sh --label my-try --spec reiher-sa-est-v1 --arch sa-toff \
  FEMOCO_SA_TWEAKS=imchxlgr FEMOCO_SA_INNER_A=4      # one knob per argument
tools/server/poll.sh <id>
python3 tools/server/frontier.py reiher-sa-est-v1
```

Each run's directory, `target/submit/runs/<label>-<time>/`, holds the circuit files, the packed
`submission.fmc`, the K = 64 `eval/score.json` and `meta.json`. `target/submit/submitted.tsv`
lists every POST. Environment: `FEMOCO_SERVER`, `FEMOCO_EVAL`, `FEMOCO_SERVE_BIN`, `SUBMIT_OUT`.
