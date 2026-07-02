# Mode-first command and MCP tool surface

The surface is organized **mode-first** (ADR-0006), with two shared verbs (ADR-0002):

- CLI: `lisplens line <read|edit> <file>` (Line-hash mode) and `lisplens struct <read|edit> <file>` (Structural mode).
- MCP: the mirror tools `line_read`, `line_edit`, `struct_read`, `struct_edit`.

`struct read` emits the Outline; `line read` emits the hashline-style line view. Project search will be `lisplens find` / a `find` tool later (ADR-0010). Names are self-descriptive and never expose `sexpp` or `hashline` (ADR-0006).

## Status

accepted

## Considered Options

- **Verb-implies-mode** (`read` vs `outline`, `patch` vs `edit`) — rejected: the mode is not explicit and per-mode usage is harder to measure.
- **Single unified `edit`** (mode implied by anchor) — rejected in ADR-0006.

## Consequences

- Per-mode usage is measurable, serving ADR-0001/ADR-0006's learn-then-unify goal.
- The current flat `read` / `outline` commands become `line read` / `struct read`.
