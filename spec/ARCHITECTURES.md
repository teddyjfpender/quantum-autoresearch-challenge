# Architectures: the primary unit of search

A challenge here is searched at two levels, in this order.

1. **Architecture.** How the circuit is organised: which registers exist, what they hold and
   which mechanism delivers the data the computation needs. Two circuits have different
   architectures when no change of parameters turns one into the other.
2. **Circuit.** One point within an architecture: its knobs, widths, schedules and local
   gadgets. This is exploitation of a design that already exists.

Most challenge platforms keep a single best result and reward whatever beats it. That makes the
search greedy: a new design that is not yet the best is discarded, even when it is the one that
would win after tuning. This repository keeps every architecture's best circuit instead, and
records a circuit when it advances its own architecture or the track's trade-off front
([SCORING.md](SCORING.md)).

## The registry

Each challenge has an `architectures.json`. An entry states:

| Field | Meaning |
| --- | --- |
| `id` | Lowercase slug, permanent. |
| `name` | Short display name. |
| `mechanism` | What the architecture does, in enough detail to implement. |
| `distinguishing` | The structural property that separates it from every other entry. |
| `parent` | The architecture it specialises or was derived from, if any. |
| `references` | Primary sources for the construction. |

Three words are kept apart throughout. An **architecture** is a registry entry: a way of
organising the circuit, and the unit the board is ranked by. A **builder** is the code that
emits circuits, selected by a build knob; one builder may serve several architectures and one
architecture may have several builders. A challenge may also have its own finer labels (FeMoco's
evaluator reports a taxonomy `family` per circuit); those are recorded but do not affect
standing.

Every submission declares exactly one architecture from the registry. Where a challenge states
rules that derive the architecture from the build knobs, the declaration must agree with them. The registry is kept
small on purpose: a handful of designs a reader can tell apart, not one entry per variant.

## What is and is not a new architecture

A new architecture changes the organisation of the circuit. These do **not** qualify:

- a new name, seed or parameter value;
- a different width, block size, group count or schedule of an existing mechanism;
- a local gadget that makes one stage cheaper without changing what the registers hold;
- a combination of existing levers of one architecture.

A useful test: state the property in `distinguishing`, and say which existing entry the circuit
would belong to without it. If no such property can be stated, the work is a refinement, and it
is welcome as one.

## Proposing one

1. Open a Discussion in **Ideas** with level "New architecture": the mechanism, the bottleneck
   it removes and the cheapest falsifying check.
2. In the submission pull request, append the entry to `architectures.json` and declare it in
   `submission.json`.
3. The judge validates the circuit as usual. Because the registry changed, the pull request is
   labelled `needs-architecture-review` and waits for a maintainer, who checks the entry
   against the rules above and either adds `architecture-approved` or asks for the submission
   to be filed under an existing architecture.

The first validated circuit of an approved architecture is recorded whatever its score. That is
the point: a new design gets a place on the board before it is competitive.
