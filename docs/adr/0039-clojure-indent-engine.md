# A native Clojure indent engine (cljfmt's `:inner`/`:block` model)

## Context

Clojure/ClojureScript ride the **generic Emacs Lisp fallback** (`engine_for` →
`Engine::Elisp`), so `.clj/.cljs/.cljc` reindent only on an explicit `format`, never
on a Structural edit (`has_native_engine` is false for them). Emacs bundles no
Clojure indenter, so — unlike the Common Lisp and Scheme engines (ADR-0031), which
port an Emacs indent function validated against Emacs — Clojure needs a different
source of truth.

The survey in `docs/notes/20260704-clojure-indentation-survey.md` found the whole
ecosystem has converged on **one model**: cljfmt's `:inner`/`:block` rules, adopted
verbatim by clojure-ts-mode and cljstyle, and now the authoring format of
clojure-mode too. The community style guide is normative prose for the same
("semantic") model. cljfmt is its origin, is the formatter Clojure developers
actually run, and — as a native GraalVM binary — runs here with no JVM. It is the
natural oracle.

## Decision

Add **`Engine::Clojure`**, a native port of **cljfmt's semantic indentation**, and
validate it byte-exact against **cljfmt** (`cljfmt fix` with its non-indentation
passes disabled). `Dialect::Clojure` routes to it in `engine_for` and joins
`has_native_engine`, so Clojure files auto-reindent on edit like the other faithful
engines. The rule table is cljfmt's default `indents/clojure.clj`, bundled in
`src/format/clojure.rs`. The engine reuses the shared driver, `Cols` column
arithmetic, touched-region masking, and the reindent invariant unchanged; like
every engine it only rewrites leading whitespace, so it is always parse-safe.

This is a deliberate **departure** from the other engines: the first rule model of
non-Emacs origin, and the first oracle that is not Emacs. It is justified because
Emacs is not authoritative for Clojure; cljfmt is.

### The model (empirically pinned against cljfmt 0.16.4)

To indent a code line, let `c` be its innermost enclosing collection and
`open = column of c's opening delimiter`. Indexing counts **value children only**
(comments, `#_` discards, and metadata do not consume a slot): in a list `(head a b
…)`, `head` is position 0, `a` is **arg 0**, `b` is arg 1. The body unit is **2**.

- **Collection literals** `[...]`, `{...}`, `#{...}`, and a list whose head is not a
  symbol → **align under the first element** (its column, = open + delimiter width).
- **Symbol-headed list / `#(...)`** → look up the head symbol (namespace-stripped:
  `clojure.core/when` → `when`) and apply its rule(s):
  - **default** (no rule): if an argument sits on the head's line → align every
    continuation under **arg 0**; if the head is alone on its line → **open + 1**.
    Threading macros (`->`, `->>`) have no rule and use this (stages align under the
    first stage).
  - **`[:inner 0]`** (`defn`, `fn`, `def`, …): every direct child of `c` → **open +
    2** (name and arglist included).
  - **`[:inner D]`** / **`[:inner D idx]`** (`D ≥ 1`, e.g. `reify` `[:inner 1]`,
    `letfn` `[:inner 2 0]`): a rule on the form **D+1 levels above** the line's node
    gives **open + 2** (relative to the innermost `c`); the optional `idx` restricts
    it to that ancestor's arg index.
  - **`[:block N]`** (`let`/`when` = 1, `do`/`cond`/`try` = 0, `condp`/`catch` = 2):
    a hybrid. Arg positions `< N` are *special* and use **default** indentation
    (open + 1 when they begin a line). Positions `≥ N` are *body*: they get **open +
    2** **only if the first body form (arg `N`) begins its own line**; otherwise the
    whole form falls back to **default** (align under arg 0). This is the crucial way
    `:block` differs from Emacs `lisp-indent-function`'s integer spec, which
    *double-indents* the special args — cljfmt does not.
  - A form may carry several rules (`defrecord` = `[:block 2] [:inner 1]`); each
    applies at its own tree level (top body via `:block`, method bodies via
    `:inner`), so they compose without conflict. Unknown `def…`/`with-…` heads match
    cljfmt's regex fallbacks → `[:inner 0]`.

### Scope

Clojure family only (`.clj/.cljs/.cljc`); `edn` is data and keeps collection
indentation (no macro table). **Semantic** style only in this change — cljfmt's
alternative **fixed/Tonsky** style (flat +2 for symbol-led lists, no table) is a
follow-up flag. Vertical alignment of map/binding columns (cljfmt's opt-in
`:align-*-columns?`) is out of scope, matching cljfmt's defaults. Comment-only line
indentation is left to the shared driver and may differ from cljfmt in edge cases
(a documented limitation, not an engine rule).

## Status

accepted — implemented in `src/format/clojure.rs`: the `:inner`/`:block` resolver
(walking the container stack up to depth 3 for `:inner D`), the bundled default
table, and the collection/default fallbacks; wired via `Engine::Clojure` in
`engine_for` + `has_native_engine`. Fidelity is validated against `cljfmt fix`
(indentation-only config); golden tests capture cljfmt output.

## Consequences

- Clojure files now auto-reindent on Structural edit (joins `has_native_engine`),
  and `format` matches cljfmt's semantic default on the modelled forms.
- The engine is a **pure function of the s-expression tree** — no tree-sitter, no
  Emacs — so it slots into the existing driver like the CL/Scheme engines, with a
  cljfmt-based (not Emacs-based) fidelity path added to the harness.
- Introduces a second oracle tool (cljfmt) alongside Emacs; the harness note in
  `docs/dev/formatter.md` documents the indentation-only cljfmt config.
- Deferred: the fixed/Tonsky style flag, map/binding column alignment, `edn`-specific
  handling, and any per-project `:extra-indents` (only the bundled defaults ship).
