# Feedback: `#_`-discarded forms are dropped from the tree, so a formatter can't indent inside them

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

## What would resolve it

Expose discarded forms structurally rather than dropping them — e.g. a `DatumKind::Discarded { inner }` (or keep them in `items` behind an `is_discarded` flag / a `Prefix::Discard` that survives), so a round-trip consumer can still see the span and shape. A parse option (`keep_discarded: bool`) would avoid changing the default semantics for evaluators.

## lisplens's stance

Not blocking — lisplens documents it as a known formatter limitation (`docs/dev/formatter.md`, alongside comment-only-line indentation) and leaves discarded regions indented against their enclosing form. If lispexp later preserves discarded forms, the Clojure engine (and the shared `container_at`/`in_string`) would pick them up with no rule changes, since they are just more collections in the tree.
