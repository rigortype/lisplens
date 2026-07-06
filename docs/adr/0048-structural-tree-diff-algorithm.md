# Structural tree-diff algorithm — anchored LCS, category-gated recursion

The engine behind a Structural diff (ADR-0047) is `structural_diff(old, new) ->
DiffTree`: given two `Datum`s it produces a recursive diff that shows *how* the
logic changed, not merely *that* it did. Every application — whole-file `--deep`,
single-`--unit`, the MCP two-form comparison — routes through this one function.
The design goal is to surface the exact subforms that changed while eliding
everything that did not, **deterministically and without similarity
thresholds**. This ADR records the two decisions that carry that weight — child
alignment and the recursion gate — and the non-goals they imply.

## Child alignment: anchored LCS + positional gap pairing

To align a parent's child sequence (a body, a `let` binding list, `cond`
clauses):

1. Take the **LCS of the children under `struct_eq`** (Structural equality). The
   exactly-equal children become fixed *unchanged anchors*. This is what makes
   the diff robust to insertion and removal — an inserted form does not shift
   every following form into "changed".
2. In each divergent **gap** between anchors, pair the old and new children
   **positionally** (`old[i] ↔ new[i]`) and recurse into each pair; any count
   difference is pure **added / removed**.

Two rejected alternatives and why:

- **Positional alignment alone** (zip by index) — a single insertion at the head
  cascades into "everything changed". Rejected.
- **Exact-`struct_eq` LCS with delete+insert in the gaps** (classic line-diff
  shape) — a child that changed *internally* is not LCS-equal, so it would fall
  into a gap as a delete+insert and we would never recurse into it to show the
  internal change. The positional gap pairing is precisely the fix: the lone
  `old[i]`/`new[i]` in a gap get recursed, revealing the sub-change.

No similarity metric, no threshold, fully deterministic.

## Recursion gate: structural category, not similarity

When a positionally-paired `old[i]`/`new[i]` differ, recurse only when they are
the **same structural category**; otherwise stop and emit an opaque replace:

- **Same-delimiter list ↔ same-delimiter list** → recurse. The **head is just
  child 0** — so a changed head (`when` → `unless`) surfaces naturally as a
  `replaced` on child 0, which is exactly what an agent wants to see. This is a
  deliberate divergence from `anti_unify` (ADR-0038), which *never* generalizes
  an operator: anti-unification is a safety-bounded *transform*, whereas diff is
  *observation*, and showing a head change is desirable, not dangerous.
- **Same-notation Prefixed ↔ Prefixed** (`'x`↔`'y`, metadata, unquote…) →
  recurse into `inner` (and `arg`).
- **Everything else** — differing leaves (`1`↔`2`, `foo`↔`bar`), list↔atom,
  **delimiter mismatch** (`(…)`↔`[…]`↔`{…}`), notation mismatch → **`replaced`**,
  carrying both old and new forms (truncated if huge).

A similarity-threshold gate was rejected on purpose: it would reintroduce the
heuristic dial the alignment step was designed to avoid, and trade a predictable
rule for a tunable one. The accepted cost is that two unrelated same-delimiter
lists positionally paired inside a gap will be aligned child-by-child and read a
little noisily — bounded to that one pair, and judged preferable to a threshold.

## Output model

Four node statuses — **`added` / `removed` / `changed`** (a recursed list; has
`children`) **/ `replaced`** (opaque; has `old`/`new`). `unchanged` subtrees are
not emitted. Two renderings from the one tree:

- **Text** — a *pruned structural tree* (`+`/`-`/`~`): descend only the spine of
  paths that contain a change, collapse unchanged siblings to `…`, mark changes
  inline (short leaf replaces as `old ⇒ new`, multi-line as `-`/`+` pairs). It
  keeps the `outline`/`expand` tree idiom, so *where* in the form the change sits
  is legible without a separate path grammar. The one-shot visual gestalt is left
  to the HTML view (issue #42).
- **`--json`** — the recursive DiffTree. Forms are **verbatim source-text
  fragments + a minimal kind tag**, *not* a re-serialized AST (this is a diff
  representation, not a reader re-encoding). Each `added`/`changed`/`replaced`
  node carries an **editing anchor (`line:hash`) + line** from the new version
  (removed nodes from the old), so an agent goes straight from "here is the
  change" to editing it — anchors are lisplens's shared currency. Unchanged
  children are omitted, but each emitted node keeps its child **index** so a
  renderer can place it and show elision.

## Deep view of added / removed definitions (#44)

`diff --deep` / `--unit` only *tree-diffs* **changed** definitions — an added or
removed definition has no counterpart to align against. But naming them alone
left the biggest coverage gap: a controlled experiment (raw `git diff` vs this
structural diff, summarizing `cc-engine.el` emacs-30 → emacs-31 across three
models) found the structural diff lost coverage almost entirely on the
*body-internal* detail of **added** definitions the raw diff exposes. So deep
mode also renders each added/removed definition as its expandable **Lens**
(ADR-0013) — the `expand` view: the form's subtree in pre-order, one node per
line with an anchor + preview. This is deliberately the Lens, **not** verbatim
source: previews keep it token-bounded (the whole point of the structural diff
over a raw diff), and every node is a real edit anchor an agent can drill.
Full-verbatim added bodies stay a possible `--verbose` opt-in, not the default.
`diff_files_deep` returns a `DeepDiff { changed, added, removed }`; JSON tags
each entry `changed`/`added`/`removed` and carries the Lens nodes on the latter
two.

## HTML view (#42)

A third renderer, `--html` (`diff_html` / `deep_html` / `form_diff_html`), turns
the *same* `FileDiff` / `DeepDiff` / `FormDiff` structures into a self-contained
HTML page for a human to open — no re-parsing, no new diff logic, just a view.
Inline styles, no external assets (so it works offline and as a handed-off file);
added/removed/replaced marked by colour, unchanged siblings elided to `…`. Every
fragment is HTML-escaped — non-negotiable, since Lisp source is full of `<`/`>`/`&`
(`c-forward-<>-arglist`, `&optional`). It is orthogonal to `--deep`/`--unit` and
mutually exclusive with `--json`.

## Non-goals (documented, tested)

- **Moves / reorders / tree-edit-distance are out of scope.** A reordered subform
  surfaces as change+change or add+remove; this is a *known, tested* outcome, not
  a bug — a regression test pins it so the limitation stays visible.
- **Cross-file anchor-to-anchor addressing is deferred.** `--unit` covers the
  realistic case and the MCP two-form-string path covers the general one; naming
  a form as `FILE:line:hash` on the CLI is ambiguous (the anchor already contains
  `:`) and low-demand.

## Status

accepted

Shipped in `src/diff.rs`: `diff_forms` (the engine), `diff_files_deep`
(`--deep` / `--unit`), and `diff_source_forms` (the general two-form primitive,
also the MCP form-string path), with the pruned-tree text renderer and the JSON
DiffTree. The Prefixed case recurses into `inner` only when the auxiliary `arg`
is equal (a differing prefix arg replaces the whole form) — a small pragmatic
narrowing of "recurse into inner (+ arg)". Verified end-to-end on `cc-engine.el`
emacs-30 → emacs-31 (e.g. `--unit c-foreign-truncate-lit-pos-cache` shows the
inner `c-truncate-lit-pos-cache` → `c-truncate-lit-pos/state-cache` rename).
