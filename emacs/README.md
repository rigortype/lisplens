# lisplens-parinfer.el

A native Emacs front-end for lisplens's parinfer command — Emacs Lisp, Common
Lisp, Scheme, Clojure, Racket, Fennel, Janet, Hy, LFE, Phel.

It is **not** a fork of `parinfer-rust-mode` and keeps no compatibility with it.
`parinfer-rust-mode` loads an in-process dynamic module; lisplens is a CLI, so
this package talks to **one long-lived `lisplens parinfer --server` process**
(a line-delimited JSON protocol) shared across every buffer.

## Requirements

- Emacs 27.1+ (native JSON, `replace-region-contents`).
- The `lisplens` executable on `exec-path` (`cargo install lisplens`, or point
  `lisplens-parinfer-executable` at a build).

## Install

Put `lisplens-parinfer.el` on your `load-path`, then:

```elisp
(require 'lisplens-parinfer)
;; optional: enable the keymap in Lisp buffers
(add-hook 'emacs-lisp-mode-hook #'lisplens-parinfer-mode)
```

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

Firing on every edit — the live parinfer experience — is a separate layer built
on top of this one; here the transforms are explicit commands.

## Configuration

- `lisplens-parinfer-executable` — the executable (default `"lisplens"`).
- `lisplens-parinfer-dialect-alist` — major mode → dialect name; the dialect is
  sent so the server indents per language. Unmapped modes fall back to Emacs Lisp.
- `lisplens-parinfer-nameless` — `auto` (follow `nameless-mode`), `t`, or `nil`;
  when on for Emacs Lisp, indentation is read/produced in Nameless's displayed
  columns.
- `lisplens-parinfer-timeout` — seconds to wait for a server answer.

## Verification

Byte-compiles clean on Emacs 32. Smoke-tested end to end against the real server:
indent mode infers the missing close-parens, paren mode faithfully reindents a
balanced buffer, an unbalanced buffer is left unchanged, point survives a
reindent, and the one shared process is reused across buffers.
