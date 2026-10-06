# Security

The judge must never record a circuit that does not implement the specified operator. If you
find a way to make an invalid circuit pass, to learn a lane seed before validation, to alter a
ledger row, or to run code outside the build sandbox, please report it privately:

- use GitHub's **Report a vulnerability** on this repository (Security tab), and
- include a minimal reproduction and the commit you tested.

Do not open a public issue, discussion or pull request for such a report. A confirmed soundness
bug is fixed in a new contract epoch; affected ledger rows are marked and re-validated, never
silently removed ([spec/VALIDATION.md](spec/VALIDATION.md)).

## Running the code yourself

A challenge's editable paths (for FeMoco, `challenges/femoco/src/walk/`) hold circuit-building
code written by submitters. The judge validates the circuit that code emits and screens the
source for process, network and `unsafe` use, but nobody reviews it line by line before it
merges. Treat it as untrusted on `main` as much as in a pull request:

- `./benchmark.sh` builds and runs it in a sandbox (read-only filesystem, no network) where
  bubblewrap is available, which is Linux. It warns when it runs unconfined.
- `tools/ci/pins_sandboxed.sh` runs the pin tests the same way. A plain `cargo test` runs them
  unconfined.
- On macOS there is no sandbox. Use a container or a virtual machine for code you have not read.

Everything else in the repository (the evaluator, the judge, the workflows) changes only through
maintainer-reviewed pull requests.
