# lisplens-parinfer.el

A native Emacs front-end for lisplens's parinfer command — Emacs Lisp, Common
Lisp, Scheme, Clojure, Racket, Fennel, Janet, Hy, LFE, Phel.

It is **not** a fork of `parinfer-rust-mode` and keeps no compatibility with it.
`parinfer-rust-mode` loads an in-process dynamic module; lisplens is a CLI, so
this package talks to **one long-lived `lisplens parinfer --server` process**
(a line-delimited JSON protocol) shared across every buffer.

## Requirements

- Emacs 29.1+ (native JSON, `replace-region-contents`, `string-search`, `defvar-keymap`).
- The `lisplens` executable on `exec-path` (`cargo install lisplens`, or point
  `lisplens-parinfer-executable` at a build).

## Install

Put `lisplens-parinfer.el` on your `load-path`, then:

```elisp
(require 'lisplens-parinfer)
;; enable live parinfer in Lisp buffers
(add-hook 'emacs-lisp-mode-hook #'lisplens-parinfer-mode)
```

## Live mode

`lisplens-parinfer-mode` is a **live** minor mode: while on, every edit is
reflowed by `lisplens-parinfer-live-mode` (`indent` by default) after a short
idle delay, so parens and indentation stay in sync as you type — you stop typing
close-parens and let indentation drive them. The server's cursor-line protection
keeps the paren trail on point's line from collapsing under you, and mid-edit
unbalanced input is left silently untouched (no echo-area spam).

Indent is the sensible live mode because it handles the unbalanced input that is
normal mid-edit; `paren` requires balanced parens and so refuses (does nothing)
while you type — it is offered only for completeness. Live transforms run over
the whole buffer; on very large buffers you may prefer to run the explicit
commands instead (scoping each fire to the enclosing form is a possible future
refinement).

Relevant options: `lisplens-parinfer-live-mode`, `lisplens-parinfer-idle-delay`.

## Commands

- `M-x lisplens-parinfer-paren` (`C-c C-p p`) — parens are the source of truth;
  indentation is corrected (requires balanced input).
- `M-x lisplens-parinfer-indent` (`C-c C-p i`) — indentation is the source of
  truth; close-parens are inferred from it.
- `M-x lisplens-parinfer-restart` — drop the shared server process (the next
  command starts a fresh one).

Each command transforms the active region (widened to whole lines) when one is
active, otherwise the whole (narrowed) buffer, preserving markers and point.
Point is restored from the server's reported cursor. On a refusal (unbalanced
input, an unterminated string, …) the buffer is left **untouched** and the
diagnostic is echoed.

These commands work whether or not the live minor mode is on.

## Configuration

- `lisplens-parinfer-executable` — the executable (default `"lisplens"`).
- `lisplens-parinfer-dialect-alist` — major mode → dialect name; the dialect is
  sent so the server indents per language. Unmapped modes fall back to Emacs Lisp.
- `lisplens-parinfer-nameless` — `auto` (follow `nameless-mode`), `t`, or `nil`;
  when on for Emacs Lisp, indentation is read/produced in Nameless's displayed
  columns.
- `lisplens-parinfer-live-mode` — which mode fires live (`indent` or `paren`).
- `lisplens-parinfer-idle-delay` — idle seconds before a live transform runs.
- `lisplens-parinfer-timeout` — seconds to wait for a server answer.

## Verification

Byte-compiles clean on Emacs 32. Smoke-tested end to end against the real server:
indent mode infers the missing close-parens, paren mode faithfully reindents a
balanced buffer, an unbalanced buffer is left unchanged, point survives a
reindent, and the one shared process is reused across buffers. Live mode: typing
an open paren auto-closes it after the idle delay, a mid-edit unterminated string
is left untouched with no echo-area noise, and disabling the mode removes the
hook and stops the shared process when no buffer uses it.
