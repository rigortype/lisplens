# The indenter is a standalone block-level procedure

The native indenter is factored as a **standalone, reusable procedure** that reindents a single form / subtree — a **block** — producing the reindentation as edits over that block's span and touching nothing outside it. It is a pure component: `reindent(block) → edits`.

Three entry points call it:

1. **Auto-format after a Structural edit** (ADR-0025, ADR-0027) — reindent each touched form (the edited nodes and their enclosing top-level form).
2. **The whole-file `format` command / MCP tool** — reindent every top-level form.
3. **Explicit block-level format** — reindent one form by anchor, without otherwise editing it. Exposed as a `format <anchor>` op in the Structural patch DSL (ADR-0021), so an agent can tidy exactly one form on demand.

## Status

accepted

## Consequences

- All formatting flows through one procedure, so it can be tested in isolation against the golden corpus (ADR-0026).
- Block-level formatting is a first-class, separately-invocable capability — not something reachable only as a side effect of editing.
- "Touched region" for auto-format is defined in terms of these blocks: the top-level form(s) the edits fell within.
