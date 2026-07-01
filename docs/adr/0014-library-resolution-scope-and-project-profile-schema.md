# Library resolution scope; Project profile schema

Resolves the open question in ADR-0004, informed by a survey of Common Lisp, Scheme, Clojure, and Emacs Lisp (see `docs/research/library-loading-survey.md`).

## Finding

Every dialect exposes a declared, name-based dependency graph in-repo (`import` / `require` / `:depends-on` / `Package-Requires`) and a statically inferable **intra-repo** name→file mapping (naming conventions + declared source roots + relative includes). But resolving an **external** name to a concrete file or version always requires out-of-repo, per-machine runtime configuration — search paths, implementation choice, and installed package layout.

## Decision 1 — scope

lisplens edits the **user's own repo files, not external dependencies**. It therefore does **not** resolve external imports to files. It reads declared dependency *names* only for context (Outline, Project search), never fetching or path-resolving external libraries. This removes the hardest, machine-specific part of the problem from lisplens's responsibility entirely.

## Decision 2 — Project profile schema

The Project profile persists only the irreducible tuple that is both non-inferable and actually affects editing:

- **implementation** — when extension + content is ambiguous (scheme → guile/chez/racket/chibi/gauche; cl → sbcl/ccl/…; clojure target → clj/cljs; elisp Emacs-version baseline, optional).
- **dialect / standard mode** — when ambiguous (R6RS vs R7RS).
- **source roots** — override, only when not inferable from the project's own manifest or layout.
- **(advanced, optional)** extra in-tree search roots; active build variant (Clojure alias/profile, Scheme `cond-expand` features).

## Status

accepted

## Consequences

- Consistent with ADR-0004's zero-config-first stance: most projects supply implementation and roots via their existing manifests (`deps.edn :paths`, `.asd`, `define-library` layout, `Package-Requires`), so zero-config holds; the profile only fills genuine gaps.
- Cross-file features (Project search, Mode fallback locations) rely on the intra-repo graph, which is static.
- External dependency editing/resolution is explicitly not a lisplens responsibility; that stays with each ecosystem's package manager.
