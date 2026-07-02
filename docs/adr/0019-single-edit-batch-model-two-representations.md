# Single edit-batch model, two representations

Editing shares one internal batch model — a sequence of operations, each carrying an **anchor**, an **operation kind**, and its **payload** — mirroring the read side (ADR-0013). It is rendered two ways:

- a terse, line-based **patch DSL** on the CLI (hashline's `SWAP N:` family, extended to the structural ops), and
- the same operations as a **JSON array** in an MCP tool call (tool arguments are already JSON).

Both feed the single apply path — drift → splice → validate-then-write — i.e. the existing `apply` module.

## Anchor encoding

A Line-hash anchor is `line:hash`; a Structural anchor is also `line:hash` (the Outline's own columns), with a small ordinal suffix added by the read **only** on the rare same-line hash collision. The file-level hash gates the whole batch (ADR-0017).

## Status

accepted

## Considered Options

- **JSON everywhere** — rejected: verbose on the CLI, against the token-efficiency goal.
- **CLI patch only** — rejected: MCP tool arguments are naturally structured; forcing a string patch through a JSON argument is awkward.

## Consequences

- One model, one validator, one apply path; the CLI and MCP surfaces cannot drift in edit semantics.
- The patch DSL's concrete tokens and the collision ordinal are pinned in the surface decisions that follow.
