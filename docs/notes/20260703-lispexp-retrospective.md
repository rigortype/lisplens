# Retrospective: building lisplens 0.1.0 on lispexp (2026-07-03)

A consolidated look back at using [lispexp](https://crates.io/crates/lispexp) as
lisplens's backend through the 0.1.0 release — what worked, where it hurt, and
which knowledge lisplens re-implements that lispexp could own so no consumer
writes it twice. Point asks live in [../lispexp-feedback/](../lispexp-feedback/);
this note is the wider synthesis. lisplens's constraint throughout: stay at the
form-annotator level (its ADR-0003, lispexp's reader-only ADR-0001) — no binding
resolution, no macro expansion, no evaluation.

## What lisplens actually uses

`parse` → `Datum`/`DatumKind`/`Delim`/`Span`; `LineIndex`; `Options`/`Dialect`;
`ErrorKind`/`ParseError` (validate-then-write); `annotate::{annotate_tree,
bundled_registry, Annotated, Role}` (outline / find / refs); `walk::{Class,
Walk}` (code-vs-data for refs); `indent::{IndentSpec, IndentTable,
harvest_indent_specs}` (formatter); `Datum::dot_span()` (formatter). Comments are
read via the `lex` token layer where needed.

## What worked well (keep these)

- **`annotate` + `bundled_registry(dialect)`** carried the entire read side
  (outline, expand, find, refs) with zero bespoke definition heuristics, across
  every dialect lisplens targets. `Role`-based access (`Name`, `Qualifier`,
  `DispatchValue`, `specialized_params`) made the Dispatch-signature feature fall
  out for free.
- **`indent::harvest_indent_specs`** (`declare` / `put` / `function-put` /
  `lisp-indent-hook`, typed as `IndentSpec::{Number, Defun, Function, Raw}`) is
  the backbone of formatter fidelity: it lets lisplens match a file indented by
  the author's *fully-loaded* Emacs (file-local macros like `mpc-select-save`,
  `jsonrpc-lambda`) where a batch `emacs -Q` can't. This is the single most
  valuable indent API.
- **`Datum::dot_span()` (0.5)** — the improper-list `.` byte span, our own ask,
  landed clean and let the `'(eval . FORM)` alignment work without a source
  rescan (lispexp-feedback/0002).
- **Char-literal lexing** is robust: `? ` (space), `?x`, `?\(` all classify as
  `DatumKind::Char`, so the formatter reproduced Emacs's `\sw\|\s_` head test by
  `DatumKind` alone.
- **Exhaustive `DatumKind`** (no `#[non_exhaustive]`) is a feature for a
  formatter/walker — the compiler forces an arm per kind, so new node kinds can't
  be silently mishandled. Worth the occasional `..`.
- **`ErrorKind` as a position-stable multiset** made validate-then-write (reject
  only *newly introduced* parse errors) a few lines.

## Friction & failures

- **Re-bundling Emacs indent specs (the big one).** lispexp harvests file-local
  specs but shipped **no default table**, so lisplens hardcoded 342 `(symbol →
  IndentSpec)` entries (`NUMBER_SPECS`/`DEFUN_SPECS`), harvested by hand from a
  real Emacs — and had to *re-harvest with `cc-mode` loaded* mid-project when
  `c-lang-defconst` turned out missing (it dogfooded on php-mode). Every consumer
  that wants Emacs indentation had to reproduce this same data. **Resolved:** the
  table now lives in `lispexp-emacs` and lisplens consumes it (delegation #1).
- **Comments dropped from the tree.** `;;;`-preserve, lone-`;` → `comment-column`,
  and `whitespace-after-open-paren` were detectable textually or via span gaps,
  but only because own-line comments are simple; inline-comment handling would
  need the `lex` layer. Fine, but it's Emacs-semantics knowledge living in the
  consumer (lispexp-feedback/0002).
- **`LineIndex::line_range` normalization footgun** — a byte-oriented API whose
  `line_range` silently returns terminator-stripped, non-tiling content. Worked
  around; still worth an additive verbatim accessor (lispexp-feedback/0001).
- **Emacs-semantics reverse-engineering, unavoidably consumer-side.** The
  `calculate-lisp-indent` algorithm, the Nameless composed-width formula
  (`⌊len/2⌋+1`, measured empirically against Emacs), and `.dir-locals.el` /
  file-local variable parsing (two mode-entry forms) were all re-derived in
  lisplens. The indent algorithm and Nameless formula are genuinely
  rendering/editor policy (kept in lisplens), but the two *parsers* — file-local
  `-*- … -*-`/`Local Variables:` and the `.dir-locals.el` evaluator — were
  Emacs-data reverse-engineering and have since moved to `lispexp-emacs`
  (`local_vars`, `dir_locals`); lisplens now only interprets the raw name/value
  bindings and keeps its multi-mode applicability + directory-walk precedence.

## Delegation candidates (avoid re-implementing the same knowledge)

Ranked by value × fit with lispexp's reader-only scope.

1. **Ship a bundled default indent-spec table** — **✅ shipped in `lispexp-emacs`
   and consumed (2026-07-03).** Landed as `lispexp_emacs::indent::bundled_table(Dialect)
   -> IndentTable`, the exact analogue of `annotate::bundled_registry(dialect)`.
   Rather than living in `lispexp` core, the standard Emacs data went into the
   companion crate `lispexp-emacs` (lispexp ADR-0033) — Emacs-specific,
   version-sensitive data kept out of the neutral reader. lispexp owned the type
   (`IndentTable`), the spec enum, and the harvester; only the *standard data* was
   missing, and it is now bundled byte-identically (342 entries, cc-mode
   included). lisplens **deleted its `NUMBER_SPECS`/`DEFUN_SPECS` outright** and now
   starts from `bundled_table`, `merge`-ing harvested file-local specs on top.
   Clean boundary held: the crate owns the *data*, lisplens keeps the *indent
   algorithm*.

2. **Comment/trivia spans as a first-class side channel** — the `lex` layer
   already emits `LineComment`/`BlockComment` with spans, so no new data is
   needed; the ask is ergonomic: a `Parsed.comments: Vec<Span>` (or a documented
   "correlate lex by span" example) so a formatter doesn't re-lex to find
   comments. Low priority — already served (lispexp-feedback/0002).

3. **Verbatim line access on `LineIndex`** — `line_span_full(n)` /
   `line_terminator(n)` so byte-faithful consumers stop hand-slicing
   (lispexp-feedback/0001). Low, additive.

4. **(Question, not a request) The indenter itself.** lisplens ported
   `calculate-lisp-indent` / `lisp-indent-function` / `lisp-indent-specform`
   wholesale (~600 lines). It's the biggest single body of duplicated
   Emacs-semantics, but it is *rendering*, squarely outside lispexp's stated
   reader-only scope, so we are **not** asking for it — only flagging that if
   lispexp ever grows a formatting layer, this is the prize, and #1 (the spec
   data) is the natural first half of that split.

Explicitly **not** delegation candidates (keep in lisplens): Nameless emulation
(editor-specific), EditorConfig resolution, the reindent algorithm, and the CLI /
MCP / patch-DSL surface.

## Prior point asks — status

- **0001** (`LineIndex` byte API vs normalized `line_range`): open, low, additive.
- **0002** (improper-list dot span): **shipped in 0.5.0** and consumed; the
  companion comment-span item resolved via the `lex` layer (no upstream change).

## One-line summary

lispexp's read/annotate/harvest surface is excellent and carried lisplens's whole
read side; the one place lisplens meaningfully *re-implemented lispexp-shaped
knowledge* was the **bundled Emacs indent-spec table** (plus the file-local /
dir-local parsers) — now shipped in the companion **`lispexp-emacs`** crate
(`indent::bundled_table`, `local_vars`, `dir_locals`, lispexp ADR-0033) and
consumed by lisplens, so the next consumer no longer harvests Emacs by hand.
