# Submissions

A submission is a pull request against `main`. The judge workflow validates it; nobody reviews
circuits by hand. A valid circuit that earns a standing ([SCORING.md](SCORING.md)) is merged and
recorded by a bot.

## What a submission contains

| Path | Rule |
| --- | --- |
| `challenges/<challenge>/submissions/<track>/<id>/submission.json` | Required, new. The manifest. |
| `challenges/<challenge>/submissions/<track>/<id>/NOTES.md` | Required, new. Public research notes, 1 KiB to 100 KiB. |
| The challenge's editable paths (`editablePaths` in `benchmark.json`) | Optional. The circuit code. |
| `challenges/<challenge>/architectures.json` | Optional. Append one entry to propose a new architecture. |

Nothing else may change. One pull request carries one submission. `<id>` is a lowercase slug
(`a-z`, `0-9`, `-`, 3 to 64 characters) that describes the circuit. A recorded submission is
immutable; improve on it with a new one.

## The manifest

```json
{
 "schema": "qac-submission-v1",
 "challenge": "femoco",
 "track": "li",
 "architecture": "onehot-split",
 "title": "Folded item read at five groups",
 "build": {"FEMOCO_WALK_ARCH": "sa-toff", "FEMOCO_SA_TWEAKS": "…", "FEMOCO_SA_MU_O": "9"},
 "claimed": {"toffoli": 19965.5, "qubits": 427},
 "authors": ["github-login"],
 "model": "exact model name",
 "harness": "exact harness name",
 "parents": ["id-of-a-submission-this-builds-on"],
 "discussion": "https://github.com/…/discussions/12"
}
```

- `architecture` must be an id in the registry ([ARCHITECTURES.md](ARCHITECTURES.md)).
- `build` holds the challenge's build knobs. The track's spec is set by the judge.
- `model` and `harness` name the exact model and coding harness used; write `none` for a circuit
  made without one. Do not copy them from the submission you built on.
- `claimed`, `parents` and `discussion` are optional. `pin` appears only on circuits of the
  starting board and names their byte-identity test.

## Workflow

```sh
python3 challenge.py setup <challenge>
python3 challenge.py new <challenge> <track> <id> --architecture <architecture>
# edit the circuit code and the manifest's build knobs
python3 challenge.py run <challenge> challenges/<challenge>/submissions/<track>/<id>
python3 challenge.py check
```

`run` builds and evaluates locally with the same evaluator the judge uses, on lanes derived
from the circuit itself. It is a check for you, not a result: the judge samples lanes from a
seed you cannot know in advance.

Then open the pull request with the template filled in.

## What the judge does

1. **Policy.** The diff must match the table above; files must be regular and non-executable.
2. **Manifest.** `submission.json` and `NOTES.md` are checked.
3. **Recorded circuits.** With your code in place, every circuit already in the ledger must
   still build byte for byte. Put new behaviour behind a new build knob.
4. **Build.** Your builder runs in a sandbox with no network and writes the circuit files.
5. **Validate.** The trusted evaluator, taken from `main`, screens the circuit on the reference
   engine and validates it in full, on lanes seeded by the ledger key.
6. **Standing.** The row is compared with the ledger.
7. **Report.** The bot comments the result. If the circuit earns a standing it merges the pull
   request, appends the signed row to `results.tsv` and rebuilds the site feed.

A pull request that adds an architecture waits for a maintainer's `architecture-approved` label
before step 7. A new push re-runs everything; an old result never carries over to new code.

## Outcomes

| Label | Meaning |
| --- | --- |
| `submission-invalid` | Policy, manifest, build or validation failed. The comment says which. |
| `submission-not-recorded` | Valid, but it earns no standing. Nothing is merged. |
| `needs-architecture-review` | Valid and it proposes an architecture. Waiting for a maintainer. |
| `submission-recorded` | Merged; the row is in the ledger. |

## Notes

`NOTES.md` is the research record that travels with the circuit: starting point, mechanism,
why it is correct, experiments including the ones that failed, the measured result and what to
try next. Notes are public. Remove credentials, private paths and personal data.
