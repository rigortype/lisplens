# Formatter

How the native indenter works and how to keep it faithful. Decisions: ADR-0011,
ADR-0025–0028, **ADR-0031** (multi-dialect dispatch). Config: ADR-0029.

The formatter is **one shared driver + a dialect-selected engine** (ADR-0031).
`format::format(source, &FormatConfig, dialect) -> String` picks the engine via
`engine_for`; `reindent*` thread the same `dialect`. `format_elisp*` remain as
Emacs Lisp shims (Nameless is Emacs Lisp-only, ADR-0030). Three engines port an
Emacs indenter; a fourth (Clojure) ports cljfmt:

| engine | oracle / source | dialects |
| --- | --- | --- |
| `Engine::Elisp` | `lisp-mode.el` `lisp-indent-function` | Emacs Lisp + **generic fallback** for the rest |
| `Engine::CommonLisp` | `cl-indent.el` `common-lisp-indent-function` | Common Lisp |
| `Engine::Scheme` | `scheme.el` `scheme-indent-function` | Scheme, Guile, Racket, Gauche, Mosh, Gambit, superset |
| `Engine::Clojure` | **cljfmt** `:inner`/`:block` (ADR-0039); **`phel format`** for Phel (ADR-0041) | Clojure, Phel |

The driver (`src/format/mod.rs`) owns the per-line loop, string/comment rules,
touched-region masking, `Cols` column arithmetic, and rendering; each engine only
answers "what column does this code line indent to?". `has_native_engine`
(Emacs Lisp, Common Lisp, the Scheme family, Clojure, and Phel) gates
auto-format-on-edit — the generic fallback formats only on an explicit `format`
(ADR-0031). `Engine::Clojure` serves both Clojure and **Phel** (a Clojure-inspired
Lisp compiling to PHP): same `:inner`/`:block` algorithm, a per-dialect rule table
(`rules_for(name, dialect)`), and one `:block` difference — Phel body-indents a
block form's *special* args too once the body breaks, cljfmt keeps them aligned.
Phel's oracle is `phel format` (byte-exact on phel-lang's own `.phel` files). Since
lispexp 0.7.0 the Phel reader gaps (feedback 0004/0005/0006 — `;`-in-symbol, `|(…)`
short-fn, `\Foo\Bar` FQN) are all resolved; the whole corpus parses clean and the
only residual is the shared driver's closing-bracket-after-inline-comment case (a
niche one-liner, ADR-0041).

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
dot — reached via lispexp's `Datum::dot_span` (0.5). Comment-only lines are
**dialect-specific**: the Emacs-family engines (Emacs Lisp, Common Lisp, Scheme)
follow Emacs's three-way rule — `;;;` (3+) is never reindented (left in place,
like a multi-line string), a lone `;` goes to `comment-column`
(`indent-for-comment`, default 40, always — independent of nesting or prior
column), and `;;` indents as code. The **Clojure engine** (Clojure, Phel, and the
induced-table dialects Fennel/Janet/Hy/LFE/ISLisp) instead **leaves every
comment-only line exactly where it was written**, matching `cljfmt` and
`phel format` (verified byte-exact), which never reindent a comment. Comment-only
lines are found from the lexer trivia (`lex`), which classifies each dialect's own
comment character (`;`, or Janet's `#`) and distinguishes a `#` comment from a
`#(`/`#{` dispatch (lispexp feedback 0007); a trailing comment (code before it) is
never a comment-only line. Reindentation only rewrites leading whitespace, so it
can never change what the file parses to (it is always safe).

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

## Scheme engine (ADR-0031)

`format/scheme.rs` is a Rust port of `scheme-indent-function` (`scheme.el`), used
for the whole Scheme family: Scheme, Guile, Racket, Gauche, Mosh, Gambit, and the
permissive superset (`engine_for`). Emacs's own comment says the function
"duplicates almost all of `lisp-indent-function`" — so this engine is the *Emacs
Lisp* algorithm, not the CL one, differing only in:

- **A Scheme spec table** (`method_for`), transcribed from the `(put 'sym
  'scheme-indent-function …)` block in `scheme.el` (~60 entries: `begin` 0, `case`
  1, `do` 2, `lambda` 1, `let*`/`letrec` 1, `syntax-rules` `defun`, `when`/`unless`
  1, `dynamic-wind` 3, `receive` 2, the `call-with-*`/`with-*` I/O forms, SRFI
  8/11/64/204/227/253 forms, R6RS `library`, R7RS `define-record-type` /
  `define-library` / `guard`, …). The **MIT-Scheme block** is guarded by
  `scheme-mit-dialect` (default nil) so it is *not* in default `scheme-mode` and is
  omitted. Names are matched case-sensitively (Scheme, unlike CL).
- **`syntax-rules` → `defun`**, plus Emacs's fallback: any symbol head longer than
  3 chars starting with `def` and lacking an explicit property is indented as a
  `defun` (`(string-match "\`def" …)`).
- **`scheme-let-indent`** for `let` / `match-let`: a *named* let (a symbol right
  after the head, `(let loop ((i 0)) …)`) indents as `lisp-indent-specform 2`,
  an ordinary `let` as `1`.

`normal-indent` is `scheme.rs`'s own faithful port of the full
`calculate-lisp-indent` (`scheme_normal`), because `scheme-indent-function`
returns it directly for the data path, distinguished args past the second, and
body forms — cases where the Emacs Lisp engine's partial `normal_indent` (which
that engine only needs for cases 1–2, computing body alignment explicitly) is not
enough. The unifying rule for a symbol-head call is *skip exactly the first
element, align under the second* — which covers `(f a\n b)`, `(f\n a\n b)`, and a
multi-element first line (`'#u8(\n #xff #xff\n 0 0)` aligns under the second byte).

Two shared-helper refinements the Scheme corpus forced, both Emacs-faithful and
regression-checked against the Elisp/CL golden tests:

- **`head_is_symbol_like` now inspects a char literal's glyph.** Emacs Lisp `?a`
  is symbol-like (`?` is expression-prefix syntax, so the point lands on the word
  `a`), but Scheme's `#\a` is **not** (`#` is prefix punctuation), so a
  char-literal-led list/vector indents as *data* (under its first element). The
  helper now returns true for `Char` only when the token starts with `?`.
- **`whitespace-after-open-paren` counts only a space/tab on the paren's own
  line**, not a trailing newline — so `'#u8(` at end of line then numeric bytes
  indents as a call (under the second element), not as data.

`#(…)` / `#u8(…)` vectors reach the engine because `container_at` now descends
into a `HashLiteral`'s inner list; a char-literal head (`#\x0030`) is then data
and a numeric head (`#xff`) is a call, matching Emacs.

Known gap (a *flat-harness* artifact, not an engine bug — the engine matches a
real Emacs on the properly-structured file): a non-`def`-prefixed macro whose
*source* is indented like a definition (e.g. chibi's `%define-syntax`, which some
Scheme editors treat as a macro via runtime introspection). On fully de-indented
input Emacs's own `beginning-of-defun`-based reindent can mis-scan and reproduce
the file's definition-style column, where lisplens correctly gives the
function-call column `scheme-mode` produces from a clean buffer. Cross-check such
diffs against the original file or a fixed-point reindent, exactly like the CL
`defmethod` caveat below.

One residual gap, narrow and not Scheme-specific:

- **Racket infix dots** `(a . op . b)` (two dots in one list) — the continuation
  of such a form is off; a niche reader construct outside `scheme-mode`'s own
  model.

**Multi-byte columns are handled** (shared, all engines): `Cols::col` measures the
line content up to each position by **display width** (East Asian Width, via
`unicode-width`), not UTF-8 byte length, matching Emacs's `current-column`. So a
wide/multi-byte glyph before an alignment target advances the column as Emacs
would — `漢`/`Ａ` = 2, `λ`/`☆` (ambiguous) = 1 — and `(λλλλ arg` / `(漢漢漢漢 arg`
continuations land byte-exact. ASCII is unchanged (display width == byte length).
The `Cols::col` inputs stay byte offsets (lispexp's `LineIndex` is byte-based);
only the width *measurement* of the content slice is display-aware.

## Clojure engine (ADR-0039)

`src/format/clojure.rs` — **not** an Emacs port. Emacs bundles no Clojure indenter,
so this engine targets **cljfmt** (the formatter Clojure developers run, and the
origin of the model the whole ecosystem — clojure-ts-mode, cljstyle, modern
clojure-mode — converged on). The survey is `docs/notes/20260704-clojure-indentation-survey.md`.

Two styles (ADR-0039 semantic, ADR-0040 fixed). **Semantic** (default) is the model
below. **Fixed / Tonsky** (`format --tonsky`, or a `clojure-ts-indent-style: fixed`
file-/dir-local → `FormatConfig.clojure_fixed_indent`) replaces only the
round-list-with-symbol-head branch: every symbol-headed list body indents a flat
`open + 2` (no rule table, no align-under-first-argument, threading included);
collections, reader conditionals, and non-symbol heads are identical to semantic.
Its oracle is `cljfmt fix --config` with `{:indents {#re ".*" [[:inner 0]]}}`.

The semantic model is cljfmt's `:inner`/`:block` rules, a **pure function of the
s-expression tree** (no tree-sitter, no Emacs). To indent a line inside innermost `c`
(`open` = `c`'s open-delimiter column; value children counted, so comments/`#_`/
metadata never consume a slot):

- **collections** `[]`/`{}`/`#{}` and reader conditionals `#?(…)`/`#?@(…)` → align
  under the first element;
- **round lists / `#(…)`** are the *call* model: **default** aligns continuations
  under arg 0 (or `open + 1` when the head is alone), threading `->`/`->>` included;
  **`[:inner 0]`** (`defn`, `fn`) → every direct child `open + 2`; **`[:inner D]`**
  / **`[:inner D idx]`** (`reify`, `letfn`, …) → a rule on the form `D+1` levels up
  gives `open + 2`; **`[:block N]`** → args `< N` are special (default), args `≥ N`
  are body (`open + 2`) only when the first body form begins its own line, else the
  form falls back to default. Namespaces are stripped for lookup; unknown
  `def…`/`with-…` heads hit the regex fallback (`[:inner 0]`).

The bundled table `rules_for` is cljfmt's `indents/clojure.clj` (plus the merged
`compojure.clj`/`fuzzy.clj` defaults), verbatim. `[:block N]` differs from Emacs's
integer spec: it does **not** double-indent the special args.

**Metadata and prefixed heads** (pinned on the wider corpora below):
- `^{…} form` holds its map in the `Prefixed` node's `arg`, so `container_at` and
  `in_string` descend `arg` too — the map's keys align under the first key and a
  docstring inside metadata stays untouched.
- An argument is located by its **form** start, past any `^metadata` prefix
  (`form_start`): a `^Tag` sitting on the head line while its form wraps to the next
  does not make the argument look "already completed" (so `(doto ^Tag⏎(f)⏎(g))`
  keeps `(f)` as the special arg). The *alignment column*, though, is the element's
  true start — cljfmt aligns a continuation under the `^`.
- A `^meta` **head** is transparent for rule lookup (`(^:m when …)` uses `when`'s
  `:block 1`), but a quote / var-quote / unquote head is **not** a symbol head
  (`(#'foo …)`, `('foo …)` → default alignment) — cljfmt keys rules on the bare
  symbol token, unlike the Emacs engines' transparent `backward-prefix-chars`.

Fidelity is validated against `cljfmt fix` — see the harness section. On **eight
repos** (hiccup, ring, reitit, clj-kondo, next.jdbc, malli, integrant, jsonista —
663 real `.clj/.cljs/.cljc`, excluding clj-kondo's deliberately-malformed linter
fixtures) lisplens is byte-exact with cljfmt on code-line indentation in the
**semantic** style with **zero** divergences. This includes `#_`-discarded
multi-line forms: since lispexp 0.7.0 the formatter parses with
`Options.keep_discarded`, so a discard stays in the tree (as `Prefixed { Discard }`)
and lines *inside* it indent against the discarded form — and a discard *counts* as
a value child for the `:inner`/`:block` model, matching cljfmt (feedback 0003, now
resolved). The **fixed** style matches too, with one residual off-by-one in an
obscure `(#?(…) …)` reader-conditional-headed call. The one remaining known
limitation is that comment-only line indentation is the shared driver's and may
differ from cljfmt.

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
- The **Scheme** engine uses the same shape with `scheme-mode` (no `$REQ`
  libraries needed — the Scheme table is bundled in `scheme.rs`, and `.rkt` is
  routed Racket→Scheme but validated against `scheme-mode`, the only Scheme
  indenter Emacs bundles):

  ```sh
  EM=/Applications/Emacs.app/Contents/MacOS/Emacs
  sed 's/^[[:space:]]*//' SRC.scm > mine.scm; cp mine.scm em.scm
  cargo run -q -- format mine.scm
  $EM -Q --batch --eval "(progn (find-file \"em.scm\") (scheme-mode) (setq indent-tabs-mode nil) (indent-region (point-min) (point-max)) (write-file \"em.scm\"))"
  diff em.scm mine.scm
  ```

  Corpora: `../lispexp/tests/corpus/{chibi-scheme,gauche}/…` (`.scm`/`.sld`) and
  `../lispexp/tests/corpus/typed-racket/…` (`.rkt`). On these the overwhelming
  majority of files are byte-exact; residual diffs are the `%define-syntax`
  flat-harness artifact above, or corpora indented under non-default settings
  (cross-check against the original file, per the note below).
- The **Clojure** engine's oracle is **cljfmt** (`cljfmt fix`), not Emacs — a
  native GraalVM binary (no JVM needed). Disable cljfmt's non-indentation passes so
  the diff isolates indentation, then compare a de-indented copy reformatted by each
  (both from-scratch and as a fixed point on the already-formatted file):

  ```sh
  LL=target/debug/lisplens
  cat > fixed.edn <<'EDN'
  {:remove-surrounding-whitespace? false :remove-trailing-whitespace? false
   :insert-missing-whitespace? false :remove-consecutive-blank-lines? false
   :remove-multiple-non-indenting-spaces? false :sort-ns-references? false
   :indentation? true}
  EDN
  CF=$(pwd)/fixed.edn                 # semantic; add `:indents {#re ".*" [[:inner 0]]}}` for Tonsky
  sed 's/^[[:space:]]*//' SRC.clj > mine.clj; cp mine.clj cf.clj
  cljfmt fix --config "$CF" --no-read-clj-config-files cf.clj
  $LL format mine.clj                 # add `--tonsky` when the config is the fixed one
  diff cf.clj mine.clj
  ```

  **Gotcha:** cljfmt reads `.cljfmt.edn` from the **current working directory**, not
  the target file's directory — always pass `--config` (and `--no-read-clj-config-files`)
  or a stray/absent CWD config silently falls back to cljfmt's *default* (semantic,
  whitespace-normalising) config and the comparison is wrong.

  On broad realistic corpora (ns/require, destructuring, nested `let`/`try`,
  threading, `defmulti`/`defmethod`, `deftype`/`reify`/`letfn`, `defmacro` with
  backquote, `condp`, `#(…)`, `#?(…)`, `#_`-discards, metadata) lisplens is
  byte-exact vs cljfmt in **both** styles across eight repos (663 real files) —
  semantic with **zero** divergences (the former `#_`-discard residuals resolved by
  `Options.keep_discarded`, lispexp 0.7.0). Known limitation: comment-only lines are
  the shared
  driver's, not the engine's, and can differ from cljfmt.
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
original.

A **page break (`^L`) sharing a line with code**, inside a form, still indents a
column or two off (`  ^L  (message "x")` as a `let`'s first body form: Emacs puts
the following body lines at the body indent, lisplens aligns them under the
`(message`). Two things differ from Emacs and interact: Emacs's `current-column`
counts a `^L` as **2** columns (it is displayed as `^L`), where `display_width`
gives it 0; and `specform`'s body branch aligns under the first body form, which
is an approximation of `lisp-indent-specform`'s real rule (a `body-indent` vs
`normal-indent` comparison) that only coincides while the first body form sits
exactly at the body indent — which a leading `^L` is the one realistic way to
break. Harmless for the `^L` convention itself, where the page break is always
alone on its line (that case, and `^L` before a comment, match Emacs exactly and
are pinned by `a_page_break_*` in `format::tests`). Close it by reading
`lisp-indent-specform` against the oracle rather than by guesswork.

**Corpus audit (2026-07, after the `cl-flet` fix).** A three-way sweep of the
local elpa + php-mode corpus (1167 files) — maintained file vs `emacs -Q --batch
indent-region` vs `lisplens format`, leading whitespace normalized to columns so
tabs-vs-spaces isn't counted — surfaced ~481 diverging lines, but almost all are
lisplens being *right*, not gaps: the maintained file (and the `-Q` oracle) are
stale with respect to the file's own indent declarations. Two shapes dominate,
both the **inverse of the harness caveat above** (there the oracle is wrong; here
the maintained file is too): (a) a macro with `(declare (indent N))` that the
author didn't reindent their file to — e.g. yaml.el's `yaml--rep` (186 lines, all
lisplens-correct once the declare is honored); and (b) a macro defined inside
`(eval-and-compile …)` / `(eval-when-compile …)` — lisplens's harvester descends
into those wrappers, so it applies the spec a real loaded Emacs would, while a
naive oracle that only reads top-level forms misses it (poly-lock.el's
`with-buffer-prepared-for-poly-lock`, 82 lines). A declare-aware oracle (harvest
the file's specs, `put` them, then `indent-region`) collapses these to zero. Bottom
line: real-world elpa fidelity is effectively complete; treat a fresh corpus diff
as guilty-until-proven — reproduce the form in isolation and cross-check a
declare-aware Emacs before believing it's a lisplens bug.

Only two *genuine* gaps survived that audit, both rare (1–2 corpus files each) and
so far unfixed:

- **`lisp-indent-local-overrides` file-local variable** (Emacs 30+). A file's
  trailing `Local Variables:` block can rebind the indent of specific symbols for
  that file only — e.g. with-editor.el sets `((cond . 0) (interactive . 0))`, which
  moves its `cond` clauses to open+2. lisplens harvests `(declare …)` and `(put …
  'lisp-indent-function …)` but not this file-local, so it uses the default spec
  and lands a column off. (Found only in with-editor.el across the corpus.)
- **A quote/backquote at end of line before a data list** — `(func '` then the
  quoted `((…) (…))` on the next line (the `lsp-register-custom-settings '` idiom,
  lsp-xml.el). Emacs indents the quoted list near flush (enclosing-open + 1),
  lisplens aligns it under the argument position, which for a long function name
  pushes a big data table ~30 columns right. This is the `(COLUMN . start)`
  prefix-interaction case named at the top of this section; fixing it touches
  `normal_indent`'s prefix handling, so verify no regression on the common
  same-line-prefix alignment first.

The **Common Lisp** and **Scheme-family** engines are both landed
(ADR-0031, see above), and **Clojure** has its own engine (ADR-0039, cljfmt
oracle); the remaining dialects Emacs has no indenter for (Fennel, Janet, Hy, LFE,
…) ride the generic Emacs Lisp fallback until they get one.

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
