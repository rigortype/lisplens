# Mode-first command surface with batch edits; internal backend names stay private

The CLI and MCP surface is organized **mode-first**: each mode (Structural, Line-hash) has its own read and edit commands, rather than a single verb set whose mode is implied by the anchor form. This keeps the two modes measurable and comparable, serving ADR-0001's goal of accumulating best practices before any unification.

Each mode's edit command accepts a **Batch** of same-mode operations, drift-checked against a single read snapshot and applied all-or-nothing (atomic, validate-then-write per ADR-0005). One round-trip therefore carries many edits — the core token-efficiency lever.

Command and option names are **self-descriptive and never expose the internal backend names** (`lispexp`, `hashline`): those are implementation structure, not user-facing vocabulary.

## Status

accepted

## Considered Options

- **Verb-first surface, mode implied by anchor form** — rejected for now: fewer commands and one mental model, but it blurs the two modes and undercuts ADR-0001's comparison goal.
- **hashline-style single patch DSL** — not adopted as such; the Batch mechanism borrows its "many edits per call" benefit without a bespoke mini-language.

## Consequences

- More commands than a unified surface — accepted as the cost of measurability.
- A Batch is single-mode; mixing Structural and Line-hash operations in one call is not supported, which keeps drift and validation simple.
