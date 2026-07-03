# Multi-dialect indenter: one driver, an engine per bundled Emacs indenter

The native indenter (ADR-0011/0026) shipped Emacs Lisp only. Extending it to the
other Lisp dialects lisplens edits raised the question of *how many indenters*
there really are. Emacs — our fidelity oracle — bundles exactly **three**
distinct Lisp indentation engines, not one per language:

- `lisp-indent-function` / `calculate-lisp-indent` (`lisp-mode.el`) — Emacs Lisp,
  and the base `lisp-mode` fallback.
- `common-lisp-indent-function` (`cl-indent.el`) — Common Lisp's richer style.
- `scheme-indent-function` (`scheme.el`) — the Scheme family.

Third-party dialects (Clojure, Fennel, Janet, Hy, LFE, …) have **no** indenter
bundled in Emacs, so there is no in-tree oracle for them.

## Decision

Structure the formatter as **one shared driver + a dialect-selected engine**.
The driver (`src/format/mod.rs`) owns everything dialect-independent: the per-line
loop, string/comment rules, touched-region masking (ADR-0025/0028), `Cols`
column arithmetic (reindent-aware, ADR-0026), and rendering (ADR-0029). Per code
line it asks an **engine** for the indent column.

- `Engine::Elisp` — the existing `lisp-indent-function` port. Also the **generic
  fallback** for every dialect without a dedicated engine.
- `Engine::CommonLisp` — a faithful port of `common-lisp-indent-function`
  (`src/format/commonlisp.rs`): multi-level backtracking, a `path`, the
  `lisp-indent-259` spec walker (nil / int / `&lambda` / `&rest` / `&body` /
  `&whole` / destructuring sublists / named methods), the bundled standard CL
  table, and the special methods (`tagbody`, `do`, `defmethod`, the lambda hack,
  simple/extended `loop`, package-prefix stripping).

`engine_for(dialect)` maps `CommonLisp` → the CL engine and everything else →
Elisp. The Scheme family's `scheme-indent-function` engine is future work; those
dialects ride the generic fallback until it lands.

**Auto-format on edit is gated to dialects with a *faithful* engine**
(`has_native_engine`: Emacs Lisp and Common Lisp today). The generic fallback is
fine for an explicit `format` (the user opted in) but is **not** trusted to
silently reflow the touched region of a dialect it does not model (e.g. Clojure),
which would corrupt idiomatic indentation. Line-hash edits stay literal
(ADR-0027) regardless.

## Status

accepted

## Consequences

- The public surface gains `format(source, config, dialect)` and threads
  `dialect` through `reindent*`; `format_elisp*` stay as Emacs Lisp shims (Nameless
  is Emacs Lisp-only, ADR-0030). `format` now works for any recognised dialect
  instead of erroring off Emacs Lisp.
- The Common Lisp engine reuses the driver's `calculate-lisp-indent` model for
  `normal-indent`, but needs the *full* three-case computation (including "align
  under the previous sibling on its own line"), which the Emacs Lisp engine never
  needed — Common Lisp reaches it through `&body`/`&rest`. It lives in the CL
  engine as `cl_normal_indent`.
- Fidelity is validated per engine against its Emacs mode (CL: `lisp-mode` with
  `lisp-indent-function` = `common-lisp-indent-function`). Unit tests use
  Emacs-captured golden output, so they stay environment-independent. See
  `docs/dev/formatter.md` for the per-engine harness and the known
  `lisp-indent-defmethod` flat-input caveat.
- Adding the Scheme engine is a localised change: a new `Engine` variant, a
  `src/format/scheme.rs`, and one arm in `engine_for` / `has_native_engine`. The
  driver and the other engines are untouched.
- Bundled-table provenance is unchanged (lispexp ADR-0033 owns the Emacs Lisp
  table); the CL standard table is carried in the CL engine, harvested from
  `cl-indent.el`.
