# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-03

First release. lisplens is a CLI and MCP tool that lets an AI agent read a Lisp file's shape cheaply, get a stable content-hash anchor for any target, and edit by that anchor without resending the whole file — drift-checked, syntax-validated, and written atomically. It is polyglot (the dialect is guessed from the file extension) and built on the [lispexp](https://crates.io/crates/lispexp) reader.

### Added

- **Structural mode** — address code by definition and drill into any inner node via the parse tree. `struct read` outlines a file's definitions and expands one to list inner-node anchors; `struct edit` applies a patch of paredit-style ops: `replace`, `delete`, `wrap`, `raise`, `splice`, `slurp-fwd`/`slurp-back`, `barf-fwd`/`barf-back`, `split`, `join`, `rename`, and `format` (reindent one anchored form in place).
- **Line-hash mode** — address code by line, dialect-agnostically. `line read` gives a `[path#hash]` header plus per-line `N:hash|content`; `line edit` applies `replace` / `delete` / `insert-after` / `insert-before`.
- **Anchored, drift-safe edits** — every edit is a patch with a `@ <file-hash>` header gating the whole batch; a mismatched or drifted file is refused. Edits are validated (never make a file's syntax worse than it was) and written atomically, preserving mode and following symlinks. lisplens owns whitespace, so agents supply content, not spacing.
- **Project queries** — `find <name> [dir]` locates definitions by name across a project, and `refs <name> [dir]` finds symbol occurrences tagged as code or data.
- **Native Emacs Lisp formatter** — `format <file>` reindents an `.el` file, a faithful port of Emacs's `calculate-lisp-indent` (byte-exact with Emacs on common code). It honors `indent-tabs-mode`, `tab-width`, `lisp-body-indent`, and `comment-column` resolved from file-local variables, `.dir-locals.el`, and `.editorconfig`; offers optional [Nameless](https://github.com/Malabarba/Nameless) awareness (`--nameless`, or a `nameless-mode` local); and auto-formats the touched region on a Structural edit.
- **MCP server** — `lisplens mcp` exposes the same operations over stdio for MCP clients.
- Polyglot coverage: Common Lisp, Scheme, Emacs Lisp, Clojure, Racket, Fennel, Janet, Hy, LFE, Phel, and more.

[Unreleased]: https://github.com/rigortype/lisplens/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rigortype/lisplens/releases/tag/v0.1.0
