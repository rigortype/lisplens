# Patch DSL grammar: heredoc payloads

The CLI edit surface (`line edit` / `struct edit`) reads a **Patch** — the terse-text rendering of an edit Batch (ADR-0019) — from stdin. Concrete spellings below are the v1 proposal and may be adjusted; the load-bearing decisions are the heredoc payload fencing and the anchor encoding.

## Grammar

```
@ <file-hash>                     # snapshot assertion; drift → reject the batch
<verb> <anchor> [args] [<<TAG]
  ...payload lines...
TAG
```

- **Header.** A leading `@ <file-hash>` line asserts the snapshot the Patch was built against (the batch-level drift gate, ADR-0017).
- **Anchor.** `L:H` = `line:hash` (the Outline's own columns). On the rare same-line hash collision the read emits `L:H:N` with a 1-based ordinal `N`.
- **Payload.** For ops that carry text, the op line ends with `<<TAG`; payload runs until a line equal to `TAG` (heredoc; the agent picks a non-colliding tag). MCP passes the same payload as a JSON string, so fencing is a CLI-only concern.
- **No-payload ops** are a single line.

## Verbs

- **Shared (both modes):** `replace <anchor> <<TAG…`, `delete <anchor>`, `insert-after <anchor> <<TAG…`, `insert-before <anchor> <<TAG…`.
- **Structural-only:** `wrap <anchor> <<TAG…` (payload = the enclosing prefix), `raise <anchor>`, `splice <anchor>`, `slurp-fwd <anchor>`, `slurp-back <anchor>`, `barf-fwd <anchor>`, `barf-back <anchor>`, `split <anchor> @<index>`, `join <anchor> <anchor2>`, `rename <anchor> <<TAG…` (payload = the new symbol).

## MCP mirror

The same ops as a JSON array, e.g. `{ "op": "replace", "anchor": "12:a3f2", "text": "…" }`, `{ "op": "raise", "anchor": "30:9a2c" }` — one apply path (ADR-0019).

## Status

accepted

## Consequences

- Heredoc payloads are robust against Lisp content; the agent chooses the tag.
- The collision ordinal (`L:H:N`) is now pinned, closing the open item from ADR-0018/ADR-0019.
- Verb spellings and per-op arg forms are a proposal; changing them does not affect the batch model or apply path.
