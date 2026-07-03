# `extract --count` — block (anchor + run) extraction

## Context

ADR-0034 built `extract` (pull *one* form at an anchor into a new function) and
named **block extraction — a contiguous sibling *run*, `anchor + count`** as its
top follow-up. This ADR adds it.

The IDE analogue is "Extract Method" over a multi-line *selection*: several
adjacent statements in a body that form a logical unit, pulled into a named helper
called once in their place. In Lisp that selection is a run of **contiguous sibling
forms** — same parent, adjacent in source. The forms need not be identical
(that would be a different transform — *folding* repeats into a loop, e.g.
`(foo)(foo)(foo)` → `(dotimes (_ 3) (foo))`, which requires recognizing repetition
and synthesizing iteration; **out of scope**, deferred until a real need appears).

## Decision

Add an optional **count** to `extract`: extract the run of `count` contiguous
sibling forms starting at the anchored form. `count` defaults to `1` — the ADR-0034
single-form path, unchanged.

```
lisplens extract <file> <anchor> <name> [param...] [--count N]
```
MCP `extract`: optional `count` (integer, default `1`).

Still a **pure cut + wrap**, per ADR-0034 (no free-variable inference, no symbol
substitution — the user asserts the parameters, lisplens guarantees parse-safety):

- Build `(defun NAME (PARAMS) form₁ … form_N)` — the run wrapped verbatim (interior
  whitespace and comments preserved by taking the source slice from the first
  form's start to the last form's end). A **multi-line body** (any run of two-plus
  forms, or a multi-line single form) is placed on its own line after the arglist so
  reindent lays it out as a conventional body; a single-line body stays inline (the
  ADR-0034 one-liner is unchanged).
- Replace the whole run with `(NAME PARAMS)`.

### The run

Resolved from the anchored form's **parent + index** (`resolve::Located`): the
siblings are the parent list's items, or the top-level forms when the anchor is
top-level. The run is `siblings[index .. index + count]`.

- `count < 1` → error.
- `index + count > siblings.len()` (the run would cross the sibling group's end) →
  error (`RunExceedsSiblings`); no partial write.
- `count = 1` reduces to exactly ADR-0034: the run span equals the single node's
  span, so the single-form path is preserved by construction.

### Value semantics

A function body is an implicit `progn`: `(NAME PARAMS)` evaluates to the **last**
form's value, the earlier forms for effect. So a block extraction is
behaviour-preserving **only in a body / `progn` position** (a `defun`/`let`/`when`
body, a top-level effect sequence) — where only the last form's value is used and
the rest run for side effects. As with ADR-0034, lisplens does not judge this; the
user asserts it. Same non-local-exit caveats apply (`return`/`throw`/`recur`/`&body`
capture / lexical closure beyond the params).

### Placement, def form, safety

Unchanged from ADR-0034. The new def goes immediately before the enclosing
top-level form (which, for a top-level run, is the run itself → before its first
form; for a nested run, the top-level form containing it). Per-dialect wrapper
(`defun`/`define`/`defn`; others error). Splice → reindent touched forms on
native-engine dialects → validate-then-write (ADR-0005).

## Status

accepted — implemented in `src/refactor.rs`: `extract_into_function` becomes a thin
`count = 1` wrapper over a new `extract_block_into_function`.

## Consequences

- The core generalizes cheaply: only the selection span changes (one node → a
  run's first-start .. last-end); the def-assembly, call, enclosing-form lookup,
  and pipeline are shared with ADR-0034. The enclosing-form finder already handles
  both cases — a multi-form top-level run is contained by no single top-level datum,
  so it falls back to inserting before the run.
- `ExtractError` gains `RunExceedsSiblings`.
- Still deferred (ADR-0034's list, minus block): free-variable inference,
  multi-site extraction, non-`defun` kinds, non-default placement — plus
  *fold-repeats-into-a-loop* (a distinct procedure, noted above).
