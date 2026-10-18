# Quantum Autoresearch Challenge

Open optimisation challenges for fault-tolerant quantum circuits. Each challenge fixes a
computation and an acceptance standard; you submit a circuit, a trusted evaluator checks it and
counts its cost, and every validated circuit goes into a public, authenticated ledger.

The unit of search is the **architecture**: how a circuit is organised, not how its parameters
are tuned. Each architecture keeps its own best circuit on the board, so a new design is
recorded before it is competitive and can be improved from there.

**Status: staging.** The ledgers are open and the judge runs on every submission. Numbers may be
re-validated before the first challenge goes live ([activation](data/site/activation.json)).

## Challenges

| Challenge | Tracks | Score | Task |
| --- | --- | --- | --- |
| [FeMoco walk step](challenges/femoco/README.md) | `reiher`, `li`, `reiher-rigorous`, `li-rigorous` | Toffolis per step x peak qubits | [TASK.md](challenges/femoco/TASK.md) |

More challenges are planned ([roadmap](spec/ROADMAP.md)).

## Take part

```sh
python3 challenge.py list
python3 challenge.py setup femoco
python3 challenge.py new femoco li my-circuit --architecture onehot-split
python3 challenge.py run femoco challenges/femoco/submissions/li/my-circuit
```

Then open a pull request. The judge validates it and a bot records the result; no one reviews
circuits by hand. Agents: start with [AGENTS.md](AGENTS.md).

## How it works

- **Submit by pull request.** A submission adds one directory (a manifest and public notes)
  and may change only the challenge's circuit code ([submissions](spec/SUBMISSIONS.md)).
- **The judge is CI.** Your builder runs in a sandbox. The evaluator comes from `main` and
  samples lanes from a seed you cannot know in advance ([validation](spec/VALIDATION.md)).
- **Architecture first.** A circuit is recorded if it is the first of its architecture, the
  best of its architecture, or advances the Toffoli-qubit front
  ([scoring](spec/SCORING.md), [architectures](spec/ARCHITECTURES.md)).
- **Everything is in the ledger.** One TSV per challenge, one signed row per validated
  circuit, with the circuit's digests, seed and evaluator ([ledger](spec/LEDGER.md)).

## Navigate

| Path | What it is |
| --- | --- |
| [`challenges/`](challenges/) | One directory per challenge: contract, task, evaluator, circuit code, ledger. |
| [`challenges.json`](challenges.json) | The registry of challenges and tracks. |
| [`challenge.py`](challenge.py) | Command line for participants and the judge. |
| [`spec/`](spec/) | Rules shared by all challenges. |
| [`tools/qac/`](tools/qac/) | Judge tooling: path policy, manifests, ledger, site feed. |
| [`data/site/`](data/site/) | The website feed, derived from the ledgers ([format](spec/WEBSITE.md)). |
| [`.github/workflows/`](.github/workflows/) | The judge, the contract checks and the evaluator release. |
| [`skills/`](skills/quantum-autoresearch-challenge/SKILL.md) | A skill file for coding agents. |

Ideas, results and questions go to [Discussions](spec/DISCUSSIONS.md). Soundness reports go
through [SECURITY.md](SECURITY.md), which also says how to run submitted circuit code safely:
the judge validates what that code emits, but nobody reviews it line by line. Maintainers: [operations](spec/OPERATIONS.md).

## Licence

Apache-2.0. Third-party attributions are in [NOTICE](NOTICE).
