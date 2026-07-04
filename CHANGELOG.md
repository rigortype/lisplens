# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `--dialect NAME` — force the dialect for a single-file command (kebab-case, e.g. `--dialect islisp`) instead of guessing from the file extension, so an ambiguous extension like `.lsp` (Common Lisp / AutoLISP / ISLisp) can be read as the one you mean. Project-wide `find`/`refs` keep their per-file guess.

## [0.2.0] - 2026-07-04

The polyglot release. lisplens grows from an Emacs-Lisp-focused tool into a structural editor and formatter for the whole Lisp family, along two lines. First, a set of parse-safe refactoring commands — `check`, `rename`, `inline`, `rewrite`, and `extract` — that do symbol-accurate, drift-checked transformations as single atomic operations and refuse anything that would break the parse. Second, native indent engines for Common Lisp, the Scheme family, Clojure, and Phel, each a faithful port validated byte-exact against that language's own reference formatter (Emacs, `cljfmt`, `phel format`), so `format` is now correct across dialects rather than approximating them through the Emacs Lisp indenter.

### Added

- **`check` — a standalone parse-check.** `lisplens check <file>` (and MCP `check`) parses a file by dialect and prints `path:line: message` diagnostics, staying silent with exit 0 when clean and returning non-zero on parse errors — the validity guarantee lisplens already enforces on every edit, surfaced so agents and CI need not shell out to Emacs `check-parens`.
- **`rename` — safe whole-file symbol rename.** `lisplens rename <old> <new> <file>` (MCP `rename`) renames a symbol across a file, symbol-exact in both code and data and never inside strings, comments, or keywords, so sibling symbols like `foo-bar` survive when you rename `foo`; the touched forms are reindented and any edit that would break the parse is refused.
- **`inline` — safe function inline-expand.** `lisplens inline <name> <file>` (MCP `inline`) expands a function at its call sites over the provably safe subset — a single non-recursive `defun`/`defsubst`/`cl-defun`/`cl-defsubst` or Scheme `(define (name …) …)` with required-only parameters — substituting the body directly (niladic) or through an order-preserving `let` (with parameters); macros, recursion, `&`-lambda-lists, and arity mismatches are refused with a reason instead of mis-expanded.
- **`rewrite` — a structural pattern→template rewrite.** `lisplens rewrite <file>` (spec on stdin; MCP `rewrite`) is a parse-safe "structural sed": a datum matcher with metavariables, syntactic classes, non-linear repeats, and a trailing-sequence match, applied whole-tree over a splice→reindent→validate pipeline. A user guide and a verified cookbook ship in [`docs/rewrite.md`](docs/rewrite.md).
- **`extract` — pull a form into a new function.** `lisplens extract <file> <anchor> <name> [param…]` (MCP `extract`) cuts the form at `anchor` into a new definition and replaces it with a call, per-dialect (`defun`/`define`/`defn`). Options compose: `--count N` extracts a run of N contiguous sibling forms, `--kind HEAD` names the definition's leading operator (e.g. `defsubst`, `defn-`), `--all` folds every structurally-equal occurrence into one function, and `--also ANCHOR` anti-unifies several differing sites into one generalized function with inferred parameters.
- **Native Common Lisp indenter.** `format` indents `.lisp`/`.lsp`/`.cl`/`.asd` with a faithful port of Emacs's `common-lisp-indent-function` — `loop`, `tagbody`, `do`, `defmethod`, the lambda hack, and lambda-list-keyword alignment — validated byte-exact against Emacs `lisp-mode`.
- **Native Scheme-family indenter.** `format` indents the Scheme family (Scheme, Guile, Racket, Gauche, Mosh, Gambit) with a port of Emacs's `scheme-indent-function`, including the named-`let` method and the MIT dialect table, validated byte-exact against Emacs `scheme-mode` on the chibi-scheme, gauche, and typed-racket sources.
- **Native Clojure indent engine.** `format` indents `.clj`/`.cljs`/`.cljc` with a native port of `cljfmt`'s semantic `:inner`/`:block` model — the standard the Clojure ecosystem converged on — validated byte-exact against `cljfmt fix` across eight real repositories. `format --tonsky` selects the alternative fixed/Tonsky style (a flat `+2` body indent), also selectable through a `clojure-ts-indent-style: fixed` file- or dir-local.
- **Native Phel indent engine.** `format` indents `.phel`. Phel's own `phel format` is a PHP port of `cljfmt`'s model, so Phel shares the Clojure engine with its own indent table (phel-lang 0.47), validated byte-exact against `phel format` on the phel-lang source.

### Changed

- **`format` is now polyglot, dispatching by dialect.** One shared driver selects a native engine per file — Emacs Lisp, Common Lisp, the Scheme family, Clojure, or Phel — and dialects Emacs bundles no indenter for still ride the Emacs Lisp engine as a fallback. Auto-format of the touched region on a Structural edit is enabled for every dialect with a faithful native engine.
- **Indent alignment is measured by display width, not UTF-8 byte length.** A wide or multi-byte glyph before an alignment target now advances the column as Emacs's `current-column` does (`漢`/`Ａ` count as 2, `λ` as 1), so a continuation under such text lands correctly; pure-ASCII output is unchanged.
- **`#_` / `#;` discarded forms now indent faithfully.** A discarded form is kept in the parse tree, so lines inside a multi-line discard indent against the discarded form — matching `cljfmt` — instead of against the enclosing container. This came with `lispexp` 0.6 → 0.7, which also fixes three Phel reader constructs (a `;` inside a symbol, the `|(…)` short anonymous function, and PHP `\Foo\Bar` fully-qualified names) so they read as single correct forms — making `rename`/`refs`/`extract` symbol-accurate on Phel too — plus `lispexp-emacs` 0.1 → 0.2.

## [0.1.1] - 2026-07-04

A dependency-only release: the Emacs Lisp data and parsers lisplens used to carry in-tree moved out to the new `lispexp-emacs` companion crate. Output is unchanged.

### Changed

- The Emacs Lisp formatter now sources its bundled indent-spec table, and its file-local (`-*- … -*-` / `Local Variables:`) and `.dir-locals.el` variable parsers, from the `lispexp-emacs` crate instead of carrying them in-tree — lisplens keeps the indent algorithm, Nameless awareness, EditorConfig, and config precedence. Dependencies: `lispexp` 0.5 → 0.6 plus `lispexp-emacs` 0.1; the indent table was verified byte-identical, so indentation is unchanged.

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

[Unreleased]: https://github.com/rigortype/lisplens/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/rigortype/lisplens/releases/tag/v0.2.0
[0.1.1]: https://github.com/rigortype/lisplens/releases/tag/v0.1.1
[0.1.0]: https://github.com/rigortype/lisplens/releases/tag/v0.1.0
