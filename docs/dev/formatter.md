# Formatter

How the native indenter works and how to keep it faithful. Decisions: ADR-0011,
ADR-0025–0028, **ADR-0031** (multi-dialect dispatch). Config: ADR-0029.

The formatter is **one shared driver + a dialect-selected engine** (ADR-0031).
`format::format(source, &FormatConfig, dialect) -> String` picks the engine via
`engine_for`; `reindent*` thread the same `dialect`. `format_elisp*` remain as
Emacs Lisp shims (Nameless is Emacs Lisp-only, ADR-0030). Emacs bundles three
distinct Lisp indenters and each is one engine here:

| engine | Emacs source | dialects |
| --- | --- | --- |
| `Engine::Elisp` | `lisp-mode.el` `lisp-indent-function` | Emacs Lisp + **generic fallback** for the rest |
| `Engine::CommonLisp` | `cl-indent.el` `common-lisp-indent-function` | Common Lisp |
| *(Scheme — future)* | `scheme.el` `scheme-indent-function` | Scheme family |

The driver (`src/format/mod.rs`) owns the per-line loop, string/comment rules,
touched-region masking, `Cols` column arithmetic, and rendering; each engine only
answers "what column does this code line indent to?". `has_native_engine`
(Emacs Lisp, Common Lisp) gates auto-format-on-edit — the generic fallback formats
only on an explicit `format` (ADR-0031).

## Emacs Lisp engine

`format_elisp` is a Rust port of Emacs's `calculate-lisp-indent` /
`lisp-indent-function` (`~/local/src/emacs/lisp/emacs-lisp/lisp-mode.el`).

### Model

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

Standard specs are **bundled** by the companion crate
`lispexp_emacs::indent::bundled_table(Dialect::EmacsLisp)` (lispexp ADR-0033) — 342
entries (326 core + 16 from `cc-mode` et al., see below), the byte-identical
former `NUMBER_SPECS` / `DEFUN_SPECS` tables lisplens used to carry in
`src/format.rs`. The formatter starts from that table; file-local
`(declare (indent …))` and `(put 'sym 'lisp-indent-function …)` are layered on
via lispexp `harvest_indent_specs`. Rendering uses `FormatConfig` (spaces, or
tabs + trailing spaces).

### Regenerating the bundled table (Emacs is the source of truth)

The table now lives in `lispexp-emacs` (`crates/lispexp-emacs/src/indent.rs`),
so regeneration is a change to *that* crate, not lisplens — the dump procedure
below is reproduced here for reference and is identical to the one documented in
its `indent` module.

`emacs -Q --batch --load dump.el`:

```elisp
;; dump.el
(require 'cl-lib)(require 'cl-macs)(require 'pcase)(require 'subr-x)
(require 'seq)(require 'let-alist)(require 'rx)(require 'map)(require 'gv)(require 'cl-generic)
(require 'cc-mode)  ; common core package; adds c-lang-defconst etc. (dogfooded on php-mode)
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

## Common Lisp engine (ADR-0031)

`format/commonlisp.rs` is a Rust port of `common-lisp-indent-function`
(`~/local/src/emacs/lisp/emacs-lisp/cl-indent.el`). It is a *different, richer*
algorithm than the Emacs Lisp engine — worth understanding before touching it:

- **Multi-level backtracking.** Where `lisp-indent-function` looks only at the
  innermost containing list, this walks *up* to `MAX_BACKTRACKING` (3) levels
  (`backward-up-list`), building a `path` (the child index at each level,
  outermost first — `foo` is `(0 3 1)` in `((a b c (d foo) f) g)`). Backtracking
  is what reaches `flet` from inside a local-function body. `sexp_column` is fixed
  at the *innermost* list's column throughout the walk.
- **A spec language, `lisp-indent-259`.** A symbol's method is an integer, `defun`,
  a named function, or a list of `nil` / integer / `&lambda` / `&rest` / `&body` /
  `&whole` / destructuring sublists / function symbols. The walker consumes `path`
  and `method` together; a destructuring sublist is `(&whole X . submethod)` and is
  entered by skipping `&whole X` (`cddr`). The standard table is bundled in
  `method_for` (harvested from `cl-indent.el`'s alist, aliases resolved, `defun`
  expanded to `(4 &lambda &body)`).
- **`normal-indent` needs all three `calculate-lisp-indent` cases** — including
  "align under the previous sibling on its own line," which the Emacs Lisp engine
  never needed (it computes body alignment explicitly). Common Lisp reaches it via
  `&body`/`&rest`, so `cl_normal_indent` implements the full computation.
- **Named methods**: `tagbody`, `do`, `defmethod` (counts qualifiers), the
  `lambda`/`function` hack; plus the `loop` special-case (simple vs extended) and
  lambda-list keyword alignment (`&key` continuation at keyword + 2, Emacs's
  default). **Package prefixes are stripped** as a fallback (`cl:defconstant` →
  `defconstant`, the "pleblisp" feature). Backquote data (`'(…)`) vs code, and
  `,`/`,@`/`#(` reader prefixes, follow `cl-indent.el`'s per-level char checks.

Known gaps (close against the oracle): `lisp-indent-backquote-substitution` fine
cases in deeply backquoted macros, and the `#'function` column-shaving variant of
the lambda hack.

## Fidelity harness (the main tool for first release)

Emacs binary: `/Applications/Emacs.app/Contents/MacOS/Emacs`. For each file:
strip indentation, format with lisplens, diff against Emacs `indent-region`. The
recipe below is the **Emacs Lisp** engine; the **Common Lisp** engine uses the
same shape with `lisp-mode` and `common-lisp-indent-function`:

```sh
emacs -Q --batch --eval "(progn (require 'cl-indent) (find-file \"em.lisp\") \
  (lisp-mode) (setq indent-tabs-mode nil) \
  (setq lisp-indent-function 'common-lisp-indent-function) \
  (indent-region (point-min) (point-max)) (write-file \"em.lisp\"))"
```

**Caveat — `defmethod` under a flat harness.** `lisp-indent-defmethod` counts
method qualifiers via `beginning-of-defun` + `forward-sexp`, which mis-scans when
*every* line has been de-indented to column 0 (it treats each `(` at column 0 as a
defun start). So the harness's Emacs side reports the wrong body column for
`defmethod` forms on stripped input — where lisplens is in fact right (it matches a
real Emacs on the properly-structured file). Cross-check `defmethod` diffs against
the original, or with a fixed-point test on the real file. On `cl-ppcre` + the
`gpg`/`gpgme` CL sources every residual flat-harness diff is either this caveat, a
trailing newline, or the two known-gap cases above.

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
original. The **Common Lisp** engine is landed (ADR-0031, see above); the
**Scheme family** (`scheme-indent-function`) is the next engine, and the
remaining dialects ride the generic Emacs Lisp fallback until they get one.

## Touched-region reindent (ADR-0025/0028)

`format::reindent_range(source, config, ranges)` reindents only the top-level
forms overlapping any byte `range`, leaving every other line byte-identical — the
whole-file loop with a per-line "is this line in a touched form?" gate
(`touched_line_mask`). Each touched form is reindented in full so its internal
alignment stays self-consistent (a form's alignment targets are all within it).
`apply_struct_patch` calls it after `edit::splice_tracked` (which returns each
edit's post-splice byte span, in caller order), gated to Emacs Lisp; the returned
file-hash is of the reindented content. Line-hash edits stay literal (ADR-0027).

Two scopes share one engine, via `Touched { expand, exact }`: a content edit's
span is an `expand` range (pull in the whole enclosing top-level form), while the
`format <anchor>` op reindents `exact`ly the anchored form — possibly nested, in
full context (`reindent_block`). The `format` op is carried as an *identity edit*
(replace the node with its own bytes) so `splice_tracked` hands back its
post-splice span for free and any conflict with another op is caught by splice.

**Auto-format is Nameless-aware when configured.** `reindent` takes an
`Option<&Nameless>`; `apply_struct_patch` builds one (per-file, from the file
name) when `config.nameless` resolves true — a `nameless-mode` file-/dir-local
(ADR-0029/0030) — and passes it through, so Structural-editing a
Nameless-indented file (e.g. `php-mode/lisp` with
`((emacs-lisp-mode (nameless-mode . t)))`) keeps its composed-prefix alignment
instead of reflowing to non-Nameless columns. Without that signal the edit path
is plain (correct for the non-Nameless corpora). Surfaced dogfooding php-mode —
see `docs/notes/20260703-dogfooding-php-mode.md`.
