# lisplens — status snapshot

Ephemeral snapshot. **Durable knowledge is in the dev docs** (see `AGENTS.md` →
Codebase): `docs/dev/architecture.md`, `docs/dev/formatter.md`, `CONTEXT.md`,
`docs/adr/`.

## Handoff — resume here (2026-07-04 session end): finish the Phel branch after lispexp lands

**Where.** On branch **`design/phel-engine`** (PR #11, open), rebased onto
`origin/master` — linear: two commits (the Phel indent engine ADR-0041, and the
consolidated `docs/lispexp-feedback/`). `master` has everything up to the Clojure
work + PR #12 (the AGENTS ground rule). The Clojure engine (ADR-0039/0040 + fidelity
fixes) is **merged to master** via PRs #7/#8/#9/#10; Phel (ADR-0041) is PR #11.

**Blocked on lispexp — this is the whole next step.** The Phel work surfaced a set
of **lispexp reader/lexer gaps**, all recorded in `docs/lispexp-feedback/` (index in
its `README.md`). The lispexp maintainer is **implementing them upstream** (already
visible in `~/repo/rust/lispexp`: `lexer.rs` has a `pipe_anon_fn` path + `phel.rs`
tests for 0005). Per the **AGENTS ground rule**, lisplens does **not** PR/commit
lispexp — it records needs in `docs/lispexp-feedback/` and consumes a published
version. The open notes:
- **0003** `#_`/`#;` discarded forms dropped → proposed `Options.keep_discarded`
  keeping them as `Prefixed { Discard, … }` (Clojure `#_`-inside residual).
- **0004** Phel `;`-inside-a-symbol → proposed `Options.line_comment_in_atom`.
- **0005** Phel `|(…)` anon-fn unrecognized → proposed `roles.short_fn = Some('|')`.
- **0006** Phel `\Foo\Bar` FQN mis-read as `\`-char literals → proposed a Phel
  char-literal boundary guard.

**Next steps (when lispexp publishes the fixes):**
1. Bump `lispexp = "0.x"` in `Cargo.toml` to the version with the fixes.
2. For **0003**: set `opts.keep_discarded = true` where the *formatter* parses (build
   the `Options` then mutate — `Options` is `#[non_exhaustive]`). **Then re-validate
   arg-index handling vs cljfmt**: a kept `Prefix::Discard` now occupies a value slot,
   so `src/format/clojure.rs`'s value-index counting must treat it the way cljfmt does
   (skip vs count) — this is the one non-trivial consumption bit.
3. For **0004/0005/0006**: the Phel parse just becomes correct; re-run the corpora
   and confirm the residuals resolve (see the harness recipes in `docs/dev/formatter.md`).
   Corpora were fetched under the scratchpad this session (reitit/ring/hiccup/clj-kondo/
   next.jdbc/malli/integrant/jsonista for Clojure; `phel-lang` for Phel) — re-clone if gone.
4. Remove the now-obsolete "known limitation" wording in `docs/dev/formatter.md` and
   mark the resolved feedback notes (follow note 0002's RESOLVED pattern + the README).
5. Merge PR #11.

**Process rules learned this session (also in AGENTS + memory):** never edit/commit/PR
the **lispexp** repo — record in `docs/lispexp-feedback/`. Interactive doc edits may
commit on **`master`** directly; **never push `master` without explicit permission**;
topic branches — push/force-push freely.

**Quality gate (all green on #11):** 161 tests, `cargo fmt --check`,
`clippy --all-targets`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps`; tree clean.
41 ADRs. CI runs `cargo fmt --check` on `dtolnay/rust-toolchain@stable` — keep the
local toolchain on CI stable (`rustup update stable`) or the Format step drifts.

**Gotchas.**
- A committed **PostToolUse hook** (`.claude/settings.json`) auto-runs `cargo fmt`
  after any `.rs` Edit/Write — expect post-edit reformats.
- **cljfmt reads `.cljfmt.edn` from CWD**, not the target file's dir — pass
  `--config … --no-read-clj-config-files` in the Clojure harness or it silently uses
  the default (semantic) config (cost us 192 false divergences once).
- The Clojure/Phel engine is shared (`Engine::Clojure`, `rules_for(name, dialect)`);
  Phel's one `:block` difference is body-indenting *special* args too (see ADR-0041).
- Phel value-aligns binding vectors and the flat-harness `lisp-indent-defmethod`
  caveat remain out of scope / documented.

**Candidate next work (after #11 lands):** (a) remaining `extract` opt-ins —
free-var **inference**, skeleton auto-discovery for `--also`, multi-*file* extraction;
(b) native indenters for the other non-bundled dialects (Fennel/Janet/Hy/LFE);
(c) move `calculate-lisp-indent` into `lispexp-emacs`
(`docs/notes/20260704-delegation-boundary-review.md`).

## Now

- **Native Phel indent engine landed** (ADR-0041) — Phel (a Clojure-inspired Lisp
  compiling to PHP) no longer rides the generic Emacs Lisp fallback. `phel format`
  turns out to be a **PHP port of cljfmt's `:inner`/`:block` model**, so
  `Dialect::Phel` routes to `Engine::Clojure` with a per-dialect table
  (`rules_for(name, dialect)` + `phel_rules_for` = Phel's
  `FormatterFactory::{INNER,BLOCK}_INDENT_SYMBOLS`, phel-lang 0.47 verbatim). Phel is
  a strict subset (only `[:inner 0]`/`[:block N]`, no nested inner/regex/reader-cond)
  with **one `:block` difference**: Phel body-indents a block form's *special* args
  too once the body breaks (`(when-not⏎(test)⏎(body))` → both at +2), where cljfmt
  keeps the test at +1 — `block_indent` takes the dialect and skips the special-arg
  default only for non-Phel. Phel joins `has_native_engine` (auto-reindent on edit).
  Oracle: `phel format` (PHP+Composer+phel installed). Validated **byte-exact
  307/310** on phel-lang's own `.phel` files; the 3 residuals: a `;`-inside-a-symbol
  lispexp tokeniser gap (`docs/lispexp-feedback/0004`, upstream) and two niche
  one-liners (closing `])` after inline comments; a `#(…)` doc-example off-by-one).
  Phel value-aligns binding vectors (like cljfmt's opt-in alignment) — out of scope
  for a leading-whitespace formatter. 161 tests (Phel golden). Third oracle
  (`phel format`) joins Emacs and cljfmt.

- **Clojure engine validated on 8 repos + metadata/prefix-head fixes** (follow-up
  to ADR-0039/0040). Expanded the real-code validation to **eight** repos (hiccup,
  ring, reitit, clj-kondo, next.jdbc, malli, integrant, jsonista — 663 real
  `.clj/.cljs/.cljc`, excluding clj-kondo's deliberately-malformed linter fixtures),
  in **both** semantic and fixed styles vs `cljfmt fix --config`. Found and fixed
  three real bugs, taking the **semantic** style to **zero non-`#_` code-indent
  divergences** (all ~20 residuals are the upstream `#_`-discard limitation); the
  **fixed** style has one residual (an obscure `(#?(…) …)` reader-conditional-headed
  call, off-by-one). Fixes: (1) an argument is located by its **form** start past any
  `^metadata` prefix (`form_start`), so a `^Tag` on the head line while its form
  wraps doesn't make the arg look completed (`(doto ^Tag⏎(f)⏎(g))`); the alignment
  *column* stays the element's true start (cljfmt aligns under the `^`). (2) a
  `^meta` **head** is transparent for rule lookup (`(^:m when …)` uses when's rule),
  but a quote/var-quote/unquote head (`#'foo`, `'foo`) is **not** a symbol head →
  default alignment (cljfmt keys on the bare symbol token, unlike the Emacs engines'
  transparent prefixes). Learned the cljfmt `--config`-from-CWD gotcha the hard way
  (an early fixed-mode run silently used the default semantic config → 192 false
  divergences). 160 tests (2 regression goldens). No new ADR (engine tuning).

- **Clojure fixed / Tonsky indent style landed** (ADR-0040) — the opt-in
  alternative to the default semantic style (a deliberate rebellion against
  Emacs-descended indentation: no rule table, no align-under-first-argument). Every
  **symbol-headed** round list body indents a flat `open + 2` — function calls,
  `do`, threading `->`/`->>`, `defn`, all of it; collections `[]`/`{}`/`#{}`, reader
  conditionals `#?(…)`, and non-symbol heads stay identical to semantic (data /
  default). Selected by `FormatConfig.clojure_fixed_indent`, enabled by
  `format --tonsky` (CLI) or a `clojure-ts-indent-style: fixed` file-/dir-local
  (flows into auto-format-on-edit via ADR-0029). Implementation is a few lines —
  fixed short-circuits only the symbol-headed round-list branch to `open + body`,
  reusing the whole engine. Oracle: `cljfmt fix --config {:indents {#re ".*"
  [[:inner 0]]}}`; validated **byte-exact 268/272** on reitit + ring + hiccup (same
  residual 4 `#_`-discard files as semantic). Harness gotcha learned: cljfmt reads
  `.cljfmt.edn` from **CWD**, so pass `--config`/`--no-read-clj-config-files` or it
  silently uses the default semantic config. 158 tests (fixed golden + config
  resolution). `--tonsky` in `run_format` (now flag-parsed); `clojure::indent` takes
  a `fixed` flag.

- **Clojure engine validated on real-world corpora + two fidelity fixes**
  (follow-up to ADR-0039). Ran the whole reitit + ring + hiccup source (272
  `.clj/.cljs/.cljc`) through `cljfmt fix` (indentation-only) vs lisplens: **268
  byte-exact on code-line indentation** (223 fully byte-identical incl. comments);
  the residual differences were comment-only lines / trailing-internal whitespace
  (out of scope — `format` rewrites leading whitespace only) plus two real bugs,
  now fixed: (1) **metadata `^{…}` maps** — the map lives in the `Prefixed` node's
  `arg`, which the shared `container_at` (and `in_string`) ignored, so a metadata
  map's continuation keys mis-indented and a docstring *inside* metadata got
  reindented; both now descend `arg`, aligning keys under the first key and leaving
  string interiors untouched. The **only** remaining code-indent divergence class is
  `#_`-discarded multi-line forms (4 files): lispexp drops discarded forms from the
  tree, so lines inside them indent against the enclosing form — an upstream reader
  limitation, documented in `docs/lispexp-feedback/0003-discarded-forms-dropped.md`.
  156 tests (1 metadata regression golden). No new ADR (engine/driver fix).

- **Native Clojure indent engine landed** (ADR-0039) — Clojure no longer rides the
  generic Emacs Lisp fallback; `Engine::Clojure` (`src/format/clojure.rs`) is a
  native port of **cljfmt's** semantic `:inner`/`:block` model (the standard the
  whole ecosystem — cljfmt, clojure-ts-mode, cljstyle, modern clojure-mode —
  converged on), and Clojure joins `has_native_engine` so `.clj/.cljs/.cljc`
  auto-reindent on Structural edit. Unlike the CL/Scheme engines this is **not** an
  Emacs port and the oracle is **not** Emacs: it targets **cljfmt** (`cljfmt fix`, a
  native GraalVM binary — no JVM), whose `indents/clojure.clj` table is bundled
  verbatim. The model (empirically pinned vs cljfmt 0.16.4): collections + `#?(…)`
  align under the first element; round lists/`#(…)` are the call model — default
  aligns under arg 0 (threading `->`/`->>` included), `[:inner 0]` bodies +2,
  `[:inner D]`/`[:inner D idx]` walk up ≤3 levels (reify/letfn), `[:block N]`
  specials use default and body +2 only when the first body form begins a line
  (this is where `:block` differs from Emacs's integer spec — no double-indent).
  Verified **byte-exact vs cljfmt** on broad corpora (ns/require, destructuring,
  nested let/try, threading, defmulti/deftype/reify/letfn, defmacro+backquote,
  condp, reader conditionals). 155 tests (6 Clojure goldens). Survey:
  `docs/notes/20260704-clojure-indentation-survey.md`. Deferred: the fixed/Tonsky
  style flag, map/binding column alignment, `edn`-specific handling, per-project
  `:extra-indents`, and comment-only-line parity.

- **`extract --also` (generalizing multi-site / anti-unification) landed**
  (ADR-0038) — `extract` gains a repeatable `--also ANCHOR` (MCP `also: []`): the
  primary anchor plus every `--also` site are **anti-unified** — their common
  skeleton becomes the function body, each position where they diverge becomes an
  inferred parameter, and each site calls with its own sub-terms. Example: `(* x 2)`
  + `(* x 3)` → `(defun scale (arg1) (* x arg1))`, calls `(scale 2)` / `(scale 3)`.
  **Explicit sites, no discovery** — the safe choice for the first `extract` that
  *infers* structure (anti-unification's param count grows with site heterogeneity,
  so precise site selection keeps output clean). **Standard AU, list-structured:**
  recurse through co-structured lists (same delim+arity), keep the operator fixed,
  parameterize the first divergence (leaf or whole subtree). Refusals
  (`NotGeneralizable`, no write): differing operators (never generalize a list
  head), no common skeleton, differing improper-list tails, or a param-count
  mismatch. **Ceiling held, not crossed deeper** — AU generalizes *only what
  differs*; a symbol common to every site (a free local like `x` above) is baked in
  unchanged, exactly as single-site extract; **no binding analysis** (free-var
  *inference* still deferred). Params are generated `arg1..` (collision-free) or
  caller-named. `--also` is a distinct site mode: combining with `--all` or `--count
  >1` is refused. `extract_generalized` + `anti_unify` in `src/refactor.rs`; all
  three site paths (single / identical-multi / generalizing) share the
  `finish_extraction` tail (now per-site call text). 149 tests. Deferred: free-var
  inference, skeleton auto-discovery, `--also` over runs, non-linear hole merging.

- **`extract --all` (multi-site) landed** (ADR-0037) — `extract` gains an optional
  `--all` flag (MCP `all`): extract **every occurrence structurally equal to the
  anchored selection** into one new function, replacing each with the call. "The
  same" is `struct_eq` (formatting-modulo structural equality, the same relation
  `rewrite` uses). For `count == 1` a site is any node anywhere (whole-tree
  `for_each_node` walk, so it catches subterms, not just siblings); for `count > 1`
  a site is any **window of N contiguous siblings** equal to the anchored run
  (`for_each_sibling_group` sliding window). Overlapping candidate windows keep the
  outermost (`keep_outermost_spans`). The def is inserted once, before the earliest
  site's enclosing top-level form. **No generalization** — sites must be identical
  *including* arguments, so the same `(NAME PARAMS)` replaces each; anti-unification
  (sites differing in an argument) stays deferred as the move that actually crosses
  the ADR-0003 ceiling. Composes with `--count` and `--kind` (orthogonal knobs). A
  form appearing once degrades to single-site extract; `ExtractOutcome.sites` now
  reports the count. `extract_multi_site` in `src/refactor.rs`; single- and
  multi-site share a `finish_extraction` splice/reindent/validate tail. 142 tests.
  Deferred: free-var inference / anti-unification, multi-*file* extraction.

- **`extract --kind` landed** (ADR-0036) — `extract` gains an optional
  `--kind HEAD` (MCP `kind`) that names the leading operator of the emitted
  definition, defaulting to the dialect's plain-function head (so ADR-0034/0035
  output is unchanged when absent). Only the **head** is swapped; the definition's
  **shape family stays the dialect's**: Flat `(HEAD NAME (params) body)` (elisp/CL,
  default `defun`; e.g. `defsubst`, `cl-defun`), Nested `(HEAD (NAME params) body)`
  (Scheme, default `define`; e.g. `define-inline`), Bracket `(HEAD NAME [params]
  body)` (Clojure, default `defn`; e.g. `defn-`). `HEAD` is **not validated** — any
  symbol is placed verbatim (the ADR-0003 ceiling: user asserts semantics, lisplens
  guarantees parse-safety, same as params / `rewrite` templates). Dialects with no
  known shape family are still refused (`UnsupportedDialect`) — `--kind` does not
  unlock them. `def_shape(dialect) -> Option<(default_head, DefShape)>` centralizes
  the three families; `def_form` takes the `kind` override; `extract_block_into_function`
  threads it; `extract_into_function` stays the `count=1`, `kind=None` wrapper. Wired
  through CLI (`parse_extract_opts`) + MCP (`kind` field + inputSchema). 137 tests.
  Deferred: free-var inference, multi-site, non-default placement, fold-repeats.

- **Block extraction landed** (ADR-0035) — `extract` gains an optional
  `--count N` (MCP `count`, default 1): extract a run of `N` **contiguous sibling
  forms** starting at the anchor into `(defun NAME (PARAMS) form₁ … form_N)`,
  replacing the run with one `(NAME PARAMS)` call. Same pure cut+wrap as ADR-0034
  (no free-var inference); the run is resolved from the anchored node's parent +
  index (top-level forms when the anchor is top-level), and refused with
  `RunExceedsSiblings` if it crosses the sibling group (no partial write). `count=1`
  reproduces the single-form path exactly. `def_form` now places a **multi-line
  body on its own line** (a run, or a multi-line single form) so reindent lays it
  out conventionally; single-line bodies stay inline (ADR-0034 one-liner unchanged).
  `extract_block_into_function` in `src/refactor.rs`; `extract_into_function` is the
  `count=1` wrapper. 135 tests. Value = only in a body/`progn` position (implicit
  progn → last form's value). Deferred: free-var inference, multi-site, non-`defun`
  kinds; and the distinct *fold-repeats-into-a-loop* transform (`(foo)(foo)(foo)` →
  `(dotimes …)`), parked until a real need.

- **`extract` implemented** (ADR-0034) — the last ADR-0032 member: `lisplens
  extract <file> <anchor> <name> [param...]` (+ MCP `extract`) pulls the form at
  `anchor` into a new function and replaces it with a call. **User supplies the
  name + params; lisplens does not infer free variables** (stays within the
  ADR-0003 semantic ceiling — like `rewrite`, the user asserts, lisplens
  guarantees parse-safety). A pure cut+wrap (no symbol substitution): builds
  `(defun NAME (PARAMS) <selection>)` before the enclosing top-level form and
  `(NAME PARAMS)` in place, per-dialect def form (elisp/CL `defun`, Scheme
  `define`, Clojure `defn []`; others error), reindented + validated.
  `extract_into_function` in `src/refactor.rs`. 131 tests. **The ADR-0032
  refactoring family (check/rename/inline/rewrite/extract) is complete.** Future
  opt-ins: free-var inference, block (`anchor+count`) extraction, non-`defun` kinds.

- **`rewrite` implemented** (ADR-0033): `lisplens rewrite <file>` (spec on stdin)
  + MCP `rewrite` — a structural pattern→template "sed" in `src/refactor.rs`
  (`rewrite_in_file`): a `Datum` matcher (metavariables + classes + non-linear +
  trailing sequence), `struct_eq` (span/line-ignoring `DatumKind` compare, literal
  leaves), whole-tree outermost single-pass collection, and a verbatim template
  substituter, over the splice→reindent→validate pipeline. Verified on the ADR's
  examples from the CLI (guard removal, if→when, progn-unwrap sequence,
  class-guarded fold, non-linear, deletion, drift, error cases). 127 tests. User
  guide + a verified rewrite cookbook in **`docs/rewrite.md`** (the "presets are
  documentation" deliverable). `extract` renamed → `rewrite`; the true "extract
  into a new function" is the one unbuilt ADR-0032 member.

- **`inline` command landed** (ADR-0032): `lisplens inline <name> <file>` (+ MCP
  `inline`) expands a function at its call sites — the benchmark's inline-expand as
  one atomic step. Restricted to the provably safe subset: a single
  `defun`/`defsubst`/`cl-defun`/`cl-defsubst` or Scheme `(define (name …) …)` with
  required-only params and a non-recursive body; niladic → body substituted
  directly, with-params → `(let ((p a) …) body)` (single-eval, order-preserving,
  what `defsubst` compiles to). Macros, variables, `&`-lambda-lists, recursion,
  arity mismatch → **refused** with a reason, never mis-expanded; only outermost of
  nested same-name calls per run; definition left in place; touched forms
  reindented + validated. `inline_definition_in_file` in `src/refactor.rs`.
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
  repeatedly). `check`/`diagnostics_text` in `lib.rs`. (Merged to master via PR #1.)

- **Polyglot native formatter — every Emacs-bundled Lisp indenter now has an
  engine** (ADR-0031, 2026-07-04). The formatter dispatches by dialect over one
  shared driver + three faithful engines: Emacs Lisp (`lisp-indent-function`),
  Common Lisp (`common-lisp-indent-function`), and the Scheme family
  (`scheme-indent-function`) — all three validated byte-exact against their Emacs
  major mode. Dialects Emacs bundles no indenter for
  (Clojure/Fennel/Janet/Hy/LFE/Phel/ISLisp/AutoLisp) ride the Emacs Lisp engine as
  the generic fallback (explicit `format` only; auto-format-on-edit is gated to
  `has_native_engine`). CL was built hands-on; the Scheme engine was delegated to
  a subagent (isolated worktree) and reviewed before merge. **Merged to
  `origin/master`** (the whole formatter + display-width + fmt hook are on
  master). Per-engine detail below.
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
- (Test/ADR counts and git state are current in the Handoff block at the top.)
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
5. **Native indenters for the remaining non-bundled dialects** (Fennel/Janet/Hy/
   LFE/…), which currently ride the generic Emacs Lisp fallback. Emacs bundles no
   oracle for them, so each needs its own reference + spec (a separate,
   design-first effort per family). **Clojure is now done** (ADR-0039, cljfmt
   oracle). Not required for the Emacs-bundled scope.
6. **MCP edit JSON op-array** (ADR-0019) and **S-expr structural addresses**
   (ADR-0018 defers these). Each is its own design-first chunk on a separate
   surface.
