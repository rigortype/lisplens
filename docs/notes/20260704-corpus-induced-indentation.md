# Corpus-induced indentation — recovering a formatter for oracle-less dialects

Status: **design note / proposal** (exploratory). A follow-up **ADR-0042** records the
decision once the reverse-validation step below confirms the method works. First target:
**EISL (Easy-ISLisp)** — <https://github.com/sasagawa888/eisl>.

## Problem

The native formatter dispatches by dialect to a faithful engine, chosen to match the
tool that dialect's community actually runs (ADR-0031/0039/0041): Emacs Lisp / Common
Lisp / Scheme → Emacs; Clojure → cljfmt; Phel → `phel format`. Dialects Emacs bundles no
indenter for and that have no canonical formatter — Fennel, Janet, Hy, LFE, ISLisp,
AutoLISP — currently ride the **generic Emacs Lisp fallback**, which is a guess: it
imposes Emacs's `lisp-indent-function` model on a language whose community may indent
nothing like Emacs.

We cannot port a reference formatter these dialects do not have. Can we **recover one
empirically** instead?

## Idea

Our engines are already **parameterized by a small, closed set of knobs** (ADR-0039/0041):

- an **alignment mode** for a symbol-headed call — align continuations under arg 0, or
  flat `open + body` (the Tonsky/fixed variant, ADR-0040);
- a **body-indent width** (usually 2); and
- a **rule table** `head → {Block N | Inner D [idx]}` — the only per-symbol data.

Everything else is fixed structure (collections align under their first element, etc.).

A language's own **source + test suite is hand-aligned by its authors** — a de-facto
house style even where no formatter is published. So: parse that corpus, observe how each
line is actually indented relative to its structural context, and **fit the knobs above**
to reproduce the observed indentation. The corpus is simultaneously the **training data**
and the **validation oracle** (a closed loop — with a train/test split to keep it honest).

The payoff is that the learner's **output is exactly the artifact we hand-bundle today**
— a `rules_for`-shaped table plus the two mode flags. It plugs into the existing
`Engine::Clojure`-style engine with **no new runtime code**; only the *source* of the
table changes (induced from a corpus, rather than harvested from Emacs or bundled from
cljfmt's edn). This is the third supply route for the same metadata (see the
metadata-ization discussion in `20260704-clojure-indentation-survey.md`'s neighborhood).

## Method (sketch)

For every multi-line form `(HEAD arg0 arg1 …)` in the corpus whose body wraps, measure
each continuation line's column against the open column and the arg-0 column, and turn it
into evidence for HEAD:

- all args align under arg 0 → **default** (no rule);
- N args on the head line, then body at `open + w` → **`:block N`**;
- everything at `open + w` → **`:inner 0`**; nested `+w` → **`:inner D`**.

Aggregate per head symbol; take the consensus. Estimate the **global** knobs (body width
`w` as the mode of the per-form deltas; alignment mode from whether symbol-headed calls
generally align under arg 0 or at `open + w`). **Regularize toward `default`**: emit a
rule only above a minimum support count and consistency fraction; everything sparse or
noisy stays default (safe — it just aligns under arg 0).

This reuses the engine's own introspection **in reverse**: the same `container_at` / `Cols`
/ parse-tree machinery that *computes* an indent is what *measures* the observed one.

### Confounds to respect (this is estimation, not inversion)

Hand code is noisy, so robust statistics are mandatory. The identifiability traps:

- **arg-0-width degeneracy.** "Align under arg 0's column" and "`open + fixed offset`"
  coincide when arg 0 is one glyph wide. Disambiguating needs occurrences with a **wide
  arg 0**. (Display-width columns matter here — we already measure them, ADR-0031.)
- **`:block N` needs variety.** N is only determined by forms that wrap the body *and*
  carry ≥ 2 args; without them `:block 1` and default are indistinguishable.
- **The circular trap.** If the corpus was itself formatted by Emacs (many Lispers use
  Emacs!), induction just re-learns Emacs badly — better to use the Emacs engine directly.
  The method earns its keep only where the corpus was **not** Emacs-formatted.

## Validation-first: reverse-validate on a dialect that *has* an oracle

Before trusting the learner on an oracle-less dialect, run it where we already know the
answer — **Clojure** (or Emacs Lisp):

1. Induce a table from the Clojure corpus.
2. Compare the induced table to the **known** `cljfmt indents/clojure.clj`, and reformat
   the corpus with the induced model and diff against **real cljfmt**.

If induction recovers cljfmt's table at high fidelity, the method is **falsified-or-not**
with a concrete number, before it ever touches a dialect we cannot check. This is the
gate; do not productize an induced engine that has not passed it.

## Why EISL is the ideal first oracle-less target

EISL (Easy-ISLisp, an ISLisp implementation in C) is unusually well-suited:

- **A real, sizable, self-consistent corpus.** 185 `.lsp` files across `example/` (53),
  `library/` (45), `verify/` (42), `tests/` (22), `bench/` (22). Crucially, EISL ships its
  **own editor, `edlis`**, which auto-indents (`edlis.c`: `ed_indent`, `calc_tabs()`,
  `ed_lparen_col`/`ed_rparen_col`). A corpus normalized by one editor is **low-noise** —
  close to the deterministic-oracle ideal, unlike a multi-editor grab-bag.
- **A genuinely distinctive house style** — not Emacs, not cljfmt. Observed in
  `library/logger.lsp`: a `defun` body indented `+3`, and `let` bodies aligned **under the
  binding vector** (arg 0), with a corpus-wide first-body-line width histogram mixing 4
  (724×), 2 (476×), and 3 (364×). No off-the-shelf engine produces this, so induction is
  genuinely additive — and because the corpus was formatted by `edlis`, **not** Emacs, the
  circular trap does not apply.
- **A bonus answer key.** EISL is oracle-less *by our productization definition* (no
  standalone formatter, no Emacs mode to port) yet carries a ground truth: `edlis`'s
  `calc_tabs()` **is** the indentation algorithm. So the PoC can both **induce** the table
  from the corpus **and cross-check** it against `calc_tabs` (read the C, or run `edlis`
  on de-indented input). A rare chance to validate an induced oracle-less model directly.
- **The parse side is ready.** lispexp already has `Dialect::Islisp` (annotation profile,
  builtins, detection), so reading EISL is a matter of routing, not new reader work.

## Prerequisites / dependencies

1. **ISLisp routing for `.lsp`.** `dialect_for_path` (`src/lib.rs`) maps `.lsp` →
   `Dialect::CommonLisp`; EISL uses `.lsp` for ISLisp. `.lsp` is genuinely ambiguous
   (CL / AutoLISP / ISLisp), so this wants content detection (lispexp's `detect`) or an
   explicit `--dialect` / dir-local override — **not** a blind remap. Precondition for the
   experiment, orthogonal to the induction itself.
2. **A knob-exposed engine.** The induced engine can reuse the `Engine::Clojure` shape
   (per-dialect `rules_for` + alignment flag + body width). ISLisp's `(the <type> …)`
   declaration lines and `let`-under-binding suggest its style may sit closer to the
   Emacs `calculate-lisp-indent` family than to cljfmt — an open question the induction
   answers: **the learner can target whichever substrate fits better**, since both are
   parameterized the same way at the level the learner sets.

## Scope and honesty

This yields a **descriptive** formatter (reproduces how this community writes) rather than
a **prescriptive** one (enforces a published spec) — the right stance when there is no
spec. It recovers only the **regular** part (alignment mode, body width, well-attested
`:block`/`:inner` heads); irregular/procedural constructs are unlearnable and safely
default. Always **report coverage** ("N heads learned at ≥ K support, ≥ P% consistent;
the rest default") — never present an induced table as complete when it is not.

## Plan

0. **ISLisp routing** — content-detect `.lsp` (or a `--dialect islisp` / dir-local) so
   EISL reads as ISLisp, not CL.
1. **Reverse-validate** — build the learner; induce on the Clojure/Elisp corpus; measure
   how well it recovers the known table and reformats vs the real oracle. Gate.
2. **Induce EISL** — run the learner on the EISL corpus; **cross-check against `edlis`**
   (`calc_tabs` / running the editor); report fidelity + coverage.
3. **Ship** — if the numbers hold, add an ISLisp engine as an *induced table* on the
   shared engine, and record the decision in **ADR-0042**. Then repeat for
   Fennel/Janet/Hy/LFE, each against its own de-facto corpus.

Deferred until the gate passes: any runtime/CLI surface. The learner is offline tooling
(it *produces* a bundled table), not a per-format-call cost.

## Progress

- **Step 0 done** — a `--dialect NAME` override lands on single-file commands (PR #13), so
  EISL `.lsp` reads as ISLisp (`--dialect islisp`). Confirmed content detection cannot
  route to ISLisp (lispexp registers no signal), so an explicit override is the honest
  route. Note: 5/185 EISL files use an EISL reader extension (`|>` pipe) that fails to
  parse under *both* CL and ISLisp — an EISL-specific construct, not a routing issue;
  exclude them from the corpus (or file lispexp feedback later).

- **Step 1 PASSED — the reverse-validation gate is green.** A learner (PoC in the
  scratchpad) induced, per head symbol, the binary "body-indents (`open+2`) vs
  aligns-under-arg 0" from **851 Clojure files** (reitit/ring/hiccup/malli/integrant,
  26 017 measured multi-line occurrences), then compared to cljfmt's known table
  (`rules_for`). Method: for each multi-line round list with a symbol head, vote `body`
  if the first own-line child sits at `open+2`, `align` if at `open+1` (head-alone) or
  under arg 0's column; aggregate per head at support ≥ 4, consistency ≥ 0.60.

  | | precision | recall | F1 | accuracy |
  | --- | --- | --- | --- | --- |
  | naïve | 0.49 | **0.97** | 0.65 | 0.75 |
  | + `:inner`-inheritance fix | **0.76** | **0.97** | **0.85** | **0.91** |

  **Recall is the clean win (0.97):** of the 62 heads cljfmt body-indents, the learner
  recovered 60 from the corpus alone. The naïve precision gap was almost entirely one
  **methodological leak**: method/impl forms nested inside `reify`/`deftype`/`defrecord`/
  protocol body-indent because of the *ancestor's* `:inner` rule, and the learner
  mis-attributed that to the method head. Suppressing measurement of forms directly under
  an `:inner`-family head (a real, general fix) cut false positives 63 → 19 with no recall
  loss. Two further findings, both showing the *learner* right where the reference is
  incomplete/loose:
  - Most residual "false positives" (`time`, `async`, `bench`, `for-all`, `profile`,
    `quick-bench`, `conform`, project fns) are **genuine project-local body-indenting
    macros** cljfmt's built-in table doesn't list — so true precision against a complete
    ground truth is *higher* than 0.76.
  - The two "false negatives" are `do` (a `:block 0` that legitimately degrades to
    align-under-arg 0 when its first form stays on the head line — an identifiability
    nuance the binary can't capture) and `with-meta` (a function that cljfmt's fuzzy
    `with-*` regex over-matches — the learner's `align` is the correct call).

  **Verdict:** induction recovers the binary body/align table at F1 ≈ 0.85 (understated),
  cleanly separates rule-bearing heads from function calls, and even surfaces gaps and
  over-matches in the static reference. The core method is validated. Deferred to Step 2:
  the finer `:block N` vs `:inner D` layer (needs occurrences where a *special* arg wraps
  — the head-line-child histograms hint at N but conflate inner/block), and porting the
  learner from the scratchpad PoC into repo tooling once EISL induction begins.
