# Delegation boundary review: what else belongs in lispexp-emacs (2026-07-04)

Written right after lisplens migrated its bundled indent table and its
file-local / dir-local **parsers** to the new companion crate `lispexp-emacs`
(commit `02a293a`, lispexp ADR-0033). This note answers: what Emacs-specific
logic still lives in lisplens that *another* Emacs-Lisp tool would likely
re-implement, and is the current division of responsibility right? Sequel to
[20260703-lispexp-retrospective.md](20260703-lispexp-retrospective.md).

## Where things sit now

| Crate | Holds | Nature |
| --- | --- | --- |
| **lispexp** (core) | reader (`parse`/`Datum`/`LineIndex`/`annotate`/`walk`), indent **types** + `harvest_indent_specs` | polyglot, reader-only (ADR-0001) |
| **lispexp-emacs** | indent **data** (`indent::bundled_table`), Emacs **parsers** (`local_vars`, `dir_locals`) | Emacs-specific — **data + parsing** |
| **lisplens** | indent **algorithm** (`format.rs`), Nameless (`nameless.rs`), EditorConfig, config resolution, CLI / MCP / patch-DSL | the tool |

The split is a correct first step but **incomplete for the stated goal**:
lispexp-emacs today owns *data* and *parsing* but not *behavior*. The single
most-reused piece of Emacs behavior — the indenter — is still in lisplens.

## Remaining delegation candidates (by likelihood another tool re-implements it)

### 1. The indent algorithm — top candidate (`src/format.rs`, ~600 lines of the 784)

A faithful Rust port of Emacs's `calculate-lisp-indent` / `lisp-indent-function`
/ `lisp-indent-specform`. **Every tool that wants Emacs-faithful `.el`
indentation reproduces exactly this** — it tracks a fixed spec (Emacs's own
indenter), so it is high-reuse Emacs knowledge, not tool-specific policy. The
retrospective kept it in lisplens under the rule "lispexp is reader-only"; that
rule **does not bind `lispexp-emacs`**, which is already Emacs-specific and holds
non-reader logic (the table, the config parsers). So the calculus has changed:
this is now the prime thing to move.

- Public surface that would become the crate's formatter API: `format_elisp`,
  `format_elisp_nameless`, `reindent`, `reindent_range`, `reindent_block`,
  `Touched`.
- Move condition: the algorithm depends on `crate::config::FormatConfig` (four
  fields: `indent_tabs`, `tab_width`, `body_indent`, `comment_column`) and
  `crate::nameless::Nameless`. Replace those with a `lispexp_emacs`-side
  `IndentOptions` (primitives) + owning Nameless. lisplens keeps `config::resolve`
  (discovery) and the edit pipeline (`patch.rs`), and passes resolved params /
  byte ranges into the moved API.

### 2. Nameless emulation — move with #1 (`src/nameless.rs`, 118 lines)

The composed-width model (`⌊len/2⌋ + 1`, reverse-engineered against Emacs) is
Emacs-ecosystem knowledge any nameless-aware formatter needs. The algorithm (#1)
consumes it, so it should travel together; also reusable on its own.

### 3. Emacs config resolution — second-tier (`src/config.rs`)

`set_var` (interpreting `indent-tabs-mode` / `tab-width` / `lisp-body-indent` /
`comment-column` / `nameless-mode` raw text into typed values) plus
`apply_dir_locals`'s **discovery + precedence** (walk ancestors, `.dir-locals-2.el`
over `.dir-locals.el`, last-wins) reproduce Emacs's `hack-dir-local-variables`
behavior. Parsing already moved; a `lispexp_emacs::config::resolve_local_vars(path)`
that walks + parses + typed-parses raw bindings would spare the next tool this,
while the tool still maps the bindings onto its own config struct.

## Stays in lisplens (correct)

EditorConfig resolution (not Emacs, not lispexp's domain), the `FormatConfig`
*final shape* (tool-specific — though its *fields* are Emacs vars), the config
precedence *composition* (defaults → EditorConfig → dir → file), and the whole
CLI / MCP / patch-DSL / edit pipeline.

## Conclusion

Direction is right; the highest-reuse behavior isn't delegated yet. With
`lispexp-emacs` established as the Emacs-specific companion, **moving #1 (the
indent algorithm) and #2 (Nameless) into it** would most reduce re-implementation
across Emacs-Lisp tooling — turning it from an "Emacs data + parsers" crate into
an "Emacs data + parsers + indenter" crate, with lisplens reduced to config
discovery + the editing surface. #3 is a smaller follow-up. Recommend scheduling
#1+#2 onto the `lispexp-emacs` roadmap (a bigger surface for it to own/maintain,
but it stops every consumer re-porting `calculate-lisp-indent`); do it as its own
migration (crate-side implementation + lisplens consume-side swap), mirroring the
table migration.
