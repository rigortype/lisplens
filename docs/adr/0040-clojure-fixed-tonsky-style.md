# Clojure fixed / Tonsky indentation style

## Context

ADR-0039 gave Clojure a native engine matching **cljfmt's default semantic
`:inner`/`:block` style** and deferred cljfmt's alternative **fixed / "Tonsky"**
style ([tonsky.me/blog/clojurefmt](https://tonsky.me/blog/clojurefmt/)). That style
is a deliberate rebellion against the Emacs-descended semantic indentation the other
engines port: it drops the per-symbol rule table and align-under-first-argument
entirely, in exchange for a single rule that never shifts when you rename a function
or add an argument. clojure-ts-mode ships it as `clojure-ts-indent-style 'fixed`;
the community is genuinely split, so lisplens should offer both.

## Decision

Add an opt-in **fixed** style to the Clojure engine, selected by
`FormatConfig.clojure_fixed_indent` (default `false` = semantic). It is enabled by:

- the CLI flag **`format --tonsky`**, or
- a **`clojure-ts-indent-style: fixed`** file-/dir-local (Emacs-style, resolved
  through the ADR-0029 config pipeline, so auto-format-on-edit honours it too).

The fixed rule (empirically pinned against `cljfmt fix` with
`{:indents {#re ".*" [[:inner 0]]}}`, the config that turns cljfmt fixed):

- **collection literals** `[]`/`{}`/`#{}` and **reader conditionals** `#?(…)` →
  align under the first element (identical to semantic — these are data);
- **round lists / `#(…)`**: a **symbol head** → body at a flat **`open + 2`**,
  always (function calls, `do`, threading `->`/`->>`, `defn`, everything — no rule
  table, no align-under-first-argument); a **non-symbol head** → the default
  alignment (align under arg 0), same as semantic.

So fixed reuses the whole engine and only replaces the round-list-with-symbol-head
branch: instead of consulting the `:inner`/`:block` table it returns `open + body`.
Everything else — collections, reader conditionals, metadata `arg` descent, the
`container_at`/`in_string` gates — is shared verbatim.

## Status

accepted — implemented: `FormatConfig.clojure_fixed_indent`, resolved from
`clojure-ts-indent-style` (config) and `--tonsky` (CLI); `clojure::indent` takes a
`fixed` flag and short-circuits the symbol-headed round list to `open + body`.
Validated byte-exact against `cljfmt fix --config` (fixed config) on the reitit +
ring + hiccup corpora — 268/272, the same residual 4 `#_`-discard files as the
semantic style (the upstream reader limitation, `docs/lispexp-feedback/0003`).

## Consequences

- Both indentation cultures are supported from one engine; the fixed path is a
  handful of lines because fixed is a *simplification* of the semantic model.
- `--tonsky` is per-invocation; `clojure-ts-indent-style` is per-project (and flows
  into auto-format-on-edit). Semantic stays the default.
- Deferred: reading the style from a `.cljfmt.edn`/`.zprint` project file (only the
  Emacs-style file-/dir-local and the CLI flag are wired), and any *partial* fixed
  configs (a project that overrides only some symbols) — lisplens offers the two
  canonical styles, not arbitrary `:indents` maps.
