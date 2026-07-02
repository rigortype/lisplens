# Formatting runs on edit (touched region) plus a `format` command

lisplens formats on **every edit**, reindenting only the **touched forms** (the edited nodes and their enclosing forms) while leaving untouched code byte-identical — so agents never manage whitespace (ADR-0011) and an edit does not churn unrelated lines or invalidate unrelated anchors. It also offers a standalone **`format`** command (and `format` MCP tool) that reformats a whole file, for initial adoption or a one-off pass.

The native spec-driven indenter is the default backend (ADR-0011).

## Pipeline

An edit becomes: **apply edits → reindent the touched region → validate-then-write → atomic write.** The returned file-hash is of the **formatted** content, and validation and warnings (ADR-0005, ADR-0024) run on the formatted content — what actually lands on disk.

## Status

accepted

## Considered Options

- **Whole-file auto-format on every edit** — rejected: massive churn; it rewrites lines the agent did not touch, blows up diffs, and invalidates every other anchor.
- **A `format` command only** (no auto-format on edit) — rejected: edits would not be formatted, violating ADR-0011.

## Consequences

- Touched-region reindent must identify the affected forms and reindent just those — the indenter's main complexity.
- Because the result reflects the formatted content, an agent's next batch gates on the post-format hash (re-read if needed).
