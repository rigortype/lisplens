# Structural diff — a read-only observation of how two versions differ

lisplens gains a `diff` command (and mirrored MCP tool): a **Structural diff**
(see `CONTEXT.md`) that compares two versions of a file — or two forms — *modulo
formatting* and reports how the second differs from the first. Its north star is
letting an AI agent read **how a unit's logic changed** between two versions and
focus attention there, not producing a textual patch. The motivating use case is
observing how a definition evolved across releases (e.g. `cc-engine.el` from
emacs-30 to emacs-31). This ADR records the diff *model*; the tree-diff
*algorithm* it drills with is ADR-0048.

Structural diff is the **inverse orientation of a Patch** (ADR-0021): a Patch
*applies* edits to produce a new version; a Structural diff *observes* the
difference between two given versions and never writes. This is why the glossary
keeps `diff` off Patch's vocabulary — they are different things, now
deliberately reconciled by the boundary note on the `Structural diff` term.

## The comparable unit

The unit an agent's attention map is keyed on is a **top-level definition** (an
`outline` entry at depth 0 that has a name). Deliberate scope choices:

- **Nested definitions are not separate units.** A change inside an inner
  `defun`/`cl-flet`/`lambda` surfaces as part of its *enclosing* definition's
  change (the ADR-0048 tree diff), not as its own attention-map row —
  double-counting it would blur where to look.
- **Top-level non-definition forms** (`(require …)`, `(provide …)`,
  `(add-hook …)`) have no stable name key, so they are not diffed individually;
  the map reports only whether "other top-level forms" changed, as one summary
  line. In Emacs Lisp such declarative top-level forms are rare beyond `require`,
  so the loss is small and the alternative (diffing anonymous side-effecting
  forms positionally) is noisy.

## Matching key

Units are matched across versions by **`(kind, name, dispatch-signature?)`**.
Plain definitions carry no dispatch signature, so this collapses to the
`(kind, name)` pair `disappeared_definitions` already uses. Method-like forms
(`cl-defmethod`, Clojure `defmethod`, …) add their **Dispatch signature** (the
existing glossary term / form-annotator handle) as a third component, so two
methods of the same generic are distinct units and pair up correctly rather than
collapsing into one ambiguous "it changed". The key is fixed from the start —
even though the first target corpus is defun-centric — because changing it later
would silently change diff identity, an expensive reversal.

**No rename detection.** A renamed definition surfaces as one removed + one
added unit; output groups Added / Removed / Changed so a reader can eyeball a
remove+add pair as a likely rename. Heuristic body-similarity matching (possibly
aided by a *textual*-diff signal, a promising future combination) is kept out of
the core: its threshold uncertainty does not belong in the foundation, and the
north star is the *content* change of a matched unit, not rename tracking.

**A key can repeat within one file.** Emacs code routinely carries a
`(defvar x)` forward declaration and a later `(defvar x nil)` — same
`(kind, name)` key, two genuinely distinct forms. Matching keeps a *list* per key
(not one instance) and pairs within it by consuming `struct_eq`-equal instances
first (unchanged), then pairing the remainder positionally (changed) with the tail
as added/removed. Keeping a single instance per key mispairs the duplicates and
falsely reports a change even when a file is diffed against itself — the empty
self-diff is the invariant this protects.

## Changed predicate

A unit is **changed** iff it is present in both versions and **not
`struct_eq`** — Structural equality (formatting-modulo). Reindentation- or
comment-only churn therefore never registers as changed, matching the
formatting-insensitive premise. The ADR-0008 anchor hash is verbatim (it hashes
the raw span, whitespace included) so it may serve only as a *fast path* — equal
hashes prove unchanged and skip `struct_eq`; differing hashes fall through to
`struct_eq` for the real decision. Using the anchor hash alone would wrongly flag
whitespace edits.

## Surface & exit code

`diff <old> <new>` prints a compact text map (unchanged units omitted, grouped
Added/Removed/Changed) and `--json` the same classification for agents; the MCP
`diff` tool mirrors it. `--deep` / `--unit NAME` drill into changed units via
ADR-0048. Dialect resolves as for other single-file commands (`--dialect`
honored); both sides are assumed one dialect.

**Exit code is 0 whether or not there are differences** — differences are normal
output, read from text/JSON (identical-modulo-formatting files produce empty
output). Non-zero is reserved for real errors (missing file, parse failure,
dialect mismatch), consistent with lisplens's own `check` convention rather than
`diff(1)`'s "1 = differences". A future opt-in `--exit-code` flag could offer
`diff(1)` semantics if a CI need appears.

## Status

accepted

The definition-level model here shipped as the `diff` command / MCP tool
(`src/diff.rs`), verified on `cc-engine.el` emacs-30 → emacs-31. Drilling a
changed unit's internals (`--deep` / `--unit`) is the ADR-0048 tree diff, a
later slice.
