# Native indenter: spec-driven, fidelity grounded in Emacs and community formatters

The native indenter computes each line's indentation from the **indent spec** of its enclosing form's head (lispexp's `indent::{IndentSpec, IndentTable}`), following the `lisp-indent-function` model: a function call aligns its arguments, a special form (`defun`, `let`, `when`, …) indents by its spec's amount. An **unknown head defaults to function-call alignment** — the same graceful degradation Emacs's engine has used for decades.

## Fidelity is grounded in real behavior

Lisp-family indentation is, historically, whatever **Emacs** does — each dialect's conventions were shaped by its Emacs mode. So the indenter's fidelity is not invented; it is **grounded in and validated against**:

- **Indent specs harvested by lispexp** (`declare` indent / `lisp-indent-function` declarations) — the primary, in-source source of truth.
- **The Emacs indentation engine's behavior** and each dialect's **community-accepted formatter** (e.g. Clojure `cljfmt` / `zprint`, Emacs Lisp's own indent, Scheme mode, Racket `raco fmt`) — for the conventions not carried by an in-source spec.
- **Empirically measured indentation from real Emacs buffers** — a golden corpus, indented by Emacs / the community formatter, diffed against lisplens's output to validate fidelity and to drive additions to the bundled spec tables.

Custom per-project specs come from the Project profile (ADR-0004).

## Status

accepted

## Consequences

- Implementation is phased: v1 bundles the common specs and falls back to call-alignment for unknown heads; the golden corpus measures the gap and guides which specs to add next.
- The external formatter backend (ADR-0011) can double as a ground-truth oracle for building and checking the corpus.
- Faithfulness is a measurable property (corpus diff), not a claim.
