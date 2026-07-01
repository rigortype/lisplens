# One canonical result model; terse-text default; structured JSON opt-in on MCP

Reads and edit results share a **single internal model** (anchors, hashes, spans, diagnostics). Both the CLI and the MCP server render it as **terse text by default** — the token-efficient wire format the whole product optimizes for. The MCP server can additionally return **structured JSON on request** for consumers that want typed data.

- **Line-hash reads** use hashline's format: a `[path#FILEHASH]` header, then `LINE:hash|content` lines.
- The **Structural Outline** uses a compact `line hash kind name` form, with nesting shown by indentation.

Every read **embeds the file-level and per-anchor hashes**, so its output is directly usable as edit-anchor input — saving a second round-trip. Exact delimiter glyphs are deferred (see the open path-syntax question).

## Status

accepted

## Considered Options

- **CLI text / MCP JSON as two diverging formats** — rejected: divergence and double maintenance; a single model keeps the two surfaces aligned.
- **JSON-first everywhere** — rejected: typed but verbose, at odds with the token-efficiency goal. JSON stays opt-in.

## Consequences

- Terse text is the contract; agents parse anchors directly from it.
- Reads are self-anchoring: their output feeds straight back as edit input.
