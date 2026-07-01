# Zero-config inference scans a ranked set of repo-local manifests

To honor zero-config (ADR-0004) while covering real projects, lisplens infers dialect, source roots, and declared dependencies by scanning a prioritized set of repo-local manifests **before** consulting the Project profile. Details and sources are in `docs/research/library-loading-survey.md`.

## Per-ecosystem scan order

- **Emacs Lisp** — `.el` `Package-Requires` / `-pkg.el` first (deps + Emacs baseline + package name), then `Cask` / `Eask` (declarative S-expr: archives, deps, `files` globs), then `Eldev` (well-known `eldev-*` calls only). `makem.sh` carries nothing. Add `.cask/` / `.eask/` / `.eldev/` to load-path when resolving deps.
- **Common Lisp** — `qlfile` / `qlfile.lock` (qlot) or `clpmfile` / `clpmfile.lock` (CLPM) for declared deps + pins, `.asd` (system/source roots), `.ros` header `exec ros … -L <impl>` (weak per-script implementation hint).
- **Scheme** — `Akku.manifest` / `Akku.lock` (Akku: deps + pins; `.akku/lib` load path via `.akku/env`), plus per-file `define-library` / `library` names and extensions (`.sld` ⇒ R7RS, `.sls`/`.sps` ⇒ R6RS) and `#!r6rs`/`#!r7rs` directives for dialect.
- **Racket** — `info.rkt` (`deps` / `build-deps` + `collection` for source roots), interpreted via the restricted `#lang info` grammar.
- **Clojure** — `deps.edn` / `project.clj` (`:paths`, `:deps`, `:aliases` / `:profiles`).

Several of these manifests are themselves S-expressions, so lisplens parses them with its own backend (lispexp).

## Status

accepted

## Consequences

- Zero-config coverage is materially wider than a bare extension map: most tooled projects need no Project profile for deps or source roots.
- Manifest parsing is **best-effort** — match known directive heads and ignore dynamic forms. Most manifests are `read`-safe S-expressions (`Akku.manifest`/lock, `clpmfile`/lock, `.asd`, `Cask`/`Eask`, line-based `qlfile`). Two need more than `read`: `info.rkt` (restricted `#lang info` grammar with `getenv` — interpret in a sandbox) and `Eldev` (arbitrary Elisp — recognize only `eldev-*` calls).
- **No package manager declares the target implementation** (confirmed across Akku, qlot, CLPM, Roswell, raco pkg, Clojure tooling). Implementation / target choice is the **universal irreducible signal** and remains the primary Project profile field (ADR-0014), optionally aided by a secondary hint from CI files or a per-script `.ros -L`.
