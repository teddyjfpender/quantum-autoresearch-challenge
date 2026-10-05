# Security

The judge must never record a circuit that does not implement the specified operator. If you
find a way to make an invalid circuit pass, to learn a lane seed before validation, to alter a
ledger row, or to run code outside the build sandbox, please report it privately:

- use GitHub's **Report a vulnerability** on this repository (Security tab), and
- include a minimal reproduction and the commit you tested.

Do not open a public issue, discussion or pull request for such a report. A confirmed soundness
bug is fixed in a new contract epoch; affected ledger rows are marked and re-validated, never
silently removed ([spec/VALIDATION.md](spec/VALIDATION.md)).

Running a challenge executes circuit-building code from pull requests. The benchmark script
runs it in a sandbox with no network; run untrusted submissions only that way.
