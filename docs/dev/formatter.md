# Formatter (Emacs Lisp)

How the native indenter works and how to keep it faithful. Decisions: ADR-0011,
ADR-0025–0028. Config: ADR-0029.

`format::format_elisp(source, &FormatConfig) -> String` is a Rust port of Emacs's
`calculate-lisp-indent` / `lisp-indent-function`
(`~/local/src/emacs/lisp/emacs-lisp/lisp-mode.el`).

## Model

Per line, find the innermost containing list and indent by its head symbol's
indent spec:

- **`Number(n)`** (specform): the first `n` args are *distinguished* — the 1st/2nd
  land at `open_col + 4` (2×`lisp-body-indent`), a 3rd+ distinguished aligns like
  a call; args past `n` are *body* at `open_col + 2`.
- **`Defun`**: body at `open_col + 2`.
- **no spec / unknown head / named indent fn** (can't run): function-call
  alignment — under the first argument if it's on the open-paren's line, else
  `open_col + 1`; if the head is itself a list, under that first element.

Multi-line strings are left untouched. Reindentation only rewrites leading
whitespace, so it can never change what the file parses to (it is always safe).

### The key invariant — do not regress

Columns are computed against the **already-reindented earlier lines** (the `Cols`
struct holds each line's original and new indent). An alignment target always
sits on its container's open line, which is processed before any line inside it,
so its new column is known. Using original columns instead breaks nested reflow
(deep forms shift). This fix took the fidelity from partial to byte-exact on
nested code.

## Indent specs

Standard specs are **bundled** in `NUMBER_SPECS` / `DEFUN_SPECS` in
`src/format.rs` (326 entries). File-local `(declare (indent …))` and
`(put 'sym 'lisp-indent-function …)` are layered on via lispexp
`harvest_indent_specs`. Rendering uses `FormatConfig` (spaces, or tabs +
trailing spaces).

### Regenerating the bundled table (Emacs is the source of truth)

`emacs -Q --batch --load dump.el`:

```elisp
;; dump.el
(require 'cl-lib)(require 'cl-macs)(require 'pcase)(require 'subr-x)
(require 'seq)(require 'let-alist)(require 'rx)(require 'map)(require 'gv)(require 'cl-generic)
(mapatoms (lambda (s)
  (let ((v (and (or (fboundp s) (macrop s))
                (function-get s 'lisp-indent-function 'macro))))
    (when (or (integerp v) (eq v 'defun))
      (princ (format "%s %s\n" (symbol-name s) (if (eq v 'defun) "defun" v)))))))
```

Note the **`'macro`** third argument to `function-get` — without it, macro
`(declare (indent …))` specs read as nil (this was an early bug: `cl-defun`,
`pcase`, etc. all came back nil). Filter to Rust-safe identifier names; all 326
were integer/defun (zero function specs), so the whole set is usable.

## Fidelity harness (the main tool for first release)

Emacs binary: `/Applications/Emacs.app/Contents/MacOS/Emacs`. For each file:
strip indentation, format with lisplens, diff against Emacs `indent-region`.

```sh
REQ="(progn (require 'cl-lib)(require 'cl-macs)(require 'pcase)(require 'subr-x)(require 'seq)(require 'let-alist)(require 'rx)(require 'map)"
sed 's/^[[:space:]]*//' FILE.el > mine.el; cp mine.el em.el
cargo run -q -- format mine.el
/Applications/Emacs.app/Contents/MacOS/Emacs -Q --batch --eval "$REQ (find-file \"em.el\") (emacs-lisp-mode) (setq indent-tabs-mode nil) (indent-region (point-min) (point-max)) (write-file \"em.el\"))"
diff mine.el em.el
```

- `$REQ` deliberately leaves the `(progn …` **open** so the `--eval` string
  closes it; a stray closing paren silently skips the reformat and leaves `em.el`
  flat (a garbage comparison). Watch for this.
- Set `indent-tabs-mode nil` so Emacs emits spaces (its default is tabs).
- Load the same libraries the bundled table was captured with, else Emacs uses
  nil specs for cl-*/pcase/… and the comparison is unfair.
- Corpora: `../lispexp/tests/corpus/{magit,lem}/…`, and random samples from
  `~/local/src/emacs/lisp/`.

Format **unit tests** use Emacs-captured golden output, so they stay
environment-independent — the harness above is for manual/CI fidelity checks,
not `cargo test`.

## Config resolution (ADR-0029)

`config::resolve(path, source) -> FormatConfig{indent_tabs, tab_width}`.
Precedence (high→low): file-local (`-*-` header + `Local Variables:` footer) >
`.dir-locals-2.el` > `.dir-locals.el` (up the tree, nearer wins) >
`.editorconfig` (up to `root=true`, glob-matched) > defaults (spaces,
tab-width 8). dir-locals are parsed with lispexp; the EditorConfig glob supports
`*` `**` `?` `[set]` `{alt}`.

## Known fidelity gaps

Nested specforms where Emacs's `(COLUMN . start)` list-return semantics differ
from the plain column (e.g. a long `if-let` condition), and package-local macros
not in the bundled/harvested specs (e.g. sgml-mode's own, ob-ruby). Close them
one at a time with the harness. Other dialects, touched-region auto-format, and
`lisp-body-indent`/`indent_size` overrides are future work.
