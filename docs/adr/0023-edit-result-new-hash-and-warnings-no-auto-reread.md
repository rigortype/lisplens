# Edit result: new hash + warnings, no auto re-read

On **success**, an edit returns the **new file-hash** (the gate for a subsequent batch) plus any **validate-then-write warnings** (ADR-0005 — a touched definition whose parens still balance but whose form-spec role tagging degraded).

On **failure**, the batch is all-or-nothing: it returns the **failing operation and the reason** (drift / new-parse-errors / overlap / out-of-bounds) and **writes nothing**.

It does **not** auto-emit a fresh read. The Batch model (ADR-0006) is "one read, then one batch," so intermediate re-reads are rare; an agent that wants fresh anchors calls `read`. (A `--reread` option can be added later.) The result is terse text by default, JSON on MCP (ADR-0013).

## Status

accepted

## Consequences

- Responses stay cheap; the agent re-reads only when it actually needs new anchors.
- Because strict gating (ADR-0017) invalidates every prior anchor after a write, chaining edits without a re-read is intentionally not a supported fast path — batching is.
