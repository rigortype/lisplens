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
  a call; args past `n` are *body*. Body forms align under the **first** body
  form; only when that first body form itself begins the line (nothing body
  before it) do they land at `open_col + 2`. So `(progn (a)` puts a later `(b)`
  under `(a)`, not at `open_col + 2`.
- **`Defun`**: body at `open_col + 2`.
- **no spec / unknown head / named indent fn** (can't run): alignment depends on
  the head. A **symbol-like head** (`lisp-indent-function`'s `\sw\|\s_` test:
  symbol, keyword, number, char, or a reader-prefixed form wrapping one — `,sym`)
  is a *function call*: align under the first argument if it's on the open-paren's
  line, else under the head. A **non-symbol head** (string, list, or a prefix
  wrapping one — `'(…)`, `,(…)`) is *data*: align every element under the first
  one. Whitespace right after the open paren (`( a b`) also forces
  align-under-first (Emacs's `whitespace-after-open-paren`), which is what indents
  a dotted tail's `. ,x` line. When no element is completed on an earlier line,
  indent `open_col + 1`.

A dotted-tail sublist — `(a . (b c))`, which Emacs reads as `(a b c)` — opens its
own containing sexp; `container_at` descends into the tail so its elements indent
against it. For a lone-car dotted pair (`'(eval . FORM)`) Emacs instead treats
the `.` itself as the first argument, so the tail's continuation aligns under the
dot — reached via lispexp's `Datum::dot_span` (0.5). Comment-only lines follow
Emacs's three-way rule: `;;;` (3+) is never reindented (left in place, like a
multi-line string), a lone `;` goes to `comment-column` (`indent-for-comment`,
default 40, always — independent of nesting or prior column), and `;;` indents as
code. Reindentation only rewrites leading whitespace, so it can never change what
the file parses to (it is always safe).

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

## Nameless-aware indentation (ADR-0030)

Some elisp (e.g. `php-mode`'s `lisp/`) is indented as it *displays* under
[Nameless](https://github.com/Malabarba/Nameless), which composes a package's
namespace prefix to a shorter glyph so Emacs measures alignment against the
displayed width. Opt in with `format --nameless FILE` (off by default — the
other corpora are not Nameless). When on, `format_elisp_nameless` builds a
`Nameless` (in `src/nameless.rs`) from the file name and the default
`nameless-global-aliases`; `Cols::col` then subtracts, from every column it
measures, the width saved by composed prefixes beginning earlier on the line.

- **Current name** is discovered from the file name the way Nameless does
  (`php-mode.el` → `php`); `php-foo` composes `php-` (4) → `:` (1), saving 3.
- **Aliases** default to `fl` → `font-lock`; `font-lock-` (10) → `fl:` (2),
  saving 8. The composed width is `⌊len(display+":")/2⌋ + 1` — Nameless's
  `(Br . Bl)` composition packs glyphs to ~half width (verified against Emacs).
- **`nameless-private-prefix` is not modelled**: it is width-neutral (the extra
  separator char is matched by an extra glyph), affecting only the shown glyph.

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
- For a Nameless corpus (`~/repo/emacs/php-mode/lisp`), format with
  `lisplens format --nameless` and enable Nameless on the Emacs side:
  `-l nameless.el … (nameless-mode 1) (font-lock-ensure)` **before**
  `indent-region`. The forced `font-lock-ensure` is essential — batch redisplay
  never applies the composition, so without it Nameless changes nothing and the
  comparison is against plain indentation. Keep the buffer name `*.el` so
  `nameless-current-name` auto-discovers. On `php-mode`'s five source files all
  seven Nameless-affected lines match Emacs; the residual diffs are the same
  spec-driven long tail as the non-Nameless corpora.

Format **unit tests** use Emacs-captured golden output, so they stay
environment-independent — the harness above is for manual/CI fidelity checks,
not `cargo test`.

## Config resolution (ADR-0029)

`config::resolve(path, source) -> FormatConfig{indent_tabs, tab_width, body_indent, comment_column}`.
Precedence (high→low): file-local (`-*-` header + `Local Variables:` footer) >
`.dir-locals-2.el` > `.dir-locals.el` (up the tree, nearer wins) >
`.editorconfig` (up to `root=true`, glob-matched) > defaults (spaces,
tab-width 8, `lisp-body-indent` 2, `comment-column` 40). `body_indent` is Emacs's
`lisp-body-indent` — the width of one structural step (`open_col + body`, and
`2×body` for a specform's 1st/2nd distinguished args); EditorConfig `indent_size`
maps to it. `comment_column` is Emacs's `comment-column` (lone-`;` alignment).
dir-locals are parsed with lispexp; the EditorConfig glob supports
`*` `**` `?` `[set]` `{alt}`.

## Known fidelity gaps

Nested specforms where Emacs's `(COLUMN . start)` list-return semantics differ
from the plain column (e.g. a long `if-let` condition); package-local macros not
in the bundled/harvested specs (e.g. sgml-mode's own, ob-ruby); and some `rx`
forms led by a char literal (`(? …)` = the space char `?\s`), whose sub-form
alignment is still off by a column or two. Close them one at a time with the
harness. Note the harness's Emacs side can't see a file's own `(declare (indent
…))` (it doesn't evaluate the file), so a file that indents by its own macros
will show harness diffs where lisplens is in fact right — cross-check against the
original. Other dialects and touched-region auto-format are future work.
