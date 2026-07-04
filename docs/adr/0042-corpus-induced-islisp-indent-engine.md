# A corpus-induced ISLisp indent engine (first induced dialect)

## Context

The formatter dispatches by dialect to an engine that matches the tool that
dialect's community runs: Emacs Lisp / Common Lisp / Scheme → Emacs, Clojure →
cljfmt, Phel → `phel format` (ADR-0031/0039/0041). Dialects Emacs bundles no
indenter for and that have **no canonical formatter** — Fennel, Janet, Hy, LFE,
ISLisp, AutoLISP — rode the generic Emacs Lisp fallback, which imposes Emacs's
model on a language whose community may indent nothing like Emacs.

We cannot port a reference formatter these dialects do not have. The proposal
(`docs/notes/20260704-corpus-induced-indentation.md`) is to **recover the indent
parameters empirically from the language's own hand-aligned corpus** — its source
and tests — since that corpus is a de-facto house style even where no formatter is
published. The engine is already parameterised by a small, closed set of knobs
(alignment mode, body width, a `head → rule` table), so the learner's output is
exactly the artifact we otherwise hand-bundle, and plugs in with no new runtime.

**Validated first on a dialect that *has* an oracle (the gate).** A learner induced,
per head, "body-indents vs aligns-under-arg 0" from 851 Clojure files and was scored
against cljfmt's known table: recall 0.97, precision 0.76 / F1 0.85 (understated —
most residual "false positives" are real project macros cljfmt's table omits). The
one methodological fix that mattered — don't attribute an ancestor's `:inner` indent
to a nested method head — is general. The method recovers the reference table with
high fidelity, so it is trustworthy on oracle-less dialects.

**First oracle-less target: ISLisp, via EISL** (Easy-ISLisp,
<https://github.com/sasagawa888/eisl>). EISL is ideal: 184 `.lsp` files across
`library`/`example`/`verify`/`tests`/`bench`; a distinctive house style (not Emacs,
not cljfmt); and a **bonus answer key** — EISL ships its own editor `edlis`, whose
`calc_tabs()` (`edlis.c`) *is* the indentation algorithm. That algorithm is a
strikingly simple **binary**: a *special* head (a 12-entry hardcoded `special[]` in
`syn_highlight.c`) body-indents at a fixed `paren + 4`; every other head aligns
under arg 0 (`findnext`). So ISLisp sits in the **Emacs-family shape** (special vs
not), not cljfmt's graded `:block N/:inner D` table — and `if`/`cond`/`for`/`when`
are *not* special, i.e. they align under arg 0 (the distinctive EISL style). lispexp
already reads ISLisp (`Dialect::Islisp`).

Inducing from the EISL corpus recovered EISL's body width ≈ 4 (distinct from
Clojure's 2), got recall 1.0 on the learnable special forms, and — the interesting
result — surfaced a **richer** special set than `edlis`'s minimal 12: the corpus
authors body-indent `defmethod`, `dolist`, `dotimes`, `flet`, `labels`, `lambda`,
`defclass`, `defgeneric`, `defpublic`, `unwind-protect`, `with-open-*`, … which
`edlis`'s table omits. The corpus is richer than the editor's rule set, so induction
yields a *better* formatter spec than `calc_tabs` alone — descriptive over
prescriptive.

## Decision

Add a native **EISL** indent style as an **induced table on the shared Clojure
engine** (ADR-0039), matching edlis's model exactly — but make it **opt-in**, not
the ISLisp default:

- A `FormatConfig.islisp_eisl` flag selects it. `engine_for(Dialect::Islisp) →
  Engine::Clojure` **only when the flag is set**; plain ISLisp falls to the generic
  Emacs Lisp fallback (its behaviour before this ADR). ISLisp does **not** join
  `has_native_engine`, so the opt-in style applies on an explicit `format`, not on
  auto-reindent-on-edit (ISLisp is not extension-detected, so edits leave it
  byte-identical rather than reflowing via the fallback).
- `islisp_rules_for` maps each **special** head to `[:inner 0]` (body always
  `open + body`) and every other head to the default alignment (align under arg 0) —
  which is precisely edlis's "special → `paren + 4`, else `findnext`". The special
  set is edlis's 12-entry `special[]` **plus** the corpus-attested body forms it
  omits (`defmethod`, `dolist`, `lambda`, `labels`, …); `if`/`cond`/`for`/`when` are
  deliberately left aligning, per both edlis and the induction.
- EISL's body width is **4** (edlis's `paren + 4`), passed at the dispatch instead
  of the Lisp default 2 that Clojure/Phel use — and only for the opt-in style.

**Why opt-in.** EISL's `open + 4` / align-under-arg-0 rule is *one community's*
convention (Easy-ISLisp's), cross-implementation checking found ISLisp indentation
is per-community, and `.lsp` is genuinely ambiguous (Common Lisp / AutoLISP /
ISLisp) with no lispexp detection signal. Making it the silent default for every
`--dialect islisp` file would impose EISL's house style on ISLisp users who don't
follow it. So the style is named and requested explicitly:

- **`--dialect islisp-eisl`** — a lisplens-only pseudo-dialect that resolves to
  `Dialect::Islisp` and sets `islisp_eisl` (Step 0's plain `--dialect islisp`
  remains the generic fallback).
- **`islisp-indent-style: eisl`** — a file-/dir-local that resolves the same flag
  (ADR-0029), so a project or file can opt in once and have `format` honour it.

The `.lsp` extension default stays Common Lisp, unchanged.

The learner itself stays **offline tooling** — it *produces* a bundled table; there
is no per-format-call cost. (It remains a scratchpad PoC for now; graduating it into
repo tooling that regenerates tables is follow-up work.)

## Status

Accepted — implemented as an **opt-in style** in `src/format/clojure.rs`
(`islisp_rules_for`, dialect-aware `rules_for`), `src/format/mod.rs`
(config-aware `engine_for`, the flag-gated body-width branch), `src/config.rs`
(`FormatConfig.islisp_eisl` + the `islisp-indent-style` file-local), and
`src/main.rs` (the `--dialect islisp-eisl` form). Validated two ways:

- **Golden test** (`islisp_special_forms_body_indent_and_calls_align`, run with
  `islisp_eisl` on) locks edlis's model: `defun`/`let`/`defmethod` bodies at
  `open + 4`, nested specials from their own paren, and `if`/`foo` continuations
  aligning under arg 0.
- **Corpus fit.** On EISL-native code (`library`/`example`/`verify`/`tests`, the
  `.lsp` that parse), the induced engine matches **75.2%** of code-line indentation,
  vs **54.2%** for the old generic Emacs Lisp fallback and 55.7% for Common Lisp — a
  +21-point gain, the core justification. The `bench/` directory is an outlier
  (25%): its files are Gabriel benchmarks ported from Common Lisp / Scheme carrying
  their original indentation, so they follow no EISL rule — a concrete instance of
  the "multi-style corpus" caveat, not an engine gap.

## Consequences

- ISLisp gains a faithful EISL formatter where none existed — a large improvement
  over the generic fallback — behind an explicit opt-in, so ISLisp users who don't
  follow EISL's house style are unaffected (plain `--dialect islisp` is unchanged).
- **Corpus induction is established as a repeatable path** for the remaining
  no-canonical-formatter dialects: induce from that dialect's de-facto corpus,
  validate against any available reference, ship an induced table. This method was
  then applied to Fennel/Janet/Hy/LFE (ADR-0043).
- The result is **descriptive** (matches how EISL is written, better than `edlis`'s
  minimal table) rather than prescriptive; it normalises toward the plurality style.
  Self-consistency is bounded by how consistent the corpus itself is — an honest
  limit, reported per corpus, never presented as byte-exact against a spec.
- Deferred: the finer per-form body width (some specials cluster at 2/3, not just 4 —
  the corpus is not uniform); graduating the learner from the scratchpad PoC into
  repo tooling; and applying the method to the other fallback dialects.
