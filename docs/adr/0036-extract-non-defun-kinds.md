# `extract --kind` — non-`defun` definition kinds

## Context

ADR-0034 fixed `extract`'s new definition to a plain function (`defun`/`define`/`defn` per dialect) and named "`defsubst`/inline/private variants" a future flag. This ADR adds that flag. The recurring asks: an Emacs Lisp `defsubst` or `cl-defun`/`cl-defsubst`, a Clojure private `defn-`, a Scheme `define-inline`.

## Decision

Add an optional **kind** to `extract`: `--kind HEAD` (MCP `kind`) names the leading operator of the emitted definition. It defaults to the dialect's plain-function head, so the ADR-0034/0035 output is unchanged when the flag is absent.

```
lisplens extract <file> <anchor> <name> [param...] [--count N] [--kind HEAD]
```

`--kind` substitutes only the **head**; the definition's **shape family stays the dialect's** (that is what decides where the name, the arglist, and its bracket go):

- Emacs Lisp / Common Lisp — flat: `(HEAD NAME (params) body)` (default `defun`; e.g. `defsubst`, `cl-defun`, `cl-defsubst`).
- Scheme family — nested: `(HEAD (NAME params) body)` (default `define`; e.g. `define-inline`).
- Clojure — bracket: `(HEAD NAME [params] body)` (default `defn`; e.g. `defn-`).

Dialects with no known shape family are still refused (`--kind` does not unlock them — the bracket/nesting is unknown), exactly as ADR-0034.

### The head is not validated (ADR-0003 ceiling)

`HEAD` is taken verbatim — any symbol is accepted and placed as the operator; lisplens does not check that it names a real function-defining macro. This is the same contract as `extract`'s params and `rewrite`'s templates: **the user asserts the semantics, lisplens guarantees parse-safety**. The splice → validate-then-write pipeline still guarantees the result parses; a `--kind` naming a nonexistent macro parses but won't compile — the user's assertion to make, mirroring an unused/wrong param. An allowlist was considered and rejected: it would need per-dialect maintenance and contradicts the ceiling that governs the rest of the family.

The head is intended for **function-defining variants** that share the dialect's plain-function shape. A kind whose real syntax differs (Emacs `cl-defmethod` with specializers, Scheme `define-values`) would get the plain shape and so the wrong output — out of the intended use, not separately guarded.

## Status

accepted — implemented in `src/refactor.rs`: `def_form` gains a `kind` override over a `def_shape` (head + shape family) per dialect; `extract_block_into_function` threads it; `extract_into_function` stays the `count = 1`, `kind = None` wrapper.

## Consequences

- Small, contained: only the wrapper head varies; body wrapping, the run, placement, reindent, and validation are unchanged from ADR-0034/0035.
- `def_shape(dialect) -> Option<(default_head, DefShape)>` centralizes the three shape families; adding a dialect or default is one arm.
- Still deferred (ADR-0034/0035 list, minus block + kinds): free-variable inference, multi-site extraction, non-default placement, and the distinct fold-repeats-into-a-loop transform.
