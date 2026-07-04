# Feedback: `#_`-discarded forms are dropped from the tree, so a formatter can't indent inside them — RESOLVED (lispexp 0.7.0)

**Shipped in lispexp 0.7.0** as the opt-in `Options.keep_discarded` (default `false`), which keeps a discarded form as `DatumKind::Prefixed { prefix: Prefix::Discard, inner }` exactly as proposed below. lisplens's formatter sets it (`src/format/mod.rs`); `container_at` descends the kept node, so lines inside a multi-line discard indent against it. **Downstream re-validation done:** a kept discard *counts* as a value child for the Clojure `:inner`/`:block` model — matching cljfmt, which walks every node (a `#_` in a body slot degrades the block to default alignment just as a real form would). Confirmed byte-exact vs `cljfmt fix` on the reitit/ring/hiccup/malli/integrant corpora (373 files, **zero** code-indent divergences; the six former `#_`-residual files now match). Regression goldens in `src/format/clojure.rs`.

Upstream feedback to [lispexp](https://crates.io/crates/lispexp) from lisplens, a **verbatim / round-trip** consumer (it reindents by structural position and edits by splicing source spans). Severity: **low** (rare, and only affects discarded/dead code), but it is the one remaining source of indentation divergence vs cljfmt on real Clojure corpora, so it is worth a note.

## Confirmed facts (lispexp 0.6)

The reader (Clojure dialect) **omits a `#_`-discarded form from its parent's `items`** entirely — the datum is not represented anywhere in the tree, only the surviving siblings are. For

```clojure
[a
 #_["/spec" {:c d}
    ["/x" {:p 1}]]
 b]
```

the outer vector's `items` are `[Symbol(a) @1..2, Symbol(b) @29..30]` — the discarded `#_[…]` (spanning bytes 3..27) is gone. Nothing marks that a value-bearing region sits between `a` and `b`.

This is *semantically* correct — `#_` means "discard the next form" — and is the right default for an evaluator or a structural query. It matches how the reader also drops `#_#_ x y` (two-form discard).

## The sharp edge (for a formatter / round-trip consumer)

lisplens' formatter indents each line by finding its innermost containing collection (`container_at`) and asking the dialect engine for a column. Because the discarded form is absent from `items`:

- `container_at(offset)` for a line *inside* `#_[…]` cannot find the discarded vector — it returns the **outer** container instead, so the discarded form's continuation lines indent against the wrong parent (flat, one level too shallow).
- cljfmt (built on rewrite-clj, which keeps every node — discards included — as `uneval` nodes) indents inside discarded forms normally, so lisplens diverges there.

On the reitit + ring + hiccup corpora (272 files) this is the **only** remaining class of code-line indentation divergence: 4 files, all `#_`-discard, once the metadata-`arg` descent bugs were fixed on the lisplens side. Everything else is byte-exact vs cljfmt.

## What would resolve it (proposed, non-breaking)

Add an opt-in parse option — `Options.keep_discarded: bool`, default `false` — that keeps a discarded form in the tree **as a `DatumKind::Prefixed { prefix: Prefix::Discard, inner }`** instead of dropping it. This reuses the existing prefix machinery the reader already builds for `'`, `` ` ``, `~`, `^`, … (`Discard` is the one prefix currently intercepted and dropped), so it needs **no new `DatumKind` variant** — the enum stays exhaustive — and consumers that descend into `Prefixed` (like lisplens' `container_at`) pick it up unchanged. Nested discards (`#_#_ a b`) then nest as `Prefixed { Discard, Prefixed { Discard, a } }`.

Off by default keeps the current semantics for evaluators / structural queries; a round-trip consumer sets the flag. Since `Options` is `#[non_exhaustive]`, a caller sets it by mutating the field (`opts.keep_discarded = true`), not struct-update. The maintainer implements this upstream; lisplens will then set the flag and consume the published version (per the AGENTS ground rule — lisplens does not PR lispexp).

Downstream note: once discarded forms are in `items`, they occupy a value-index slot — a formatter's arg-index counting must then treat a `Prefix::Discard` node the way the reference formatter (cljfmt) does, so the lisplens consumption should re-validate indexing against cljfmt on `#_`-bearing forms.

## lisplens's stance

Not blocking — lisplens documents it as a known formatter limitation (`docs/dev/formatter.md`, alongside comment-only-line indentation) and leaves discarded regions indented against their enclosing form. If lispexp later preserves discarded forms, the Clojure engine (and the shared `container_at`/`in_string`) would pick them up with no rule changes, since they are just more collections in the tree.
