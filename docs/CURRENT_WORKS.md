# lisplens — status snapshot

Ephemeral snapshot. **Durable knowledge is in the dev docs** (see `AGENTS.md` →
Codebase): `docs/dev/architecture.md`, `docs/dev/formatter.md`, `CONTEXT.md`,
`docs/adr/`.

## Now

- **`inline` command landed** (ADR-0032): `lisplens inline <name> <file>` (+ MCP
  `inline`) expands a function at its call sites — the benchmark's inline-expand as
  one atomic step. Restricted to the provably safe subset: a single
  `defun`/`defsubst`/`cl-defun`/`cl-defsubst` or Scheme `(define (name …) …)` with
  required-only params and a non-recursive body; niladic → body substituted
  directly, with-params → `(let ((p a) …) body)` (single-eval, order-preserving,
  what `defsubst` compiles to). Macros, variables, `&`-lambda-lists, recursion,
  arity mismatch → **refused** with a reason, never mis-expanded; only outermost of
  nested same-name calls per run; definition left in place; touched forms
  reindented + validated. `inline_definition_in_file` in `src/refactor.rs`. 118
  tests. **Next in ADR-0032: `extract` (needs the s-expr pattern-language design
  first — also covers guard-removal `(when flag (foo))` → `(foo)`, `progn`
  unwrap, etc.).**
- **`rename` command landed** (ADR-0032): `lisplens rename <old> <new> <file>`
  (+ MCP `rename`) renames a symbol across a file — **symbol-exact in code and
  data**, never substrings/keywords/strings/comments, so sibling symbols survive
  by construction (no `(?!-)` lookahead). Collapses the benchmark's proven idiom
  (`refs → line edit batch → refs`) into one call: splice → reindent the touched
  top-level forms (native engines) → validate-then-write, reporting the site
  count + new file hash; a missing `from` is an error, not a silent no-op.
  Verified on the benchmark's own trap (`c-macro-cache` renamed, `-get`/`-start-pos`
  siblings untouched). New `src/refactor.rs` (the home for ADR-0032 procedures).
  113 tests.
- **`check` command landed** (ADR-0032, first of the refactoring procedures): a
  standalone parse-check — `lisplens check <file>` (+ MCP `check`) parses by
  dialect and reports `path:line: message` diagnostics, silent + exit 0 when
  clean, non-zero on parse errors. Surfaces the guarantee lisplens already
  enforces on every edit (validate-then-write, ADR-0005) so agents/CI need not
  shell out to `emacs -Q --batch check-parens` (the benchmark baseline did,
  repeatedly). `check`/`diagnostics_text` in `lib.rs`. On branch
  `feat/refactoring-procedures`.

- **Polyglot native formatter — every Emacs-bundled Lisp indenter now has an
  engine** (ADR-0031, 2026-07-04). The formatter dispatches by dialect over one
  shared driver + three faithful engines: Emacs Lisp (`lisp-indent-function`),
  Common Lisp (`common-lisp-indent-function`), and the Scheme family
  (`scheme-indent-function`) — all three validated byte-exact against their Emacs
  major mode. Dialects Emacs bundles no indenter for
  (Clojure/Fennel/Janet/Hy/LFE/Phel/ISLisp/AutoLisp) ride the Emacs Lisp engine as
  the generic fallback (explicit `format` only; auto-format-on-edit is gated to
  `has_native_engine`). CL was built hands-on; the Scheme engine was delegated to
  a subagent (isolated worktree) and reviewed before merge. State: on **`master`**,
  **unpushed** — 2 commits ahead of `origin/master` (`6a7be54` CL, `412bec2`
  Scheme); `origin/master` still at `d4a4da4`. `cargo fmt --check` /
  `clippy --all-targets` clean. Per-engine detail below.
- **Display-width columns** (all engines, `unicode-width`): `Cols::col` now
  measures line content by East Asian Width, matching Emacs's `current-column`,
  so a wide/multi-byte glyph before an alignment target advances the column as
  Emacs would (`漢`/`Ａ` = 2, `λ`/`☆` ambiguous = 1). `(λλλλ arg` / `(漢漢漢漢 arg`
  continuations are now byte-exact vs Emacs; ASCII output is unchanged (0
  divergence re-formatting 80 magit/cl-ppcre/gauche files against the pre-fix
  binary). 107 tests (1 new multibyte golden). Closes the byte-column half of the
  former cross-cutting gap; only Racket infix dots remain. `col` runs
  `unicode-width` on every call — an ASCII byte-length fast path was benchmarked
  and **reverted** as not worth the state (~1–3 % of the indent pass on a 620 KB
  file; the real cost is `container_at`'s per-line tree re-descent). See
  `docs/notes/20260704-formatter-width-perf.md`.
- **Scheme-family indenter landed** (ADR-0031, 2026-07-04): `src/format/scheme.rs`
  — a faithful Rust port of `scheme-indent-function` (`scheme.el`), the *Emacs
  Lisp* algorithm with a Scheme spec table, `syntax-rules`/`def…` → defun, and the
  `scheme-let-indent` named-let method, plus its own full `calculate-lisp-indent`
  `normal-indent`. `engine_for` routes the whole family (Scheme, Guile, Racket,
  Gauche, Mosh, Gambit, superset) here; `has_native_engine` now covers it too, so
  auto-format-on-edit is enabled for Scheme. The bundled table is dumped from a
  real Emacs (the runtime union of the core + DSSSL + **MIT** `put` blocks —
  `scheme-mit-dialect` defaults to `t`, a key correction). Validated byte-exact vs
  Emacs `scheme-mode` on the chibi-scheme / gauche / typed-racket corpora: the
  overwhelming majority of files match (chibi 601/610 ≈ 99%, gauche 841/881
  parseable ≈ 95%, racket ≈ 94%). Residual diffs are the `beginning-of-defun`
  flat-harness artifact — a macro or nested `define` whose *source* is indented
  as a definition (e.g. chibi's `%define-syntax`, `tree-match`), where lisplens
  matches a *clean* Emacs buffer but Emacs's own from-scratch reindent of fully
  de-indented input mis-scans — plus non-UTF-8 / CRLF test-data files and corpora
  indented under non-default settings. The remaining dialects Emacs bundles no
  indenter for (Clojure/Fennel/Janet/Hy/LFE/…) still ride the generic Emacs Lisp
  fallback. 106 tests pass (5 new Scheme goldens, captured from the Emacs oracle).
  The engine also carried a few **shared-helper** refinements, all
  regression-checked by re-formatting 47 magit/lem Elisp + 25 cl-ppcre CL files
  with the pre- and post-merge binaries (**0 output divergence** on both corpora):
  `head_is_symbol_like` now treats a `#\`-char literal as data (Scheme) but a
  `?`-char as symbol-like (Emacs Lisp); `whitespace-after-open-paren` counts only a
  same-line space/tab, not a trailing newline; `container_at` descends into
  `#(…)`/`#u8(…)` vectors; and `specform`'s body-form branch was corrected — when
  the first body form shares the head's line it now falls to `normal-indent`
  (align under the previous element) instead of under that first body form. That
  last one is a **latent Emacs Lisp fix**, verified against the oracle:
  `(when cond (a)⏎(b))` now lands `(b)` at col 6 (Emacs) where the old shared code
  gave col 11 (it just never occurred in the magit/lem corpus, so no golden caught
  it).
- **Common Lisp indenter landed** (ADR-0031, 2026-07-04): the formatter is now
  **one shared driver + a dialect-selected engine**. `src/format.rs` became
  `src/format/mod.rs` (driver + Emacs Lisp engine) plus `src/format/commonlisp.rs`
  — a faithful Rust port of `common-lisp-indent-function` (`cl-indent.el`):
  multi-level backtracking + `path`, the `lisp-indent-259` spec walker, the
  bundled CL table, `tagbody`/`do`/`defmethod`/lambda-hack/`loop`, package-prefix
  stripping, and lambda-list keyword alignment. `format(source, config, dialect)`
  dispatches; `.lisp/.lsp/.cl/.asd` → CL engine, non-bundled dialects
  (Clojure/Fennel/…) → generic Emacs Lisp fallback. Auto-format-on-edit gated to
  `has_native_engine` (Emacs Lisp, Common Lisp). Byte-exact vs Emacs `lisp-mode`
  on `cl-ppcre` + the `gpg`/`gpgme` CL sources (residual diffs are the
  `lisp-indent-defmethod` flat-harness caveat, trailing newlines, or two
  documented gaps). This was the first engine after Emacs Lisp and the template
  for the Scheme engine above.
- **Released 0.1.0** (2026-07-03) — on [crates.io](https://crates.io/crates/lisplens)
  (`cargo install lisplens`) and as pre-built binaries on the GitHub Release for
  x86_64/aarch64 Linux + macOS and x86_64 Windows. Tag `vX.Y.Z` → GitHub Actions
  publishes (`.github/workflows/release.yml`); next bump via the
  `lisplens-release-prep` skill. No pinned MSRV (binary tool; deps track recent
  stable Rust).
- **Released 0.1.1** (2026-07-04) — a dependency-only release consuming
  `lispexp-emacs` 0.1 (on `lispexp` 0.6): the bundled indent table and the
  file-local / dir-local **parsers** moved out of lisplens into the companion
  crate (lispexp ADR-0033, commit `02a293a`) — table verified byte-identical,
  −78 net lines, output unchanged. On crates.io + GitHub Release binaries.
- **Delegation boundary reviewed** (`docs/notes/20260704-delegation-boundary-review.md`):
  the current split (lispexp-emacs = Emacs *data + parsers*) is right but
  incomplete — the highest-reuse Emacs *behavior*, the `calculate-lisp-indent`
  indent algorithm in `src/format.rs` (+ `nameless.rs`), is the top remaining
  candidate to move into lispexp-emacs; Emacs config resolution is a smaller
  follow-up. Not started — a roadmap item for lispexp-emacs.
- 106 tests pass, `cargo fmt --check` / `cargo clippy --all-targets` clean; tree
  clean. 31 ADRs.
- **Touched-region auto-format on Structural edit (ADR-0025/0028) is wired**:
  `apply_struct_patch` reindents the top-level forms an edit fell within
  (`format::reindent_range` + `edit::splice_tracked`), for dialects with a
  faithful native engine (`has_native_engine`: Emacs Lisp, Common Lisp, the Scheme
  family); other dialects stay byte-identical; Line-hash stays literal (ADR-0027).
- **`format <anchor>` Structural verb (ADR-0028 point 3)**: reindent exactly one
  anchored form in place — even nested, in full context (`format::reindent_block`,
  the `exact` scope of `Touched`). Carried as an identity edit so it shares the
  splice/conflict path. 13 Structural verbs now.
- On **lispexp 0.5** (`dot_span` for improper-list dots — our upstream ask,
  shipped).
- **`lisp-body-indent` / EditorConfig `indent_size` overrides** now resolved
  through `FormatConfig.body_indent` (ADR-0029), scaling every structural step;
  byte-exact vs Emacs with `lisp-body-indent` 4.
- **Lone `;` own-line comments → `comment-column`** (`FormatConfig.comment_column`,
  default 40) matching Emacs `indent-for-comment`. High-value: emacs `lisp/`
  sweep improved 17 files, 0 regressions (ansi-color 11→0, woman 23→2, …).
- **First-release goal: a faithful Emacs Lisp formatter.**
- **Long-tail closed** (all verified byte-exact vs Emacs, 0 regressions across
  emacs `lisp/` + magit/lem sweeps): data lists vs function calls
  (`lisp-indent-function`'s non-symbol-head path), `progn`-style body forms that
  start on the open line, dotted-tail sublists (`(a . (b c))`), `;;;` comment
  lines left in place, and `whitespace-after-open-paren` (`( a b` aligns under
  the first element). `php-mode/lisp` is effectively 100% faithful: 12/13 files
  byte-exact, and the 13th (php-mode-debug.el) is a harness artifact.
- **Harness caveat drives the apparent remaining diffs.** batch Emacs doesn't
  evaluate a file, so it misses that file's own `(declare (indent N))` macros
  (mpc-select-save, jsonrpc-lambda, define-icon, …). lisplens *harvests* those,
  so where the harness "differs" lisplens actually matches the checked-in file —
  confirmed on mpc.el, tab-bar.el, jsonrpc.el, php-mode-debug.el. Real fidelity
  is far above the raw byte-exact count. See [[formatter-harness-declare-caveat]].
- **Nameless-aware indentation (ADR-0030)**: `format --nameless` models
  Nameless's namespace-prefix composition (`php-`→`:`, `font-lock-`→`fl:`).
- **Dogfooded on php-mode** (fixed its Emacs-32 build via lisplens patches;
  `docs/notes/20260703-dogfooding-php-mode.md`). Both findings now **fixed**:
  the bundle includes `cc-mode` specs (`c-lang-defconst` etc., 342 entries); and
  Structural-edit auto-format is **Nameless-aware when configured** — a
  `nameless-mode` file-/dir-local resolves `FormatConfig.nameless` and flows into
  the edit path (ADR-0029/0030). Also fixed a dir-locals parser bug (only read
  the dotted mode-entry form, not php-mode's `(MODE (VAR . VAL) …)` form).

## Deferred (future work — not blocking first release)

The Emacs Lisp formatter is effectively complete; what remains is deliberately
parked. In rough priority for whenever it is picked up again:

1. **Formatter's true long tail (niche).** Nested specforms where Emacs's
   `(COLUMN . start)` list-return semantics differ from the plain column (e.g. a
   long `if-let` condition), and package-local macros absent from the
   bundled/harvested specs. Hard to even *locate*: the batch harness buries them
   under declare-artifacts (see the harness caveat above), so finding them needs
   a fair reference — compare against the original file, not batch Emacs. Low
   value, high effort; parked.
2. **More real-world elisp validation.** Header/footer and tab-mode files;
   config resolution end-to-end on real repos. Easy to start, open-ended; run the
   harness on new corpora when convenient.
3. **Single `;` inline (not own-line) comment alignment** — the own-line case is
   done; inline comments would need the `lex` trivia layer (lispexp-feedback/0002).
4. **Racket infix dot** `(a . op . b)` (two dots in one list) — the continuation
   is off; a niche reader construct, engine-agnostic. (The other cross-cutting
   gap, byte- vs display-column measurement, is now **fixed** — see the
   display-width bullet above.)
5. **Native indenters for the non-bundled dialects** (Clojure/Fennel/Janet/Hy/LFE/
   …), which currently ride the generic Emacs Lisp fallback. Emacs bundles no
   oracle for them, so each needs its own reference + spec (a separate,
   design-first effort per family). Not required for the Emacs-bundled scope.
6. **MCP edit JSON op-array** (ADR-0019) and **S-expr structural addresses**
   (ADR-0018 defers these). Each is its own design-first chunk on a separate
   surface.
