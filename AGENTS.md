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

## Agent skills

### Issue tracker

Issues and PRDs live as GitHub issues (via the `gh` CLI). External PRs are also a triage surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Default label vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`CONTEXT.md` + `docs/adr/` at the repo root). See `docs/agents/domain.md`.
