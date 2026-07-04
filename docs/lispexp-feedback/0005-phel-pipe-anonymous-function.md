# Feedback: Phel's `|(…)` anonymous-function reader macro is not recognized — RESOLVED (lispexp 0.7.0)

**Shipped in lispexp 0.7.0** as `Options.pipe_anon_fn`, set for the Phel preset. `|(…)` now reads as one short-anonymous-function form (the `HashFn` prefix, like Clojure's `#(…)`), so a form's argument count is correct for symbol-accurate passes; `#(…)` keeps working and a bare `|` stays an ordinary symbol constituent. lisplens consumes it via `Options::for_dialect(Dialect::Phel)`; formatting a `(map |(inc $) x)` is byte-exact vs `phel format`. Regression golden in `src/format/clojure.rs`. (Note: newer Phel deprecates `|(…)` in favour of `#(…)`, but both still read.)

Upstream feedback to [lispexp](https://crates.io/crates/lispexp) from lisplens. Severity: **medium** — it does not (currently) break the formatter's indentation, but it mis-structures a *common* Phel form, which is wrong for any **symbol-accurate** operation (rename, refs, extract) that counts a form's arguments. Found while validating lisplens' Phel indenter against `phel format`.

## Confirmed facts (lispexp 0.6, `Dialect::Phel`)

Phel's short anonymous function is `|(…)` — e.g. `(map |(+ $ 1) xs)`, with `$` / `$1` / `$2` as the implicit parameters. Phel reads `|(+ $ 1)` as **one** form (a `|`-prefixed list), exactly as Clojure reads `#(+ % 1)`.

lispexp's Phel reader instead reads `|` as a bare symbol and the list as a separate datum. Parsing `(map |(+ $ 1) xs)` gives a list of **four** items:

```
Symbol("map")   Symbol("|")   List[Symbol("+") Symbol("$") Number("1")]   Symbol("xs")
```

where Phel (and a correct reader) yield **three**: `map`, the anonymous function `|(+ $ 1)`, and `xs`.

The cause: `Options::phel()` is `Options::clojure()`, and Clojure's short-function syntax is `#(…)` (`roles.short_fn` is unset — Clojure uses the `#(` path), so `|` is left as an ordinary atom character. `|` is not an atom terminator, so `|(+ $ 1)` tokenises as the atom `|` up to the `(`, then a plain list.

Why it slipped past the formatter corpus: the mis-parse puts a symbol `|` exactly where the anonymous function's column would be, so *argument-alignment* happens to land in the same place — lisplens is byte-exact with `phel format` on 307/310 of phel-lang's own files despite this. But the **tree is wrong** (an extra `|` sibling, one fewer real argument), which is what a rename/refs/extract pass would act on.

## What would resolve it (proposed)

lispexp already has the mechanism and a precedent: the lexer maps `roles.short_fn` to a `Prefix::HashFn`, and **Janet** sets `short_fn: Some('|')`. Phel needs the same — set its char role's `short_fn` to `'|'`:

```rust
pub fn phel() -> Self {
    Options {
        line_comment_in_atom: true, // (feedback 0004)
        roles: CharRoles { short_fn: Some('|'), ..CharRoles::clojure() },
        ..Options::clojure()
    }
}
```

Then `|(+ $ 1)` reads as `Prefixed { prefix: HashFn, inner: (+ $ 1) }`, matching Phel. (Open question for the maintainer: whether Phel *also* accepts Clojure's `#(…)` — a doc example under `docs/examples/` uses `#(…)` — or whether `#(` should be dropped from the Phel preset; the `|(…)` gap is the clear one.)

## lisplens's stance

Not blocking the formatter, but it makes Phel's tree structurally wrong for symbol-accurate edits, so it is worth fixing before lisplens exposes rename/refs/extract on `.phel`. Recorded here per the AGENTS ground rule (lisplens records upstream needs; it does not PR lispexp).
