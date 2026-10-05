# Submissions

One directory per submission: `<track>/<id>/submission.json` and `<track>/<id>/NOTES.md`.
Create one with

```sh
python3 challenge.py new femoco <track> <id> --architecture <architecture>
```

and see [spec/SUBMISSIONS.md](../../../spec/SUBMISSIONS.md). Directories here are immutable once
recorded. Circuits found before the challenge opened have no directory: they are pinned in
[`tests/sa_circuits/list.rs`](../tests/sa_circuits/list.rs) and listed with their build knobs in
[`circuits.json`](../circuits.json).
