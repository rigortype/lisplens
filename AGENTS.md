# lisplens

## Codebase

lisplens is a CLI + MCP tool for token-efficient, polyglot Lisp editing by AI
agents, built on the lispexp reader. Orient with these before changing code:

- **[docs/dev/architecture.md](docs/dev/architecture.md)** — module map, CLI/MCP surface, patch DSL, the edit safety pipeline.
- **[docs/dev/formatter.md](docs/dev/formatter.md)** — the native Emacs Lisp indenter: model, the reindent invariant (do not regress), the bundled indent-spec table + how to regenerate it from Emacs, and the fidelity harness.
- **[CONTEXT.md](CONTEXT.md)** — domain glossary. **[docs/adr/](docs/adr/)** — the architecture decisions (read the ones touching your area; never contradict one silently, per `docs/agents/domain.md`).
- **[docs/lispexp-integration.md](docs/lispexp-integration.md)** / **[docs/lispexp-feedback/](docs/lispexp-feedback/)** — how the lispexp backend is used, and outstanding upstream asks.
- **[docs/CURRENT_WORKS.md](docs/CURRENT_WORKS.md)** — status snapshot + next steps (ephemeral; durable knowledge is in the dev docs above).

Conventions: Rust edition 2021, current stable (no pinned MSRV — a binary tool whose deps track recent Rust); keep `cargo fmt`, `cargo test`, and `cargo clippy --all-targets` green. Commit as work lands, with imperative subjects and ADR refs.

**Ground rule — lispexp is upstream; never change it directly.** lisplens is built on the lispexp reader, but every change lisplens needs there — reader/lexer behaviour, a new API — is requested **only** by adding a note to [`docs/lispexp-feedback/`](docs/lispexp-feedback/): the phenomenon, the diagnosis, and a concrete proposed fix. Never edit, commit to, or open a PR against the lispexp repo (or any other upstream dependency) — even when the fix is small and you can see it. The maintainer decides and implements upstream changes; lisplens then consumes a published version and marks the feedback note resolved. Downstream-authored upstream PRs are out of bounds.

Markdown destined for GitHub (PR descriptions, release notes, issue bodies) must **not** hard-wrap prose lines — GitHub renders a hard line break as `<br>`, so wrapped paragraphs display with ragged breaks. Write each paragraph as one long line and let it soft-wrap. (In-repo docs keep their usual wrapping; this convention applies to text GitHub renders.)

## Agent skills

### Issue tracker

Issues and PRDs live as GitHub issues (via the `gh` CLI). External PRs are also a triage surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`CONTEXT.md` + `docs/adr/` at the repo root). See `docs/agents/domain.md`.
