# Contributing

There are two kinds of pull request, and they are handled differently.

**Submissions** add a circuit to a challenge. They may touch only the challenge's editable
paths, one new submission directory and (to propose a new architecture) the architecture
registry. The judge workflow validates them and a bot records the result. Read
[spec/SUBMISSIONS.md](spec/SUBMISSIONS.md) and the challenge's `TASK.md` first.

**Everything else** (the judge, specs, documentation, tooling) is reviewed by a maintainer.
A change to a trusted path changes the verifier digest that every later ledger row carries, so
it needs a stated reason and passing pins. A change to an acceptance rule or a spec is a new
contract epoch, never an edit in place ([spec/VALIDATION.md](spec/VALIDATION.md)).

Before opening any pull request:

```sh
python3 -m unittest discover -s tests
python3 challenge.py check
```

Use [Discussions](spec/DISCUSSIONS.md) for ideas, questions and results. Report anything that
could let an invalid circuit pass through [SECURITY.md](SECURITY.md), not in public.

By contributing you agree that your contribution is licensed under Apache-2.0 and that your
submission notes are public.
