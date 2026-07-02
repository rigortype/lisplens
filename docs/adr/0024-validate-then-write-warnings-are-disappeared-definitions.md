# Validate-then-write warnings are disappeared definitions

A validate-then-write **warning** (ADR-0005) fires when a definition that was recognized **before** an edit is no longer recognized **after** it. Definitions are identified by `(kind, name)` via lispexp's Form annotator (the same registry the Outline uses). The edit still **succeeds** — parens balance and no new parse errors (ADR-0005) — the warning only signals that the edit may have broken a definition's form even though the file parses.

New definitions appearing after an edit are **not** warnings.

The warnings are computed at the apply layer by taking the Outline of the source before and after the edit and diffing the `(kind, name)` sets; they are returned in the edit `Outcome` (ADR-0023) and shown by the CLI and MCP.

## Status

accepted

## Considered Options

- **Role-slot-change detection** (name/arglist/docstring/body reassigned on a "touched" definition) — rejected: identifying the "touched" definition is fuzzy and harder to compute than a set diff.
- **Count-only** (fewer recognized definitions) — rejected: too coarse; it doesn't say *which* definition.

## Consequences

- Computed by annotating before and after with the file's dialect registry; no need to locate the edit.
- A definition **renamed by an edit** shows as one disappeared + one new, so the disappeared name warns. That is acceptable — a rename via a raw edit is worth flagging; the agent can ignore an expected warning.
- Methods are keyed by `(kind, name)` without the signature, so deleting one of several same-named methods does not warn while any remains — an accepted v1 simplification.
