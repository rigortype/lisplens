# `extract` — pull a form into a new function

## Context

ADR-0032 listed a family of refactoring procedures; ADR-0033 built `rewrite` (the
structural pattern→template engine, formerly called `extract`) and **reserved the
name `extract` for the genuinely different "extract a selection into a *new*
function"** — the one unbuilt member. This ADR designs it (grilling session).

The tension is the **semantic ceiling** (ADR-0003: lisplens is syntactic and never
evaluates). "Extract function" in IDEs earns its keep by **inferring parameters** —
the free *local* variables of the selection. But distinguishing a free local
(→ a parameter) from a global function/variable reference needs scope analysis
over binding forms (`let`/`lambda`/`defun` args …), which is dialect- and
macro-dependent and pushes past the ceiling.

## Decision

**The user supplies the new name and the parameter list; lisplens does the
mechanical, parse-safe extraction — no free-variable inference (v1).** This keeps
the ceiling (like `rewrite`, the user asserts, lisplens guarantees safety) and
fits the agent context: an agent reading the code already knows the free variables
and can pass them, so inference would only add a propose→confirm round-trip.

Because the selection already uses its free variables by name and the call site is
in their scope, extraction is a **pure cut + wrap** — no symbol substitution:

- Build `(defun NAME (PARAMS) <selection-text-verbatim>)`.
- Replace the selection with `(NAME PARAMS)` — the call passes the same symbols.

### Surface

```
lisplens extract <file> <anchor> <name> [param...]
```
MCP `extract` tool: `{ file, anchor, name, params: [..] }`.

- `<anchor>` = `line:hash[:ordinal]` (ADR-0018) — a **single form** (the selection).
  A contiguous sibling *run* (block extraction, `anchor + count`) is the top
  follow-up; multi-site "extract a repeated expression into a shared function"
  (rewrite + def-creation) is further future.
- `<name>` = the new function's name. `[param...]` = the parameter symbols, which
  are **both the arglist and the call arguments** (identical — passing a param a
  different expression, i.e. partial specialization, is out of scope).
- No params → a niladic extraction: `(defun NAME () BODY)` + `(NAME)`.

### Placement, def form, kind

- **Placement**: the new definition goes **immediately before the enclosing
  top-level form** (helper near use), separated by a blank line. (End-of-file /
  user-chosen placement is future.)
- **Def form** (per dialect — only the wrapper is dialect-specific, the body is the
  verbatim selection):
  - Emacs Lisp / Common Lisp → `(defun NAME (p …) BODY)`
  - Scheme family → `(define (NAME p …) BODY)`
  - Clojure → `(defn NAME [p …] BODY)`
  - Any other dialect → **error** (`extract not supported for <dialect> yet`) rather
    than emit wrong syntax. The set extends as dialects gain a known def form.
- **Kind**: a plain function. `defsubst`/inline/private variants are a future flag.

### Safety

Parse-safe only (validate-then-write, ADR-0005), like every procedure. Extraction
is **behaviour-preserving only if** the user's params cover the free locals *and*
the selection has no context-dependent **non-local exit** (`return`/`throw`/
`cl-return`/`recur`, `&body` capture, a self-recursive call, a lexical closure over
more than the params). lisplens does not judge these — the user asserts them. An
unused param is not an error. The touched forms (the new def and the call site's
enclosing form) are reindented on native-engine dialects (ADR-0031); Clojure has
no native reindent engine yet, so its extracted def is correct but not reflowed.

## Status

accepted — implemented in `src/refactor.rs` (`extract_into_function`), CLI
`extract` + MCP `extract`.

## Consequences

- `extract_into_function` joins `src/refactor.rs`: resolve the anchor
  (`resolve::resolve`), find the enclosing top-level form, and emit two edits — a
  zero-width def insertion at the enclosing form's start and a replacement of the
  selection with the call — through the usual splice → reindent → validate → write
  pipeline. Disjoint edits (the insert precedes the selection), so `splice_tracked`
  handles them directly.
- Because it is a pure cut + wrap, the engine is small: no matcher, no
  substitution — just text assembly plus anchor resolution. The value is
  collapsing the hand-done cut / write-defun / replace-with-call / reindent /
  parse-check into one safe step.
- This completes the ADR-0032 member list (`check`, `rename`, `inline`, `rewrite`,
  `extract`). Free-variable inference, block (`anchor + count`) extraction,
  multi-site extraction, non-`defun` kinds, and non-default placement are the
  documented future opt-ins.
