# The `rewrite` procedure: a structural s-expr pattern language

## Context

ADR-0032 sketched an `extract`/`fold` member — "find sub-forms matching a
structural pattern and replace them" — and parked its hard problems
(metavariables, non-linear matching, structural equality modulo formatting,
cross-file scope) as design-first. This ADR resolves that design (from a grilling
session). Two clarifications reshape the member:

1. **It is a "structural sed", not a behaviour-preserving refactor.** Unlike
   `rename`/`inline` (which preserve behaviour), a general pattern→template
   rewrite does not: guard removal `(when flag (foo))` → `(foo)` deliberately
   drops the guard. So lisplens guarantees only **parse-safety** (validate-then-
   write) and **exact structural matching** (no substring / comment / data-
   position false hits) — never that a rewrite preserves meaning. The user
   asserts semantic validity; lisplens is the safe blade, not the judge. (Even a
   fold that duplicates an operand — `(double $n)` → `(* 2 $n)` on `(getnum)` —
   is the user's call; the pattern language only *lets them express* the safety
   distinction, below.)

2. **`extract` is renamed `rewrite`.** What ADR-0032 called `extract` is a
   *general pattern→template rewrite engine* — the substrate under fold-to-a-call,
   guard removal, `progn` unwrap, and `(if c a nil)` → `(when c a)`. The name
   `extract` is reserved for the genuinely different, larger future member
   "extract a selection into a **new** function" (which must invent a name, infer
   parameters, and place a definition — not expressible as pattern→template).

## Decision

A general **`rewrite <file>`** procedure (CLI + MCP `rewrite` tool) reads a
`pattern → template` spec from **stdin**, one rewrite per invocation (batching is
future). The grammar reuses the patch DSL's user-chosen heredoc tags (ADR-0021)
so a pattern body containing a fixed marker word can't terminate the block:

```
[@ <file-hash>]
pattern <<TAG
<pattern s-expr>
TAG
template <<TAG
<template s-expr>
TAG
```

The `@ <file-hash>` drift gate is **optional** (deliberately unlike `struct
edit`): with it, rewrite gets the same strict drift guarantee as a patch; without
it, rewrite reads and edits the file in one shot like `rename`/`inline` (no extra
`line read` to fetch a hash, and re-running the same spec stays idempotent rather
than drift-erroring). Named presets (guard-remove, `progn`-unwrap, if→when) are v1
**documentation examples**, not built-in flags.

### The pattern language

Both pattern and template are s-exprs, **parsed with the target file's dialect**
(so `[]`, `#:kw`, … work). A distinguished leaf syntax marks metavariables;
everything else is a **literal**, matched structurally.

- **Metavariable** `$name` — captures the single form at its position.
- **Wildcard** `$_` — matches one form, captures nothing.
- **Sequence metavariable** `$name...` (≡ `$name ...`, whitespace-insensitive —
  both tokenizations normalize to the same thing) — captures a contiguous run of
  sibling forms. At most one per list, fixed elements allowed before it, capturing
  to the end (a trailing fixed suffix after the run is future). **`...` is a
  sequence marker only immediately after a metavariable token** (`$body...` /
  `$body ...`); anywhere else `...` is a literal symbol matched structurally. The
  one thing this forbids is a literal `...` *directly after* a metavariable — a
  cost paid only when rewriting Scheme `syntax-rules`, and documented.
- **Metavariable class** `$name:class` — a *syntactic* match filter (fails the
  match when the bound form is not of the class):
  - `any` (default) · `atom` (Symbol/Keyword/Number/Str/Char/Bool) · `lit`
    (Number/Str/Char/Bool + a plain-quoted datum `'x` — no variable, so backquote
    is excluded as it may hold an unquote; a vector `#(…)` is not `lit` in v1) ·
    `sym` (a bare symbol) · `list` (a compound form / call). `nil`/`t`/keywords are
    `atom`, not `lit` — for duplication safety `atom` is the workhorse.
  - Classes are the user's tool for **duplication safety**: to fold
    `(double $n)` ↔ `(* 2 $n)` without duplicating a side effect, constrain
    `$n:atom` / `$n:lit` so `(getnum)` won't match. A *semantic* constant (a
    `defconstant`ed symbol) is out of scope — not syntactically decidable.
  - Role is **match filter only**; a class does not select among templates
    (conditional rewrite is future — write two constrained patterns instead). A
    class may annotate the wildcard (`$_:list`) and a sequence (`$body:list...`,
    applied to *each* element of the run).
- A literal `$` in code (a `$`-prefixed symbol, e.g. a gensym) is escaped by a
  leading **`$$`** — one `$` is stripped, so `$$foo` → literal `$foo` and
  `$$$foo` → literal `$$foo`, in both pattern and template.

A **pattern is a single form** — it matches one sub-form anywhere in the tree, not
a sequence of consecutive top-level siblings (matching an adjacent-form run, for
extract-repeated-code, is future). A bare `$x` pattern (matches *every* form) is a
legal footgun, allowed. All the metavariable tokens above (`$name`, `$name...`,
`$name:class`, `$_`, `$$foo`) were verified to tokenize as a **single `Symbol`**
in Common Lisp, Emacs Lisp, Scheme, Clojure, Gambit, and Racket, so the sigil
syntax needs no per-dialect special-casing.

### Matching semantics

Matching walks the lispexp `Datum` tree. **Structural equality (modulo
formatting)** is: recursive comparison of `DatumKind`, **ignoring `span`/`line`**
(hence whitespace and comments), with **leaf text compared literally** — no
reader-sugar (`'x` ≢ `(quote x)`), number (`1` ≢ `1.0`), or symbol-case (CL
`FOO` ≢ `foo`) normalization. (Sugar / number / CL-case folding are flagged for
later; note `Datum`'s derived `PartialEq` is unusable here — it compares `span`.)

- **Non-linear matching**: a metavariable repeated in the pattern must bind
  **structurally-equal** forms at each occurrence — the safety net that makes
  `(+ $n $n)` match only when both are equal. The wildcard `$_` is exempt (each
  `$_` is independent). A sequence metavariable may repeat too (its first binding
  fixes the run, so matching stays backtrack-free); the "one per list" cap is
  per-list.
- **Improper (dotted) lists** are matched literally, a consequence of "no
  normalization": lispexp keeps `(a . (b c))` and `(a b c)` distinct, so a
  `(a b c)` pattern does not match a dotted `(a . (b c))`. A dotted pattern
  `(a . $rest)` matches the tail as an ordinary element.
- **Search scope**: the **whole tree**, everywhere the structure appears
  (including quoted data). This is the honest "structural sed" reading and avoids
  the fuzzy code/data classification of quasiquote; a `--code-only` filter is
  future. Matches inside quoted data *do* rewrite — the reported site count + line
  numbers let the user verify.
- **Overlap**: rewrite the **outermost, non-overlapping** matches, all of them, in
  a **single pass** — no fixpoint, so a self-reproducing template cannot loop
  (`--repeat[=N]` is a future opt-in). Nested matches are reached by re-running.
- **Zero matches is success** (exit 0), count reported first (`rewrote 0
  site(s)`) — a search-replace legitimately finds nothing, and this keeps
  `rewrite` idempotent. Deliberately asymmetric with `rename`'s "missing symbol is
  an error": a `rename` `from` names a symbol *expected to exist*; a `rewrite`
  pattern names a *shape that may or may not*.

### Template expansion

- A captured metavariable expands to the **verbatim source text** of its matched
  form (preserving inner comments/formatting — the sub-form is *relocated*, not
  re-rendered). A sequence metavariable expands to the verbatim contiguous span it
  captured. When a metavariable matched non-linearly (two structurally-equal but
  textually-different forms — e.g. one carried a comment), the **first** binding's
  text is emitted.
- Output = the template text with `$name`/`$name...` tokens replaced, the rest
  emitted as written. A template is **zero or more forms**: `$body...` alone is
  guard removal, several forms is a multi-form expansion, and an **empty template
  is a deletion** (parse-safe; a removed definition surfaces as a
  `disappeared_definitions` warning, ADR-0024).
- **Reindent scope**: after splicing, the whole *enclosing top-level form* is
  reindented (native-engine dialects only; ADR-0025/0031) — like `struct edit`,
  not `line edit`. So the diff can be **wider than the rewritten span** (sibling
  indentation "fixed" too), and on a non-native dialect (Clojure, …) a multi-line
  captured form relocated to a new depth keeps its old indentation (parse-safe but
  not reflowed).
- **Comments are lost outside captured spans.** Only comments *inside* a captured
  metavariable's span survive; a comment on a literal part of the match (e.g. next
  to `$flag`, or after the last form) is dropped by the rewrite.
- **Duplication is allowed** (a metavariable used twice duplicates its text) —
  the user's responsibility, guarded by `:atom`/`:lit`.
- Rejected specs: a template metavariable not bound in the pattern; a
  sequence-vs-single arity mismatch between pattern and template; either side
  failing to parse.

### Scope & pipeline

Single-file (like `rename`/`inline`; project-wide is future, joining ADR-0032's
cross-file-atomicity problem). Standard pipeline: drift gate → match → splice
(non-overlapping) → reindent touched forms → validate-then-write (reject new
parse errors, ADR-0005) → atomic write; reports the site count + new file hash.

## Status

accepted — implemented in `src/refactor.rs` (`rewrite_in_file`), CLI `rewrite`
+ MCP `rewrite`.

## Consequences

- `rewrite` joins `src/refactor.rs` beside `rename`/`inline`, reusing the same
  span→edit + safety pipeline. The genuinely new machinery is (a) the **matcher**
  — structural-equality + metavariable binding (single, sequence, classed,
  non-linear) over `Datum` — and (b) the **template substituter** (verbatim text
  splice), plus the stdin spec grammar.
- The **structural-equality** function (span/line-ignoring recursive `DatumKind`
  compare, literal leaves) is reusable and gives ADR-0032's "structural equality
  modulo formatting" open problem a concrete, predictable definition.
- ADR-0032's member 3 becomes `rewrite`; its terminology and ship order are
  updated. The true "extract into a new function" stays a distinct, unblocked
  future member.
- New ubiquitous-language terms (rewrite, pattern, template, metavariable,
  sequence metavariable, metavariable class, structural equality) are recorded in
  `CONTEXT.md`.
- The user/agent guide + the canonical-rewrite cookbook (guard removal, `progn`
  unwrap, if→when, safe fold, delete, …) live in **`docs/rewrite.md`** — the "v1
  presets are documentation" deliverable, in lieu of built-in preset flags.
- Known deferrals, each a clean future opt-in: `--code-only`, `--repeat[=N]`,
  reader-sugar / number / CL-case folding, trailing-suffix sequence patterns,
  semantic `const` classification, built-in presets, project-wide scope, and the
  separate "extract into a new function".
