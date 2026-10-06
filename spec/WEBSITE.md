# Website feed

A site reads this repository at one immutable commit of `main` through
`data/site/sources.json`. Everything under `data/site/` except `activation.json` and
`<challenge>/challenge.json` is derived from the ledgers by `python3 challenge.py site`; the
contract check fails if it is stale.

## Files

| File | Schema | Content |
| --- | --- | --- |
| `data/site/sources.json` | `qac-site-sources-v1` | Paths of every file below, per challenge. |
| `data/site/activation.json` | `qac-site-activation-v1` | `status` (`staging`, `live`, `closed`) and the gates behind it. Authored. |
| `challenges.json` | `qac-registry-v1` | The challenges and their tracks. |
| `challenges/<id>/benchmark.json` | `qac-benchmark-v1` | The contract: metric, tracks, paths, validation, acceptance. |
| `challenges/<id>/architectures.json` | `qac-architectures-v1` | The architecture registry. |
| `challenges/<id>/targets.json` | `qac-targets-v1` | Published points, with their conventions. |
| `challenges/<id>/results.tsv` | [LEDGER.md](LEDGER.md) | One row per validated circuit. |
| `data/site/<id>/leaderboard.json` | `qac-leaderboard-v1` | Per track: best circuit, architecture elites, front, history, targets. |
| `data/site/<id>/challenge.json` | `qac-site-content-v1` | Display copy: titles, summaries, rules, how to take part. Authored. |

## `leaderboard.json`

```
{ schema, challenge, metric, ledger, ledgerRows,
  tracks: [ { track, title, spec, circuits,
              best,                       // lowest score in the track
              baseline,                   // the track's baseline circuit (a ledger row)
              belowBaseline,              // 1 - best.score / baseline.score
              architectures: [ { id, name, parent, circuits, firstUnixTime,
                                 elite, fewestQubits, fewestToffoli,
                                 history } ],   // running best score of the architecture
              front,                      // (qubits, toffoli) Pareto front, by qubits
              history,                    // running best score of the track
              targets } ] }
```

Every circuit object has the same keys: `unixTime`, `track`, `architecture`, `toffoli`,
`qubits`, `toffoliTimesQubits`, `score`, `samples`, `engine`, `opsSha256`, `verifierSha256`,
`commit`, `pr`, `author`, `model`, `harness`, `submission`, `kind`, `standing`, `note`.

## Conventions

- `score` is `toffoli x qubits`; `toffoliTimesQubits` is the same number, kept for readers of
  the first feed version.
- Lower is better for `score`, `toffoli` and `qubits`. `benchmark.json` states the metric's
  name, formula, components and units, so nothing about units needs to be hard-coded.
- The primary grouping is `architecture`; circuits are the secondary level.
- Improvement over time is `history`: each entry is a circuit that lowered the running best.
- Pull request and author data are also available live from the GitHub API through `pr`.
- While `activation.json` says `staging`, numbers may be re-validated and should be shown as
  provisional.
