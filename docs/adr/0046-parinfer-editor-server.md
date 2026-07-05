# parinfer editor server — a persistent line-delimited process

## Context

The `parinfer` command (ADR-0045) is a one-shot stdin→stdout transform. That is
right for scripting and one-off normalization, but the Emacs live minor-mode
(issues #30–33) fires on every edit, and spawning a fresh process per keystroke
pays the OS process-creation cost each time — too slow for interactive use.

parinfer's own editor plugins solve this by loading an in-process dynamic module.
lisplens deliberately is **not** that (ADR-0045, execution model A): it is a CLI
process, and we keep it one. The editor instead holds **one warm process** and
talks to it repeatedly.

## Decision

Add `lisplens parinfer --server`: a persistent, **line-delimited JSON** server.

- Reads one JSON **request object per line** from stdin and writes exactly one
  JSON **answer per line** to stdout, staying alive until EOF.
- **Stateless per request.** Each request carries its own
  `{mode, text, dialect?, nameless?, name?, cursorLine?, cursorX?}` — the same
  shape as the MCP `parinfer` tool — so one process serves every editor buffer;
  there is no session or per-buffer state to manage.
- Answer is the shared `{text, success, error, cursorX, cursorLine}` shape
  (ADR-0045). A malformed or blank line yields an error answer
  (`bad-json` / `bad-request`) rather than desynchronizing the stream — one line
  in, one line out, always.

The engine is untouched: the server is a thin loop over
`parinfer::run_json_line`, which parses a line and dispatches to
`parinfer::run_json` → the existing `parinfer::run` + `answer_to_json`. The MCP
`parinfer` tool now shares `run_json`, so the CLI one-shot, the MCP tool, and the
server all speak one request/answer shape.

**Why a dedicated line protocol rather than reusing `lisplens mcp`.** The MCP
server is also a persistent stdio process with a `parinfer` tool, but it is
JSON-RPC 2.0 with an `initialize` handshake and `tools/call` envelope — more than
an editor needs, and heavier to drive from Emacs Lisp. A bare
`request-line → answer-line` protocol is the smallest thing that works and the
easiest to implement on the editor side.

## Status

accepted

## Consequences

- `lisplens parinfer --server` in `main.rs` (`run_parinfer_server`), plus
  `parinfer::run_json` / `run_json_line` shared by the server and MCP. No engine
  change; the one-shot `parinfer` command is unchanged.
- The line protocol is the contract the Emacs minor-mode (#32) drives, and it is
  where cursor protection (#31) and both modes are exercised interactively.
- Deferred: batching / cancellation of in-flight requests, and any per-buffer
  incremental state (would only matter if we ever add real smart mode, which
  ADR-0045 keeps out of scope).
