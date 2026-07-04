# A native Phel indent engine (shares the Clojure engine + Phel's table)

## Context

[Phel](https://phel-lang.org/) is a Clojure-inspired Lisp that compiles to PHP. It
ships its own formatter, `phel format`, and lisplens already recognises `.phel`
(`Dialect::Phel`) — but Phel rode the generic Emacs Lisp fallback (`engine_for` →
`Engine::Elisp`), so its output did not match `phel format`.

`phel format` turns out to be a **PHP port of cljfmt's `:inner`/`:block` model** —
its `Formatter/Domain/Rules/Indenter/{Inner,Block,List}Indenter.php` mirror cljfmt's
indenters, and `FormatterFactory` carries a Phel-specific symbol table. So Phel can
reuse lisplens' Clojure engine (ADR-0039) almost verbatim, differing only in the
rule *table* and one `:block` detail.

`phel` (with PHP + Composer) is installed on this machine, so — like cljfmt for
Clojure — it is a real, runnable oracle.

## Decision

Route `Dialect::Phel` to `Engine::Clojure` and select the indent table inside the
engine by dialect (`rules_for(name, dialect)`), adding `phel_rules_for` — Phel's
`FormatterFactory::{INNER,BLOCK}_INDENT_SYMBOLS` (phel-lang 0.47), verbatim. Phel
joins `has_native_engine`, so `.phel` files auto-reindent on a Structural edit.
Validate byte-exact (on leading indentation) against `phel format`.

Phel is a **strict subset** of the shared model — only `[:inner 0]` and `[:block
N]`, no nested `:inner`, no regex fallback, no reader conditionals — plus **one
semantic difference in `:block`**:

- cljfmt keeps a block form's *special* args (`arg_index < N`) on the default
  alignment (`open + 1` when they wrap).
- Phel's `BlockIndenter` applies the inner indent to **every** child once the body
  breaks: with the head alone, `(when-not⏎(test)⏎(body))` puts *both* the test and
  the body at `open + 2` (cljfmt would put the test at `open + 1`).

`block_indent` takes the dialect and skips the special-arg default only for
non-Phel. Everything else — the `:inner`/default/collection logic, metadata and
prefixed-head handling, the `container_at`/`in_string` gates — is shared unchanged.
The fixed/Tonsky flag (ADR-0040) is Clojure-only; Phel is always semantic.

Phel value-column-**aligns** binding vectors (`(let [a   1 …])`), which lisplens —
rewriting leading whitespace only — cannot reproduce (the same class as cljfmt's
opt-in alignment). This is out of scope; fidelity is measured on leading
indentation, where the alignment padding does not change a line's indent.

## Status

accepted — implemented in `src/format/clojure.rs` (`phel_rules_for`, dialect-aware
`rules_for`/`block_indent`) and `src/format/mod.rs` (`engine_for`,
`has_native_engine`). Validated against `phel format` on **phel-lang's own 310
`.phel` files: 307 byte-exact** on leading indentation. The 3 residuals are: a
`;`-inside-a-symbol token that lispexp's Phel reader splits at `;` (an upstream
tokenisation gap — `docs/lispexp-feedback/0004-phel-semicolon-in-symbol.md`), and
two niche one-liners (a closing `])` after inline comments in a `defstruct` field
vector, and a `#(…)`-nested off-by-one in a doc example that uses Clojure-style
`#(…)` rather than Phel's `|(…)`).

## Consequences

- Phel gets a faithful formatter for free by sharing the Clojure engine — the only
  new code is the table and the one-line `:block` branch.
- A **third oracle** (`phel format`) joins Emacs and cljfmt in the fidelity harness.
- The table tracks phel-lang 0.47; a future Phel adding/removing a form (e.g.
  `with-open`, present on Phel's `main` but not 0.47) would need a table refresh,
  same as cljfmt's table.
- Deferred: binding value-column alignment (out of scope for a leading-whitespace
  formatter), and the two niche one-liners.
